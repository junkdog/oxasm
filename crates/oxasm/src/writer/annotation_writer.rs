//! [`AnnotationWriter`]: ASM's `AnnotationWriter`, which writes the `annotation`, `type_annotation`
//! and `element_value` JVMS structures of an annotation.
//!
//! ASM links the annotations of an attribute in a list of writers, each with its own byte
//! vector. Here the owner of an attribute keeps an [`AnnotationList`] of these byte vectors, and
//! an annotation writer borrows the one it fills (and the symbol table). Nested annotation and
//! array writers fill their parent's byte vector, as in ASM.

use crate::constant::AnnotationValue;
use crate::error::WriteError;
use crate::jstr::JavaStr;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::AnnotationVisitor;
use crate::writer::byte_vector::ByteVector;
use crate::writer::symbol_table::SymbolTable;

/// The annotations of one `Runtime[In]Visible[Type]Annotations` attribute (or of one parameter
/// of a `Runtime[In]VisibleParameterAnnotations` attribute), in visit order: each one an
/// `annotation` or `type_annotation` structure.
pub(crate) type AnnotationList = Vec<ByteVector>;

/// An [`AnnotationVisitor`] that generates a corresponding `annotation` or `type_annotation`
/// structure, as defined in the Java Virtual Machine Specification (JVMS).
pub(crate) struct AnnotationWriter<'a> {
    /// The constant pool to which this annotation must be added.
    symbol_table: &'a mut SymbolTable,
    /// Whether values are named or not. AnnotationWriter instances used for annotation default
    /// and annotation arrays use unnamed values (i.e. they generate an `element_value`
    /// structure for each value, instead of an `element_name_index` followed by an
    /// `element_value`).
    use_named_values: bool,
    /// The `annotation` or `type_annotation` JVMS structure corresponding to the annotation
    /// values visited so far. All the fields of these structures, except the last one - the
    /// `element_value_pairs` array, must be set before this writer is constructed. The
    /// `element_value_pairs` array is filled incrementally in the various visit methods.
    ///
    /// Note: as an exception to the above rules, for AnnotationDefault attributes (which contain
    /// a single `element_value` by definition), this byte vector is initially empty when passed
    /// to the constructor, and `num_element_value_pairs_offset` is set to `None`.
    annotation: &'a mut ByteVector,
    /// The offset in `annotation` where `num_element_value_pairs` must be stored (or `None` for
    /// the case of AnnotationDefault attributes).
    num_element_value_pairs_offset: Option<usize>,
    /// The number of element value pairs visited so far.
    num_element_value_pairs: i32,
    /// Whether the count is stored when this writer is dropped: ASM's attribute writers call
    /// `visitEnd` on the annotations of their lists "in case the user forgot", but not on nested
    /// annotations and arrays.
    end_on_drop: bool,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl<'a> AnnotationWriter<'a> {
    /// Constructs a new annotation writer. By hypothesis, `num_element_value_pairs` is stored
    /// in the last unsigned short of `annotation`, if it is not empty.
    pub(crate) fn new(
        symbol_table: &'a mut SymbolTable,
        use_named_values: bool,
        annotation: &'a mut ByteVector,
        end_on_drop: bool,
    ) -> AnnotationWriter<'a> {
        let num_element_value_pairs_offset = annotation.size().checked_sub(2);
        AnnotationWriter {
            symbol_table,
            use_named_values,
            annotation,
            num_element_value_pairs_offset,
            num_element_value_pairs: 0,
            end_on_drop,
        }
    }

    /// Creates a new annotation writer for an `annotation` of the given type, appended to
    /// `annotations` (ASM's `create(symbolTable, descriptor, previousAnnotation)`).
    pub(crate) fn create(
        symbol_table: &'a mut SymbolTable,
        descriptor: &JavaStr,
        annotations: &'a mut AnnotationList,
    ) -> AnnotationWriter<'a> {
        // Create a ByteVector to hold an 'annotation' JVMS structure.
        // See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.16.
        let mut annotation = ByteVector::new();
        // Write type_index and reserve space for num_element_value_pairs.
        annotation
            .put_short(symbol_table.add_constant_utf8(descriptor))
            .put_short(0);
        annotations.push(annotation);
        let annotation = annotations.last_mut().expect("pushed above");
        AnnotationWriter::new(symbol_table, true, annotation, true)
    }

    /// Creates a new annotation writer for a `type_annotation` of the given type, appended to
    /// `annotations` (ASM's `create(symbolTable, typeRef, typePath, descriptor,
    /// previousAnnotation)`), or records ASM's `IllegalArgumentException` and returns `None` for
    /// a type reference of an unknown sort.
    pub(crate) fn create_type_annotation(
        symbol_table: &'a mut SymbolTable,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        annotations: &'a mut AnnotationList,
    ) -> Option<AnnotationWriter<'a>> {
        // Create a ByteVector to hold a 'type_annotation' JVMS structure.
        // See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.20.
        let mut type_annotation = ByteVector::new();
        // Write target_type, target_info, and target_path.
        if let Err(error) = put_target(type_ref.get_value() as i32, &mut type_annotation) {
            symbol_table.fail(error);
            return None;
        }
        put_type_path(type_path, &mut type_annotation);
        // Write type_index and reserve space for num_element_value_pairs.
        type_annotation
            .put_short(symbol_table.add_constant_utf8(descriptor))
            .put_short(0);
        annotations.push(type_annotation);
        let annotation = annotations.last_mut().expect("pushed above");
        Some(AnnotationWriter::new(symbol_table, true, annotation, true))
    }

    /// Puts the `element_name_index` of a value, if values are named.
    fn put_name(&mut self, name: Option<&JavaStr>) {
        if self.use_named_values {
            let Some(name) = name else {
                // ASM's NullPointerException on addConstantUtf8(null).
                self.symbol_table.fail(WriteError::InvalidArgument(
                    "a named annotation value without a name".to_owned(),
                ));
                return;
            };
            let name_index = self.symbol_table.add_constant_utf8(name);
            self.annotation.put_short(name_index);
        }
    }

    /// Puts an `element_value` of the given tag, whose `const_value_index` is an integer
    /// constant.
    fn put_integer(&mut self, tag: u8, value: i32) {
        let index = self.symbol_table.add_constant_integer(value).index;
        self.annotation.put12(i32::from(tag), index);
    }

    /// Puts an array `element_value` of the given length, whose elements are put by
    /// `put_element`.
    fn put_array<T: Copy>(&mut self, values: &[T], mut put_element: impl FnMut(&mut Self, T)) {
        self.annotation.put12(i32::from(b'['), values.len() as i32);
        for &value in values {
            put_element(self, value);
        }
    }

    /// Stores the number of element value pairs (ASM's `visitEnd`).
    fn end(&mut self) {
        if let Some(offset) = self.num_element_value_pairs_offset {
            let data = self.annotation.data_mut();
            data[offset] = (self.num_element_value_pairs >> 8) as u8;
            data[offset + 1] = self.num_element_value_pairs as u8;
        }
    }
}

impl Drop for AnnotationWriter<'_> {
    fn drop(&mut self) {
        if self.end_on_drop {
            self.end();
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl AnnotationVisitor for AnnotationWriter<'_> {
    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        // Case of an element_value with a const_value_index, class_info_index or array_index
        // field. See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.16.1.
        self.num_element_value_pairs += 1;
        self.put_name(name);
        match value {
            AnnotationValue::String(value) => {
                let index = self.symbol_table.add_constant_utf8(value);
                self.annotation.put12(i32::from(b's'), index);
            }
            AnnotationValue::Byte(value) => self.put_integer(b'B', i32::from(*value)),
            AnnotationValue::Boolean(value) => self.put_integer(b'Z', i32::from(*value)),
            AnnotationValue::Char(value) => self.put_integer(b'C', i32::from(*value)),
            AnnotationValue::Short(value) => self.put_integer(b'S', i32::from(*value)),
            AnnotationValue::Type(value) => {
                let index = self.symbol_table.add_constant_utf8(&value.get_descriptor());
                self.annotation.put12(i32::from(b'c'), index);
            }
            AnnotationValue::ByteArray(values) => {
                self.put_array(values, |w, v| w.put_integer(b'B', i32::from(v)));
            }
            AnnotationValue::BooleanArray(values) => {
                self.put_array(values, |w, v| w.put_integer(b'Z', i32::from(v)));
            }
            AnnotationValue::ShortArray(values) => {
                self.put_array(values, |w, v| w.put_integer(b'S', i32::from(v)));
            }
            AnnotationValue::CharArray(values) => {
                self.put_array(values, |w, v| w.put_integer(b'C', i32::from(v)));
            }
            AnnotationValue::IntArray(values) => {
                self.put_array(values, |w, v| w.put_integer(b'I', v));
            }
            AnnotationValue::LongArray(values) => self.put_array(values, |w, v| {
                let index = w.symbol_table.add_constant_long(v).index;
                w.annotation.put12(i32::from(b'J'), index);
            }),
            AnnotationValue::FloatArray(values) => self.put_array(values, |w, v| {
                let index = w.symbol_table.add_constant_float(v).index;
                w.annotation.put12(i32::from(b'F'), index);
            }),
            AnnotationValue::DoubleArray(values) => self.put_array(values, |w, v| {
                let index = w.symbol_table.add_constant_double(v).index;
                w.annotation.put12(i32::from(b'D'), index);
            }),
            // ASM's `symbolTable.addConstant(value)` and `".s.IFJDCS".charAt(symbol.tag)`.
            AnnotationValue::Int(value) => self.put_integer(b'I', *value),
            AnnotationValue::Long(value) => {
                let index = self.symbol_table.add_constant_long(*value).index;
                self.annotation.put12(i32::from(b'J'), index);
            }
            AnnotationValue::Float(value) => {
                let index = self.symbol_table.add_constant_float(*value).index;
                self.annotation.put12(i32::from(b'F'), index);
            }
            AnnotationValue::Double(value) => {
                let index = self.symbol_table.add_constant_double(*value).index;
                self.annotation.put12(i32::from(b'D'), index);
            }
        }
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        // Case of an element_value with an enum_const_value field.
        // See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.16.1.
        self.num_element_value_pairs += 1;
        self.put_name(name);
        let descriptor_index = self.symbol_table.add_constant_utf8(descriptor);
        let value_index = self.symbol_table.add_constant_utf8(value);
        self.annotation
            .put12(i32::from(b'e'), descriptor_index)
            .put_short(value_index);
    }

    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        // Case of an element_value with an annotation_value field.
        // See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.16.1.
        self.num_element_value_pairs += 1;
        self.put_name(name);
        // Write tag and type_index, and reserve 2 bytes for num_element_value_pairs.
        let descriptor_index = self.symbol_table.add_constant_utf8(descriptor);
        self.annotation
            .put12(i32::from(b'@'), descriptor_index)
            .put_short(0);
        Some(Box::new(AnnotationWriter::new(
            self.symbol_table,
            true,
            self.annotation,
            false,
        )))
    }

    fn visit_array<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        // Case of an element_value with an array_value field.
        // https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.16.1
        self.num_element_value_pairs += 1;
        self.put_name(name);
        // Write tag, and reserve 2 bytes for num_values. Here we take advantage of the fact that
        // the end of an element_value of array type is similar to the end of an 'annotation'
        // structure: an unsigned short num_values followed by num_values element_value, versus
        // an unsigned short num_element_value_pairs, followed by num_element_value_pairs {
        // element_name_index, element_value } tuples. This allows us to use an AnnotationWriter
        // with unnamed values to visit the array elements. Its num_element_value_pairs will
        // correspond to the number of array elements and will be stored in what is in fact
        // num_values.
        self.annotation.put12(i32::from(b'['), 0);
        Some(Box::new(AnnotationWriter::new(
            self.symbol_table,
            false,
            self.annotation,
            false,
        )))
    }

    fn visit_end(&mut self) {
        self.end();
    }
}

// -------------------------------------------------------------------------------------------------
// Utility methods
// -------------------------------------------------------------------------------------------------

/// Returns the size of a `Runtime[In]Visible[Type]Annotations` attribute containing the given
/// annotations, and adds the attribute name to the constant pool if `attribute_name` is not
/// `None` (ASM's `computeAnnotationsSize`).
pub(crate) fn compute_annotations_size(
    symbol_table: &mut SymbolTable,
    annotations: &AnnotationList,
    attribute_name: Option<&JavaStr>,
) -> usize {
    if let Some(attribute_name) = attribute_name {
        symbol_table.add_constant_utf8(attribute_name);
    }
    // The attribute_name_index, attribute_length and num_annotations fields use 8 bytes.
    8 + annotations.iter().map(ByteVector::size).sum::<usize>()
}

/// Returns the size of the four `Runtime[In]Visible[Type]Annotations` attributes of a member,
/// for the lists that are not empty, and adds their names to the constant pool.
pub(crate) fn compute_member_annotations_size(
    symbol_table: &mut SymbolTable,
    annotations: &MemberAnnotations,
) -> usize {
    let mut size = 0;
    for (list, name) in annotations.lists() {
        if !list.is_empty() {
            size += compute_annotations_size(symbol_table, list, Some(name));
        }
    }
    size
}

/// Puts a `Runtime[In]Visible[Type]Annotations` attribute containing the given annotations in
/// the given byte vector (ASM's `putAnnotations`).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int lengths
pub(crate) fn put_annotations(
    annotations: &AnnotationList,
    attribute_name_index: i32,
    output: &mut ByteVector,
) {
    // For num_annotations.
    let attribute_length = 2 + annotations.iter().map(ByteVector::size).sum::<usize>();
    output.put_short(attribute_name_index);
    output.put_int(attribute_length as i32);
    output.put_short(annotations.len() as i32);
    for annotation in annotations {
        output.put_byte_array(annotation.data());
    }
}

/// Puts the four `Runtime[In]Visible[Type]Annotations` attributes of a member in the given byte
/// vector, for the lists that are not empty.
pub(crate) fn put_member_annotations(
    symbol_table: &mut SymbolTable,
    annotations: &MemberAnnotations,
    output: &mut ByteVector,
) {
    for (list, name) in annotations.lists() {
        if !list.is_empty() {
            let name_index = symbol_table.add_constant_utf8(name);
            put_annotations(list, name_index, output);
        }
    }
}

/// Returns the size of a `Runtime[In]VisibleParameterAnnotations` attribute containing the
/// given annotations, and adds the attribute name to the constant pool (ASM's
/// `computeParameterAnnotationsSize`).
pub(crate) fn compute_parameter_annotations_size(
    symbol_table: &mut SymbolTable,
    attribute_name: &JavaStr,
    annotations: &[AnnotationList],
    annotable_parameter_count: usize,
) -> usize {
    // Note: attributeName is added to the constant pool by the call to computeAnnotationsSize
    // below. This assumes that there is at least one non-null element in the annotationWriters
    // sub-array (which is ensured by the lazy instantiation of this array in MethodWriter).
    // The attribute_name_index, attribute_length and num_parameters fields use 7 bytes, and each
    // element of the parameter_annotations array uses 2 bytes for its num_annotations field.
    let mut attribute_size = 7 + 2 * annotable_parameter_count;
    for list in parameter_lists(annotations, annotable_parameter_count) {
        if let Some(list) = list
            && !list.is_empty()
        {
            attribute_size +=
                compute_annotations_size(symbol_table, list, Some(attribute_name)) - 8;
        }
    }
    attribute_size
}

/// Puts a `Runtime[In]VisibleParameterAnnotations` attribute containing the given annotations
/// in the given byte vector (ASM's `putParameterAnnotations`).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int lengths
pub(crate) fn put_parameter_annotations(
    attribute_name_index: i32,
    annotations: &[AnnotationList],
    annotable_parameter_count: usize,
    output: &mut ByteVector,
) {
    // The num_parameters field uses 1 byte, and each element of the parameter_annotations array
    // uses 2 bytes for its num_annotations field.
    let mut attribute_length = 1 + 2 * annotable_parameter_count;
    for list in parameter_lists(annotations, annotable_parameter_count).flatten() {
        attribute_length += list.iter().map(ByteVector::size).sum::<usize>();
    }
    output.put_short(attribute_name_index);
    output.put_int(attribute_length as i32);
    output.put_byte(annotable_parameter_count as i32);
    for list in parameter_lists(annotations, annotable_parameter_count) {
        let list = list.map_or(&[][..], Vec::as_slice);
        output.put_short(list.len() as i32);
        for annotation in list {
            output.put_byte_array(annotation.data());
        }
    }
}

/// The annotation lists of the first `count` parameters (`None` past the end of `annotations`,
/// where ASM fails with an `ArrayIndexOutOfBoundsException`, which the method writer checks
/// before).
fn parameter_lists(
    annotations: &[AnnotationList],
    count: usize,
) -> impl Iterator<Item = Option<&AnnotationList>> {
    (0..count).map(|i| annotations.get(i))
}

/// The four annotation lists of a class, field, method or record component (ASM's
/// `lastRuntime[In]Visible[Type]Annotation` fields).
#[derive(Debug, Default)]
pub(crate) struct MemberAnnotations {
    pub(crate) visible: AnnotationList,
    pub(crate) invisible: AnnotationList,
    pub(crate) visible_type: AnnotationList,
    pub(crate) invisible_type: AnnotationList,
}

impl MemberAnnotations {
    /// The lists, with their attribute names, in ASM's order.
    fn lists(&self) -> [(&AnnotationList, &'static JavaStr); 4] {
        use crate::constants::{
            RUNTIME_INVISIBLE_ANNOTATIONS, RUNTIME_INVISIBLE_TYPE_ANNOTATIONS,
            RUNTIME_VISIBLE_ANNOTATIONS, RUNTIME_VISIBLE_TYPE_ANNOTATIONS,
        };
        [
            (&self.visible, RUNTIME_VISIBLE_ANNOTATIONS),
            (&self.invisible, RUNTIME_INVISIBLE_ANNOTATIONS),
            (&self.visible_type, RUNTIME_VISIBLE_TYPE_ANNOTATIONS),
            (&self.invisible_type, RUNTIME_INVISIBLE_TYPE_ANNOTATIONS),
        ]
    }

    /// The number of attributes these annotations make.
    pub(crate) fn attribute_count(&self) -> usize {
        self.lists().iter().filter(|(l, _)| !l.is_empty()).count()
    }

    /// A writer for a new annotation (ASM's `visitAnnotation` of a member writer).
    pub(crate) fn visit_annotation<'a>(
        &'a mut self,
        symbol_table: &'a mut SymbolTable,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let list = if visible {
            &mut self.visible
        } else {
            &mut self.invisible
        };
        Some(Box::new(AnnotationWriter::create(
            symbol_table,
            descriptor,
            list,
        )))
    }

    /// A writer for a new type annotation (ASM's `visitTypeAnnotation` of a member writer).
    pub(crate) fn visit_type_annotation<'a>(
        &'a mut self,
        symbol_table: &'a mut SymbolTable,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let list = if visible {
            &mut self.visible_type
        } else {
            &mut self.invisible_type
        };
        AnnotationWriter::create_type_annotation(
            symbol_table,
            type_ref,
            type_path,
            descriptor,
            list,
        )
        .map(|w| Box::new(w) as Box<dyn AnnotationVisitor + 'a>)
    }
}

/// Puts the `target_type` and `target_info` JVMS structures corresponding to the given type
/// reference (ASM's `TypeReference.putTarget`).
pub(crate) fn put_target(
    target_type_and_info: i32,
    output: &mut ByteVector,
) -> Result<(), WriteError> {
    let target_type = (target_type_and_info as u32 >> 24) as u8;
    match target_type {
        TypeReference::CLASS_TYPE_PARAMETER
        | TypeReference::METHOD_TYPE_PARAMETER
        | TypeReference::METHOD_FORMAL_PARAMETER => {
            output.put_short((target_type_and_info as u32 >> 16) as i32);
        }
        TypeReference::FIELD | TypeReference::METHOD_RETURN | TypeReference::METHOD_RECEIVER => {
            output.put_byte(i32::from(target_type));
        }
        TypeReference::CAST
        | TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT
        | TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT
        | TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT
        | TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT => {
            output.put_int(target_type_and_info);
        }
        TypeReference::CLASS_EXTENDS
        | TypeReference::CLASS_TYPE_PARAMETER_BOUND
        | TypeReference::METHOD_TYPE_PARAMETER_BOUND
        | TypeReference::THROWS
        | TypeReference::EXCEPTION_PARAMETER
        | TypeReference::INSTANCEOF
        | TypeReference::NEW
        | TypeReference::CONSTRUCTOR_REFERENCE
        | TypeReference::METHOD_REFERENCE => {
            output.put12(
                i32::from(target_type),
                (target_type_and_info & 0x00FF_FF00) >> 8,
            );
        }
        _ => return Err(WriteError::InvalidArgument(String::new())),
    }
    Ok(())
}

/// Puts the given type path in the given byte vector (ASM's `TypePath.put`).
pub(crate) fn put_type_path(type_path: Option<&TypePath>, output: &mut ByteVector) {
    match type_path {
        None => {
            output.put_byte(0);
        }
        Some(type_path) => {
            output.put_byte_array(type_path.as_structure());
        }
    }
}
