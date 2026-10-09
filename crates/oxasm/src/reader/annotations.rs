//! ASM's `ClassReader` section "Methods to parse annotations, type annotations and parameter
//! annotations".

use std::borrow::Cow;

use crate::constant::{AnnotationValue, Constant};
use crate::jstr::JavaStr;
use crate::ty::Type;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::{
    AnnotationVisitor, ClassVisitor, FieldVisitor, MethodVisitor, RecordComponentVisitor,
};

use super::context::Context;
use super::{ClassReader, ReadError, ReadErrorKind, required};

/// ASM 9.11's bound on the nesting of annotation values ("Too many nested annotations").
const MAX_ANNOTATION_DEPTH: u32 = 256;

/// A visitor with `visit_annotation` and `visit_type_annotation` methods: the class, field,
/// method and record component visitors, whose annotation attributes are read alike.
pub(super) trait AnnotationHost {
    fn host_visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>>;

    fn host_visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>>;
}

macro_rules! annotation_host {
    ($($visitor:ident),*) => {$(
        impl AnnotationHost for dyn $visitor + '_ {
            fn host_visit_annotation<'a>(
                &'a mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                self.visit_annotation(descriptor, visible)
            }

            fn host_visit_type_annotation<'a>(
                &'a mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                self.visit_type_annotation(type_ref, type_path, descriptor, visible)
            }
        }
    )*};
}

annotation_host!(
    ClassVisitor,
    FieldVisitor,
    MethodVisitor,
    RecordComponentVisitor
);

impl<'b> ClassReader<'b> {
    /// Visits the `Runtime[In]VisibleAnnotations` attribute at `offset`, if not 0 (the loop that
    /// ASM repeats for classes, fields, methods and record components).
    pub(super) fn read_annotations_attribute<H: AnnotationHost + ?Sized>(
        &self,
        host: &mut H,
        offset: i64,
        visible: bool,
    ) -> Result<(), ReadError> {
        if offset == 0 {
            return Ok(());
        }
        let num_annotations = self.u16_at(offset)?;
        let mut current_annotation_offset = offset + 2;
        for _ in 0..num_annotations {
            // Parse the type_index field.
            let annotation_descriptor = self.utf8_at(current_annotation_offset)?;
            let annotation_descriptor = required(annotation_descriptor, current_annotation_offset)?;
            current_annotation_offset += 2;
            // Parse num_element_value_pairs and element_value_pairs and visit these values.
            let mut annotation_visitor = host.host_visit_annotation(annotation_descriptor, visible);
            current_annotation_offset = self.read_element_values(
                annotation_visitor.as_deref_mut(),
                current_annotation_offset,
                /* named= */ true,
                0,
            )?;
        }
        Ok(())
    }

    /// Visits the `Runtime[In]VisibleTypeAnnotations` attribute at `offset`, if not 0 (the loop
    /// that ASM repeats for classes, fields, methods and record components).
    pub(super) fn read_type_annotations_attribute<H: AnnotationHost + ?Sized>(
        &self,
        host: &mut H,
        context: &mut Context<'_>,
        offset: i64,
        visible: bool,
    ) -> Result<(), ReadError> {
        if offset == 0 {
            return Ok(());
        }
        let num_annotations = self.u16_at(offset)?;
        let mut current_annotation_offset = offset + 2;
        for _ in 0..num_annotations {
            // Parse the target_type, target_info and target_path fields.
            current_annotation_offset =
                self.read_type_annotation_target(context, current_annotation_offset)?;
            // Parse the type_index field.
            let annotation_descriptor = self.utf8_at(current_annotation_offset)?;
            let annotation_descriptor = required(annotation_descriptor, current_annotation_offset)?;
            current_annotation_offset += 2;
            // Parse num_element_value_pairs and element_value_pairs and visit these values.
            let mut annotation_visitor = host.host_visit_type_annotation(
                TypeReference::new(context.current_type_annotation_target),
                context.current_type_annotation_target_path.as_ref(),
                annotation_descriptor,
                visible,
            );
            current_annotation_offset = self.read_element_values(
                annotation_visitor.as_deref_mut(),
                current_annotation_offset,
                /* named= */ true,
                0,
            )?;
        }
        Ok(())
    }

    /// Parses a `Runtime[In]VisibleTypeAnnotations` attribute of a Code attribute to find the
    /// offset of each `type_annotation` entry it contains, to find the corresponding labels, and
    /// to visit the try catch block annotations (ASM's `readTypeAnnotations`).
    ///
    /// Returns the start offset of each entry of the `Runtime[In]VisibleTypeAnnotations_attribute`
    /// 'annotations' array field.
    pub(super) fn read_type_annotations(
        &self,
        method_visitor: &mut dyn MethodVisitor,
        context: &mut Context<'_>,
        runtime_type_annotations_offset: i64,
        visible: bool,
    ) -> Result<Vec<i64>, ReadError> {
        let mut current_offset = runtime_type_annotations_offset;
        // Read the num_annotations field and create an array to store the type_annotation
        // offsets.
        let num_annotations = self.u16_at(current_offset)?;
        let mut type_annotations_offsets = Vec::with_capacity(usize::from(num_annotations));
        current_offset += 2;
        // Parse the 'annotations' array field.
        for _ in 0..num_annotations {
            type_annotations_offsets.push(current_offset);
            // Parse the type_annotation's target_type and the target_info fields. The size of the
            // target_info field depends on the value of target_type.
            let target_type = self.i32_at(current_offset)? as u32;
            match (target_type >> 24) as u8 {
                TypeReference::LOCAL_VARIABLE | TypeReference::RESOURCE_VARIABLE => {
                    // A localvar_target has a variable size, which depends on the value of their
                    // table_length field. It also references bytecode offsets, for which we need
                    // labels.
                    let table_length = self.u16_at(current_offset + 1)?;
                    current_offset += 3;
                    for _ in 0..table_length {
                        let start_pc = i64::from(self.u16_at(current_offset)?);
                        let length = i64::from(self.u16_at(current_offset + 2)?);
                        // Skip the index field (2 bytes).
                        current_offset += 6;
                        context.current_method_labels.create_label(start_pc)?;
                        context
                            .current_method_labels
                            .create_label(start_pc + length)?;
                    }
                }
                TypeReference::CAST
                | TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT
                | TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT
                | TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT
                | TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT => current_offset += 4,
                TypeReference::CLASS_EXTENDS
                | TypeReference::CLASS_TYPE_PARAMETER_BOUND
                | TypeReference::METHOD_TYPE_PARAMETER_BOUND
                | TypeReference::THROWS
                | TypeReference::EXCEPTION_PARAMETER
                | TypeReference::INSTANCEOF
                | TypeReference::NEW
                | TypeReference::CONSTRUCTOR_REFERENCE
                | TypeReference::METHOD_REFERENCE => current_offset += 3,
                // TypeReference type which can't be used in Code attribute, or which is unknown.
                sort => {
                    return Err(ReadError::new(
                        current_offset,
                        ReadErrorKind::InvalidTypeAnnotationTarget(sort),
                    ));
                }
            }
            // Parse the rest of the type_annotation structure, starting with the target_path
            // structure (whose size depends on its path_length field).
            let path_length = i64::from(self.u8_at(current_offset)?);
            if (target_type >> 24) as u8 == TypeReference::EXCEPTION_PARAMETER {
                // Parse the target_path structure and create a corresponding TypePath.
                let path = self.type_path_at(current_offset, path_length)?;
                current_offset += 1 + 2 * path_length;
                // Parse the type_index field.
                let annotation_descriptor = self.utf8_at(current_offset)?;
                let annotation_descriptor = required(annotation_descriptor, current_offset)?;
                current_offset += 2;
                // Parse num_element_value_pairs and element_value_pairs and visit these values.
                let mut annotation_visitor = method_visitor.visit_try_catch_annotation(
                    TypeReference::new(target_type & 0xFFFF_FF00),
                    path.as_ref(),
                    annotation_descriptor,
                    visible,
                );
                current_offset = self.read_element_values(
                    annotation_visitor.as_deref_mut(),
                    current_offset,
                    /* named= */ true,
                    0,
                )?;
            } else {
                // We don't want to visit the other target_type annotations, so we just skip them
                // (which requires some parsing because the element_value_pairs array has a
                // variable size). First, skip the target_path structure:
                current_offset += 3 + 2 * path_length;
                // Then skip the num_element_value_pairs and element_value_pairs fields (by
                // reading them with a null AnnotationVisitor).
                current_offset = self.read_element_values(
                    /* annotation_visitor= */ None,
                    current_offset,
                    /* named= */ true,
                    0,
                )?;
            }
        }
        Ok(type_annotations_offsets)
    }

    /// Returns the bytecode offset corresponding to the specified JVMS 'type_annotation'
    /// structure, or -1 if there is no such type_annotation of if it does not have a bytecode
    /// offset.
    pub(super) fn get_type_annotation_bytecode_offset(
        &self,
        type_annotation_offsets: Option<&[i64]>,
        type_annotation_index: usize,
    ) -> Result<i32, ReadError> {
        let Some(&offset) = type_annotation_offsets.and_then(|o| o.get(type_annotation_index))
        else {
            return Ok(-1);
        };
        if self.u8_at(offset)? < TypeReference::INSTANCEOF {
            return Ok(-1);
        }
        Ok(i32::from(self.u16_at(offset + 1)?))
    }

    /// Parses the header of a JVMS type_annotation structure to extract its target_type,
    /// target_info and target_path (the result is stored in the given context), and returns the
    /// start offset of the rest of the type_annotation structure.
    pub(super) fn read_type_annotation_target(
        &self,
        context: &mut Context<'_>,
        type_annotation_offset: i64,
    ) -> Result<i64, ReadError> {
        let mut current_offset = type_annotation_offset;
        // Parse and store the target_type structure.
        let mut target_type = self.i32_at(type_annotation_offset)? as u32;
        match (target_type >> 24) as u8 {
            TypeReference::CLASS_TYPE_PARAMETER
            | TypeReference::METHOD_TYPE_PARAMETER
            | TypeReference::METHOD_FORMAL_PARAMETER => {
                target_type &= 0xFFFF_0000;
                current_offset += 2;
            }
            TypeReference::FIELD
            | TypeReference::METHOD_RETURN
            | TypeReference::METHOD_RECEIVER => {
                target_type &= 0xFF00_0000;
                current_offset += 1;
            }
            TypeReference::LOCAL_VARIABLE | TypeReference::RESOURCE_VARIABLE => {
                target_type &= 0xFF00_0000;
                let table_length = self.u16_at(current_offset + 1)?;
                current_offset += 3;
                let mut starts = Vec::with_capacity(usize::from(table_length));
                let mut ends = Vec::with_capacity(usize::from(table_length));
                let mut indices = Vec::with_capacity(usize::from(table_length));
                for _ in 0..table_length {
                    let start_pc = i64::from(self.u16_at(current_offset)?);
                    let length = i64::from(self.u16_at(current_offset + 2)?);
                    let index = self.u16_at(current_offset + 4)?;
                    current_offset += 6;
                    starts.push(context.current_method_labels.create_label(start_pc)?);
                    ends.push(
                        context
                            .current_method_labels
                            .create_label(start_pc + length)?,
                    );
                    indices.push(index);
                }
                context.current_local_variable_annotation_range_starts = starts;
                context.current_local_variable_annotation_range_ends = ends;
                context.current_local_variable_annotation_range_indices = indices;
            }
            TypeReference::CAST
            | TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT
            | TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT
            | TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT
            | TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT => {
                target_type &= 0xFF00_00FF;
                current_offset += 4;
            }
            TypeReference::CLASS_EXTENDS
            | TypeReference::CLASS_TYPE_PARAMETER_BOUND
            | TypeReference::METHOD_TYPE_PARAMETER_BOUND
            | TypeReference::THROWS
            | TypeReference::EXCEPTION_PARAMETER => {
                target_type &= 0xFFFF_FF00;
                current_offset += 3;
            }
            TypeReference::INSTANCEOF
            | TypeReference::NEW
            | TypeReference::CONSTRUCTOR_REFERENCE
            | TypeReference::METHOD_REFERENCE => {
                target_type &= 0xFF00_0000;
                current_offset += 3;
            }
            sort => {
                return Err(ReadError::new(
                    current_offset,
                    ReadErrorKind::InvalidTypeAnnotationTarget(sort),
                ));
            }
        }
        context.current_type_annotation_target = target_type;
        // Parse and store the target_path structure.
        let path_length = i64::from(self.u8_at(current_offset)?);
        context.current_type_annotation_target_path =
            self.type_path_at(current_offset, path_length)?;
        // Return the start offset of the rest of the type_annotation structure.
        Ok(current_offset + 1 + 2 * path_length)
    }

    /// The type_path structure at `offset`, of `path_length` steps, or `None` if it is empty
    /// (ASM's `pathLength == 0 ? null : new TypePath(classFileBuffer, offset)`).
    ///
    /// ASM's `TypePath` is a lazy window into the class file, but the type_index field that
    /// follows it is always read next, so a truncated path fails at the same point either way.
    fn type_path_at(&self, offset: i64, path_length: i64) -> Result<Option<TypePath>, ReadError> {
        if path_length == 0 {
            return Ok(None);
        }
        let structure = self.slice_at(offset, 1 + 2 * path_length)?;
        Ok(TypePath::from_structure(structure))
    }

    /// Reads a `Runtime[In]VisibleParameterAnnotations` attribute and makes the given visitor visit
    /// it.
    pub(super) fn read_parameter_annotations(
        &self,
        method_visitor: &mut dyn MethodVisitor,
        runtime_parameter_annotations_offset: i64,
        visible: bool,
    ) -> Result<(), ReadError> {
        let mut current_offset = runtime_parameter_annotations_offset;
        let num_parameters = self.u8_at(current_offset)?;
        current_offset += 1;
        method_visitor.visit_annotable_parameter_count(num_parameters, visible);
        for i in 0..num_parameters {
            let num_annotations = self.u16_at(current_offset)?;
            current_offset += 2;
            for _ in 0..num_annotations {
                // Parse the type_index field.
                let annotation_descriptor = self.utf8_at(current_offset)?;
                let annotation_descriptor = required(annotation_descriptor, current_offset)?;
                current_offset += 2;
                // Parse num_element_value_pairs and element_value_pairs and visit these values.
                let mut annotation_visitor =
                    method_visitor.visit_parameter_annotation(i, annotation_descriptor, visible);
                current_offset = self.read_element_values(
                    annotation_visitor.as_deref_mut(),
                    current_offset,
                    /* named= */ true,
                    0,
                )?;
            }
        }
        Ok(())
    }

    /// Reads the element values of a JVMS 'annotation' structure and makes the given visitor
    /// visit them. This method can also be used to read the values of the JVMS 'array_value'
    /// field of an annotation's 'element_value'.
    ///
    /// - `annotation_offset`: the start offset of an 'annotation' structure (excluding its
    ///   type_index field) or of an 'array_value' structure.
    /// - `named`: if the annotation values are named or not. This should be true to parse the
    ///   values of a JVMS 'annotation' structure, and false to parse the JVMS 'array_value' of an
    ///   annotation's element_value.
    /// - `depth`: the depth of these annotation values in the annotation tree structure (ASM
    ///   9.11's bound, ADR-06).
    ///
    /// Returns the end offset of the JVMS 'annotation' or 'array_value' structure.
    pub(super) fn read_element_values(
        &self,
        mut annotation_visitor: Option<&mut (dyn AnnotationVisitor + '_)>,
        annotation_offset: i64,
        named: bool,
        depth: u32,
    ) -> Result<i64, ReadError> {
        if depth >= MAX_ANNOTATION_DEPTH {
            return Err(ReadError::new(
                annotation_offset,
                ReadErrorKind::RecursionLimit,
            ));
        }
        let mut current_offset = annotation_offset;
        // Read the num_element_value_pairs field (or num_values field for an array_value).
        let num_element_value_pairs = self.u16_at(current_offset)?;
        current_offset += 2;
        if named {
            // Parse the element_value_pairs array.
            for _ in 0..num_element_value_pairs {
                let element_name = self.utf8_at(current_offset)?;
                current_offset = self.read_element_value(
                    annotation_visitor.as_deref_mut(),
                    current_offset + 2,
                    element_name,
                    depth,
                )?;
            }
        } else {
            // Parse the array_value array.
            for _ in 0..num_element_value_pairs {
                current_offset = self.read_element_value(
                    annotation_visitor.as_deref_mut(),
                    current_offset,
                    /* element_name= */ None,
                    depth,
                )?;
            }
        }
        if let Some(annotation_visitor) = annotation_visitor {
            annotation_visitor.visit_end();
        }
        Ok(current_offset)
    }

    /// Reads a JVMS 'element_value' structure and makes the given visitor visit it.
    ///
    /// Returns the end offset of the JVMS 'element_value' structure.
    pub(super) fn read_element_value(
        &self,
        annotation_visitor: Option<&mut (dyn AnnotationVisitor + '_)>,
        element_value_offset: i64,
        element_name: Option<&JavaStr>,
        depth: u32,
    ) -> Result<i64, ReadError> {
        let mut current_offset = element_value_offset;
        let Some(annotation_visitor) = annotation_visitor else {
            return Ok(match self.u8_at(current_offset)? {
                // enum_const_value
                b'e' => current_offset + 5,
                // annotation_value
                b'@' => self.read_element_values(None, current_offset + 3, true, depth + 1)?,
                // array_value
                b'[' => self.read_element_values(None, current_offset + 1, false, depth + 1)?,
                _ => current_offset + 3,
            });
        };
        let tag = self.u8_at(current_offset)?;
        current_offset += 1;
        match tag {
            // const_value_index, CONSTANT_Integer
            b'B' => {
                let value = self.int_constant_at(current_offset)? as i8;
                annotation_visitor.visit(element_name, &AnnotationValue::Byte(value));
                current_offset += 2;
            }
            // const_value_index, CONSTANT_Integer
            b'C' => {
                let value = self.int_constant_at(current_offset)? as u16;
                annotation_visitor.visit(element_name, &AnnotationValue::Char(value));
                current_offset += 2;
            }
            // const_value_index, CONSTANT_Double, CONSTANT_Float, CONSTANT_Integer or
            // CONSTANT_Long
            b'D' | b'F' | b'I' | b'J' => {
                let index = usize::from(self.u16_at(current_offset)?);
                let value = annotation_value(self.const_at(index)?, current_offset, index)?;
                annotation_visitor.visit(element_name, &value);
                current_offset += 2;
            }
            // const_value_index, CONSTANT_Integer
            b'S' => {
                let value = self.int_constant_at(current_offset)? as i16;
                annotation_visitor.visit(element_name, &AnnotationValue::Short(value));
                current_offset += 2;
            }
            // const_value_index, CONSTANT_Integer
            b'Z' => {
                let value = self.int_constant_at(current_offset)? != 0;
                annotation_visitor.visit(element_name, &AnnotationValue::Boolean(value));
                current_offset += 2;
            }
            // const_value_index, CONSTANT_Utf8
            b's' => {
                let value = required(self.utf8_at(current_offset)?, current_offset)?;
                annotation_visitor
                    .visit(element_name, &AnnotationValue::String(Cow::Borrowed(value)));
                current_offset += 2;
            }
            // enum_const_value
            b'e' => {
                let descriptor = self.utf8_at(current_offset)?;
                let value = self.utf8_at(current_offset + 2)?;
                annotation_visitor.visit_enum(
                    element_name,
                    required(descriptor, current_offset)?,
                    required(value, current_offset + 2)?,
                );
                current_offset += 4;
            }
            // class_info
            b'c' => {
                let descriptor = required(self.utf8_at(current_offset)?, current_offset)?;
                let value = Type::try_get_type(descriptor).map_err(|_| {
                    ReadError::new(current_offset, ReadErrorKind::InvalidElementValueTag(tag))
                })?;
                annotation_visitor.visit(element_name, &AnnotationValue::Type(value));
                current_offset += 2;
            }
            // annotation_value
            b'@' => {
                let descriptor = required(self.utf8_at(current_offset)?, current_offset)?;
                let mut nested = annotation_visitor.visit_annotation(element_name, descriptor);
                current_offset = self.read_element_values(
                    nested.as_deref_mut(),
                    current_offset + 2,
                    true,
                    depth + 1,
                )?;
            }
            // array_value
            b'[' => {
                let num_values = self.u16_at(current_offset)?;
                current_offset += 2;
                if num_values == 0 {
                    let mut array = annotation_visitor.visit_array(element_name);
                    return self.read_element_values(
                        array.as_deref_mut(),
                        current_offset - 2,
                        /* named= */ false,
                        depth + 1,
                    );
                }
                let n = usize::from(num_values);
                let value = match self.u8_at(current_offset)? {
                    b'B' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.int_constant_at(current_offset + 1)? as i8);
                            current_offset += 3;
                        }
                        AnnotationValue::ByteArray(Cow::Owned(values))
                    }
                    b'Z' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.int_constant_at(current_offset + 1)? != 0);
                            current_offset += 3;
                        }
                        AnnotationValue::BooleanArray(Cow::Owned(values))
                    }
                    b'S' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.int_constant_at(current_offset + 1)? as i16);
                            current_offset += 3;
                        }
                        AnnotationValue::ShortArray(Cow::Owned(values))
                    }
                    b'C' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.int_constant_at(current_offset + 1)? as u16);
                            current_offset += 3;
                        }
                        AnnotationValue::CharArray(Cow::Owned(values))
                    }
                    b'I' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.int_constant_at(current_offset + 1)?);
                            current_offset += 3;
                        }
                        AnnotationValue::IntArray(Cow::Owned(values))
                    }
                    b'J' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            values.push(self.long_constant_at(current_offset + 1)?);
                            current_offset += 3;
                        }
                        AnnotationValue::LongArray(Cow::Owned(values))
                    }
                    b'F' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            let bits = self.int_constant_at(current_offset + 1)?;
                            values.push(f32::from_bits(bits as u32));
                            current_offset += 3;
                        }
                        AnnotationValue::FloatArray(Cow::Owned(values))
                    }
                    b'D' => {
                        let mut values = Vec::with_capacity(n);
                        for _ in 0..n {
                            let bits = self.long_constant_at(current_offset + 1)?;
                            values.push(f64::from_bits(bits as u64));
                            current_offset += 3;
                        }
                        AnnotationValue::DoubleArray(Cow::Owned(values))
                    }
                    _ => {
                        let mut array = annotation_visitor.visit_array(element_name);
                        return self.read_element_values(
                            array.as_deref_mut(),
                            current_offset - 2,
                            /* named= */ false,
                            depth + 1,
                        );
                    }
                };
                annotation_visitor.visit(element_name, &value);
            }
            _ => {
                return Err(ReadError::new(
                    current_offset - 1,
                    ReadErrorKind::InvalidElementValueTag(tag),
                ));
            }
        }
        Ok(current_offset)
    }

    /// `readInt(cpInfoOffsets[readUnsignedShort(offset)])`: the int at the constant pool entry
    /// whose index is at `offset` (whatever its tag).
    fn int_constant_at(&self, offset: i64) -> Result<i32, ReadError> {
        self.i32_at(self.cp_info_offset(usize::from(self.u16_at(offset)?))?)
    }

    /// `readLong(cpInfoOffsets[readUnsignedShort(offset)])`.
    fn long_constant_at(&self, offset: i64) -> Result<i64, ReadError> {
        self.i64_at(self.cp_info_offset(usize::from(self.u16_at(offset)?))?)
    }
}

/// The annotation value of a `D`, `F`, `I` or `J` element value, which ASM reads with
/// `readConst` (so any constant, whatever the tag says).
fn annotation_value(
    constant: Constant<'_>,
    offset: i64,
    index: usize,
) -> Result<AnnotationValue<'_>, ReadError> {
    Ok(match constant {
        Constant::Integer(v) => AnnotationValue::Int(v),
        Constant::Float(v) => AnnotationValue::Float(v),
        Constant::Long(v) => AnnotationValue::Long(v),
        Constant::Double(v) => AnnotationValue::Double(v),
        Constant::String(v) => AnnotationValue::String(v),
        Constant::Type(v) => AnnotationValue::Type(v),
        // Not an annotation value: ASM passes it on as is (a documented divergence).
        Constant::Handle(_) | Constant::Dynamic(_) => {
            return Err(ReadError::new(
                offset,
                ReadErrorKind::InvalidConstant(index as u16),
            ));
        }
    })
}
