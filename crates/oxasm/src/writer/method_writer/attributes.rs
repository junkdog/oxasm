//! The `method_info` of a method writer: the method copy optimization (ADR-08), and ASM's
//! `computeMethodInfoSize`, `putMethodInfo` and `collectAttributePrototypes`.

use crate::constants;
use crate::error::WriteError;
use crate::opcodes;
use crate::visitor::MethodCopySource;
use crate::writer::annotation_writer::{
    compute_annotations_size, compute_member_annotations_size, compute_parameter_annotations_size,
    put_annotations, put_member_annotations, put_parameter_annotations,
};
use crate::writer::attributes::{
    AttributePrototypes, CodeContext, compute_standard_attributes_size, put_standard_attributes,
};
use crate::writer::byte_vector::ByteVector;
use crate::writer::handler::{get_exception_table_size, put_exception_table};
use crate::writer::method_writer::MethodWriterState;
use crate::writer::symbol_table::SymbolTable;

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodWriterState {
    /// The method copy optimization (ASM's `canCopyMethodAttributes` and
    /// `setMethodAttributesSource`): if the method comes unchanged from the class reader this
    /// writer's constant pool was copied from, copies its attributes as is and returns `true`.
    pub(super) fn try_copy_method(
        &mut self,
        symbol_table: &mut SymbolTable,
        source: &MethodCopySource<'_, '_>,
    ) -> bool {
        if !self.can_copy_method_attributes(symbol_table, source) {
            return false;
        }
        // Don't visit the attributes, instead copy them from the source class reader (ASM
        // copies them later, in putMethodInfo). Note that we skip the 6 header bytes of the
        // method_info JVMS structure.
        match source
            .reader
            .slice_at(source.method_info_offset + 6, source.method_info_length - 6)
        {
            Ok(bytes) => self.copied_method_info = Some(bytes.to_vec()),
            // ASM's copy fails later, in toByteArray.
            Err(_) => symbol_table.fail(WriteError::IndexOutOfBounds(format!(
                "Range [{}, {}) out of bounds for length {}",
                source.method_info_offset + 6,
                source.method_info_offset + source.method_info_length,
                source.reader.class_file().len()
            ))),
        }
        true
    }

    /// Returns whether the attributes of this method can be copied from the attributes of the
    /// given method (assuming there is no method visitor between the given class reader and
    /// this method writer). This method should only be called just after this method writer has
    /// been created, and before any content is visited.
    fn can_copy_method_attributes(
        &self,
        symbol_table: &SymbolTable,
        source: &MethodCopySource<'_, '_>,
    ) -> bool {
        // If the method descriptor has changed, with more locals than the max_locals field of
        // the original Code attribute, if any, then the original method attributes can't be
        // copied. A conservative check on the descriptor changes alone ensures this (being more
        // precise is not worth the additional complexity, because these cases should be rare --
        // if a transform changes a method descriptor, most of the time it needs to change the
        // method's code too).
        if symbol_table.source_id() != Some(source.reader.id())
            || i32::from(source.descriptor_index) != self.descriptor_index
            || i32::from(source.signature_index) != self.signature_index
            || source.has_deprecated_attribute != (self.access_flags & opcodes::ACC_DEPRECATED != 0)
        {
            return false;
        }
        let need_synthetic_attribute = symbol_table.major_version() < opcodes::V1_5 as i32
            && self.access_flags & opcodes::ACC_SYNTHETIC != 0;
        if source.has_synthetic_attribute != need_synthetic_attribute {
            return false;
        }
        let number_of_exceptions = self.exception_index_table.len();
        if source.exceptions_offset == 0 {
            if number_of_exceptions != 0 {
                return false;
            }
        } else if source.reader.u16_at(source.exceptions_offset).ok()
            == Some(number_of_exceptions as u16)
        {
            let mut current_exception_offset = source.exceptions_offset + 2;
            for &exception_index in &self.exception_index_table {
                if source
                    .reader
                    .u16_at(current_exception_offset)
                    .map(i32::from)
                    .ok()
                    != Some(exception_index)
                {
                    return false;
                }
                current_exception_offset += 2;
            }
        }
        true
    }

    /// The context given to the Code attributes of this method (ASM's `code.data, code.length,
    /// maxStack, maxLocals` arguments).
    fn code_context<'c>(
        code: &'c ByteVector,
        max_stack: i32,
        max_locals: i32,
        label_offset: &'c dyn Fn(crate::label::Label) -> Option<i32>,
    ) -> CodeContext<'c> {
        CodeContext {
            code: code.data(),
            max_stack,
            max_locals,
            label_offset,
        }
    }

    /// Returns the size of the `method_info` JVMS structure generated by this writer, and adds
    /// the names of its attributes to the constant pool (ASM's `computeMethodInfoSize`).
    pub(crate) fn compute_method_info_size(&mut self, symbol_table: &mut SymbolTable) -> usize {
        // If this method_info must be copied from an existing one, the size computation is
        // trivial.
        if let Some(copied) = &self.copied_method_info {
            // sourceLength excludes the first 6 bytes for access_flags, name_index and
            // descriptor_index.
            return 6 + copied.len();
        }
        // 2 bytes each for access_flags, name_index, descriptor_index and attributes_count.
        let mut size = 8;
        // For ease of reference, we use here the same attribute order as in Section 4.7 of the
        // JVMS.
        if self.code.size() > 0 {
            if self.code.size() > 65535 {
                symbol_table.fail(WriteError::MethodTooLarge {
                    class_name: symbol_table.class_name().to_owned(),
                    method_name: (*self.name).to_owned(),
                    descriptor: (*self.descriptor).to_owned(),
                    code_size: self.code.size(),
                });
            }
            symbol_table.add_constant_utf8(constants::CODE);
            // The Code attribute has 6 header bytes, plus 2, 2, 4 and 2 bytes respectively for
            // max_stack, max_locals, code_length and attributes_count, plus the bytecode and the
            // exception table.
            size += 16 + self.code.size() + get_exception_table_size(&self.handlers);
            if let Some(stack_map_table_entries) = &self.stack_map_table_entries {
                let use_stack_map_table = symbol_table.major_version() >= opcodes::V1_6 as i32;
                symbol_table.add_constant_utf8(if use_stack_map_table {
                    constants::STACK_MAP_TABLE
                } else {
                    crate::jstr!("StackMap")
                });
                // 6 header bytes and 2 bytes for number_of_entries.
                size += 8 + stack_map_table_entries.size();
            }
            if let Some(line_number_table) = &self.line_number_table {
                symbol_table.add_constant_utf8(constants::LINE_NUMBER_TABLE);
                // 6 header bytes and 2 bytes for line_number_table_length.
                size += 8 + line_number_table.size();
            }
            if let Some(local_variable_table) = &self.local_variable_table {
                symbol_table.add_constant_utf8(constants::LOCAL_VARIABLE_TABLE);
                // 6 header bytes and 2 bytes for local_variable_table_length.
                size += 8 + local_variable_table.size();
            }
            if let Some(local_variable_type_table) = &self.local_variable_type_table {
                symbol_table.add_constant_utf8(constants::LOCAL_VARIABLE_TYPE_TABLE);
                // 6 header bytes and 2 bytes for local_variable_type_table_length.
                size += 8 + local_variable_type_table.size();
            }
            if !self.code_visible_type_annotations.is_empty() {
                size += compute_annotations_size(
                    symbol_table,
                    &self.code_visible_type_annotations,
                    Some(constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS),
                );
            }
            if !self.code_invisible_type_annotations.is_empty() {
                size += compute_annotations_size(
                    symbol_table,
                    &self.code_invisible_type_annotations,
                    Some(constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS),
                );
            }
            if !self.code_attributes.is_empty() {
                let labels = &self.labels;
                let label_offset = |label| labels.resolved_offset(label);
                let code =
                    Self::code_context(&self.code, self.max_stack, self.max_locals, &label_offset);
                size += self
                    .code_attributes
                    .compute_attributes_size(symbol_table, Some(code));
            }
        }
        if !self.exception_index_table.is_empty() {
            symbol_table.add_constant_utf8(constants::EXCEPTIONS);
            size += 8 + 2 * self.exception_index_table.len();
        }
        size +=
            compute_standard_attributes_size(symbol_table, self.access_flags, self.signature_index);
        size += compute_member_annotations_size(symbol_table, &self.annotations);
        if let Some(annotations) = &self.visible_parameter_annotations {
            let count =
                annotable_parameter_count(self.visible_annotable_parameter_count, annotations);
            check_parameter_count(symbol_table, count, annotations.len());
            size += compute_parameter_annotations_size(
                symbol_table,
                constants::RUNTIME_VISIBLE_PARAMETER_ANNOTATIONS,
                annotations,
                count,
            );
        }
        if let Some(annotations) = &self.invisible_parameter_annotations {
            let count =
                annotable_parameter_count(self.invisible_annotable_parameter_count, annotations);
            check_parameter_count(symbol_table, count, annotations.len());
            size += compute_parameter_annotations_size(
                symbol_table,
                constants::RUNTIME_INVISIBLE_PARAMETER_ANNOTATIONS,
                annotations,
                count,
            );
        }
        if let Some(default_value) = &self.default_value {
            symbol_table.add_constant_utf8(constants::ANNOTATION_DEFAULT);
            size += 6 + default_value.size();
        }
        if let Some(parameters) = &self.parameters {
            symbol_table.add_constant_utf8(constants::METHOD_PARAMETERS);
            // 6 header bytes and 1 byte for parameters_count.
            size += 7 + parameters.size();
        }
        if !self.attributes.is_empty() {
            size += self.attributes.compute_attributes_size(symbol_table, None);
        }
        size
    }

    /// Puts the content of the `method_info` JVMS structure generated by this writer into the
    /// given byte vector (ASM's `putMethodInfo`).
    pub(crate) fn put_method_info(
        &mut self,
        symbol_table: &mut SymbolTable,
        output: &mut ByteVector,
    ) {
        let use_synthetic_attribute = symbol_table.major_version() < opcodes::V1_5 as i32;
        let mask = if use_synthetic_attribute {
            opcodes::ACC_SYNTHETIC
        } else {
            0
        };
        output
            .put_short((self.access_flags & !mask) as i32)
            .put_short(self.name_index)
            .put_short(self.descriptor_index);
        // If this method_info must be copied from an existing one, copy it now and return
        // early.
        if let Some(copied) = &self.copied_method_info {
            output.put_byte_array(copied);
            return;
        }
        // For ease of reference, we use here the same attribute order as in Section 4.7 of the
        // JVMS.
        let mut attribute_count = 0;
        if self.code.size() > 0 {
            attribute_count += 1;
        }
        if !self.exception_index_table.is_empty() {
            attribute_count += 1;
        }
        if self.access_flags & opcodes::ACC_SYNTHETIC != 0 && use_synthetic_attribute {
            attribute_count += 1;
        }
        if self.signature_index != 0 {
            attribute_count += 1;
        }
        if self.access_flags & opcodes::ACC_DEPRECATED != 0 {
            attribute_count += 1;
        }
        // The four (type) annotation lists.
        attribute_count += self.annotations.attribute_count();
        if self.visible_parameter_annotations.is_some() {
            attribute_count += 1;
        }
        if self.invisible_parameter_annotations.is_some() {
            attribute_count += 1;
        }
        if self.default_value.is_some() {
            attribute_count += 1;
        }
        if self.parameters.is_some() {
            attribute_count += 1;
        }
        attribute_count += self.attributes.len();
        // For ease of reference, we use here the same attribute order as in Section 4.7 of the
        // JVMS.
        output.put_short(attribute_count as i32);
        if self.code.size() > 0 {
            // 2, 2, 4 and 2 bytes respectively for max_stack, max_locals, code_length and
            // attributes_count, plus the bytecode and the exception table.
            let mut size = 10 + self.code.size() + get_exception_table_size(&self.handlers);
            let mut code_attribute_count = 0;
            if let Some(stack_map_table_entries) = &self.stack_map_table_entries {
                // 6 header bytes and 2 bytes for number_of_entries.
                size += 8 + stack_map_table_entries.size();
                code_attribute_count += 1;
            }
            if let Some(line_number_table) = &self.line_number_table {
                // 6 header bytes and 2 bytes for line_number_table_length.
                size += 8 + line_number_table.size();
                code_attribute_count += 1;
            }
            if let Some(local_variable_table) = &self.local_variable_table {
                // 6 header bytes and 2 bytes for local_variable_table_length.
                size += 8 + local_variable_table.size();
                code_attribute_count += 1;
            }
            if let Some(local_variable_type_table) = &self.local_variable_type_table {
                // 6 header bytes and 2 bytes for local_variable_type_table_length.
                size += 8 + local_variable_type_table.size();
                code_attribute_count += 1;
            }
            if !self.code_visible_type_annotations.is_empty() {
                size += compute_annotations_size(
                    symbol_table,
                    &self.code_visible_type_annotations,
                    Some(constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS),
                );
                code_attribute_count += 1;
            }
            if !self.code_invisible_type_annotations.is_empty() {
                size += compute_annotations_size(
                    symbol_table,
                    &self.code_invisible_type_annotations,
                    Some(constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS),
                );
                code_attribute_count += 1;
            }
            let labels = &self.labels;
            let label_offset = |label| labels.resolved_offset(label);
            if !self.code_attributes.is_empty() {
                let code =
                    Self::code_context(&self.code, self.max_stack, self.max_locals, &label_offset);
                size += self
                    .code_attributes
                    .compute_attributes_size(symbol_table, Some(code));
                code_attribute_count += self.code_attributes.len();
            }
            let code_index = symbol_table.add_constant_utf8(constants::CODE);
            output
                .put_short(code_index)
                .put_int(size as i32)
                .put_short(self.max_stack)
                .put_short(self.max_locals)
                .put_int(self.code.size() as i32)
                .put_byte_array(self.code.data());
            match &self.exception_table {
                Some(exception_table) => {
                    output.put_byte_array(exception_table.data());
                }
                None => put_exception_table(&self.handlers, labels, output),
            }
            output.put_short(code_attribute_count as i32);
            if let Some(stack_map_table_entries) = &self.stack_map_table_entries {
                let use_stack_map_table = symbol_table.major_version() >= opcodes::V1_6 as i32;
                let name_index = symbol_table.add_constant_utf8(if use_stack_map_table {
                    constants::STACK_MAP_TABLE
                } else {
                    crate::jstr!("StackMap")
                });
                output
                    .put_short(name_index)
                    .put_int(2 + stack_map_table_entries.size() as i32)
                    .put_short(self.stack_map_table_number_of_entries)
                    .put_byte_array(stack_map_table_entries.data());
            }
            if let Some(line_number_table) = &self.line_number_table {
                let name_index = symbol_table.add_constant_utf8(constants::LINE_NUMBER_TABLE);
                output
                    .put_short(name_index)
                    .put_int(2 + line_number_table.size() as i32)
                    .put_short(self.line_number_table_length)
                    .put_byte_array(line_number_table.data());
            }
            if let Some(local_variable_table) = &self.local_variable_table {
                let name_index = symbol_table.add_constant_utf8(constants::LOCAL_VARIABLE_TABLE);
                output
                    .put_short(name_index)
                    .put_int(2 + local_variable_table.size() as i32)
                    .put_short(self.local_variable_table_length)
                    .put_byte_array(local_variable_table.data());
            }
            if let Some(local_variable_type_table) = &self.local_variable_type_table {
                let name_index =
                    symbol_table.add_constant_utf8(constants::LOCAL_VARIABLE_TYPE_TABLE);
                output
                    .put_short(name_index)
                    .put_int(2 + local_variable_type_table.size() as i32)
                    .put_short(self.local_variable_type_table_length)
                    .put_byte_array(local_variable_type_table.data());
            }
            if !self.code_visible_type_annotations.is_empty() {
                let name_index =
                    symbol_table.add_constant_utf8(constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS);
                put_annotations(&self.code_visible_type_annotations, name_index, output);
            }
            if !self.code_invisible_type_annotations.is_empty() {
                let name_index =
                    symbol_table.add_constant_utf8(constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS);
                put_annotations(&self.code_invisible_type_annotations, name_index, output);
            }
            if !self.code_attributes.is_empty() {
                let code =
                    Self::code_context(&self.code, self.max_stack, self.max_locals, &label_offset);
                self.code_attributes
                    .put_attributes(symbol_table, Some(code), output);
            }
        }
        if !self.exception_index_table.is_empty() {
            let name_index = symbol_table.add_constant_utf8(constants::EXCEPTIONS);
            output
                .put_short(name_index)
                .put_int(2 + 2 * self.exception_index_table.len() as i32)
                .put_short(self.exception_index_table.len() as i32);
            for &exception_index in &self.exception_index_table {
                output.put_short(exception_index);
            }
        }
        put_standard_attributes(
            symbol_table,
            self.access_flags,
            self.signature_index,
            output,
        );
        put_member_annotations(symbol_table, &self.annotations, output);
        if let Some(annotations) = &self.visible_parameter_annotations {
            let name_index =
                symbol_table.add_constant_utf8(constants::RUNTIME_VISIBLE_PARAMETER_ANNOTATIONS);
            let count =
                annotable_parameter_count(self.visible_annotable_parameter_count, annotations);
            put_parameter_annotations(name_index, annotations, count, output);
        }
        if let Some(annotations) = &self.invisible_parameter_annotations {
            let name_index =
                symbol_table.add_constant_utf8(constants::RUNTIME_INVISIBLE_PARAMETER_ANNOTATIONS);
            let count =
                annotable_parameter_count(self.invisible_annotable_parameter_count, annotations);
            put_parameter_annotations(name_index, annotations, count, output);
        }
        if let Some(default_value) = &self.default_value {
            let name_index = symbol_table.add_constant_utf8(constants::ANNOTATION_DEFAULT);
            output
                .put_short(name_index)
                .put_int(default_value.size() as i32)
                .put_byte_array(default_value.data());
        }
        if let Some(parameters) = &self.parameters {
            let name_index = symbol_table.add_constant_utf8(constants::METHOD_PARAMETERS);
            output
                .put_short(name_index)
                .put_int(1 + parameters.size() as i32)
                .put_byte(self.parameters_count)
                .put_byte_array(parameters.data());
        }
        if !self.attributes.is_empty() {
            self.attributes.put_attributes(symbol_table, None, output);
        }
    }

    /// Collects the attributes of this method into the given set of attribute prototypes.
    pub(crate) fn collect_attribute_prototypes(&self, prototypes: &mut AttributePrototypes) {
        self.attributes.collect_attribute_prototypes(prototypes);
        self.code_attributes
            .collect_attribute_prototypes(prototypes);
    }
}

/// The number of parameters of a parameter annotations attribute: the annotable parameter count
/// if visited, the number of parameters of the method otherwise.
fn annotable_parameter_count(
    annotable_parameter_count: i32,
    annotations: &[crate::writer::annotation_writer::AnnotationList],
) -> usize {
    if annotable_parameter_count == 0 {
        annotations.len()
    } else {
        annotable_parameter_count as usize
    }
}

/// Records ASM's `ArrayIndexOutOfBoundsException` for an annotable parameter count larger than
/// the number of parameters of the method.
fn check_parameter_count(symbol_table: &mut SymbolTable, count: usize, length: usize) {
    if count > length {
        symbol_table.fail(WriteError::IndexOutOfBounds(format!(
            "Index {length} out of bounds for length {length}"
        )));
    }
}
