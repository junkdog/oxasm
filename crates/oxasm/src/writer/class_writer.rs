//! [`ClassWriter`]: ASM's `ClassWriter`, a [`ClassVisitor`] that generates a class file.

use crate::attribute::{Attribute, AttributeCodec};
use crate::constant::Constant;
use crate::constants;
use crate::error::{ReadError, WriteError};
use crate::handle::Handle;
use crate::hierarchy::ClassHierarchy;
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::opcodes;
use crate::reader::{ClassReader, ParsingOptions};
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::{
    AnnotationVisitor, ClassVisitor, FieldVisitor, MethodVisitor, ModuleVisitor,
    RecordComponentVisitor,
};
use crate::writer::annotation_writer::{
    MemberAnnotations, compute_member_annotations_size, put_member_annotations,
};
use crate::writer::attributes::{AttributeList, AttributePrototypes};
use crate::writer::byte_vector::ByteVector;
use crate::writer::field_writer::{FieldWriter, FieldWriterState};
use crate::writer::method_writer::{Compute, MethodWriter, MethodWriterState};
use crate::writer::module_writer::{ModuleWriter, ModuleWriterState};
use crate::writer::record_component_writer::{RecordComponentWriter, RecordComponentWriterState};
use crate::writer::symbol_table::SymbolTable;

bitflags::bitflags! {
    /// What a [`ClassWriter`] computes (ASM's `COMPUTE_MAXS` and `COMPUTE_FRAMES` flags).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct ClassWriterFlags: u32 {
        /// The maximum stack size and the maximum number of local variables of methods are
        /// computed automatically. The arguments of `visit_maxs` are then ignored, but it must
        /// still be called. The `COMPUTE_MAXS` flag does not compute stack map frames.
        const COMPUTE_MAXS = 1;
        /// The stack map frames of methods, and their maximum stack size and number of local
        /// variables, are computed from scratch. Implies [`Self::COMPUTE_MAXS`]. The
        /// `visit_frame` and `visit_maxs` calls are then ignored. Computing frames needs the
        /// class hierarchy of the classes the methods use (see
        /// [`ClassWriter::with_hierarchy`]).
        const COMPUTE_FRAMES = 2;
    }
}

/// A [`ClassVisitor`] that generates a corresponding `ClassFile` structure, as defined in the Java
/// Virtual Machine Specification (JVMS). It can be used alone, to generate a class "from
/// scratch", or with one or more class readers and adapter class visitors to generate a modified
/// class from one or more existing classes.
///
/// The visitor methods cannot fail (ADR-06): an error ASM throws in the middle of a visit (an
/// invalid argument, a string too long for the constant pool, ...) is recorded, and
/// [`to_bytes`](Self::to_bytes) returns it.
///
/// `'h` is the lifetime of the class hierarchy that computing stack map frames uses, if any (see
/// [`with_hierarchy`](Self::with_hierarchy)).
pub struct ClassWriter<'h> {
    /// The flags passed to the constructor. Must be zero or more of `COMPUTE_MAXS` and
    /// `COMPUTE_FRAMES`.
    flags: ClassWriterFlags,
    /// The class hierarchy that computing stack map frames uses (ASM's `getCommonSuperClass`).
    hierarchy: Option<&'h dyn ClassHierarchy>,

    // Note: fields are ordered as in the ClassFile structure, and those related to attributes are
    // ordered as in Section 4.7 of the JVMS.
    /// The `minor_version` and `major_version` fields of the JVMS ClassFile structure.
    /// `minor_version` is stored in the 16 most significant bits, and `major_version` in the 16
    /// least significant bits.
    version: u32,
    /// The symbol table for this class (contains the `constant_pool` and the
    /// `BootstrapMethods`).
    symbol_table: SymbolTable,
    /// The `access_flags` field of the JVMS ClassFile structure. This field can contain ASM
    /// specific access flags, such as `ACC_DEPRECATED` or `ACC_RECORD`, which are removed when
    /// generating the ClassFile structure.
    access_flags: u32,
    /// The `this_class` field of the JVMS ClassFile structure.
    this_class: i32,
    /// The `super_class` field of the JVMS ClassFile structure.
    super_class: i32,
    /// The `u2 interfaces[]` field of the JVMS ClassFile structure.
    interfaces: Vec<i32>,
    /// The fields of this class (ASM's `firstField` list).
    fields: Vec<FieldWriterState>,
    /// The methods of this class (ASM's `firstMethod` list).
    methods: Vec<MethodWriterState>,
    /// The `number_of_classes` field of the InnerClasses attribute, or 0.
    number_of_inner_classes: i32,
    /// The `inner_classes` array of the InnerClasses attribute, or `None`.
    inner_classes: Option<ByteVector>,
    /// The `class_index` field of the EnclosingMethod attribute, or 0.
    enclosing_class_index: i32,
    /// The `method_index` field of the EnclosingMethod attribute.
    enclosing_method_index: i32,
    /// The `signature_index` field of the Signature attribute, or 0.
    signature_index: i32,
    /// The `source_file_index` field of the SourceFile attribute, or 0.
    source_file_index: i32,
    /// The `debug_extension` field of the SourceDebugExtension attribute, or `None`.
    debug_extension: Option<ByteVector>,
    /// The runtime (in)visible (type) annotations of this class.
    annotations: MemberAnnotations,
    /// The Module attribute of this class, or `None`.
    module_writer: Option<ModuleWriterState>,
    /// The `host_class_index` field of the NestHost attribute, or 0.
    nest_host_class_index: i32,
    /// The `number_of_classes` field of the NestMembers attribute, or 0.
    number_of_nest_member_classes: i32,
    /// The `classes` array of the NestMembers attribute, or `None`.
    nest_member_classes: Option<ByteVector>,
    /// The `number_of_classes` field of the PermittedSubclasses attribute, or 0.
    number_of_permitted_subclasses: i32,
    /// The `classes` array of the PermittedSubclasses attribute, or `None`.
    permitted_subclasses: Option<ByteVector>,
    /// The record components of this class (ASM's `firstRecordComponent` list).
    record_components: Vec<RecordComponentWriterState>,
    /// The non standard attributes of this class.
    attributes: AttributeList,
    /// Indicates what must be automatically computed in the method writers.
    compute: Compute,
}

impl std::fmt::Debug for ClassWriter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassWriter")
            .field("flags", &self.flags)
            .field("version", &self.version)
            .field("class_name", &self.symbol_table.class_name())
            .field("fields", &self.fields.len())
            .field("methods", &self.methods.len())
            .finish_non_exhaustive()
    }
}

impl ClassWriter<'static> {
    /// Constructs a new class writer.
    #[must_use]
    pub fn new(flags: ClassWriterFlags) -> ClassWriter<'static> {
        ClassWriter::with_symbol_table(SymbolTable::new(), flags)
    }

    /// Constructs a new class writer and enables optimizations for "mostly add" bytecode
    /// transformations. These optimizations are the following:
    ///
    /// - The constant pool and bootstrap methods from the original class are copied as is in the
    ///   new class, which saves time. New constant pool entries and new bootstrap methods will be
    ///   added at the end if necessary, but unused constant pool entries or bootstrap methods
    ///   *won't be removed*.
    /// - Methods that are not transformed are copied as is in the new class, directly from the
    ///   original class bytecode (i.e. without emitting visit events for all the method
    ///   instructions), which saves a *lot* of time. Untransformed methods are detected by the
    ///   fact that the class reader receives a method visitor that is a method writer of this
    ///   class writer (and not some other method visitor), directly.
    ///
    /// # Errors
    ///
    /// If the constant pool or the BootstrapMethods attribute of the class are malformed (ASM's
    /// constructor throws then).
    pub fn from_reader(
        class_reader: &ClassReader<'_>,
        flags: ClassWriterFlags,
    ) -> Result<ClassWriter<'static>, ReadError> {
        Ok(ClassWriter::with_symbol_table(
            SymbolTable::from_reader(class_reader)?,
            flags,
        ))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl<'h> ClassWriter<'h> {
    /// A new class writer over the given symbol table.
    fn with_symbol_table(symbol_table: SymbolTable, flags: ClassWriterFlags) -> ClassWriter<'h> {
        let mut writer = ClassWriter {
            flags,
            hierarchy: None,
            version: 0,
            symbol_table,
            access_flags: 0,
            this_class: 0,
            super_class: 0,
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            number_of_inner_classes: 0,
            inner_classes: None,
            enclosing_class_index: 0,
            enclosing_method_index: 0,
            signature_index: 0,
            source_file_index: 0,
            debug_extension: None,
            annotations: MemberAnnotations::default(),
            module_writer: None,
            nest_host_class_index: 0,
            number_of_nest_member_classes: 0,
            nest_member_classes: None,
            number_of_permitted_subclasses: 0,
            permitted_subclasses: None,
            record_components: Vec::new(),
            attributes: AttributeList::default(),
            compute: Compute::Nothing,
        };
        writer.set_flags(flags);
        writer
    }

    /// This class writer, with the given class hierarchy to compute stack map frames (which
    /// ASM's `getCommonSuperClass` gets by loading classes). Without one, computing a frame
    /// that needs the common super class of two classes fails.
    #[must_use]
    pub fn with_hierarchy<'a>(self, hierarchy: &'a dyn ClassHierarchy) -> ClassWriter<'a> {
        ClassWriter {
            hierarchy: Some(hierarchy),
            flags: self.flags,
            version: self.version,
            symbol_table: self.symbol_table,
            access_flags: self.access_flags,
            this_class: self.this_class,
            super_class: self.super_class,
            interfaces: self.interfaces,
            fields: self.fields,
            methods: self.methods,
            number_of_inner_classes: self.number_of_inner_classes,
            inner_classes: self.inner_classes,
            enclosing_class_index: self.enclosing_class_index,
            enclosing_method_index: self.enclosing_method_index,
            signature_index: self.signature_index,
            source_file_index: self.source_file_index,
            debug_extension: self.debug_extension,
            annotations: self.annotations,
            module_writer: self.module_writer,
            nest_host_class_index: self.nest_host_class_index,
            number_of_nest_member_classes: self.number_of_nest_member_classes,
            nest_member_classes: self.nest_member_classes,
            number_of_permitted_subclasses: self.number_of_permitted_subclasses,
            permitted_subclasses: self.permitted_subclasses,
            record_components: self.record_components,
            attributes: self.attributes,
            compute: self.compute,
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Accessors
    // ---------------------------------------------------------------------------------------------

    /// The control flow graph that `COMPUTE_MAXS` built for the last visited method, from the
    /// basic block of `start` on: for each basic block, in bytecode order, its bytecode offset
    /// and those of its successors (without the virtual successor of a block that ends with a
    /// jsr). Empty if `start` is not a basic block of this method.
    ///
    /// For the port of ASM's `ClassWriterComputeMaxsTest`, which reads the graph from
    /// package-private fields. Not part of the API.
    #[doc(hidden)]
    #[must_use]
    pub fn last_method_control_flow_graph(&self, start: Label) -> Vec<(i32, Vec<i32>)> {
        self.methods
            .last()
            .map(|method| method.control_flow_graph(start))
            .unwrap_or_default()
    }

    /// Returns true if all the given flags were passed to the constructor.
    #[must_use]
    pub fn has_flags(&self, flags: ClassWriterFlags) -> bool {
        self.flags.contains(flags)
    }

    /// Sets the flags that determine what is computed for the methods visited from now on.
    /// Calling this method in the middle of a method visit is not supported.
    pub fn set_flags(&mut self, flags: ClassWriterFlags) {
        self.compute = if flags.contains(ClassWriterFlags::COMPUTE_FRAMES) {
            Compute::AllFrames
        } else if flags.contains(ClassWriterFlags::COMPUTE_MAXS) {
            Compute::MaxStackAndLocal
        } else {
            Compute::Nothing
        };
    }

    // ---------------------------------------------------------------------------------------------
    // Other public methods
    // ---------------------------------------------------------------------------------------------

    /// Returns the content of the class file that was built by this class writer (ASM's
    /// `toByteArray`).
    ///
    /// # Errors
    ///
    /// The first error ASM would have thrown, during the visit or here: a constant pool with
    /// more than 65535 entries ([`WriteError::ClassTooLarge`]), a method with more than 65535
    /// bytes of bytecode ([`WriteError::MethodTooLarge`]), an invalid visitor argument, ...
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, WriteError> {
        loop {
            let (class_file, has_frames, has_asm_instructions) = self.write_class_file()?;
            // Third step: replace the ASM specific instructions, if any (ASM's
            // replaceAsmInstructions, which calls toByteArray recursively: a loop here, since
            // the number of rounds depends on the input).
            if !has_asm_instructions {
                return Ok(class_file);
            }
            self.replace_asm_instructions(&class_file, has_frames)?;
        }
    }

    /// The first two steps of ASM's `toByteArray`: compute the size of the class file, then fill
    /// it. Also returns whether the class has frames and ASM specific instructions.
    fn write_class_file(&mut self) -> Result<(Vec<u8>, bool, bool), WriteError> {
        self.check_error()?;
        let symbol_table = &mut self.symbol_table;
        // First step: compute the size in bytes of the ClassFile structure.
        // The magic field uses 4 bytes, 10 mandatory fields (minor_version, major_version,
        // constant_pool_count, access_flags, this_class, super_class, interfaces_count,
        // fields_count, methods_count and attributes_count) use 2 bytes each, and each
        // interface uses 2 bytes too.
        let mut size = 24 + 2 * self.interfaces.len();
        for field_writer in &mut self.fields {
            size += field_writer.compute_field_info_size(symbol_table);
        }
        for method_writer in &mut self.methods {
            size += method_writer.compute_method_info_size(symbol_table);
        }
        // For ease of reference, we use here the same attribute order as in Section 4.7 of the
        // JVMS.
        let mut attributes_count = 0;
        if let Some(inner_classes) = &self.inner_classes {
            attributes_count += 1;
            size += 8 + inner_classes.size();
            symbol_table.add_constant_utf8(constants::INNER_CLASSES);
        }
        if self.enclosing_class_index != 0 {
            attributes_count += 1;
            size += 10;
            symbol_table.add_constant_utf8(constants::ENCLOSING_METHOD);
        }
        let major_version = self.version & 0xFFFF;
        if self.access_flags & opcodes::ACC_SYNTHETIC != 0 && major_version < opcodes::V1_5 {
            attributes_count += 1;
            size += 6;
            symbol_table.add_constant_utf8(constants::SYNTHETIC);
        }
        if self.signature_index != 0 {
            attributes_count += 1;
            size += 8;
            symbol_table.add_constant_utf8(constants::SIGNATURE);
        }
        if self.source_file_index != 0 {
            attributes_count += 1;
            size += 8;
            symbol_table.add_constant_utf8(constants::SOURCE_FILE);
        }
        if let Some(debug_extension) = &self.debug_extension {
            attributes_count += 1;
            size += 6 + debug_extension.size();
            symbol_table.add_constant_utf8(constants::SOURCE_DEBUG_EXTENSION);
        }
        if self.access_flags & opcodes::ACC_DEPRECATED != 0 {
            attributes_count += 1;
            size += 6;
            symbol_table.add_constant_utf8(constants::DEPRECATED);
        }
        attributes_count += self.annotations.attribute_count();
        size += compute_member_annotations_size(symbol_table, &self.annotations);
        if symbol_table.compute_bootstrap_methods_size() > 0 {
            attributes_count += 1;
            size += symbol_table.compute_bootstrap_methods_size();
        }
        if let Some(module_writer) = &self.module_writer {
            attributes_count += module_writer.get_attribute_count();
            size += module_writer.compute_attributes_size(symbol_table);
        }
        if self.nest_host_class_index != 0 {
            attributes_count += 1;
            size += 8;
            symbol_table.add_constant_utf8(constants::NEST_HOST);
        }
        if let Some(nest_member_classes) = &self.nest_member_classes {
            attributes_count += 1;
            size += 8 + nest_member_classes.size();
            symbol_table.add_constant_utf8(constants::NEST_MEMBERS);
        }
        if let Some(permitted_subclasses) = &self.permitted_subclasses {
            attributes_count += 1;
            size += 8 + permitted_subclasses.size();
            symbol_table.add_constant_utf8(constants::PERMITTED_SUBCLASSES);
        }
        let mut record_size = 0;
        let has_record =
            self.access_flags & opcodes::ACC_RECORD != 0 || !self.record_components.is_empty();
        if has_record {
            for record_component_writer in &mut self.record_components {
                record_size +=
                    record_component_writer.compute_record_component_info_size(symbol_table);
            }
            attributes_count += 1;
            size += 8 + record_size;
            symbol_table.add_constant_utf8(constants::RECORD);
        }
        if !self.attributes.is_empty() {
            attributes_count += self.attributes.len();
            size += self.attributes.compute_attributes_size(symbol_table, None);
        }
        // IMPORTANT: this must be the last part of the ClassFile size computation, because the
        // previous statements can add attribute names to the constant pool, thereby changing its
        // size!
        size += symbol_table.constant_pool_length();
        self.check_error()?;
        let symbol_table = &mut self.symbol_table;
        let constant_pool_count = symbol_table.constant_pool_count();
        if constant_pool_count > 0xFFFF {
            return Err(WriteError::ClassTooLarge {
                class_name: symbol_table.class_name().to_owned(),
                constant_pool_count: constant_pool_count as usize,
            });
        }

        // Second step: allocate a ByteVector of the correct size (in order to avoid any array
        // copy in dynamic resizes) and fill it with the ClassFile content.
        let mut result = ByteVector::with_capacity(size);
        result
            .put_int(0xCAFE_BABE_u32 as i32)
            .put_int(self.version as i32);
        symbol_table.put_constant_pool(&mut result);
        let mask = if major_version < opcodes::V1_5 {
            opcodes::ACC_SYNTHETIC
        } else {
            0
        };
        result
            .put_short((self.access_flags & !mask) as i32)
            .put_short(self.this_class)
            .put_short(self.super_class);
        result.put_short(self.interfaces.len() as i32);
        for &interface in &self.interfaces {
            result.put_short(interface);
        }
        result.put_short(self.fields.len() as i32);
        for field_writer in &mut self.fields {
            field_writer.put_field_info(symbol_table, &mut result);
        }
        result.put_short(self.methods.len() as i32);
        let mut has_frames = false;
        let mut has_asm_instructions = false;
        for method_writer in &mut self.methods {
            has_frames |= method_writer.has_frames();
            has_asm_instructions |= method_writer.has_asm_instructions();
            method_writer.put_method_info(symbol_table, &mut result);
        }
        // For ease of reference, we use here the same attribute order as in Section 4.7 of the
        // JVMS.
        result.put_short(attributes_count as i32);
        if let Some(inner_classes) = &self.inner_classes {
            let name_index = symbol_table.add_constant_utf8(constants::INNER_CLASSES);
            result
                .put_short(name_index)
                .put_int(inner_classes.size() as i32 + 2)
                .put_short(self.number_of_inner_classes)
                .put_byte_array(inner_classes.data());
        }
        if self.enclosing_class_index != 0 {
            let name_index = symbol_table.add_constant_utf8(constants::ENCLOSING_METHOD);
            result
                .put_short(name_index)
                .put_int(4)
                .put_short(self.enclosing_class_index)
                .put_short(self.enclosing_method_index);
        }
        if self.access_flags & opcodes::ACC_SYNTHETIC != 0 && major_version < opcodes::V1_5 {
            let name_index = symbol_table.add_constant_utf8(constants::SYNTHETIC);
            result.put_short(name_index).put_int(0);
        }
        if self.signature_index != 0 {
            let name_index = symbol_table.add_constant_utf8(constants::SIGNATURE);
            result
                .put_short(name_index)
                .put_int(2)
                .put_short(self.signature_index);
        }
        if self.source_file_index != 0 {
            let name_index = symbol_table.add_constant_utf8(constants::SOURCE_FILE);
            result
                .put_short(name_index)
                .put_int(2)
                .put_short(self.source_file_index);
        }
        if let Some(debug_extension) = &self.debug_extension {
            let length = debug_extension.size();
            let name_index = symbol_table.add_constant_utf8(constants::SOURCE_DEBUG_EXTENSION);
            result
                .put_short(name_index)
                .put_int(length as i32)
                .put_byte_array(debug_extension.data());
        }
        if self.access_flags & opcodes::ACC_DEPRECATED != 0 {
            let name_index = symbol_table.add_constant_utf8(constants::DEPRECATED);
            result.put_short(name_index).put_int(0);
        }
        put_member_annotations(symbol_table, &self.annotations, &mut result);
        symbol_table.put_bootstrap_methods(&mut result);
        if let Some(module_writer) = &self.module_writer {
            module_writer.put_attributes(symbol_table, &mut result);
        }
        if self.nest_host_class_index != 0 {
            let name_index = symbol_table.add_constant_utf8(constants::NEST_HOST);
            result
                .put_short(name_index)
                .put_int(2)
                .put_short(self.nest_host_class_index);
        }
        if let Some(nest_member_classes) = &self.nest_member_classes {
            let name_index = symbol_table.add_constant_utf8(constants::NEST_MEMBERS);
            result
                .put_short(name_index)
                .put_int(nest_member_classes.size() as i32 + 2)
                .put_short(self.number_of_nest_member_classes)
                .put_byte_array(nest_member_classes.data());
        }
        if let Some(permitted_subclasses) = &self.permitted_subclasses {
            let name_index = symbol_table.add_constant_utf8(constants::PERMITTED_SUBCLASSES);
            result
                .put_short(name_index)
                .put_int(permitted_subclasses.size() as i32 + 2)
                .put_short(self.number_of_permitted_subclasses)
                .put_byte_array(permitted_subclasses.data());
        }
        if has_record {
            let name_index = symbol_table.add_constant_utf8(constants::RECORD);
            result
                .put_short(name_index)
                .put_int(record_size as i32 + 2)
                .put_short(self.record_components.len() as i32);
            for record_component_writer in &mut self.record_components {
                record_component_writer.put_record_component_info(symbol_table, &mut result);
            }
        }
        if !self.attributes.is_empty() {
            self.attributes
                .put_attributes(symbol_table, None, &mut result);
        }
        self.check_error()?;
        Ok((result.into_vec(), has_frames, has_asm_instructions))
    }

    /// Returns the first error recorded during the visit, if any.
    fn check_error(&self) -> Result<(), WriteError> {
        match self.symbol_table.error() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Prepares this writer to write the given class file again, after replacing its ASM
    /// specific instructions with standard ones (ASM's `replaceAsmInstructions`, less its final
    /// `toByteArray` call). This is done with a class reader → class writer round trip.
    fn replace_asm_instructions(
        &mut self,
        class_file: &[u8],
        has_frames: bool,
    ) -> Result<(), WriteError> {
        let prototypes = self.get_attribute_prototypes();
        self.fields.clear();
        self.methods.clear();
        self.annotations = MemberAnnotations::default();
        self.module_writer = None;
        self.nest_host_class_index = 0;
        self.number_of_nest_member_classes = 0;
        self.nest_member_classes = None;
        self.number_of_permitted_subclasses = 0;
        self.permitted_subclasses = None;
        self.record_components.clear();
        self.attributes = AttributeList::default();
        self.compute = if has_frames {
            Compute::InsertedFrames
        } else {
            Compute::Nothing
        };
        let codecs: Vec<&dyn AttributeCodec> =
            prototypes.codecs().iter().map(|codec| &**codec).collect();
        let mut parsing_options = ParsingOptions::EXPAND_ASM_INSNS;
        if has_frames {
            parsing_options |= ParsingOptions::EXPAND_FRAMES;
        }
        ClassReader::new_with_version_check(class_file, false)
            .and_then(|reader| reader.accept_with_codecs(self, &codecs, parsing_options))
            .map_err(WriteError::Reread)
    }

    /// Returns the prototypes of the attributes used by this class, its fields and its methods
    /// (ASM's `getAttributePrototypes`).
    fn get_attribute_prototypes(&self) -> AttributePrototypes {
        let mut prototypes = AttributePrototypes::default();
        self.attributes
            .collect_attribute_prototypes(&mut prototypes);
        for field_writer in &self.fields {
            field_writer.collect_attribute_prototypes(&mut prototypes);
        }
        for method_writer in &self.methods {
            method_writer.collect_attribute_prototypes(&mut prototypes);
        }
        for record_component_writer in &self.record_components {
            record_component_writer.collect_attribute_prototypes(&mut prototypes);
        }
        prototypes
    }

    // ---------------------------------------------------------------------------------------------
    // Utility methods: constant pool management for Attribute sub classes
    // ---------------------------------------------------------------------------------------------

    /// Adds a number or string constant to the constant pool of the class being built. Does
    /// nothing if the constant pool already contains a similar item. *This method is intended
    /// for custom attributes and adapters, and is normally not needed by class generators.*
    /// Returns the index of the constant in the constant pool.
    pub fn new_const(&mut self, value: &Constant<'_>) -> u16 {
        self.symbol_table.add_constant(value).index as u16
    }

    /// Adds an UTF8 string to the constant pool of the class being built. Does nothing if the
    /// constant pool already contains a similar item. Returns its index.
    pub fn new_utf8(&mut self, value: &JavaStr) -> u16 {
        self.symbol_table.add_constant_utf8(value) as u16
    }

    /// Adds a class reference to the constant pool of the class being built. Does nothing if the
    /// constant pool already contains a similar item. Returns its index.
    pub fn new_class(&mut self, value: &JavaStr) -> u16 {
        self.symbol_table.add_constant_class(value).index as u16
    }

    /// Adds a method type reference to the constant pool of the class being built. Does nothing
    /// if the constant pool already contains a similar item. Returns its index.
    pub fn new_method_type(&mut self, method_descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_method_type(method_descriptor)
            .index as u16
    }

    /// Adds a module reference to the constant pool of the class being built. Does nothing if
    /// the constant pool already contains a similar item. Returns its index.
    pub fn new_module(&mut self, module_name: &JavaStr) -> u16 {
        self.symbol_table.add_constant_module(module_name).index as u16
    }

    /// Adds a package reference to the constant pool of the class being built. Does nothing if
    /// the constant pool already contains a similar item. Returns its index.
    pub fn new_package(&mut self, package_name: &JavaStr) -> u16 {
        self.symbol_table.add_constant_package(package_name).index as u16
    }

    /// Adds a handle to the constant pool of the class being built. Does nothing if the constant
    /// pool already contains a similar item. Returns its index.
    pub fn new_handle(&mut self, handle: &Handle) -> u16 {
        self.symbol_table
            .add_constant_method_handle(
                i32::from(handle.get_tag().tag()),
                handle.get_owner(),
                handle.get_name(),
                handle.get_desc(),
                handle.is_interface(),
            )
            .index as u16
    }

    /// Adds a dynamic constant reference to the constant pool of the class being built. Does
    /// nothing if the constant pool already contains a similar item. Returns its index.
    pub fn new_constant_dynamic(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) -> u16 {
        self.symbol_table
            .add_constant_dynamic(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            )
            .index as u16
    }

    /// Adds an invokedynamic reference to the constant pool of the class being built. Does
    /// nothing if the constant pool already contains a similar item. Returns its index.
    pub fn new_invoke_dynamic(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) -> u16 {
        self.symbol_table
            .add_constant_invoke_dynamic(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            )
            .index as u16
    }

    /// Adds a field reference to the constant pool of the class being built. Does nothing if the
    /// constant pool already contains a similar item. Returns its index.
    pub fn new_field(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_fieldref(owner, name, descriptor)
            .index as u16
    }

    /// Adds a method reference to the constant pool of the class being built. Does nothing if
    /// the constant pool already contains a similar item. Returns its index.
    pub fn new_method(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) -> u16 {
        self.symbol_table
            .add_constant_methodref(owner, name, descriptor, is_interface)
            .index as u16
    }

    /// Adds a name and type to the constant pool of the class being built. Does nothing if the
    /// constant pool already contains a similar item. Returns its index.
    pub fn new_name_type(&mut self, name: &JavaStr, descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_name_and_type(name, descriptor) as u16
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl ClassVisitor for ClassWriter<'_> {
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.version = version;
        self.access_flags = access;
        self.this_class = self
            .symbol_table
            .set_major_version_and_class_name((version & 0xFFFF) as i32, name);
        if let Some(signature) = signature {
            self.signature_index = self.symbol_table.add_constant_utf8(signature);
        }
        self.super_class = super_name.map_or(0, |s| self.symbol_table.add_constant_class(s).index);
        if !interfaces.is_empty() {
            self.interfaces = interfaces
                .iter()
                .map(|interface| self.symbol_table.add_constant_class(interface).index)
                .collect();
        }
        if self.compute == Compute::MaxStackAndLocal && version & 0xFFFF >= opcodes::V1_7 {
            self.compute = Compute::MaxStackAndLocalFromFrames;
        }
    }

    fn visit_source(&mut self, file: Option<&JavaStr>, debug: Option<&JavaStr>) {
        if let Some(file) = file {
            self.source_file_index = self.symbol_table.add_constant_utf8(file);
        }
        if let Some(debug) = debug {
            let mut debug_extension = ByteVector::new();
            debug_extension.put_modified_utf8_bytes(debug);
            self.debug_extension = Some(debug_extension);
        }
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        let name_index = self.symbol_table.add_constant_module(name).index;
        let version_index = version.map_or(0, |v| self.symbol_table.add_constant_utf8(v));
        let state =
            self.module_writer
                .insert(ModuleWriterState::new(name_index, access, version_index));
        Some(Box::new(ModuleWriter {
            symbol_table: &mut self.symbol_table,
            state,
        }))
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.nest_host_class_index = self.symbol_table.add_constant_class(nest_host).index;
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        self.enclosing_class_index = self.symbol_table.add_constant_class(owner).index;
        if let (Some(name), Some(descriptor)) = (name, descriptor) {
            self.enclosing_method_index = self
                .symbol_table
                .add_constant_name_and_type(name, descriptor);
        }
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.annotations
            .visit_annotation(&mut self.symbol_table, descriptor, visible)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.annotations.visit_type_annotation(
            &mut self.symbol_table,
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        // Store the attributes in the <i>reverse</i> order of their visit by this method.
        self.attributes.push(attribute);
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        let nest_member_classes = self.nest_member_classes.get_or_insert_with(ByteVector::new);
        self.number_of_nest_member_classes += 1;
        nest_member_classes.put_short(self.symbol_table.add_constant_class(nest_member).index);
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        let permitted_subclasses = self
            .permitted_subclasses
            .get_or_insert_with(ByteVector::new);
        self.number_of_permitted_subclasses += 1;
        permitted_subclasses.put_short(
            self.symbol_table
                .add_constant_class(permitted_subclass)
                .index,
        );
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        let inner_classes = self.inner_classes.get_or_insert_with(ByteVector::new);
        // Section 4.7.6 of the JVMS states "Every CONSTANT_Class_info entry in the
        // constant_pool table which represents a class or interface C that is not a package
        // member must have exactly one corresponding entry in the classes array". To avoid
        // duplicates we keep track in the info field of the Symbol of each CONSTANT_Class_info
        // entry C whether an inner class entry has already been added for C. If so, we store
        // the index of this inner class entry (plus one) in the info field. This trick allows
        // duplicate detection in O(1) time.
        let name_symbol = self.symbol_table.add_constant_class_entry(name);
        if self.symbol_table.symbol(name_symbol).info == 0 {
            self.number_of_inner_classes += 1;
            inner_classes.put_short(self.symbol_table.symbol(name_symbol).index);
            inner_classes
                .put_short(outer_name.map_or(0, |o| self.symbol_table.add_constant_class(o).index));
            inner_classes
                .put_short(inner_name.map_or(0, |i| self.symbol_table.add_constant_utf8(i)));
            inner_classes.put_short(access as i32);
            self.symbol_table.symbol_mut(name_symbol).info = self.number_of_inner_classes;
        }
        // Else, compare the inner classes entry nameSymbol.info - 1 with the arguments of this
        // method and throw an exception if there is a difference?
    }

    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        let state =
            RecordComponentWriterState::new(&mut self.symbol_table, name, descriptor, signature);
        self.record_components.push(state);
        Some(Box::new(RecordComponentWriter {
            symbol_table: &mut self.symbol_table,
            state: self.record_components.last_mut().expect("pushed above"),
        }))
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        let state = FieldWriterState::new(
            &mut self.symbol_table,
            access,
            name,
            descriptor,
            signature,
            value,
        );
        self.fields.push(state);
        Some(Box::new(FieldWriter {
            symbol_table: &mut self.symbol_table,
            state: self.fields.last_mut().expect("pushed above"),
        }))
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let state = MethodWriterState::new(
            &mut self.symbol_table,
            access,
            name,
            descriptor,
            signature,
            exceptions,
            self.compute,
        );
        self.methods.push(state);
        Some(Box::new(MethodWriter {
            symbol_table: &mut self.symbol_table,
            state: self.methods.last_mut().expect("pushed above"),
            hierarchy: self.hierarchy,
        }))
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}
