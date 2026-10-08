//! The visitors of ASM's benchmarks: `AsmAdapter.CountingVisitor` and
//! `TypeBenchmark.CollectTypesVisitor`, event for event.

use oxasm::opcodes::FrameKind;
use oxasm::{
    AnnotationValue, AnnotationVisitor, ClassVisitor, Constant, FieldVisitor, FrameValue, Handle,
    JavaStr, JavaString, Label, MethodVisitor, ModuleVisitor, TypePath, TypeReference,
};

/// `AsmAdapter.CountingVisitor`: counts the events ASM's counts. Like ASM's, it does not count
/// `visit_method` itself, nor the nest, permitted subclass, record component, attribute,
/// `visit_code` and `visit_end` events, and it visits no module and no record component.
#[derive(Debug, Default)]
pub struct Counter {
    pub count: u64,
}

type Annotation<'a> = Option<Box<dyn AnnotationVisitor + 'a>>;

fn count_annotation(count: &mut u64) -> Annotation<'_> {
    *count += 1;
    Some(Box::new(AnnotationCounter(count)))
}

impl ClassVisitor for Counter {
    fn visit(
        &mut self,
        _: u32,
        _: u32,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) {
        self.count += 1;
    }

    fn visit_source(&mut self, _: Option<&JavaStr>, _: Option<&JavaStr>) {
        self.count += 1;
    }

    fn visit_module<'a>(
        &'a mut self,
        _: &JavaStr,
        _: u32,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        self.count += 1;
        None
    }

    fn visit_outer_class(&mut self, _: &JavaStr, _: Option<&JavaStr>, _: Option<&JavaStr>) {
        self.count += 1;
    }

    fn visit_annotation<'a>(&'a mut self, _: &JavaStr, _: bool) -> Annotation<'a> {
        count_annotation(&mut self.count)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(&mut self.count)
    }

    fn visit_inner_class(&mut self, _: &JavaStr, _: Option<&JavaStr>, _: Option<&JavaStr>, _: u32) {
        self.count += 1;
    }

    fn visit_field<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        self.count += 1;
        Some(Box::new(FieldCounter(&mut self.count)))
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(MethodCounter(&mut self.count)))
    }
}

struct AnnotationCounter<'a>(&'a mut u64);

impl AnnotationVisitor for AnnotationCounter<'_> {
    fn visit(&mut self, _: Option<&JavaStr>, _: &AnnotationValue<'_>) {
        *self.0 += 1;
    }

    fn visit_enum(&mut self, _: Option<&JavaStr>, _: &JavaStr, _: &JavaStr) {
        *self.0 += 1;
    }

    fn visit_annotation<'a>(&'a mut self, _: Option<&JavaStr>, _: &JavaStr) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_array<'a>(&'a mut self, _: Option<&JavaStr>) -> Annotation<'a> {
        count_annotation(self.0)
    }
}

struct FieldCounter<'a>(&'a mut u64);

impl FieldVisitor for FieldCounter<'_> {
    fn visit_annotation<'a>(&'a mut self, _: &JavaStr, _: bool) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(self.0)
    }
}

struct MethodCounter<'a>(&'a mut u64);

impl MethodVisitor for MethodCounter<'_> {
    fn visit_parameter(&mut self, _: Option<&JavaStr>, _: u32) {
        *self.0 += 1;
    }

    fn visit_annotation_default<'a>(&'a mut self) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_annotation<'a>(&'a mut self, _: &JavaStr, _: bool) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_annotable_parameter_count(&mut self, _: u8, _: bool) {
        *self.0 += 1;
    }

    fn visit_parameter_annotation<'a>(&'a mut self, _: u8, _: &JavaStr, _: bool) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_frame(&mut self, _: FrameKind, _: &[FrameValue<'_>], _: &[FrameValue<'_>]) {
        *self.0 += 1;
    }

    fn visit_insn(&mut self, _: u8) {
        *self.0 += 1;
    }

    fn visit_int_insn(&mut self, _: u8, _: i32) {
        *self.0 += 1;
    }

    fn visit_var_insn(&mut self, _: u8, _: u16) {
        *self.0 += 1;
    }

    fn visit_type_insn(&mut self, _: u8, _: &JavaStr) {
        *self.0 += 1;
    }

    fn visit_field_insn(&mut self, _: u8, _: &JavaStr, _: &JavaStr, _: &JavaStr) {
        *self.0 += 1;
    }

    fn visit_method_insn(&mut self, _: u8, _: &JavaStr, _: &JavaStr, _: &JavaStr, _: bool) {
        *self.0 += 1;
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        _: &JavaStr,
        _: &JavaStr,
        _: &Handle,
        _: &[Constant<'_>],
    ) {
        *self.0 += 1;
    }

    fn visit_jump_insn(&mut self, _: u8, _: Label) {
        *self.0 += 1;
    }

    fn visit_label(&mut self, _: Label) {
        *self.0 += 1;
    }

    fn visit_ldc_insn(&mut self, _: &Constant<'_>) {
        *self.0 += 1;
    }

    fn visit_iinc_insn(&mut self, _: u16, _: i16) {
        *self.0 += 1;
    }

    fn visit_table_switch_insn(&mut self, _: i32, _: i32, _: Label, _: &[Label]) {
        *self.0 += 1;
    }

    fn visit_lookup_switch_insn(&mut self, _: Label, _: &[i32], _: &[Label]) {
        *self.0 += 1;
    }

    fn visit_multi_a_new_array_insn(&mut self, _: &JavaStr, _: u8) {
        *self.0 += 1;
    }

    fn visit_insn_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_try_catch_block(&mut self, _: Label, _: Label, _: Label, _: Option<&JavaStr>) {
        *self.0 += 1;
    }

    fn visit_try_catch_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_local_variable(
        &mut self,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Label,
        _: Label,
        _: u16,
    ) {
        *self.0 += 1;
    }

    fn visit_local_variable_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &[Label],
        _: &[Label],
        _: &[u16],
        _: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        count_annotation(self.0)
    }

    fn visit_line_number(&mut self, _: u16, _: Label) {
        *self.0 += 1;
    }

    fn visit_maxs(&mut self, _: u16, _: u16) {
        *self.0 += 1;
    }
}

/// `TypeBenchmark.CollectTypesVisitor`: the field descriptors and method descriptors of a class,
/// in the order the reader visits them, duplicates included. Like ASM's, it visits no annotation
/// array, annotation default, module or record component.
#[derive(Debug)]
pub struct CollectTypes<'v> {
    pub descriptors: &'v mut Vec<JavaString>,
    pub method_descriptors: &'v mut Vec<JavaString>,
}

/// A visitor of `CollectTypes` (or its annotation visitor) for a member or an annotation.
struct Collect<'a> {
    descriptors: &'a mut Vec<JavaString>,
    method_descriptors: &'a mut Vec<JavaString>,
}

fn collect_annotation<'a>(
    descriptors: &'a mut Vec<JavaString>,
    descriptor: &JavaStr,
) -> Annotation<'a> {
    descriptors.push(descriptor.to_owned());
    Some(Box::new(AnnotationCollector(descriptors)))
}

impl ClassVisitor for CollectTypes<'_> {
    fn visit_outer_class(
        &mut self,
        _: &JavaStr,
        _: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        if let Some(descriptor) = descriptor {
            self.method_descriptors.push(descriptor.to_owned());
        }
    }

    fn visit_annotation<'a>(&'a mut self, descriptor: &JavaStr, _: bool) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_field<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        descriptor: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        self.descriptors.push(descriptor.to_owned());
        Some(Box::new(Collect {
            descriptors: self.descriptors,
            method_descriptors: self.method_descriptors,
        }))
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        descriptor: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.method_descriptors.push(descriptor.to_owned());
        Some(Box::new(Collect {
            descriptors: self.descriptors,
            method_descriptors: self.method_descriptors,
        }))
    }
}

struct AnnotationCollector<'a>(&'a mut Vec<JavaString>);

impl AnnotationVisitor for AnnotationCollector<'_> {
    fn visit_enum(&mut self, _: Option<&JavaStr>, descriptor: &JavaStr, _: &JavaStr) {
        self.0.push(descriptor.to_owned());
    }

    fn visit_annotation<'a>(
        &'a mut self,
        _: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Annotation<'a> {
        collect_annotation(self.0, descriptor)
    }
}

impl FieldVisitor for Collect<'_> {
    fn visit_annotation<'a>(&'a mut self, descriptor: &JavaStr, _: bool) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }
}

impl MethodVisitor for Collect<'_> {
    fn visit_annotation<'a>(&'a mut self, descriptor: &JavaStr, _: bool) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_parameter_annotation<'a>(
        &'a mut self,
        _: u8,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_field_insn(&mut self, _: u8, _: &JavaStr, _: &JavaStr, descriptor: &JavaStr) {
        self.descriptors.push(descriptor.to_owned());
    }

    fn visit_method_insn(
        &mut self,
        _: u8,
        _: &JavaStr,
        _: &JavaStr,
        descriptor: &JavaStr,
        _: bool,
    ) {
        self.method_descriptors.push(descriptor.to_owned());
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        _: &JavaStr,
        descriptor: &JavaStr,
        _: &Handle,
        _: &[Constant<'_>],
    ) {
        self.method_descriptors.push(descriptor.to_owned());
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, _: u8) {
        self.descriptors.push(descriptor.to_owned());
    }

    fn visit_insn_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_try_catch_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }

    fn visit_local_variable(
        &mut self,
        _: &JavaStr,
        descriptor: &JavaStr,
        _: Option<&JavaStr>,
        _: Label,
        _: Label,
        _: u16,
    ) {
        self.descriptors.push(descriptor.to_owned());
    }

    fn visit_local_variable_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &[Label],
        _: &[Label],
        _: &[u16],
        descriptor: &JavaStr,
        _: bool,
    ) -> Annotation<'a> {
        collect_annotation(self.descriptors, descriptor)
    }
}
