//! [`MethodRemapper`]: ASM's `MethodRemapper`.

use std::borrow::Cow;

use oxasm::{
    AnnotationVisitor, Constant, FrameKind, FrameValue, Handle, JavaStr, Label, MethodVisitor,
    TypePath, TypeReference,
};

use crate::annotation_remapper::AnnotationRemapper;
use crate::remapper::{Remapper, defaults};

/// A [`MethodVisitor`] that remaps types with a [`Remapper`].
pub struct MethodRemapper<'r, M, R: ?Sized> {
    mv: M,
    remapper: &'r R,
}

impl<'r, M: MethodVisitor, R: Remapper + ?Sized> MethodRemapper<'r, M, R> {
    /// Constructs a new method remapper.
    ///
    /// - `method_visitor`: the method visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited method.
    pub fn new(method_visitor: M, remapper: &'r R) -> Self {
        MethodRemapper {
            mv: method_visitor,
            remapper,
        }
    }

    /// Wraps an annotation visitor of the delegate in an annotation remapper (ASM's
    /// `createAnnotationRemapper`).
    fn annotation_remapper<'b>(
        remapper: &'r R,
        descriptor: Option<&JavaStr>,
        annotation_visitor: Box<dyn AnnotationVisitor + 'b>,
    ) -> Box<dyn AnnotationVisitor + 'b>
    where
        'r: 'b,
    {
        Box::new(AnnotationRemapper::new(
            descriptor,
            annotation_visitor,
            remapper,
        ))
    }

    /// ASM's `remapFrameTypes`: the given frame types, with their internal names remapped, or
    /// `None` if none changes.
    fn remap_frame_types<'a>(
        remapper: &'a R,
        frame_types: &'a [FrameValue<'a>],
    ) -> Option<Vec<FrameValue<'a>>> {
        let mut remapped_frame_types: Option<Vec<FrameValue<'a>>> = None;
        for (i, frame_type) in frame_types.iter().enumerate() {
            if let FrameValue::Object(internal_name) = frame_type {
                let remapped_frame_types =
                    remapped_frame_types.get_or_insert_with(|| frame_types.to_vec());
                remapped_frame_types[i] = FrameValue::Object(remapper.map_type(internal_name));
            }
        }
        remapped_frame_types
    }
}

impl<M, R: ?Sized> std::fmt::Debug for MethodRemapper<'_, M, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodRemapper").finish_non_exhaustive()
    }
}

impl<'r, M: MethodVisitor, R: Remapper + ?Sized> MethodVisitor for MethodRemapper<'r, M, R> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.mv)
    }

    fn visit_annotation_default<'b>(&'b mut self) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_visitor = self.mv.visit_annotation_default()?;
        Some(Self::annotation_remapper(
            self.remapper,
            /* descriptor= */ None,
            annotation_visitor,
        ))
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self
            .mv
            .visit_annotation(&remapper.map_desc(descriptor), visible)?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.mv.visit_type_annotation(
            type_ref,
            type_path,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }

    fn visit_parameter_annotation<'b>(
        &'b mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.mv.visit_parameter_annotation(
            parameter,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        let remapped_locals = Self::remap_frame_types(self.remapper, locals);
        let remapped_stack = Self::remap_frame_types(self.remapper, stack);
        self.mv.visit_frame(
            kind,
            remapped_locals.as_deref().unwrap_or(locals),
            remapped_stack.as_deref().unwrap_or(stack),
        );
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.mv.visit_field_insn(
            opcode,
            &self.remapper.map_type(owner),
            &self.remapper.map_field_name(owner, name, descriptor),
            &self.remapper.map_desc(descriptor),
        );
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.mv.visit_method_insn(
            opcode,
            &self.remapper.map_type(owner),
            &self.remapper.map_method_name(owner, name, descriptor),
            &self.remapper.map_method_desc(descriptor),
            is_interface,
        );
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        let remapper = self.remapper;
        let remapped_name = remapper.map_invoke_dynamic_method_name(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        let remapped_bootstrap_method_arguments: Vec<Constant<'_>> = bootstrap_method_arguments
            .iter()
            .map(|argument| remapper.map_value(argument))
            .collect();
        self.mv.visit_invoke_dynamic_insn(
            &remapped_name,
            &remapper.map_method_desc(descriptor),
            &defaults::map_handle_value(remapper, bootstrap_method_handle),
            &remapped_bootstrap_method_arguments,
        );
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.mv
            .visit_type_insn(opcode, &self.remapper.map_type(type_));
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.mv.visit_ldc_insn(&self.remapper.map_value(value));
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.mv
            .visit_multi_a_new_array_insn(&self.remapper.map_desc(descriptor), num_dimensions);
    }

    fn visit_insn_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.mv.visit_insn_annotation(
            type_ref,
            type_path,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        let remapped_type = type_.map(|type_| self.remapper.map_type(type_));
        self.mv
            .visit_try_catch_block(start, end, handler, remapped_type.as_deref());
    }

    fn visit_try_catch_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.mv.visit_try_catch_annotation(
            type_ref,
            type_path,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }

    fn visit_local_variable(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        let remapped_signature: Option<Cow<'_, JavaStr>> =
            self.remapper.map_signature(signature, true);
        self.mv.visit_local_variable(
            name,
            &self.remapper.map_desc(descriptor),
            remapped_signature.as_deref(),
            start,
            end,
            index,
        );
    }

    fn visit_local_variable_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.mv.visit_local_variable_annotation(
            type_ref,
            type_path,
            start,
            end,
            index,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Self::annotation_remapper(
            remapper,
            Some(descriptor),
            annotation_visitor,
        ))
    }
}
