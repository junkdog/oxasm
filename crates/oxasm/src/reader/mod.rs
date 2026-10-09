//! [`ClassReader`]: ASM's `ClassReader`, a parser that makes a [`ClassVisitor`] visit a class
//! file.
//!
//! The port is structural (ADR-01): the same passes over the class file, in the same order, with
//! the same leniencies. `ClassReader.java` is split along its own sections:
//!
//! - this module: constructors, accessors and `accept` (the class level);
//! - `constant_pool`: the low level parsing methods (`readUTF8`, `readConst`, ...);
//! - `members`: modules, record components, fields and methods;
//! - `code`: the Code attribute;
//! - `annotations`: annotations, type annotations and parameter annotations;
//! - `frames`: stack map frames;
//! - `context`: ASM's `Context`.
//!
//! # Errors
//!
//! ASM throws `IllegalArgumentException` on the malformed structures it detects, and fails with
//! an `ArrayIndexOutOfBoundsException` (or a `NullPointerException`, ...) on the others. Every
//! one of those is a [`ReadError`] here, returned at the same point of the visit: offsets are
//! signed and only checked when ASM reads at them, so that the reader accepts and rejects
//! exactly the class files ASM does. The deliberate exceptions (ADR-06, ADR-09) are:
//!
//! - nesting deeper than ASM 9.11's bounds (256 levels of annotations, 127 of `ConstantDynamic`,
//!   4096 `ConstantDynamic` per constant), where ASM 9.10.1 would overflow its stack;
//! - a method handle whose reference kind is not 1 to 9;
//! - a `null` string (constant pool index 0) where a visitor method takes a non nullable string,
//!   e.g. a method name: ASM passes `null` on;
//! - an annotation element value of a primitive type whose constant is a method handle or a
//!   `ConstantDynamic` (ASM passes it on as is);
//! - a non standard attribute with a negative `attribute_length` for which a codec is registered
//!   (ASM calls the prototype's `read` with it; without a codec, both fail).

#![allow(clippy::cast_possible_truncation)] // Java's narrowing casts are the specification here.

mod annotations;
mod code;
mod constant_pool;
mod context;
mod frames;
mod members;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::attribute::{Attribute, AttributeCodec, CustomAttribute, UnknownAttribute};
use crate::constant_dynamic::ConstantDynamic;
use crate::constants;
use crate::jstr::{JavaStr, JavaString};
use crate::label::Label;
use crate::opcodes;
use crate::visitor::ClassVisitor;

use context::Context;

pub use crate::error::{ReadError, ReadErrorKind};

bitflags::bitflags! {
    /// The options of [`ClassReader::accept`] (ASM's `ClassReader.SKIP_CODE`, ...).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct ParsingOptions: u32 {
        /// Skip the method code. If set, the code attributes are neither parsed nor visited.
        const SKIP_CODE = 1;
        /// Skip the `SourceFile`, `SourceDebugExtension`, `LocalVariableTable`,
        /// `LocalVariableTypeTable`, `LineNumberTable` and `MethodParameters` attributes. If set,
        /// these attributes are neither parsed nor visited (i.e. `visit_source`,
        /// `visit_local_variable`, `visit_line_number` and `visit_parameter` are not called).
        const SKIP_DEBUG = 2;
        /// Skip the stack map frames in the method code. If set, these frames are neither parsed
        /// nor visited (i.e. `visit_frame` is not called). This is useful when the class writer
        /// computes the frames.
        const SKIP_FRAMES = 4;
        /// Expand the stack map frames. By default stack map frames are visited in their original
        /// format (i.e. "expanded" for classes whose version is less than V1_6, and "compressed"
        /// for the other classes). If set, stack map frames are always visited in expanded
        /// format (`FrameKind::New`). This option adds a decompression/compression step in a
        /// reader → writer chain, but it allows a method adapter to insert frames.
        const EXPAND_FRAMES = 8;
        /// Expand the ASM specific instructions into an equivalent sequence of standard bytecode
        /// instructions. When resolving a forward jump it may happen that the signed 2 bytes
        /// offset reserved for it is not sufficient to store the bytecode offset. In this case
        /// the jump instruction is replaced with a temporary ASM specific instruction using an
        /// unsigned 2 bytes offset. Internal: the class writer uses this option to replace them
        /// with standard instructions.
        #[doc(hidden)]
        const EXPAND_ASM_INSNS = 256;
    }
}

/// The source of the reader ids, which the method copy optimization compares (ADR-08).
static NEXT_READER_ID: AtomicU64 = AtomicU64::new(1);

/// A parser to make a [`ClassVisitor`] visit a `ClassFile` structure, as defined in the Java
/// Virtual Machine Specification (JVMS). This class parses the `ClassFile` content and calls the
/// appropriate visit methods of a given [`ClassVisitor`] for each field, method and bytecode
/// instruction encountered.
///
/// The class file is borrowed, and decoded lazily: the constructor only locates the constant
/// pool entries. Strings borrow from the class file whenever their modified UTF-8 encoding is
/// also valid UTF-8, which is the case of almost all names and descriptors. A reader is
/// `Send + Sync`, and can be accepted by several visitors, from several threads.
///
/// See [JVMS 4](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html).
#[derive(Debug)]
pub struct ClassReader<'b> {
    /// The class file (ASM's `classFileBuffer`).
    class_file: &'b [u8],
    /// The offset in bytes of the `ClassFile`'s access_flags field.
    header: i64,
    /// The offset in bytes, in `class_file`, of each `cp_info` entry of the `ClassFile`'s
    /// `constant_pool` array, *plus one*. In other words, the offset of constant pool entry `i`
    /// is given by `cp_info_offsets[i] - 1`, i.e. its `cp_info`'s tag field is given by
    /// `class_file[cp_info_offsets[i] - 1]`. Entry 0 (and the second slot of long and double
    /// entries) is 0.
    cp_info_offsets: Box<[u32]>,
    /// The `String` objects corresponding to the `CONSTANT_Utf8` constant pool items, decoded on
    /// first use. Each entry is 0 until then, and then holds `UTF8_BORROWED | length << 32 |
    /// offset` for a string borrowed from `class_file` (almost all of them), or `UTF8_DECODED`
    /// for one in `constant_utf8_decoded`. Racing decoders store the same value, so a release
    /// store publishes it, without a compare-and-swap.
    constant_utf8_values: Box<[AtomicU64]>,
    /// The `CONSTANT_Utf8` constant pool items that are not valid UTF-8 as they stand, decoded
    /// on first use. Allocated on first need, as such strings are rare.
    constant_utf8_decoded: OnceLock<Box<[OnceLock<JavaString>]>>,
    /// The `ConstantDynamic` objects corresponding to the `CONSTANT_Dynamic` constant pool items,
    /// decoded on first use, with the number of `ConstantDynamic` in their tree. Empty if the
    /// constant pool has no `CONSTANT_Dynamic` entry.
    constant_dynamic_values: Box<[OnceLock<(ConstantDynamic, u32)>]>,
    /// The start offsets in `class_file` of each element of the `bootstrap_methods` array (in
    /// the `BootstrapMethods` attribute), or `None` if the constant pool has no dynamic entries.
    bootstrap_method_offsets: Option<Box<[u32]>>,
    /// A conservative estimate of the maximum length of the strings contained in the constant
    /// pool of the class (ASM's `maxStringLength`, the size of its `charBuffer`).
    max_string_length: usize,
    /// The identity of this reader, for the method copy optimization (ADR-08).
    #[allow(dead_code)] // compared by the class writer (P5)
    id: u64,
}

impl<'b> ClassReader<'b> {
    /// Constructs a new reader of the given class file.
    ///
    /// # Errors
    ///
    /// If the class file major version is newer than [`V27`](crate::opcodes::V27), if a
    /// constant pool entry has an unknown tag, or if the class file is too short for its
    /// constant pool (ASM's `IllegalArgumentException` and `ArrayIndexOutOfBoundsException`).
    pub fn new(class_file: &'b [u8]) -> Result<ClassReader<'b>, ReadError> {
        ClassReader::new_with_version_check(class_file, true)
    }

    /// The constructor, with or without the class version check (the class writer re-reads
    /// classes without it).
    pub(crate) fn new_with_version_check(
        class_file: &'b [u8],
        check_class_version: bool,
    ) -> Result<ClassReader<'b>, ReadError> {
        // Java arrays have at most 2^31 - 1 elements, which bounds every offset to an int.
        if i32::try_from(class_file.len()).is_err() {
            return Err(ReadError::new(
                0,
                ReadErrorKind::InvalidLength(class_file.len() as i64),
            ));
        }
        let mut reader = ClassReader {
            class_file,
            header: 0,
            cp_info_offsets: Box::default(),
            constant_utf8_values: Box::default(),
            constant_utf8_decoded: OnceLock::new(),
            constant_dynamic_values: Box::default(),
            bootstrap_method_offsets: None,
            max_string_length: 0,
            id: NEXT_READER_ID.fetch_add(1, Ordering::Relaxed),
        };
        // Check the class' major_version. This field is after the magic and minor_version
        // fields, which use 4 and 2 bytes respectively.
        if check_class_version {
            let major_version = reader.i16_at(6)?;
            if i32::from(major_version) > opcodes::V27 as i32 {
                return Err(ReadError::new(
                    6,
                    ReadErrorKind::UnsupportedVersion(major_version),
                ));
            }
        }
        // Create the constant pool arrays. The constant_pool_count field is after the magic,
        // minor_version and major_version fields, which use 4, 2 and 2 bytes respectively.
        let constant_pool_count = usize::from(reader.u16_at(8)?);
        let mut cp_info_offsets = vec![0u32; constant_pool_count];
        // Compute the offset of each constant pool entry, as well as a conservative estimate of
        // the maximum length of the constant pool strings. The first constant pool entry is
        // after the magic, minor_version, major_version and constant_pool_count fields, which use
        // 4, 2, 2 and 2 bytes respectively.
        let mut current_cp_info_index = 1;
        let mut current_cp_info_offset: i64 = 10;
        let mut current_max_string_length = 0;
        let mut has_bootstrap_methods = false;
        let mut has_constant_dynamic = false;
        // The offset of the other entries depend on the total size of all the previous entries.
        while current_cp_info_index < constant_pool_count {
            cp_info_offsets[current_cp_info_index] = (current_cp_info_offset + 1) as u32;
            current_cp_info_index += 1;
            let cp_info_size: i64 = match reader.u8_at(current_cp_info_offset)? {
                CONSTANT_FIELDREF_TAG
                | CONSTANT_METHODREF_TAG
                | CONSTANT_INTERFACE_METHODREF_TAG
                | CONSTANT_INTEGER_TAG
                | CONSTANT_FLOAT_TAG
                | CONSTANT_NAME_AND_TYPE_TAG => 5,
                CONSTANT_DYNAMIC_TAG => {
                    has_bootstrap_methods = true;
                    has_constant_dynamic = true;
                    5
                }
                CONSTANT_INVOKE_DYNAMIC_TAG => {
                    has_bootstrap_methods = true;
                    5
                }
                CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => {
                    current_cp_info_index += 1;
                    9
                }
                CONSTANT_UTF8_TAG => {
                    let size = 3 + i64::from(reader.u16_at(current_cp_info_offset + 1)?);
                    // The size in bytes of this CONSTANT_Utf8 structure provides a conservative
                    // estimate of the length in characters of the corresponding string, and is
                    // much cheaper to compute than this exact length.
                    current_max_string_length = current_max_string_length.max(size as usize);
                    size
                }
                CONSTANT_METHOD_HANDLE_TAG => 4,
                CONSTANT_CLASS_TAG
                | CONSTANT_STRING_TAG
                | CONSTANT_METHOD_TYPE_TAG
                | CONSTANT_PACKAGE_TAG
                | CONSTANT_MODULE_TAG => 3,
                tag => {
                    return Err(ReadError::new(
                        current_cp_info_offset,
                        ReadErrorKind::InvalidConstantPoolTag(tag),
                    ));
                }
            };
            current_cp_info_offset += cp_info_size;
        }
        reader.max_string_length = current_max_string_length;
        // The Classfile's access_flags field is just after the last constant pool entry.
        reader.header = current_cp_info_offset;
        reader.cp_info_offsets = cp_info_offsets.into_boxed_slice();
        reader.constant_utf8_values = (0..constant_pool_count)
            .map(|_| AtomicU64::new(0))
            .collect();

        // Allocate the cache of ConstantDynamic values, if there is at least one.
        if has_constant_dynamic {
            reader.constant_dynamic_values =
                (0..constant_pool_count).map(|_| OnceLock::new()).collect();
        }

        // Read the BootstrapMethods attribute, if any (only get the offset of each method).
        if has_bootstrap_methods {
            reader.bootstrap_method_offsets = Some(reader.read_bootstrap_methods_attribute()?);
        }
        Ok(reader)
    }

    // ---------------------------------------------------------------------------------------------
    // Accessors
    // ---------------------------------------------------------------------------------------------

    /// The class file this reader reads (ASM's `b`).
    #[must_use]
    pub fn class_file(&self) -> &'b [u8] {
        self.class_file
    }

    /// The offset in bytes of the `ClassFile`'s access_flags field (ASM's `header`).
    #[must_use]
    pub fn header(&self) -> usize {
        self.header as usize
    }

    /// The identity of this reader, which the method copy optimization compares (ADR-08).
    #[must_use]
    #[allow(dead_code)] // compared by the class writer (P5)
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Returns the class's access flags (see [`crate::opcodes`]). This value may not reflect
    /// Deprecated and Synthetic flags when bytecode is before 1.5 and those flags are represented
    /// by attributes.
    ///
    /// # Errors
    ///
    /// If the class file is truncated.
    pub fn get_access(&self) -> Result<u32, ReadError> {
        Ok(u32::from(self.u16_at(self.header)?))
    }

    /// Returns the internal name of the class (see
    /// [`Type::get_internal_name`](crate::Type::get_internal_name)).
    ///
    /// # Errors
    ///
    /// If the class file is malformed (including a `this_class` of 0, for which ASM returns
    /// `null`).
    pub fn get_class_name(&self) -> Result<&JavaStr, ReadError> {
        // this_class is just after the access_flags field (using 2 bytes).
        required(self.read_class_at(self.header + 2)?, self.header + 2)
    }

    /// Returns the internal name of the super class (see
    /// [`Type::get_internal_name`](crate::Type::get_internal_name)). For interfaces, the super
    /// class is `java/lang/Object`. `None` for the `java/lang/Object` class (and `module-info`).
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn get_super_name(&self) -> Result<Option<&JavaStr>, ReadError> {
        // super_class is after the access_flags and this_class fields (2 bytes each).
        self.read_class_at(self.header + 4)
    }

    /// Returns the internal names of the implemented interfaces (see
    /// [`Type::get_internal_name`](crate::Type::get_internal_name)).
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn get_interfaces(&self) -> Result<Vec<&JavaStr>, ReadError> {
        // interfaces_count is after the access_flags, this_class and super_class fields (2 bytes
        // each).
        let mut current_offset = self.header + 6;
        let interfaces_count = self.u16_at(current_offset)?;
        let mut interfaces = Vec::with_capacity(usize::from(interfaces_count));
        for _ in 0..interfaces_count {
            current_offset += 2;
            interfaces.push(required(
                self.read_class_at(current_offset)?,
                current_offset,
            )?);
        }
        Ok(interfaces)
    }

    // ---------------------------------------------------------------------------------------------
    // Public methods
    // ---------------------------------------------------------------------------------------------

    /// Makes the given visitor visit the `ClassFile` structure passed to the constructor of this
    /// reader.
    ///
    /// # Errors
    ///
    /// If the class file is malformed. The visitor has then seen a prefix of the events of the
    /// class, as in ASM, which throws in the middle of a visit.
    pub fn accept(
        &self,
        class_visitor: &mut dyn ClassVisitor,
        parsing_options: ParsingOptions,
    ) -> Result<(), ReadError> {
        self.accept_with_codecs(class_visitor, &[], parsing_options)
    }

    /// Makes the given visitor visit the `ClassFile` structure passed to the constructor of this
    /// reader, decoding the non standard attributes the given codecs know (ASM's
    /// `accept(classVisitor, attributePrototypes, parsingOptions)`). Attributes no codec knows
    /// are visited as [`Attribute::Unknown`].
    ///
    /// # Errors
    ///
    /// If the class file is malformed, or a codec fails. The visitor has then seen a prefix of
    /// the events of the class.
    pub fn accept_with_codecs(
        &self,
        class_visitor: &mut dyn ClassVisitor,
        codecs: &[&dyn AttributeCodec],
        parsing_options: ParsingOptions,
    ) -> Result<(), ReadError> {
        let mut context = Context::new(codecs, parsing_options);

        // Read the access_flags, this_class, super_class, interface_count and interfaces fields.
        let mut current_offset = self.header;
        let mut access_flags = u32::from(self.u16_at(current_offset)?);
        let this_class = self.read_class_at(current_offset + 2)?;
        let super_class = self.read_class_at(current_offset + 4)?;
        let interfaces_count = self.u16_at(current_offset + 6)?;
        let mut interfaces = Vec::with_capacity(usize::from(interfaces_count));
        current_offset += 8;
        for _ in 0..interfaces_count {
            interfaces.push(required(
                self.read_class_at(current_offset)?,
                current_offset,
            )?);
            current_offset += 2;
        }

        // Read the class attributes (the variables are ordered as in Section 4.7 of the JVMS).
        // Attribute offsets exclude the attribute_name_index and attribute_length fields.
        // - The offset of the InnerClasses attribute, or 0.
        let mut inner_classes_offset = 0;
        // - The offset of the EnclosingMethod attribute, or 0.
        let mut enclosing_method_offset = 0;
        // - The string corresponding to the Signature attribute, or None.
        let mut signature = None;
        // - The string corresponding to the SourceFile attribute, or None.
        let mut source_file = None;
        // - The string corresponding to the SourceDebugExtension attribute, or None.
        let mut source_debug_extension = None;
        // - The offset of the RuntimeVisibleAnnotations attribute, or 0.
        let mut runtime_visible_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleAnnotations attribute, or 0.
        let mut runtime_invisible_annotations_offset = 0;
        // - The offset of the RuntimeVisibleTypeAnnotations attribute, or 0.
        let mut runtime_visible_type_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleTypeAnnotations attribute, or 0.
        let mut runtime_invisible_type_annotations_offset = 0;
        // - The offset of the Module attribute, or 0.
        let mut module_offset = 0;
        // - The offset of the ModulePackages attribute, or 0.
        let mut module_packages_offset = 0;
        // - The string corresponding to the ModuleMainClass attribute, or None.
        let mut module_main_class = None;
        // - The string corresponding to the NestHost attribute, or None.
        let mut nest_host_class = None;
        // - The offset of the NestMembers attribute, or 0.
        let mut nest_members_offset = 0;
        // - The offset of the PermittedSubclasses attribute, or 0
        let mut permitted_subclasses_offset = 0;
        // - The offset of the Record attribute, or 0.
        let mut record_offset = 0;
        // - The non standard attributes, in the *reverse order* of their order in the ClassFile
        //   structure (ASM links them through their nextAttribute field, last read first).
        let mut attributes = Vec::new();

        let (fields_count, methods_count, mut current_attribute_offset) =
            self.get_member_counts_and_first_attribute_offset()?;
        for _ in 0..self.u16_at(current_attribute_offset - 2)? {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_attribute_offset)?;
            let attribute_length = self.i32_at(current_attribute_offset + 2)?;
            current_attribute_offset += 6;
            // The tests are sorted in decreasing frequency order (based on frequencies observed
            // on typical classes).
            match attribute_name {
                Some(n) if n == constants::SOURCE_FILE => {
                    source_file = self.utf8_at(current_attribute_offset)?;
                }
                Some(n) if n == constants::INNER_CLASSES => {
                    inner_classes_offset = current_attribute_offset;
                }
                Some(n) if n == constants::ENCLOSING_METHOD => {
                    enclosing_method_offset = current_attribute_offset;
                }
                Some(n) if n == constants::NEST_HOST => {
                    nest_host_class = self.read_class_at(current_attribute_offset)?;
                }
                Some(n) if n == constants::NEST_MEMBERS => {
                    nest_members_offset = current_attribute_offset;
                }
                Some(n) if n == constants::PERMITTED_SUBCLASSES => {
                    permitted_subclasses_offset = current_attribute_offset;
                }
                Some(n) if n == constants::SIGNATURE => {
                    signature = self.utf8_at(current_attribute_offset)?;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_ANNOTATIONS => {
                    runtime_visible_annotations_offset = current_attribute_offset;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    runtime_visible_type_annotations_offset = current_attribute_offset;
                }
                Some(n) if n == constants::DEPRECATED => {
                    access_flags |= opcodes::ACC_DEPRECATED;
                }
                Some(n) if n == constants::SYNTHETIC => {
                    access_flags |= opcodes::ACC_SYNTHETIC;
                }
                Some(n) if n == constants::SOURCE_DEBUG_EXTENSION => {
                    if i64::from(attribute_length)
                        > self.class_file.len() as i64 - current_attribute_offset
                    {
                        return Err(ReadError::new(
                            current_attribute_offset,
                            ReadErrorKind::InvalidLength(attribute_length.into()),
                        ));
                    }
                    // ASM decodes it in a `new char[attributeLength]`.
                    if attribute_length < 0 {
                        return Err(ReadError::new(
                            current_attribute_offset,
                            ReadErrorKind::InvalidLength(attribute_length.into()),
                        ));
                    }
                    source_debug_extension = Some(self.read_utf(
                        current_attribute_offset,
                        attribute_length,
                        attribute_length as usize,
                    )?);
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_ANNOTATIONS => {
                    runtime_invisible_annotations_offset = current_attribute_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS => {
                    runtime_invisible_type_annotations_offset = current_attribute_offset;
                }
                Some(n) if n == constants::RECORD => {
                    record_offset = current_attribute_offset;
                    access_flags |= opcodes::ACC_RECORD;
                }
                Some(n) if n == constants::MODULE => {
                    module_offset = current_attribute_offset;
                }
                Some(n) if n == constants::MODULE_MAIN_CLASS => {
                    module_main_class = self.read_class_at(current_attribute_offset)?;
                }
                Some(n) if n == constants::MODULE_PACKAGES => {
                    module_packages_offset = current_attribute_offset;
                }
                // The BootstrapMethods attribute is read in the constructor.
                Some(n) if n == constants::BOOTSTRAP_METHODS => {}
                _ => {
                    attributes.push(self.read_attribute(
                        context.codecs,
                        attribute_name,
                        current_attribute_offset,
                        attribute_length,
                        None,
                    )?);
                }
            }
            current_attribute_offset += i64::from(attribute_length);
        }

        // Visit the class declaration. The minor_version and major_version fields start 6 bytes
        // before the first constant pool entry, which itself starts at cp_info_offsets[1] - 1
        // (by definition).
        let version_offset = self.cp_info_offset(1)? - 7;
        class_visitor.visit(
            self.i32_at(version_offset)? as u32,
            access_flags,
            required(this_class, self.header + 2)?,
            signature,
            super_class,
            &interfaces,
        );
        class_visitor.visit_member_counts(fields_count, methods_count);

        // Visit the SourceFile and SourceDebugExtension attributes.
        if !parsing_options.contains(ParsingOptions::SKIP_DEBUG)
            && (source_file.is_some() || source_debug_extension.is_some())
        {
            class_visitor.visit_source(source_file, source_debug_extension.as_deref());
        }

        // Visit the Module, ModulePackages and ModuleMainClass attributes.
        if module_offset != 0 {
            self.read_module_attributes(
                class_visitor,
                module_offset,
                module_packages_offset,
                module_main_class,
            )?;
        }

        // Visit the NestHost attribute.
        if let Some(nest_host_class) = nest_host_class {
            class_visitor.visit_nest_host(nest_host_class);
        }

        // Visit the EnclosingMethod attribute.
        if enclosing_method_offset != 0 {
            let class_name = self.read_class_at(enclosing_method_offset)?;
            let method_index = self.u16_at(enclosing_method_offset + 2)?;
            let (name, type_) = if method_index == 0 {
                (None, None)
            } else {
                let name_and_type_offset = self.cp_info_offset(method_index.into())?;
                (
                    self.utf8_at(name_and_type_offset)?,
                    self.utf8_at(name_and_type_offset + 2)?,
                )
            };
            class_visitor.visit_outer_class(
                required(class_name, enclosing_method_offset)?,
                name,
                type_,
            );
        }

        // Visit the RuntimeVisibleAnnotations and RuntimeInvisibleAnnotations attributes, then
        // the RuntimeVisibleTypeAnnotations and RuntimeInvisibleTypeAnnotations attributes.
        self.read_annotations_attribute(class_visitor, runtime_visible_annotations_offset, true)?;
        self.read_annotations_attribute(
            class_visitor,
            runtime_invisible_annotations_offset,
            false,
        )?;
        self.read_type_annotations_attribute(
            class_visitor,
            &mut context,
            runtime_visible_type_annotations_offset,
            true,
        )?;
        self.read_type_annotations_attribute(
            class_visitor,
            &mut context,
            runtime_invisible_type_annotations_offset,
            false,
        )?;

        // Visit the non standard attributes.
        for attribute in attributes.iter().rev() {
            class_visitor.visit_attribute(attribute);
        }

        // Visit the NestedMembers attribute.
        if nest_members_offset != 0 {
            let mut current_nest_member_offset = nest_members_offset + 2;
            for _ in 0..self.u16_at(nest_members_offset)? {
                class_visitor.visit_nest_member(required(
                    self.read_class_at(current_nest_member_offset)?,
                    current_nest_member_offset,
                )?);
                current_nest_member_offset += 2;
            }
        }

        // Visit the PermittedSubclasses attribute.
        if permitted_subclasses_offset != 0 {
            let mut current_permitted_subclasses_offset = permitted_subclasses_offset + 2;
            for _ in 0..self.u16_at(permitted_subclasses_offset)? {
                class_visitor.visit_permitted_subclass(required(
                    self.read_class_at(current_permitted_subclasses_offset)?,
                    current_permitted_subclasses_offset,
                )?);
                current_permitted_subclasses_offset += 2;
            }
        }

        // Visit the InnerClasses attribute.
        if inner_classes_offset != 0 {
            let mut current_classes_offset = inner_classes_offset + 2;
            for _ in 0..self.u16_at(inner_classes_offset)? {
                let name = self.read_class_at(current_classes_offset)?;
                let outer_name = self.read_class_at(current_classes_offset + 2)?;
                let inner_name = self.utf8_at(current_classes_offset + 4)?;
                let access = self.u16_at(current_classes_offset + 6)?;
                class_visitor.visit_inner_class(
                    required(name, current_classes_offset)?,
                    outer_name,
                    inner_name,
                    u32::from(access),
                );
                current_classes_offset += 8;
            }
        }

        // Visit Record components.
        if record_offset != 0 {
            let record_components_count = self.u16_at(record_offset)?;
            record_offset += 2;
            for _ in 0..record_components_count {
                record_offset =
                    self.read_record_component(class_visitor, &mut context, record_offset)?;
            }
        }

        // Visit the fields and methods.
        let fields_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..fields_count {
            current_offset = self.read_field(class_visitor, &mut context, current_offset)?;
        }
        let methods_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..methods_count {
            current_offset = self.read_method(class_visitor, &mut context, current_offset)?;
        }

        // Visit the end of the class.
        class_visitor.visit_end();
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Methods to parse attributes
    // ---------------------------------------------------------------------------------------------

    /// Returns the offset in `class_file` of the first `ClassFile`'s 'attributes' array field
    /// entry.
    pub(crate) fn get_first_attribute_offset(&self) -> Result<i64, ReadError> {
        Ok(self.get_member_counts_and_first_attribute_offset()?.2)
    }

    /// Returns the `fields_count` and `methods_count` of the `ClassFile`, and the offset in
    /// `class_file` of its first 'attributes' array field entry.
    fn get_member_counts_and_first_attribute_offset(&self) -> Result<(u16, u16, i64), ReadError> {
        // Skip the access_flags, this_class, super_class, and interfaces_count fields (using 2
        // bytes each), as well as the interfaces array field (2 bytes per interface).
        let mut current_offset = self.header + 8 + i64::from(self.u16_at(self.header + 6)?) * 2;

        // Read the fields_count field, then skip the 'fields' and 'methods' array fields.
        let mut members_counts = [0; 2];
        for count in &mut members_counts {
            let members_count = self.u16_at(current_offset)?;
            *count = members_count;
            current_offset += 2;
            for _ in 0..members_count {
                // Invariant: current_offset is the offset of a field_info or method_info
                // structure. Skip the access_flags, name_index and descriptor_index fields (2
                // bytes each), and read the attributes_count field.
                let attributes_count = self.u16_at(current_offset + 6)?;
                current_offset += 8;
                // Skip the 'attributes' array field.
                for _ in 0..attributes_count {
                    // Invariant: current_offset is the offset of an attribute_info structure.
                    // Read the attribute_length field (2 bytes after the start of the
                    // attribute_info) and skip this many bytes, plus 6 for the
                    // attribute_name_index and attribute_length fields (yielding the total size
                    // of the attribute_info structure).
                    current_offset += 6 + i64::from(self.i32_at(current_offset + 2)?);
                }
            }
        }

        // Skip the ClassFile's attributes_count field.
        Ok((members_counts[0], members_counts[1], current_offset + 2))
    }

    /// Reads the BootstrapMethods attribute to compute the offset of each bootstrap method.
    fn read_bootstrap_methods_attribute(&self) -> Result<Box<[u32]>, ReadError> {
        let mut current_attribute_offset = self.get_first_attribute_offset()?;
        for _ in 0..self.u16_at(current_attribute_offset - 2)? {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_attribute_offset)?;
            let attribute_length = self.i32_at(current_attribute_offset + 2)?;
            current_attribute_offset += 6;
            if attribute_name.is_some_and(|n| n == constants::BOOTSTRAP_METHODS) {
                // Read the num_bootstrap_methods field and create an array of this size.
                let num_bootstrap_methods = self.u16_at(current_attribute_offset)?;
                let mut result = Vec::with_capacity(usize::from(num_bootstrap_methods));
                // Compute and store the offset of each 'bootstrap_methods' array field entry.
                let mut current_bootstrap_method_offset = current_attribute_offset + 2;
                for _ in 0..num_bootstrap_methods {
                    result.push(offset_to_u32(current_bootstrap_method_offset));
                    // Skip the bootstrap_method_ref and num_bootstrap_arguments fields (2 bytes
                    // each), as well as the bootstrap_arguments array field (of size
                    // num_bootstrap_arguments * 2).
                    current_bootstrap_method_offset +=
                        4 + i64::from(self.u16_at(current_bootstrap_method_offset + 2)?) * 2;
                }
                return Ok(result.into_boxed_slice());
            }
            current_attribute_offset += i64::from(attribute_length);
        }
        Err(ReadError::new(
            current_attribute_offset,
            ReadErrorKind::MissingBootstrapMethods,
        ))
    }

    /// Reads a non standard JVMS 'attribute' structure in `class_file`, with the codec of the
    /// same name if there is one (ASM's `readAttribute`).
    ///
    /// - `code`: for a Code attribute, the offset of the Code attribute's content and the
    ///   method's labels.
    fn read_attribute(
        &self,
        codecs: &[&dyn AttributeCodec],
        name: Option<&JavaStr>,
        offset: i64,
        length: i32,
        code: Option<(i64, &mut Vec<Option<Label>>)>,
    ) -> Result<Attribute, ReadError> {
        if i64::from(length) > self.class_file.len() as i64 - offset {
            return Err(ReadError::new(
                offset,
                ReadErrorKind::InvalidLength(length.into()),
            ));
        }
        // ASM compares the prototype names with `type.equals(name)`, which a null name never is.
        // `new Attribute(null)` is a valid unknown attribute though.
        let codec = name.and_then(|name| codecs.iter().find(|c| c.name() == name));
        let Some(codec) = codec else {
            let content = self.slice_at(offset, length)?;
            return Ok(Attribute::Unknown(UnknownAttribute {
                name: required(name, offset - 6)?.to_owned(),
                content: content.to_vec(),
            }));
        };
        // ASM passes a negative length on to the prototype; a codec gets a well formed slice.
        if length < 0 {
            return Err(ReadError::new(
                offset,
                ReadErrorKind::InvalidLength(length.into()),
            ));
        }
        let (code_attribute_offset, labels) = match code {
            Some((code_offset, labels)) => (Some(code_offset), Some(labels)),
            None => (None, None),
        };
        let mut cx = AttributeReadContext {
            reader: self,
            offset,
            length,
            code_attribute_offset,
            labels,
        };
        let attribute: Box<dyn CustomAttribute> = codec.read(&mut cx)?;
        Ok(Attribute::Custom(attribute))
    }
}

/// What an [`AttributeCodec`] reads an attribute from: the class reader, the attribute's position
/// in the class file and, for Code attributes, the method's labels (the arguments of ASM's
/// `Attribute.read`).
#[derive(Debug)]
pub struct AttributeReadContext<'r, 'b> {
    reader: &'r ClassReader<'b>,
    offset: i64,
    length: i32,
    code_attribute_offset: Option<i64>,
    labels: Option<&'r mut Vec<Option<Label>>>,
}

impl<'b> AttributeReadContext<'_, 'b> {
    /// The class reader, whose `read_*` methods parse the attribute content.
    #[must_use]
    pub fn reader(&self) -> &ClassReader<'b> {
        self.reader
    }

    /// The offset of the attribute content in the class file (after `attribute_length`).
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset as usize
    }

    /// The length of the attribute content.
    #[must_use]
    pub fn length(&self) -> usize {
        self.length as usize
    }

    /// The attribute content (the bytes after `attribute_length`).
    #[must_use]
    pub fn content(&self) -> &'b [u8] {
        // The reader checked that the content is in the class file before calling the codec.
        self.reader
            .slice_at(self.offset, self.length)
            .unwrap_or_default()
    }

    /// For a Code attribute, the offset in the class file of the content of the enclosing
    /// `Code` attribute; `None` otherwise (ASM's `codeAttributeOffset`, -1 when not a Code
    /// attribute).
    #[must_use]
    pub fn code_attribute_offset(&self) -> Option<usize> {
        self.code_attribute_offset.map(|o| o as usize)
    }

    /// For a Code attribute, the label of the given bytecode offset, created if needed (ASM's
    /// `Attribute.readLabel`).
    ///
    /// # Errors
    ///
    /// If this is not a Code attribute, or the offset is outside of the method code.
    pub fn read_label(&mut self, bytecode_offset: usize) -> Result<Label, ReadError> {
        let offset = self.offset;
        let labels = self.labels.as_deref_mut().ok_or_else(|| {
            ReadError::new(
                offset,
                ReadErrorKind::InvalidAttribute("labels read outside of a Code attribute".into()),
            )
        })?;
        code::read_label(bytecode_offset as i64, labels)
    }
}

/// `value`, or an error at `offset` if it is `None`: a `null` string (constant pool index 0)
/// where the visitor methods take a non nullable string.
fn required<T>(value: Option<T>, offset: i64) -> Result<T, ReadError> {
    value.ok_or_else(|| ReadError::new(offset, ReadErrorKind::InvalidConstantPoolIndex(0)))
}

/// An offset that is in the class file (a successful read happened there, or will be checked),
/// as stored in the offset tables.
fn offset_to_u32(offset: i64) -> u32 {
    // Offsets stored in tables are those of a structure being read: they are in [0, 2^31),
    // except for the bootstrap method offsets, which are checked when read.
    u32::try_from(offset).unwrap_or(u32::MAX)
}

// The constant pool tags (ASM's `Symbol.CONSTANT_*_TAG`).
pub(crate) const CONSTANT_CLASS_TAG: u8 = 7;
pub(crate) const CONSTANT_FIELDREF_TAG: u8 = 9;
pub(crate) const CONSTANT_METHODREF_TAG: u8 = 10;
pub(crate) const CONSTANT_INTERFACE_METHODREF_TAG: u8 = 11;
pub(crate) const CONSTANT_STRING_TAG: u8 = 8;
pub(crate) const CONSTANT_INTEGER_TAG: u8 = 3;
pub(crate) const CONSTANT_FLOAT_TAG: u8 = 4;
pub(crate) const CONSTANT_LONG_TAG: u8 = 5;
pub(crate) const CONSTANT_DOUBLE_TAG: u8 = 6;
pub(crate) const CONSTANT_NAME_AND_TYPE_TAG: u8 = 12;
pub(crate) const CONSTANT_UTF8_TAG: u8 = 1;
pub(crate) const CONSTANT_METHOD_HANDLE_TAG: u8 = 15;
pub(crate) const CONSTANT_METHOD_TYPE_TAG: u8 = 16;
pub(crate) const CONSTANT_DYNAMIC_TAG: u8 = 17;
pub(crate) const CONSTANT_INVOKE_DYNAMIC_TAG: u8 = 18;
pub(crate) const CONSTANT_MODULE_TAG: u8 = 19;
pub(crate) const CONSTANT_PACKAGE_TAG: u8 = 20;
