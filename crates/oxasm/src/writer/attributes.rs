//! The write half of ADR-10: [`AttributeWriteContext`], and the lists of non standard attributes
//! of the writers (ASM's `Attribute` linked lists, `computeAttributesSize`, `putAttributes` and
//! `Attribute.Set`).

use std::sync::Arc;

use crate::attribute::{Attribute, AttributeCodec};
use crate::constant::Constant;
use crate::constants::AttributeName;
use crate::handle::Handle;
use crate::jstr::{JavaStr, JavaString};
use crate::label::Label;
use crate::opcodes;
use crate::writer::byte_vector::ByteVector;
use crate::writer::symbol_table::SymbolTable;

/// What a [`CustomAttribute`](crate::CustomAttribute) can use to write its content: the constant
/// pool of the class being written (ASM's `ClassWriter.new*` methods), and, for a Code
/// attribute, the method's bytecode, maximum stack size and number of locals, and the offsets of
/// its labels.
pub struct AttributeWriteContext<'a> {
    pub(crate) symbol_table: &'a mut SymbolTable,
    pub(crate) code: Option<CodeContext<'a>>,
}

/// The Code attribute that a Code attribute is written in.
#[derive(Clone, Copy)]
pub(crate) struct CodeContext<'a> {
    /// The bytecode of the method.
    pub(crate) code: &'a [u8],
    /// The maximum stack size of the method.
    pub(crate) max_stack: i32,
    /// The maximum number of local variables of the method.
    pub(crate) max_locals: i32,
    /// The bytecode offset of a label of the method, if it is resolved.
    pub(crate) label_offset: &'a dyn Fn(Label) -> Option<i32>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Java's (unsigned short) indices
impl AttributeWriteContext<'_> {
    /// The bytecode of the method, if this is a Code attribute (ASM's `code` argument, which is
    /// `null` otherwise).
    #[must_use]
    pub fn code(&self) -> Option<&[u8]> {
        self.code.map(|c| c.code)
    }

    /// The maximum stack size of the method, if this is a Code attribute (ASM passes -1
    /// otherwise).
    #[must_use]
    pub fn max_stack(&self) -> Option<u16> {
        self.code.map(|c| c.max_stack as u16)
    }

    /// The maximum number of local variables of the method, if this is a Code attribute (ASM
    /// passes -1 otherwise).
    #[must_use]
    pub fn max_locals(&self) -> Option<u16> {
        self.code.map(|c| c.max_locals as u16)
    }

    /// The bytecode offset of the given label, if this is a Code attribute and the label has
    /// been visited (ASM's `Label.getOffset`).
    #[must_use]
    pub fn label_offset(&self, label: Label) -> Option<u16> {
        self.code
            .and_then(|c| (c.label_offset)(label))
            .map(|offset| offset as u16)
    }

    /// Adds a number or string constant to the constant pool of the class being written, if
    /// not already there, and returns its index (ASM's `ClassWriter.newConst`).
    pub fn new_const(&mut self, value: &Constant<'_>) -> u16 {
        self.symbol_table.add_constant(value).index as u16
    }

    /// Adds a UTF8 string to the constant pool, if not already there, and returns its index
    /// (ASM's `ClassWriter.newUTF8`).
    pub fn new_utf8(&mut self, value: &JavaStr) -> u16 {
        self.symbol_table.add_constant_utf8(value) as u16
    }

    /// Adds a class reference to the constant pool, if not already there, and returns its index
    /// (ASM's `ClassWriter.newClass`).
    pub fn new_class(&mut self, value: &JavaStr) -> u16 {
        self.symbol_table.add_constant_class(value).index as u16
    }

    /// Adds a method type reference to the constant pool, if not already there, and returns its
    /// index (ASM's `ClassWriter.newMethodType`).
    pub fn new_method_type(&mut self, method_descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_method_type(method_descriptor)
            .index as u16
    }

    /// Adds a module reference to the constant pool, if not already there, and returns its index
    /// (ASM's `ClassWriter.newModule`).
    pub fn new_module(&mut self, module_name: &JavaStr) -> u16 {
        self.symbol_table.add_constant_module(module_name).index as u16
    }

    /// Adds a package reference to the constant pool, if not already there, and returns its
    /// index (ASM's `ClassWriter.newPackage`).
    pub fn new_package(&mut self, package_name: &JavaStr) -> u16 {
        self.symbol_table.add_constant_package(package_name).index as u16
    }

    /// Adds a handle to the constant pool, if not already there, and returns its index (ASM's
    /// `ClassWriter.newHandle`).
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

    /// Adds a dynamic constant reference to the constant pool, if not already there, and
    /// returns its index (ASM's `ClassWriter.newConstantDynamic`).
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

    /// Adds an invokedynamic reference to the constant pool, if not already there, and returns
    /// its index (ASM's `ClassWriter.newInvokeDynamic`).
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

    /// Adds a field reference to the constant pool, if not already there, and returns its index
    /// (ASM's `ClassWriter.newField`).
    pub fn new_field(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_fieldref(owner, name, descriptor)
            .index as u16
    }

    /// Adds a method reference to the constant pool, if not already there, and returns its
    /// index (ASM's `ClassWriter.newMethod`).
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

    /// Adds a name and type to the constant pool, if not already there, and returns its index
    /// (ASM's `ClassWriter.newNameType`).
    pub fn new_name_type(&mut self, name: &JavaStr, descriptor: &JavaStr) -> u16 {
        self.symbol_table
            .add_constant_name_and_type(name, descriptor) as u16
    }
}

/// A non standard attribute of a writer, with its content once written (ASM's
/// `cachedContent`).
#[derive(Debug)]
struct WriterAttribute {
    attribute: Attribute,
    content: Option<Vec<u8>>,
}

/// The non standard attributes of a class, field, method, Code attribute or record component
/// being written, in visit order (ASM keeps them in a list in the *reverse* order, and writes
/// them in this reverse order).
#[derive(Debug, Default)]
pub(crate) struct AttributeList {
    attributes: Vec<WriterAttribute>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int lengths
impl AttributeList {
    /// Adds a visited attribute.
    pub(crate) fn push(&mut self, attribute: &Attribute) {
        let content = match attribute {
            Attribute::Unknown(a) => Some(a.content.clone()),
            Attribute::Custom(_) => None,
        };
        self.attributes.push(WriterAttribute {
            attribute: attribute.clone(),
            content,
        });
    }

    /// Whether no attribute was visited.
    pub(crate) fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// The number of attributes (ASM's `getAttributeCount`).
    pub(crate) fn len(&self) -> usize {
        self.attributes.len()
    }

    /// The content of the given attribute, written if not already done (ASM's `maybeWrite`).
    fn maybe_write<'c>(
        attribute: &'c mut WriterAttribute,
        symbol_table: &mut SymbolTable,
        code: Option<CodeContext<'_>>,
    ) -> &'c [u8] {
        if attribute.content.is_none() {
            let Attribute::Custom(custom) = &attribute.attribute else {
                unreachable!("unknown attributes have a content")
            };
            let mut cx = AttributeWriteContext { symbol_table, code };
            attribute.content = Some(custom.write(&mut cx));
        }
        attribute.content.as_deref().expect("written above")
    }

    /// Returns the size of all the attributes of this list, including their 6 header bytes, and
    /// adds their names to the constant pool (ASM's `computeAttributesSize`).
    pub(crate) fn compute_attributes_size(
        &mut self,
        symbol_table: &mut SymbolTable,
        code: Option<CodeContext<'_>>,
    ) -> usize {
        let mut size = 0;
        for attribute in self.attributes.iter_mut().rev() {
            symbol_table.add_constant_utf8(attribute.attribute.name());
            size += 6 + Self::maybe_write(attribute, symbol_table, code).len();
        }
        size
    }

    /// Puts all the attributes of this list in the given byte vector, including their 6 header
    /// bytes (ASM's `putAttributes`).
    pub(crate) fn put_attributes(
        &mut self,
        symbol_table: &mut SymbolTable,
        code: Option<CodeContext<'_>>,
        output: &mut ByteVector,
    ) {
        for attribute in self.attributes.iter_mut().rev() {
            let length = Self::maybe_write(attribute, symbol_table, code).len();
            // Put attribute_name_index and attribute_length.
            let name_index = symbol_table.add_constant_utf8(attribute.attribute.name());
            output.put_short(name_index).put_int(length as i32);
            output.put_byte_array(attribute.content.as_deref().expect("written above"));
        }
    }

    /// Adds the codecs of the attributes of this list to the given set of prototypes, in ASM's
    /// order (ASM's `Attribute.Set.addAttributes`).
    pub(crate) fn collect_attribute_prototypes(&self, prototypes: &mut AttributePrototypes) {
        for attribute in self.attributes.iter().rev() {
            prototypes.add(&attribute.attribute);
        }
    }
}

/// The attribute "prototypes" of a class writer, to re-read the class it wrote (ASM's
/// `Attribute.Set`): one per attribute name, the first one found.
#[derive(Default)]
pub(crate) struct AttributePrototypes {
    names: Vec<JavaString>,
    codecs: Vec<Arc<dyn AttributeCodec>>,
}

impl AttributePrototypes {
    /// Adds the codec of the given attribute, unless an attribute of the same name was already
    /// added (an unknown attribute counts, but has no codec).
    fn add(&mut self, attribute: &Attribute) {
        let name = attribute.name();
        if self.names.iter().any(|n| **n == *name) {
            return;
        }
        self.names.push(name.to_owned());
        if let Attribute::Custom(custom) = attribute
            && let Some(codec) = custom.codec()
        {
            self.codecs.push(codec);
        }
    }

    /// The codecs.
    pub(crate) fn codecs(&self) -> &[Arc<dyn AttributeCodec>] {
        &self.codecs
    }
}

/// Returns the size of the Synthetic, Signature and Deprecated attributes of a field, method or
/// record component (ASM's static `Attribute.computeAttributesSize`), and adds their names to
/// the constant pool.
pub(crate) fn compute_standard_attributes_size(
    symbol_table: &mut SymbolTable,
    access_flags: u32,
    signature_index: i32,
) -> usize {
    let mut size = 0;
    // Before Java 1.5, synthetic fields are represented with a Synthetic attribute.
    if access_flags & opcodes::ACC_SYNTHETIC != 0
        && symbol_table.major_version() < opcodes::V1_5 as i32
    {
        // Synthetic attributes always use 6 bytes.
        symbol_table.add_attribute_name(AttributeName::Synthetic);
        size += 6;
    }
    if signature_index != 0 {
        // Signature attributes always use 8 bytes.
        symbol_table.add_attribute_name(AttributeName::Signature);
        size += 8;
    }
    // ACC_DEPRECATED is ASM specific, the ClassFile format uses a Deprecated attribute instead.
    if access_flags & opcodes::ACC_DEPRECATED != 0 {
        // Deprecated attributes always use 6 bytes.
        symbol_table.add_attribute_name(AttributeName::Deprecated);
        size += 6;
    }
    size
}

impl std::fmt::Debug for AttributeWriteContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttributeWriteContext")
            .field("code_length", &self.code.map(|c| c.code.len()))
            .finish_non_exhaustive()
    }
}

/// Puts the Synthetic, Signature and Deprecated attributes of a field, method or record
/// component in the given byte vector (ASM's static `Attribute.putAttributes`).
pub(crate) fn put_standard_attributes(
    symbol_table: &mut SymbolTable,
    access_flags: u32,
    signature_index: i32,
    output: &mut ByteVector,
) {
    // Before Java 1.5, synthetic fields are represented with a Synthetic attribute.
    if access_flags & opcodes::ACC_SYNTHETIC != 0
        && symbol_table.major_version() < opcodes::V1_5 as i32
    {
        let name_index = symbol_table.add_attribute_name(AttributeName::Synthetic);
        output.put_short(name_index).put_int(0);
    }
    if signature_index != 0 {
        let name_index = symbol_table.add_attribute_name(AttributeName::Signature);
        output
            .put_short(name_index)
            .put_int(2)
            .put_short(signature_index);
    }
    if access_flags & opcodes::ACC_DEPRECATED != 0 {
        let name_index = symbol_table.add_attribute_name(AttributeName::Deprecated);
        output.put_short(name_index).put_int(0);
    }
}
