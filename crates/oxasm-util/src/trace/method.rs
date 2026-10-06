//! [`TraceMethodVisitor`]: ASM's `TraceMethodVisitor`.

use oxasm::constant::{Constant, FrameValue};
use oxasm::{
    AnnotationVisitor, Attribute, FrameKind, Handle, JavaStr, Label, MethodVisitor, TypePath,
    TypeReference,
};

use crate::printer::Printer;
use crate::trace::TraceAnnotationVisitor;

/// A [`MethodVisitor`] that prints the methods it visits with a [`Printer`].
///
/// It never accepts the method copy optimization of a class reader (`try_copy_method`), so a
/// method writer downstream sees every event.
pub struct TraceMethodVisitor<'a, P> {
    mv: Option<Box<dyn MethodVisitor + 'a>>,
    /// The printer to convert the visited method into text.
    pub p: P,
}

impl<P: Printer> TraceMethodVisitor<'_, P> {
    /// Constructs a new visitor, without a downstream visitor.
    pub fn new(printer: P) -> Self {
        TraceMethodVisitor {
            mv: None,
            p: printer,
        }
    }
}

impl<'a, P: Printer> TraceMethodVisitor<'a, P> {
    /// Constructs a new visitor that also forwards the events to `method_visitor`.
    pub fn with_delegate(method_visitor: Option<Box<dyn MethodVisitor + 'a>>, printer: P) -> Self {
        TraceMethodVisitor {
            mv: method_visitor,
            p: printer,
        }
    }
}

impl<P> std::fmt::Debug for TraceMethodVisitor<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceMethodVisitor").finish_non_exhaustive()
    }
}

/// Forwards a method call to the downstream visitor, if any.
macro_rules! downstream {
    ($self:ident.$method:ident($($arg:expr),*)) => {
        if let Some(mv) = &mut $self.mv {
            mv.$method($($arg),*);
        }
    };
}

impl<P: Printer> MethodVisitor for TraceMethodVisitor<'_, P> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        match &mut self.mv {
            Some(mv) => Some(&mut **mv),
            None => None,
        }
    }

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        self.p.visit_parameter(name, access);
        downstream!(self.visit_parameter(name, access));
    }

    fn visit_annotation_default<'b>(&'b mut self) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self.p.visit_annotation_default();
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_annotation_default());
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self.p.visit_method_annotation(descriptor, visible);
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_annotation(descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .p
            .visit_method_type_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        self.p
            .visit_annotable_parameter_count(parameter_count, visible);
        downstream!(self.visit_annotable_parameter_count(parameter_count, visible));
    }

    fn visit_parameter_annotation<'b>(
        &'b mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .p
            .visit_parameter_annotation(parameter, descriptor, visible);
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_parameter_annotation(parameter, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.p.visit_method_attribute(attribute);
        downstream!(self.visit_attribute(attribute));
    }

    fn visit_code(&mut self) {
        self.p.visit_code();
        downstream!(self.visit_code());
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        self.p.visit_frame(kind, locals, stack);
        downstream!(self.visit_frame(kind, locals, stack));
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.p.visit_insn(opcode);
        downstream!(self.visit_insn(opcode));
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.p.visit_int_insn(opcode, operand);
        downstream!(self.visit_int_insn(opcode, operand));
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.p.visit_var_insn(opcode, var_index);
        downstream!(self.visit_var_insn(opcode, var_index));
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.p.visit_type_insn(opcode, type_);
        downstream!(self.visit_type_insn(opcode, type_));
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.p.visit_field_insn(opcode, owner, name, descriptor);
        downstream!(self.visit_field_insn(opcode, owner, name, descriptor));
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.p
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
        downstream!(self.visit_method_insn(opcode, owner, name, descriptor, is_interface));
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.p.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        downstream!(self.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments
        ));
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.p.visit_jump_insn(opcode, label);
        downstream!(self.visit_jump_insn(opcode, label));
    }

    fn visit_label(&mut self, label: Label) {
        self.p.visit_label(label);
        downstream!(self.visit_label(label));
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.p.visit_ldc_insn(value);
        downstream!(self.visit_ldc_insn(value));
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.p.visit_iinc_insn(var_index, increment);
        downstream!(self.visit_iinc_insn(var_index, increment));
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.p.visit_table_switch_insn(min, max, dflt, labels);
        downstream!(self.visit_table_switch_insn(min, max, dflt, labels));
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.p.visit_lookup_switch_insn(dflt, keys, labels);
        downstream!(self.visit_lookup_switch_insn(dflt, keys, labels));
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.p
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
        downstream!(self.visit_multi_a_new_array_insn(descriptor, num_dimensions));
    }

    fn visit_insn_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .p
            .visit_insn_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_insn_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.p.visit_try_catch_block(start, end, handler, type_);
        downstream!(self.visit_try_catch_block(start, end, handler, type_));
    }

    fn visit_try_catch_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .p
            .visit_try_catch_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .mv
            .as_mut()
            .and_then(|mv| mv.visit_try_catch_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
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
        self.p
            .visit_local_variable(name, descriptor, signature, start, end, index);
        downstream!(self.visit_local_variable(name, descriptor, signature, start, end, index));
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
        let annotation_printer = self.p.visit_local_variable_annotation(
            type_ref, type_path, start, end, index, descriptor, visible,
        );
        let av = self.mv.as_mut().and_then(|mv| {
            mv.visit_local_variable_annotation(
                type_ref, type_path, start, end, index, descriptor, visible,
            )
        });
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        self.p.visit_line_number(line, start);
        downstream!(self.visit_line_number(line, start));
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.p.visit_maxs(max_stack, max_locals);
        downstream!(self.visit_maxs(max_stack, max_locals));
    }

    fn visit_end(&mut self) {
        self.p.visit_method_end();
        downstream!(self.visit_end());
    }
}
