//! [`CheckMethodAdapter`]: ASM's `CheckMethodAdapter` (and its `MethodWriterWrapper`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use oxasm::constant::{Constant, FrameValue};
use oxasm::opcodes::*;
use oxasm::{
    AnnotationVisitor, Attribute, ClassWriterFlags, FrameKind, Handle, HandleKind, Interner,
    JavaStr, JavaString, Label, MethodVisitor, TypePath, TypeReference, TypeSort,
};
use oxasm_analysis::{Analyzer, AnalyzerHooks, BasicValue, BasicVerifier, Frame};
use oxasm_tree::MethodNode;

use crate::check::CheckAnnotationAdapter;
use crate::check::frame_analyzer::check_frame_analyzer;
use crate::check::support::{
    INVALID, Utf16, check_access, check_constant, check_descriptor, check_field_signature,
    check_internal_name, check_method_descriptor, check_method_identifier, check_type_ref,
    check_type_ref_sort, check_unqualified_name, illegal_argument, illegal_state,
    unsupported_operation,
};
use crate::printer::{Printer, TextNode};
use crate::textifier::Textifier;
use crate::trace::TraceMethodVisitor;

/// The "generic" instruction visit methods (those that take an opcode argument).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisitMethod {
    Insn,
    IntInsn,
    VarInsn,
    TypeInsn,
    FieldInsn,
    MethodInsn,
    JumpInsn,
}

impl VisitMethod {
    /// The name of the Java enum constant.
    fn java_name(self) -> &'static str {
        match self {
            VisitMethod::Insn => "VISIT_INSN",
            VisitMethod::IntInsn => "VISIT_INT_INSN",
            VisitMethod::VarInsn => "VISIT_VAR_INSN",
            VisitMethod::TypeInsn => "VISIT_TYPE_INSN",
            VisitMethod::FieldInsn => "VISIT_FIELD_INSN",
            VisitMethod::MethodInsn => "VISIT_METHOD_INSN",
            VisitMethod::JumpInsn => "VISIT_JUMP_INSN",
        }
    }
}

/// The method to use to visit each opcode (`OPCODE_METHODS`); `None` for the opcodes that have a
/// specific visit method (or none at all).
fn opcode_method(opcode: u8) -> Option<VisitMethod> {
    match opcode {
        NOP..=DCONST_1 => Some(VisitMethod::Insn),
        BIPUSH | SIPUSH | NEWARRAY => Some(VisitMethod::IntInsn),
        ILOAD..=ALOAD | ISTORE..=ASTORE | RET => Some(VisitMethod::VarInsn),
        IALOAD..=SALOAD | IASTORE..=LXOR | I2L..=DCMPG | IRETURN..=RETURN => {
            Some(VisitMethod::Insn)
        }
        ARRAYLENGTH | ATHROW | MONITORENTER | MONITOREXIT => Some(VisitMethod::Insn),
        IFEQ..=JSR | IFNULL | IFNONNULL => Some(VisitMethod::JumpInsn),
        GETSTATIC..=PUTFIELD => Some(VisitMethod::FieldInsn),
        INVOKEVIRTUAL..=INVOKEINTERFACE => Some(VisitMethod::MethodInsn),
        NEW | ANEWARRAY | CHECKCAST | INSTANCEOF => Some(VisitMethod::TypeInsn),
        // LDC, the xLOAD_n and xSTORE_n, IINC, the switches, INVOKEDYNAMIC, WIDE and
        // MULTIANEWARRAY.
        _ => None,
    }
}

/// What ASM's `MethodWriterWrapper` tells about the method writer downstream of a check adapter
/// whose class adapter's downstream is a class writer: what it computes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MethodWriterWrapper {
    /// The class version number.
    pub(crate) version: u32,
    /// The flags of the class writer.
    pub(crate) flags: ClassWriterFlags,
}

impl MethodWriterWrapper {
    fn computes_maxs(self) -> bool {
        self.flags.contains(ClassWriterFlags::COMPUTE_MAXS)
            || self.flags.contains(ClassWriterFlags::COMPUTE_FRAMES)
    }

    fn computes_frames(self) -> bool {
        self.flags.contains(ClassWriterFlags::COMPUTE_FRAMES)
    }

    fn requires_frames(self) -> bool {
        (self.version & 0xffff) >= V1_7
    }
}

/// The downstream visitor of a check adapter.
enum Downstream<'a> {
    /// The events are forwarded as they are visited.
    Direct(Option<Box<dyn MethodVisitor + 'a>>),
    /// The events are collected in a method node, checked with an analyzer at the end of the
    /// method, and only then forwarded (ASM's anonymous `MethodNode` subclass).
    DataFlow(Box<DataFlow<'a>>),
}

struct DataFlow<'a> {
    node: MethodNode,
    /// The visitor to which the method is forwarded once checked.
    next: Option<Box<dyn MethodVisitor + 'a>>,
    /// Whether `next` is a method writer, and what it computes.
    method_writer: Option<MethodWriterWrapper>,
}

impl DataFlow<'_> {
    /// Checks the method with an analyzer, and forwards it to `next` (the `visitEnd` of ASM's
    /// `MethodNode` subclass).
    fn visit_end(&mut self) {
        let original_max_locals = self.node.max_locals;
        let original_max_stack = self.node.max_stack;
        // If 'next' is a method writer of a class writer with no flags to compute the max stack
        // and locals nor the stack map frames, valid max stack and locals must be provided
        // (and valid frames, if it does not compute them). Otherwise they are assumed not to be
        // needed at this stage.
        let (check_max_stack_and_locals, check_frames) = match self.method_writer {
            Some(method_writer) => (
                !method_writer.computes_maxs(),
                method_writer.requires_frames() && !method_writer.computes_frames(),
            ),
            None => (false, false),
        };
        let result = if check_frames {
            analyze(
                &mut check_frame_analyzer(BasicVerifier::new()),
                &mut self.node,
                check_max_stack_and_locals,
            )
        } else {
            analyze(
                &mut Analyzer::new(BasicVerifier::new()),
                &mut self.node,
                check_max_stack_and_locals,
            )
        };
        if let Err(message) = result {
            illegal_argument(message);
        }
        if let Some(next) = &mut self.next {
            self.node.max_locals = original_max_locals;
            self.node.max_stack = original_max_stack;
            self.node.accept_method_visitor(&mut **next);
        }
    }
}

/// Analyzes `method` with `analyzer`, and returns the message of ASM's exception if this fails:
/// the analyzer's, followed by the method's instructions and frames.
fn analyze<H: AnalyzerHooks<BasicValue>>(
    analyzer: &mut Analyzer<BasicVerifier, H>,
    method: &mut MethodNode,
    check_max_stack_and_locals: bool,
) -> Result<(), JavaString> {
    let owner = oxasm::jstr!("dummy");
    let result = if check_max_stack_and_locals {
        analyzer.analyze(owner, method).map(|_| ())
    } else {
        analyzer.analyze_and_compute_maxs(owner, method).map(|_| ())
    };
    let Err(error) = result else {
        return Ok(());
    };
    // ASM's analyzeAndComputeMaxs sets maxStack to -1 for the analysis.
    let max_stack = if check_max_stack_and_locals {
        i64::from(method.max_stack)
    } else {
        -1
    };
    let mut message = error.message;
    message.push(' ');
    print_analyzer_result(
        method,
        analyzer.get_frames(),
        max_stack,
        i64::from(method.max_locals),
        &mut message,
    );
    Err(message)
}

/// Prints the instructions of a method, with the frame computed by an analyzer before each one
/// (ASM's `CheckClassAdapter.printAnalyzerResult`). `max_stack` and `max_locals` are the
/// method's, as ASM sees them.
pub(crate) fn print_analyzer_result(
    method: &MethodNode,
    frames: &[Option<Frame<BasicValue>>],
    max_stack: i64,
    max_locals: i64,
    out: &mut JavaString,
) {
    let mut trace_method_visitor = TraceMethodVisitor::new(Textifier::new());

    out.push_java_str(method.resolve(method.name));
    out.push_java_str(method.resolve(method.desc));
    out.push('\n');
    for (i, id) in method.instructions.ids().enumerate() {
        method
            .instructions
            .accept_insn(id, method.interner(), &mut trace_method_visitor);

        let mut string_builder = JavaString::new();
        match frames.get(i).and_then(Option::as_ref) {
            None => string_builder.push('?'),
            Some(frame) => {
                for j in 0..frame.get_locals() {
                    let name = unqualified_name(&frame.get_local(j).to_java_string());
                    string_builder.push_java_str(&name);
                    string_builder.push(' ');
                }
                string_builder.push_str(" : ");
                for j in 0..frame.get_stack_size() {
                    let name = unqualified_name(&frame.get_stack(j).to_java_string());
                    string_builder.push_java_str(&name);
                    string_builder.push(' ');
                }
            }
        }
        let length = i64::try_from(string_builder.len_utf16()).unwrap_or(i64::MAX);
        for _ in length..max_stack + max_locals + 1 {
            string_builder.push(' ');
        }
        // Integer.toString(i + 100000).substring(1).
        out.push_str(&(i + 100_000).to_string()[1..]);
        out.push(' ');
        out.push_java_str(&string_builder);
        out.push_str(" : ");
        push_last_text(out, &trace_method_visitor.p);
    }
    for try_catch_block in &method.try_catch_blocks {
        try_catch_block.accept(method.interner(), &mut trace_method_visitor);
        out.push(' ');
        push_last_text(out, &trace_method_visitor.p);
    }
    out.push('\n');
}

/// Appends the last element of the text of a printer, as Java's string concatenation converts
/// it: a string as is, a child printer's text as a `List.toString()`.
fn push_last_text(out: &mut JavaString, printer: &dyn Printer) {
    fn push_node(out: &mut JavaString, node: &TextNode) {
        match node {
            TextNode::Str(s) => out.push_java_str(s),
            TextNode::Printer(p) => {
                out.push('[');
                for (i, child) in p.get_text().iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    push_node(out, child);
                }
                out.push(']');
            }
        }
    }
    if let Some(last) = printer.get_text().last() {
        push_node(out, last);
    }
}

/// The name of a value without its package (`CheckClassAdapter.getUnqualifiedName`).
fn unqualified_name(name: &JavaStr) -> JavaString {
    let utf16 = Utf16::new(name);
    let units: Vec<u16> = name.encode_utf16().collect();
    let Some(last_slash_index) = utf16.last_index_of('/') else {
        return name.to_owned();
    };
    let mut end_index = units.len();
    if units[end_index - 1] == u16::from(b';') {
        end_index -= 1;
    }
    match utf16.last_index_of('[') {
        None => JavaString::from_utf16(&units[last_slash_index + 1..end_index]),
        Some(last_bracket_index) => {
            let mut result = JavaString::from_utf16(&units[..=last_bracket_index]);
            result.push_java_str(&JavaString::from_utf16(
                &units[last_slash_index + 1..end_index],
            ));
            result
        }
    }
}

/// The index of the instruction designated by each visited label: owned, or shared with the
/// other methods of a class (by a `CheckClassAdapter`).
enum LabelInsnIndices<'a> {
    Owned(HashMap<Label, usize>),
    Shared(&'a mut HashMap<Label, usize>),
}

/// A [`MethodVisitor`] that checks that its methods are properly used, and forwards them to an
/// optional downstream visitor.
///
/// More precisely, it checks each instruction individually: each visit method checks some
/// preconditions based *only* on its arguments, such as the fact that the given opcode is
/// correct for the visit method. It can also perform some basic data flow checks (those that
/// can be performed without the full class hierarchy; see [`BasicVerifier`]): in a method whose
/// descriptor is `()V`, the invalid instruction `IRETURN`, or the invalid sequence `IADD L2I`,
/// are detected if the data flow checks are enabled ([`with_data_flow`](Self::with_data_flow)).
/// The method is then forwarded at its end, once checked.
///
/// The checks that ASM makes on arguments that the visitor types make impossible (a `null`
/// label, a variable index that is not an unsigned short, ...) have no counterpart.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckMethodAdapter<'a> {
    /// The class version number (`version`), which a `CheckClassAdapter` sets. Names are checked
    /// as Java identifiers below Java 5, and the instructions that need a minimum version are
    /// checked against it.
    pub version: u32,
    /// The access flags of the visited method (only known with the data flow checks).
    access: u32,
    /// The number of method parameters that can have runtime visible annotations. 0 means that
    /// all the parameters from the method descriptor can have annotations.
    visible_annotable_parameter_count: u8,
    /// The number of method parameters that can have runtime invisible annotations. 0 means that
    /// all the parameters from the method descriptor can have annotations.
    invisible_annotable_parameter_count: u8,
    visit_code_called: bool,
    visit_max_called: bool,
    visit_end_called: bool,
    /// The number of visited instructions so far.
    insn_count: usize,
    /// The index of the instruction designated by each visited label.
    label_insn_indices: LabelInsnIndices<'a>,
    /// The labels referenced by the visited method.
    referenced_labels: HashSet<Label>,
    /// The index of the instruction corresponding to the last visited stack map frame.
    last_frame_insn_index: Option<usize>,
    /// The number of visited frames in expanded form.
    num_expanded_frames: usize,
    /// The number of visited frames in compressed form.
    num_compressed_frames: usize,
    /// The exception handler ranges: the start and end labels of each exception handler block.
    handlers: Vec<(Label, Label)>,
    mv: Downstream<'a>,
}

impl<'a> CheckMethodAdapter<'a> {
    /// Constructs a new adapter that forwards the events to `method_visitor`, without data flow
    /// checks.
    pub fn new(method_visitor: Option<Box<dyn MethodVisitor + 'a>>) -> Self {
        CheckMethodAdapter::with_downstream(0, Downstream::Direct(method_visitor))
    }

    /// Constructs a new adapter that performs basic data flow checks, and then forwards the
    /// method to `method_visitor`, at its end. `access`, `name` and `descriptor` are those of
    /// the method.
    pub fn with_data_flow(
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        method_visitor: Option<Box<dyn MethodVisitor + 'a>>,
    ) -> Self {
        CheckMethodAdapter::with_data_flow_in(
            Arc::default(),
            access,
            name,
            descriptor,
            method_visitor,
        )
    }

    /// As [`with_data_flow`](Self::with_data_flow), storing the method in a node whose strings
    /// are symbols of the given interner (which a [`CheckClassAdapter`] shares between the
    /// methods of a class).
    ///
    /// [`CheckClassAdapter`]: crate::check::CheckClassAdapter
    pub fn with_data_flow_in(
        interner: Arc<Interner>,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        method_visitor: Option<Box<dyn MethodVisitor + 'a>>,
    ) -> Self {
        let name = interner.intern(name);
        let descriptor = interner.intern(descriptor);
        CheckMethodAdapter::with_downstream(
            access,
            Downstream::DataFlow(Box::new(DataFlow {
                node: MethodNode::new(interner, access, name, descriptor, None, Vec::new()),
                next: method_visitor,
                method_writer: None,
            })),
        )
    }

    fn with_downstream(access: u32, mv: Downstream<'a>) -> Self {
        CheckMethodAdapter {
            version: 0,
            access,
            visible_annotable_parameter_count: 0,
            invisible_annotable_parameter_count: 0,
            visit_code_called: false,
            visit_max_called: false,
            visit_end_called: false,
            insn_count: 0,
            label_insn_indices: LabelInsnIndices::Owned(HashMap::new()),
            referenced_labels: HashSet::new(),
            last_frame_insn_index: None,
            num_expanded_frames: 0,
            num_compressed_frames: 0,
            handlers: Vec::new(),
            mv,
        }
    }

    /// Uses the given map of the index of the instruction designated by each visited label so
    /// far (in other methods), which this adapter updates with the labels of the visited method
    /// (ASM's `labelInsnIndices` constructor argument).
    #[must_use]
    pub fn with_label_insn_indices(
        mut self,
        label_insn_indices: &'a mut HashMap<Label, usize>,
    ) -> Self {
        self.label_insn_indices = LabelInsnIndices::Shared(label_insn_indices);
        self
    }

    /// Tells a data flow checking adapter that its downstream visitor is a method writer of a
    /// class writer with the given flags (ASM's `MethodWriterWrapper`).
    pub(crate) fn with_method_writer(mut self, method_writer: MethodWriterWrapper) -> Self {
        if let Downstream::DataFlow(data_flow) = &mut self.mv {
            data_flow.method_writer = Some(method_writer);
        }
        self
    }

    fn label_insn_indices(&self) -> &HashMap<Label, usize> {
        match &self.label_insn_indices {
            LabelInsnIndices::Owned(map) => map,
            LabelInsnIndices::Shared(map) => map,
        }
    }

    fn label_insn_indices_mut(&mut self) -> &mut HashMap<Label, usize> {
        match &mut self.label_insn_indices {
            LabelInsnIndices::Owned(map) => map,
            LabelInsnIndices::Shared(map) => map,
        }
    }

    /// The visitor to which the events are forwarded as they are visited.
    fn mv(&mut self) -> Option<&mut dyn MethodVisitor> {
        match &mut self.mv {
            Downstream::Direct(Some(mv)) => Some(&mut **mv),
            Downstream::Direct(None) => None,
            Downstream::DataFlow(data_flow) => Some(&mut data_flow.node),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Utility methods
    // ---------------------------------------------------------------------------------------------

    fn check_visit_code_called(&self) {
        if !self.visit_code_called {
            illegal_state("Cannot visit instructions before visitCode has been called.");
        }
    }

    fn check_visit_maxs_not_called(&self) {
        if self.visit_max_called {
            illegal_state("Cannot visit instructions after visitMaxs has been called.");
        }
    }

    fn check_visit_end_not_called(&self) {
        if self.visit_end_called {
            illegal_state("Cannot visit elements after visitEnd has been called.");
        }
    }

    fn check_insn_state(&self) {
        self.check_visit_code_called();
        self.check_visit_maxs_not_called();
    }

    /// Checks a stack map frame value.
    fn check_frame_value(&mut self, value: &FrameValue<'_>) {
        match value {
            FrameValue::Object(name) => {
                check_internal_name(self.version, name, "Invalid stack frame value");
            }
            FrameValue::Uninitialized(label) => self.check_label(*label, false, "label"),
            _ => {}
        }
    }

    /// Checks that a label has been visited, if `check_visited`, and records it as referenced.
    fn check_label(&mut self, label: Label, check_visited: bool, message: &str) {
        if check_visited && !self.label_insn_indices().contains_key(&label) {
            illegal_argument(format_args!("{INVALID}{message} (must be visited first)"));
        }
        self.referenced_labels.insert(label);
    }

    /// Checks that the given value is a valid operand for the `LDC` instruction.
    fn check_ldc_constant(&self, value: &Constant<'_>) {
        let version = self.version & 0xffff;
        match value {
            Constant::Type(ty) => {
                let sort = ty.get_sort();
                if !matches!(sort, TypeSort::Object | TypeSort::Array | TypeSort::Method) {
                    illegal_argument("Illegal LDC constant value");
                }
                if sort != TypeSort::Method && version < V1_5 {
                    illegal_argument("ldc of a constant class requires at least version 1.5");
                }
                if sort == TypeSort::Method && version < V1_7 {
                    illegal_argument("ldc of a method type requires at least version 1.7");
                }
            }
            Constant::Handle(handle) => {
                if version < V1_7 {
                    illegal_argument("ldc of a Handle requires at least version 1.7");
                }
                self.check_handle(handle);
            }
            Constant::Dynamic(constant_dynamic) => {
                if version < V11 {
                    illegal_argument("ldc of a ConstantDynamic requires at least version 11");
                }
                check_method_identifier(
                    self.version,
                    constant_dynamic.get_name(),
                    "constant dynamic name",
                );
                check_descriptor(self.version, constant_dynamic.get_descriptor(), false);
                self.check_ldc_constant(&Constant::Handle(
                    constant_dynamic.get_bootstrap_method().clone(),
                ));
                for argument in constant_dynamic.get_bootstrap_method_arguments() {
                    self.check_ldc_constant(argument);
                }
            }
            _ => check_constant(value),
        }
    }

    /// The checks of a method handle constant (its tag is valid by construction).
    fn check_handle(&self, handle: &Handle) {
        let tag = handle.get_tag();
        check_internal_name(self.version, handle.get_owner(), "handle owner");
        if matches!(
            tag,
            HandleKind::GetField
                | HandleKind::GetStatic
                | HandleKind::PutField
                | HandleKind::PutStatic
        ) {
            check_descriptor(self.version, handle.get_desc(), false);
        } else {
            check_method_descriptor(self.version, handle.get_desc());
        }
        let handle_name = handle.get_name();
        if !(handle_name == "<init>" && tag == HandleKind::NewInvokeSpecial) {
            check_method_identifier(self.version, handle_name, "handle name");
        }
    }
}

/// Checks that the method to visit the given opcode is the given method.
fn check_opcode_method(opcode: u8, method: VisitMethod) {
    if opcode > IFNONNULL {
        illegal_argument(format_args!("Invalid opcode: {opcode}"));
    }
    if opcode_method(opcode) != Some(method) {
        illegal_argument(format_args!(
            "Invalid combination of opcode and method: {opcode}, {}",
            method.java_name()
        ));
    }
}

/// Checks that the given value is a signed byte.
fn check_signed_byte(value: i32, message: &str) {
    if i8::try_from(value).is_err() {
        illegal_argument(format_args!("{message} (must be a signed byte): {value}"));
    }
}

/// Checks that the given value is a signed short.
fn check_signed_short(value: i32, message: &str) {
    if i16::try_from(value).is_err() {
        illegal_argument(format_args!("{message} (must be a signed short): {value}"));
    }
}

impl std::fmt::Debug for CheckMethodAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckMethodAdapter")
            .field("version", &self.version)
            .field("access", &self.access)
            .field("insn_count", &self.insn_count)
            .finish_non_exhaustive()
    }
}

impl MethodVisitor for CheckMethodAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        self.mv()
    }

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        if let Some(name) = name {
            check_unqualified_name(self.version, name, "name");
        }
        check_access(access, ACC_FINAL | ACC_MANDATED | ACC_SYNTHETIC);
        if let Some(mv) = self.mv() {
            mv.visit_parameter(name, access);
        }
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        check_descriptor(self.version, descriptor, false);
        let av = self
            .mv()
            .and_then(|mv| mv.visit_annotation(descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        check_type_ref_sort(
            type_ref,
            &[
                TypeReference::METHOD_TYPE_PARAMETER,
                TypeReference::METHOD_TYPE_PARAMETER_BOUND,
                TypeReference::METHOD_RETURN,
                TypeReference::METHOD_RECEIVER,
                TypeReference::METHOD_FORMAL_PARAMETER,
                TypeReference::THROWS,
            ],
        );
        check_type_ref(type_ref);
        check_descriptor(self.version, descriptor, false);
        let av = self
            .mv()
            .and_then(|mv| mv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_annotation_default<'b>(&'b mut self) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        let av = self.mv().and_then(|mv| mv.visit_annotation_default());
        CheckAnnotationAdapter::boxed(av, false)
    }

    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        self.check_visit_end_not_called();
        if visible {
            self.visible_annotable_parameter_count = parameter_count;
        } else {
            self.invisible_annotable_parameter_count = parameter_count;
        }
        if let Some(mv) = self.mv() {
            mv.visit_annotable_parameter_count(parameter_count, visible);
        }
    }

    fn visit_parameter_annotation<'b>(
        &'b mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        let count = if visible {
            self.visible_annotable_parameter_count
        } else {
            self.invisible_annotable_parameter_count
        };
        if count > 0 && parameter >= count {
            illegal_argument("Invalid parameter index");
        }
        check_descriptor(self.version, descriptor, false);
        let av = self
            .mv()
            .and_then(|mv| mv.visit_parameter_annotation(parameter, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.check_visit_end_not_called();
        if let Some(mv) = self.mv() {
            mv.visit_attribute(attribute);
        }
    }

    fn visit_code(&mut self) {
        if self.access & ACC_ABSTRACT != 0 {
            unsupported_operation("Abstract methods cannot have code");
        }
        self.visit_code_called = true;
        if let Some(mv) = self.mv() {
            mv.visit_code();
        }
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        if self.last_frame_insn_index == Some(self.insn_count) {
            illegal_state("At most one frame can be visited at a given code location.");
        }
        self.last_frame_insn_index = Some(self.insn_count);
        let r#type = kind.to_i32();
        let (max_num_local, max_num_stack) = match kind {
            FrameKind::New | FrameKind::Full => (usize::MAX, usize::MAX),
            FrameKind::Same => (0, 0),
            FrameKind::Same1 => (0, 1),
            FrameKind::Append | FrameKind::Chop(_) => (3, 0),
            FrameKind::Insert => illegal_argument(format_args!("Invalid frame type {type}")),
        };
        let num_local = match kind {
            FrameKind::Chop(count) => usize::from(count),
            _ => local.len(),
        };
        if num_local > max_num_local {
            illegal_argument(format_args!(
                "Invalid numLocal={num_local} for frame type {type}"
            ));
        }
        if stack.len() > max_num_stack {
            illegal_argument(format_args!(
                "Invalid numStack={} for frame type {type}",
                stack.len()
            ));
        }
        if !matches!(kind, FrameKind::Chop(_)) {
            for value in local {
                self.check_frame_value(value);
            }
        }
        for value in stack {
            self.check_frame_value(value);
        }
        if kind == FrameKind::New {
            self.num_expanded_frames += 1;
        } else {
            self.num_compressed_frames += 1;
        }
        if self.num_expanded_frames > 0 && self.num_compressed_frames > 0 {
            illegal_argument("Expanded and compressed frames must not be mixed.");
        }
        if let Some(mv) = self.mv() {
            mv.visit_frame(kind, local, stack);
        }
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::Insn);
        if let Some(mv) = self.mv() {
            mv.visit_insn(opcode);
        }
        self.insn_count += 1;
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::IntInsn);
        match opcode {
            BIPUSH => check_signed_byte(operand, "Invalid operand"),
            SIPUSH => check_signed_short(operand, "Invalid operand"),
            // NEWARRAY.
            _ => {
                if !(T_BOOLEAN..=T_LONG).contains(&operand) {
                    illegal_argument(format_args!(
                        "Invalid operand (must be an array type code T_...): {operand}"
                    ));
                }
            }
        }
        if let Some(mv) = self.mv() {
            mv.visit_int_insn(opcode, operand);
        }
        self.insn_count += 1;
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::VarInsn);
        if let Some(mv) = self.mv() {
            mv.visit_var_insn(opcode, var_index);
        }
        self.insn_count += 1;
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::TypeInsn);
        check_internal_name(self.version, type_, "type");
        if opcode == NEW && type_.starts_with('[') {
            illegal_argument(format_args!("NEW cannot be used to create arrays: {type_}"));
        }
        if let Some(mv) = self.mv() {
            mv.visit_type_insn(opcode, type_);
        }
        self.insn_count += 1;
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::FieldInsn);
        check_internal_name(self.version, owner, "owner");
        check_unqualified_name(self.version, name, "name");
        check_descriptor(self.version, descriptor, false);
        if let Some(mv) = self.mv() {
            mv.visit_field_insn(opcode, owner, name, descriptor);
        }
        self.insn_count += 1;
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::MethodInsn);
        if opcode != INVOKESPECIAL || name != "<init>" {
            check_method_identifier(self.version, name, "name");
        }
        check_internal_name(self.version, owner, "owner");
        check_method_descriptor(self.version, descriptor);
        if opcode == INVOKEVIRTUAL && is_interface {
            illegal_argument("INVOKEVIRTUAL can't be used with interfaces");
        }
        if opcode == INVOKEINTERFACE && !is_interface {
            illegal_argument("INVOKEINTERFACE can't be used with classes");
        }
        if opcode == INVOKESPECIAL && is_interface && (self.version & 0xffff) < V1_8 {
            illegal_argument("INVOKESPECIAL can't be used with interfaces prior to Java 8");
        }
        if let Some(mv) = self.mv() {
            mv.visit_method_insn(opcode, owner, name, descriptor, is_interface);
        }
        self.insn_count += 1;
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.check_insn_state();
        check_method_identifier(self.version, name, "name");
        check_method_descriptor(self.version, descriptor);
        let tag = bootstrap_method_handle.get_tag();
        if tag != HandleKind::InvokeStatic && tag != HandleKind::NewInvokeSpecial {
            illegal_argument(format_args!("invalid handle tag {}", tag.tag()));
        }
        for argument in bootstrap_method_arguments {
            self.check_ldc_constant(argument);
        }
        if let Some(mv) = self.mv() {
            mv.visit_invoke_dynamic_insn(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            );
        }
        self.insn_count += 1;
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.check_insn_state();
        check_opcode_method(opcode, VisitMethod::JumpInsn);
        self.check_label(label, false, "label");
        if let Some(mv) = self.mv() {
            mv.visit_jump_insn(opcode, label);
        }
        self.insn_count += 1;
    }

    fn visit_label(&mut self, label: Label) {
        self.check_insn_state();
        self.check_label(label, false, "label");
        if self.label_insn_indices().contains_key(&label) {
            illegal_state("Already visited label");
        }
        let insn_count = self.insn_count;
        self.label_insn_indices_mut().insert(label, insn_count);
        if let Some(mv) = self.mv() {
            mv.visit_label(label);
        }
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.check_insn_state();
        self.check_ldc_constant(value);
        if let Some(mv) = self.mv() {
            mv.visit_ldc_insn(value);
        }
        self.insn_count += 1;
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.check_insn_state();
        if let Some(mv) = self.mv() {
            mv.visit_iinc_insn(var_index, increment);
        }
        self.insn_count += 1;
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.check_insn_state();
        if max < min {
            illegal_argument(format_args!(
                "Max = {max} must be greater than or equal to min = {min}"
            ));
        }
        self.check_label(dflt, false, "default label");
        // Java's int arithmetic.
        if i32::try_from(labels.len()).ok() != Some(max.wrapping_sub(min).wrapping_add(1)) {
            illegal_argument("There must be max - min + 1 labels");
        }
        for (i, label) in labels.iter().enumerate() {
            self.check_label(*label, false, &format!("label at index {i}"));
        }
        if let Some(mv) = self.mv() {
            mv.visit_table_switch_insn(min, max, dflt, labels);
        }
        self.insn_count += 1;
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.check_visit_maxs_not_called();
        self.check_visit_code_called();
        self.check_label(dflt, false, "default label");
        if keys.len() != labels.len() {
            illegal_argument("There must be the same number of keys and labels");
        }
        if keys.windows(2).any(|pair| pair[1] < pair[0]) {
            illegal_argument("The keys must be sorted in increasing order");
        }
        for (i, label) in labels.iter().enumerate() {
            self.check_label(*label, false, &format!("label at index {i}"));
        }
        if let Some(mv) = self.mv() {
            mv.visit_lookup_switch_insn(dflt, keys, labels);
        }
        self.insn_count += 1;
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.check_insn_state();
        check_descriptor(self.version, descriptor, false);
        if !descriptor.starts_with('[') {
            illegal_argument(format_args!(
                "Invalid descriptor (must be an array type descriptor): {descriptor}"
            ));
        }
        if num_dimensions < 1 {
            illegal_argument(format_args!(
                "Invalid dimensions (must be greater than 0): {num_dimensions}"
            ));
        }
        let max_dimensions = Utf16::new(descriptor)
            .last_index_of('[')
            .map_or(0, |i| i + 1);
        if usize::from(num_dimensions) > max_dimensions {
            illegal_argument(format_args!(
                "Invalid dimensions (must not be greater than numDimensions(descriptor)): \
                 {num_dimensions}"
            ));
        }
        if let Some(mv) = self.mv() {
            mv.visit_multi_a_new_array_insn(descriptor, num_dimensions);
        }
        self.insn_count += 1;
    }

    fn visit_insn_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_insn_state();
        check_type_ref_sort(
            type_ref,
            &[
                TypeReference::INSTANCEOF,
                TypeReference::NEW,
                TypeReference::CONSTRUCTOR_REFERENCE,
                TypeReference::METHOD_REFERENCE,
                TypeReference::CAST,
                TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT,
                TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT,
                TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT,
                TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT,
            ],
        );
        check_type_ref(type_ref);
        check_descriptor(self.version, descriptor, false);
        let av = self
            .mv()
            .and_then(|mv| mv.visit_insn_annotation(type_ref, type_path, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.check_insn_state();
        self.check_label(start, false, "start label");
        self.check_label(end, false, "end label");
        self.check_label(handler, false, "handler label");
        let label_insn_indices = self.label_insn_indices();
        if label_insn_indices.contains_key(&start)
            || label_insn_indices.contains_key(&end)
            || label_insn_indices.contains_key(&handler)
        {
            illegal_state("Try catch blocks must be visited before their labels");
        }
        if let Some(type_) = type_ {
            check_internal_name(self.version, type_, "type");
        }
        if let Some(mv) = self.mv() {
            mv.visit_try_catch_block(start, end, handler, type_);
        }
        self.handlers.push((start, end));
    }

    fn visit_try_catch_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_insn_state();
        check_type_ref_sort(type_ref, &[TypeReference::EXCEPTION_PARAMETER]);
        check_type_ref(type_ref);
        check_descriptor(self.version, descriptor, false);
        let av = self
            .mv()
            .and_then(|mv| mv.visit_try_catch_annotation(type_ref, type_path, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
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
        self.check_insn_state();
        check_unqualified_name(self.version, name, "name");
        check_descriptor(self.version, descriptor, false);
        if let Some(signature) = signature {
            check_field_signature(signature);
        }
        self.check_label(start, true, "start label");
        self.check_label(end, true, "end label");
        let label_insn_indices = self.label_insn_indices();
        if label_insn_indices[&end] < label_insn_indices[&start] {
            illegal_argument("Invalid start and end labels (end must be greater than start)");
        }
        if let Some(mv) = self.mv() {
            mv.visit_local_variable(name, descriptor, signature, start, end, index);
        }
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
        self.check_insn_state();
        check_type_ref_sort(
            type_ref,
            &[
                TypeReference::LOCAL_VARIABLE,
                TypeReference::RESOURCE_VARIABLE,
            ],
        );
        check_type_ref(type_ref);
        check_descriptor(self.version, descriptor, false);
        if end.len() != start.len() || index.len() != start.len() {
            illegal_argument(
                "Invalid start, end and index arrays (must be non null and of identical length",
            );
        }
        for (&start, &end) in start.iter().zip(end) {
            self.check_label(start, true, "start label");
            self.check_label(end, true, "end label");
            let label_insn_indices = self.label_insn_indices();
            if label_insn_indices[&end] < label_insn_indices[&start] {
                illegal_argument("Invalid start and end labels (end must be greater than start)");
            }
        }
        // Not wrapped in a CheckAnnotationAdapter, as in ASM.
        self.mv().and_then(|mv| {
            mv.visit_local_variable_annotation(
                type_ref, type_path, start, end, index, descriptor, visible,
            )
        })
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        self.check_insn_state();
        self.check_label(start, true, "start label");
        if let Some(mv) = self.mv() {
            mv.visit_line_number(line, start);
        }
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.check_insn_state();
        self.visit_max_called = true;
        let label_insn_indices = self.label_insn_indices();
        if self
            .referenced_labels
            .iter()
            .any(|label| !label_insn_indices.contains_key(label))
        {
            illegal_state("Undefined label used");
        }
        for (start, end) in &self.handlers {
            if label_insn_indices[end] <= label_insn_indices[start] {
                illegal_state("Empty try catch block handler range");
            }
        }
        if let Some(mv) = self.mv() {
            mv.visit_maxs(max_stack, max_locals);
        }
    }

    fn visit_end(&mut self) {
        self.check_visit_end_not_called();
        self.visit_end_called = true;
        match &mut self.mv {
            Downstream::Direct(Some(mv)) => mv.visit_end(),
            Downstream::Direct(None) => {}
            Downstream::DataFlow(data_flow) => {
                data_flow.node.visit_end();
                data_flow.visit_end();
            }
        }
    }
}
