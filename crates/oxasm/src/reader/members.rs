//! ASM's `ClassReader` section "Methods to parse modules, fields and methods".

use crate::constants;
use crate::jstr::JavaStr;
use crate::opcodes::{ACC_DEPRECATED, ACC_SYNTHETIC};
use crate::visitor::{
    ClassVisitor, FieldVisitor, MethodCopySource, MethodVisitor, RecordComponentVisitor,
};

use super::context::Context;
use super::{ClassReader, ParsingOptions, ReadError, required};

impl<'b> ClassReader<'b> {
    /// Reads the Module, ModulePackages and ModuleMainClass attributes and visits them.
    ///
    /// - `module_offset`: the offset of the Module attribute (excluding the attribute_info's
    ///   attribute_name_index and attribute_length fields).
    /// - `module_packages_offset`: the offset of the ModulePackages attribute (excluding the
    ///   attribute_info's attribute_name_index and attribute_length fields), or 0.
    /// - `module_main_class`: the string corresponding to the ModuleMainClass attribute, or
    ///   `None`.
    pub(super) fn read_module_attributes(
        &self,
        class_visitor: &mut dyn ClassVisitor,
        module_offset: i64,
        module_packages_offset: i64,
        module_main_class: Option<&JavaStr>,
    ) -> Result<(), ReadError> {
        // Read the module_name_index, module_flags and module_version_index fields and visit
        // them.
        let mut current_offset = module_offset;
        let module_name = self.read_class_at(current_offset)?;
        let module_flags = u32::from(self.u16_at(current_offset + 2)?);
        let module_version = self.utf8_at(current_offset + 4)?;
        current_offset += 6;
        let module_name = required(module_name, module_offset)?;
        let Some(mut module_visitor) =
            class_visitor.visit_module(module_name, module_flags, module_version)
        else {
            return Ok(());
        };

        // Visit the ModuleMainClass attribute.
        if let Some(module_main_class) = module_main_class {
            module_visitor.visit_main_class(module_main_class);
        }

        // Visit the ModulePackages attribute.
        if module_packages_offset != 0 {
            let package_count = self.u16_at(module_packages_offset)?;
            let mut current_package_offset = module_packages_offset + 2;
            for _ in 0..package_count {
                let package = self.read_class_at(current_package_offset)?;
                module_visitor.visit_package(required(package, current_package_offset)?);
                current_package_offset += 2;
            }
        }

        // Read the 'requires_count' and 'requires' fields.
        let requires_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..requires_count {
            // Read the requires_index, requires_flags and requires_version fields and visit
            // them.
            let requires = self.read_class_at(current_offset)?;
            let requires_flags = u32::from(self.u16_at(current_offset + 2)?);
            let requires_version = self.utf8_at(current_offset + 4)?;
            module_visitor.visit_require(
                required(requires, current_offset)?,
                requires_flags,
                requires_version,
            );
            current_offset += 6;
        }

        // Read the 'exports_count' and 'exports' fields, then the 'opens_count' and 'opens'
        // fields.
        for opens in [false, true] {
            let count = self.u16_at(current_offset)?;
            current_offset += 2;
            for _ in 0..count {
                // Read the exports_index (opens_index), exports_flags (opens_flags),
                // exports_to_count (opens_to_count) and exports_to_index (opens_to_index) fields
                // and visit them.
                let package = self.read_class_at(current_offset)?;
                let package = required(package, current_offset)?;
                let flags = u32::from(self.u16_at(current_offset + 2)?);
                let to_count = self.u16_at(current_offset + 4)?;
                current_offset += 6;
                let mut to = Vec::with_capacity(usize::from(to_count));
                for _ in 0..to_count {
                    to.push(required(
                        self.read_class_at(current_offset)?,
                        current_offset,
                    )?);
                    current_offset += 2;
                }
                if opens {
                    module_visitor.visit_open(package, flags, &to);
                } else {
                    module_visitor.visit_export(package, flags, &to);
                }
            }
        }

        // Read the 'uses_count' and 'uses' fields.
        let uses_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..uses_count {
            module_visitor.visit_use(required(
                self.read_class_at(current_offset)?,
                current_offset,
            )?);
            current_offset += 2;
        }

        // Read the 'provides_count' and 'provides' fields.
        let provides_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..provides_count {
            // Read the provides_index, provides_with_count and provides_with_index fields and
            // visit them.
            let provides = required(self.read_class_at(current_offset)?, current_offset)?;
            let provides_with_count = self.u16_at(current_offset + 2)?;
            current_offset += 4;
            let mut provides_with = Vec::with_capacity(usize::from(provides_with_count));
            for _ in 0..provides_with_count {
                provides_with.push(required(
                    self.read_class_at(current_offset)?,
                    current_offset,
                )?);
                current_offset += 2;
            }
            module_visitor.visit_provide(provides, &provides_with);
        }

        // Visit the end of the module attributes.
        module_visitor.visit_end();
        Ok(())
    }

    /// Reads a record component and visits it.
    ///
    /// Returns the offset of the first byte following the record component.
    pub(super) fn read_record_component(
        &self,
        class_visitor: &mut dyn ClassVisitor,
        context: &mut Context<'_>,
        record_component_offset: i64,
    ) -> Result<i64, ReadError> {
        let mut current_offset = record_component_offset;
        let name = self.utf8_at(current_offset)?;
        let descriptor = self.utf8_at(current_offset + 2)?;
        current_offset += 4;

        // Read the record component attributes (the variables are ordered as in Section 4.7 of
        // the JVMS).

        // Attribute offsets exclude the attribute_name_index and attribute_length fields.
        // - The string corresponding to the Signature attribute, or None.
        let mut signature = None;
        // - The offset of the RuntimeVisibleAnnotations attribute, or 0.
        let mut runtime_visible_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleAnnotations attribute, or 0.
        let mut runtime_invisible_annotations_offset = 0;
        // - The offset of the RuntimeVisibleTypeAnnotations attribute, or 0.
        let mut runtime_visible_type_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleTypeAnnotations attribute, or 0.
        let mut runtime_invisible_type_annotations_offset = 0;
        // - The non standard attributes, in the order of the ClassFile structure.
        let mut attributes = Vec::new();

        let attributes_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..attributes_count {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_offset)?;
            let attribute_length = self.i32_at(current_offset + 2)?;
            current_offset += 6;
            // The tests are sorted in decreasing frequency order (based on frequencies observed
            // on typical classes).
            match attribute_name {
                Some(n) if n == constants::SIGNATURE => {
                    signature = self.utf8_at(current_offset)?;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_ANNOTATIONS => {
                    runtime_visible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    runtime_visible_type_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_ANNOTATIONS => {
                    runtime_invisible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS => {
                    runtime_invisible_type_annotations_offset = current_offset;
                }
                _ => {
                    attributes.push(self.read_attribute(
                        context.codecs,
                        attribute_name,
                        current_offset,
                        attribute_length,
                        None,
                    )?);
                }
            }
            current_offset += i64::from(attribute_length);
        }

        let Some(mut record_component_visitor) = class_visitor.visit_record_component(
            required(name, record_component_offset)?,
            required(descriptor, record_component_offset + 2)?,
            signature,
        ) else {
            return Ok(current_offset);
        };
        let visitor: &mut dyn RecordComponentVisitor = &mut *record_component_visitor;

        // Visit the RuntimeVisibleAnnotations and RuntimeInvisibleAnnotations attributes, then
        // the RuntimeVisibleTypeAnnotations and RuntimeInvisibleTypeAnnotations attributes.
        self.read_annotations_attribute(visitor, runtime_visible_annotations_offset, true)?;
        self.read_annotations_attribute(visitor, runtime_invisible_annotations_offset, false)?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_visible_type_annotations_offset,
            true,
        )?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_invisible_type_annotations_offset,
            false,
        )?;

        // Visit the non standard attributes (ASM visits its reversed list from its head).
        for attribute in attributes.iter().rev() {
            visitor.visit_attribute(attribute);
        }

        // Visit the end of the record component.
        visitor.visit_end();
        Ok(current_offset)
    }

    /// Reads a JVMS field_info structure and makes the given visitor visit it.
    ///
    /// Returns the offset of the first byte following the field_info structure.
    pub(super) fn read_field(
        &self,
        class_visitor: &mut dyn ClassVisitor,
        context: &mut Context<'_>,
        field_info_offset: i64,
    ) -> Result<i64, ReadError> {
        // Read the access_flags, name_index and descriptor_index fields.
        let mut current_offset = field_info_offset;
        let mut access_flags = u32::from(self.u16_at(current_offset)?);
        let name = self.utf8_at(current_offset + 2)?;
        let descriptor = self.utf8_at(current_offset + 4)?;
        current_offset += 6;

        // Read the field attributes (the variables are ordered as in Section 4.7 of the JVMS).
        // Attribute offsets exclude the attribute_name_index and attribute_length fields.
        // - The value corresponding to the ConstantValue attribute, or None.
        let mut constant_value = None;
        // - The string corresponding to the Signature attribute, or None.
        let mut signature = None;
        // - The offset of the RuntimeVisibleAnnotations attribute, or 0.
        let mut runtime_visible_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleAnnotations attribute, or 0.
        let mut runtime_invisible_annotations_offset = 0;
        // - The offset of the RuntimeVisibleTypeAnnotations attribute, or 0.
        let mut runtime_visible_type_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleTypeAnnotations attribute, or 0.
        let mut runtime_invisible_type_annotations_offset = 0;
        // - The non standard attributes, in the order of the ClassFile structure.
        let mut attributes = Vec::new();

        let attributes_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..attributes_count {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_offset)?;
            let attribute_length = self.i32_at(current_offset + 2)?;
            current_offset += 6;
            // The tests are sorted in decreasing frequency order (based on frequencies observed
            // on typical classes).
            match attribute_name {
                Some(n) if n == constants::CONSTANT_VALUE => {
                    let constant_value_index = self.u16_at(current_offset)?;
                    constant_value = if constant_value_index == 0 {
                        None
                    } else {
                        Some(self.const_at(usize::from(constant_value_index))?)
                    };
                }
                Some(n) if n == constants::SIGNATURE => {
                    signature = self.utf8_at(current_offset)?;
                }
                Some(n) if n == constants::DEPRECATED => {
                    access_flags |= ACC_DEPRECATED;
                }
                Some(n) if n == constants::SYNTHETIC => {
                    access_flags |= ACC_SYNTHETIC;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_ANNOTATIONS => {
                    runtime_visible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    runtime_visible_type_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_ANNOTATIONS => {
                    runtime_invisible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS => {
                    runtime_invisible_type_annotations_offset = current_offset;
                }
                _ => {
                    attributes.push(self.read_attribute(
                        context.codecs,
                        attribute_name,
                        current_offset,
                        attribute_length,
                        None,
                    )?);
                }
            }
            current_offset += i64::from(attribute_length);
        }

        // Visit the field declaration.
        let Some(mut field_visitor) = class_visitor.visit_field(
            access_flags,
            required(name, field_info_offset + 2)?,
            required(descriptor, field_info_offset + 4)?,
            signature,
            constant_value.as_ref(),
        ) else {
            return Ok(current_offset);
        };
        let visitor: &mut dyn FieldVisitor = &mut *field_visitor;

        // Visit the RuntimeVisibleAnnotations and RuntimeInvisibleAnnotations attributes, then
        // the RuntimeVisibleTypeAnnotations and RuntimeInvisibleTypeAnnotations attributes.
        self.read_annotations_attribute(visitor, runtime_visible_annotations_offset, true)?;
        self.read_annotations_attribute(visitor, runtime_invisible_annotations_offset, false)?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_visible_type_annotations_offset,
            true,
        )?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_invisible_type_annotations_offset,
            false,
        )?;

        // Visit the non standard attributes.
        for attribute in attributes.iter().rev() {
            visitor.visit_attribute(attribute);
        }

        // Visit the end of the field.
        visitor.visit_end();
        Ok(current_offset)
    }

    /// Reads a JVMS method_info structure and makes the given visitor visit it.
    ///
    /// Returns the offset of the first byte following the method_info structure.
    pub(super) fn read_method<'s>(
        &'s self,
        class_visitor: &mut dyn ClassVisitor,
        context: &mut Context<'s>,
        method_info_offset: i64,
    ) -> Result<i64, ReadError> {
        // Read the access_flags, name_index and descriptor_index fields.
        let mut current_offset = method_info_offset;
        context.current_method_access_flags = u32::from(self.u16_at(current_offset)?);
        context.current_method_name = self.utf8_at(current_offset + 2)?;
        context.current_method_descriptor = self.utf8_at(current_offset + 4)?;
        current_offset += 6;

        // Read the method attributes (the variables are ordered as in Section 4.7 of the JVMS).
        // Attribute offsets exclude the attribute_name_index and attribute_length fields.
        // - The offset of the Code attribute, or 0.
        let mut code_offset = 0;
        // - The offset of the Exceptions attribute, or 0.
        let mut exceptions_offset = 0;
        // - The strings corresponding to the Exceptions attribute (ASM's null is empty).
        let mut exceptions = Vec::new();
        // - Whether the method has a Synthetic attribute.
        let mut synthetic = false;
        // - The constant pool index contained in the Signature attribute, or 0.
        let mut signature_index = 0;
        // - The offset of the RuntimeVisibleAnnotations attribute, or 0.
        let mut runtime_visible_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleAnnotations attribute, or 0.
        let mut runtime_invisible_annotations_offset = 0;
        // - The offset of the RuntimeVisibleParameterAnnotations attribute, or 0.
        let mut runtime_visible_parameter_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleParameterAnnotations attribute, or 0.
        let mut runtime_invisible_parameter_annotations_offset = 0;
        // - The offset of the RuntimeVisibleTypeAnnotations attribute, or 0.
        let mut runtime_visible_type_annotations_offset = 0;
        // - The offset of the RuntimeInvisibleTypeAnnotations attribute, or 0.
        let mut runtime_invisible_type_annotations_offset = 0;
        // - The offset of the AnnotationDefault attribute, or 0.
        let mut annotation_default_offset = 0;
        // - The offset of the MethodParameters attribute, or 0.
        let mut method_parameters_offset = 0;
        // - The non standard attributes, in the order of the ClassFile structure.
        let mut attributes = Vec::new();

        let attributes_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..attributes_count {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_offset)?;
            let attribute_length = self.i32_at(current_offset + 2)?;
            current_offset += 6;
            // The tests are sorted in decreasing frequency order (based on frequencies observed
            // on typical classes).
            match attribute_name {
                Some(n) if n == constants::CODE => {
                    if !context.parsing_options.contains(ParsingOptions::SKIP_CODE) {
                        code_offset = current_offset;
                    }
                }
                Some(n) if n == constants::EXCEPTIONS => {
                    exceptions_offset = current_offset;
                    let number_of_exceptions = self.u16_at(exceptions_offset)?;
                    exceptions = Vec::with_capacity(usize::from(number_of_exceptions));
                    let mut current_exception_offset = exceptions_offset + 2;
                    for _ in 0..number_of_exceptions {
                        exceptions.push(required(
                            self.read_class_at(current_exception_offset)?,
                            current_exception_offset,
                        )?);
                        current_exception_offset += 2;
                    }
                }
                Some(n) if n == constants::SIGNATURE => {
                    signature_index = self.u16_at(current_offset)?;
                }
                Some(n) if n == constants::DEPRECATED => {
                    context.current_method_access_flags |= ACC_DEPRECATED;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_ANNOTATIONS => {
                    runtime_visible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    runtime_visible_type_annotations_offset = current_offset;
                }
                Some(n) if n == constants::ANNOTATION_DEFAULT => {
                    annotation_default_offset = current_offset;
                }
                Some(n) if n == constants::SYNTHETIC => {
                    synthetic = true;
                    context.current_method_access_flags |= ACC_SYNTHETIC;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_ANNOTATIONS => {
                    runtime_invisible_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS => {
                    runtime_invisible_type_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_PARAMETER_ANNOTATIONS => {
                    runtime_visible_parameter_annotations_offset = current_offset;
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_PARAMETER_ANNOTATIONS => {
                    runtime_invisible_parameter_annotations_offset = current_offset;
                }
                Some(n) if n == constants::METHOD_PARAMETERS => {
                    method_parameters_offset = current_offset;
                }
                _ => {
                    attributes.push(self.read_attribute(
                        context.codecs,
                        attribute_name,
                        current_offset,
                        attribute_length,
                        None,
                    )?);
                }
            }
            current_offset += i64::from(attribute_length);
        }

        // Visit the method declaration.
        let signature = if signature_index == 0 {
            None
        } else {
            Some(self.utf(usize::from(signature_index))?)
        };
        let Some(mut method_visitor) = class_visitor.visit_method(
            context.current_method_access_flags,
            required(context.current_method_name, method_info_offset + 2)?,
            required(context.current_method_descriptor, method_info_offset + 4)?,
            signature,
            &exceptions,
        ) else {
            return Ok(current_offset);
        };
        let visitor: &mut dyn MethodVisitor = &mut *method_visitor;

        // If the returned MethodVisitor is in fact a MethodWriter, it means there is no method
        // adapter between the reader and the writer. In this case, it might be possible to copy
        // the method attributes directly into the writer. If so, return early without visiting
        // the content of these attributes (ADR-08: the hidden, non forwarding try_copy_method).
        let copy_source = MethodCopySource {
            reader: self,
            has_synthetic_attribute: synthetic,
            has_deprecated_attribute: (context.current_method_access_flags & ACC_DEPRECATED) != 0,
            descriptor_index: self.u16_at(method_info_offset + 4)?,
            signature_index,
            exceptions_offset,
            method_info_offset,
            method_info_length: current_offset - method_info_offset,
        };
        if visitor.try_copy_method(&copy_source) {
            return Ok(current_offset);
        }

        // Visit the MethodParameters attribute.
        if method_parameters_offset != 0
            && !context.parsing_options.contains(ParsingOptions::SKIP_DEBUG)
        {
            let parameters_count = self.u8_at(method_parameters_offset)?;
            let mut current_parameter_offset = method_parameters_offset + 1;
            for _ in 0..parameters_count {
                // Read the name_index and access_flags fields and visit them.
                let name = self.utf8_at(current_parameter_offset)?;
                let access = u32::from(self.u16_at(current_parameter_offset + 2)?);
                visitor.visit_parameter(name, access);
                current_parameter_offset += 4;
            }
        }

        // Visit the AnnotationDefault attribute.
        if annotation_default_offset != 0 {
            let mut annotation_visitor = visitor.visit_annotation_default();
            self.read_element_value(
                annotation_visitor.as_deref_mut(),
                annotation_default_offset,
                None,
                0,
            )?;
            if let Some(annotation_visitor) = annotation_visitor.as_deref_mut() {
                annotation_visitor.visit_end();
            }
        }

        // Visit the RuntimeVisibleAnnotations and RuntimeInvisibleAnnotations attributes, then
        // the RuntimeVisibleTypeAnnotations and RuntimeInvisibleTypeAnnotations attributes.
        self.read_annotations_attribute(visitor, runtime_visible_annotations_offset, true)?;
        self.read_annotations_attribute(visitor, runtime_invisible_annotations_offset, false)?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_visible_type_annotations_offset,
            true,
        )?;
        self.read_type_annotations_attribute(
            visitor,
            context,
            runtime_invisible_type_annotations_offset,
            false,
        )?;

        // Visit the RuntimeVisibleParameterAnnotations attribute.
        if runtime_visible_parameter_annotations_offset != 0 {
            self.read_parameter_annotations(
                visitor,
                runtime_visible_parameter_annotations_offset,
                /* visible= */ true,
            )?;
        }

        // Visit the RuntimeInvisibleParameterAnnotations attribute.
        if runtime_invisible_parameter_annotations_offset != 0 {
            self.read_parameter_annotations(
                visitor,
                runtime_invisible_parameter_annotations_offset,
                /* visible= */ false,
            )?;
        }

        // Visit the non standard attributes.
        for attribute in attributes.iter().rev() {
            visitor.visit_attribute(attribute);
        }

        // Visit the Code attribute.
        if code_offset != 0 {
            visitor.visit_code();
            self.read_code(visitor, context, code_offset)?;
        }

        // Visit the end of the method.
        visitor.visit_end();
        Ok(current_offset)
    }
}
