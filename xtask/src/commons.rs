//! oxasm's side of the commons parity modes (P8): the class read and written back through an
//! oxasm-commons adapter, as the oracle's `commons --mode` does with ASM's (`Oracle.commons`).

use std::borrow::Cow;
use std::path::Path;

use anyhow::Context;
use oxasm::constant::{Constant, FrameValue};
use oxasm::opcodes::{ACC_ABSTRACT, ACC_NATIVE, GETSTATIC, INVOKESPECIAL, INVOKEVIRTUAL};
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, JavaString, MethodVisitor,
    ParsingOptions,
};
use oxasm::{FrameKind, Handle, Label, Type, jstr};
use oxasm_commons::{
    Advice, AdviceAdapter, AnalyzerAdapter, ClassRemapper, CodeSizeEvaluator, JsrInlinerAdapter,
    LocalVariablesSorter, SerialVersionUidAdder, SimpleRemapper, StaticInitMerger,
    TryCatchBlockSorter,
};

/// The adapter of a commons mode (the oracle's `commons --mode`, see `Oracle.commons`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Commons {
    /// `ClassRemapper(SimpleRemapper(mapping))`, with the tier's mapping.
    Remap,
    /// `SerialVersionUIDAdder`.
    Svuid,
    /// `StaticInitMerger` with the prefix `clinit$`.
    StaticInitMerger,
    /// Every method in a `TryCatchBlockSorter`.
    TcbSort,
    /// Every method in a `JSRInlinerAdapter`.
    Jsr,
    /// Every method in a `LocalVariablesSorter` (`EXPAND_FRAMES`).
    Lvs,
    /// Every method with code in an `AdviceAdapter` that generates `push(1); pop()` on enter
    /// and `push(opcode); pop()` on exit (`EXPAND_FRAMES`).
    Advice,
    /// Every method in an `InstructionAdapter`.
    InstructionAdapter,
    /// AnalyzerAdapterTest's frame inserter (`EXPAND_FRAMES`).
    AnalyzerAdapter,
    /// Each method's `CodeSizeEvaluator` minimum and maximum sizes, as text.
    CodeSize,
}

impl Commons {
    /// Whether oxasm computes its side of this mode (yet).
    pub fn has_oxasm_side(self) -> bool {
        true
    }
}

/// The mapping of the `remap` mode: the oracle's `mapping` output, as key-value pairs.
pub type Mapping = SimpleRemapper;

/// Reads a mapping file in the format of the oracle's `mapping` command: one `key \t value`
/// line per entry, semi-UTF-8.
pub fn read_mapping(path: &Path) -> anyhow::Result<Mapping> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut mapping = rustc_hash::FxHashMap::default();
    for line in bytes.split(|&b| b == b'\n').filter(|line| !line.is_empty()) {
        let tab = line
            .iter()
            .position(|&b| b == b'\t')
            .with_context(|| format!("malformed mapping line in {}", path.display()))?;
        let field = |bytes: &[u8]| {
            JavaStr::from_semi_utf8(bytes)
                .map(JavaStr::to_owned)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
        };
        mapping
            .entry(field(&line[..tab])?)
            .or_insert(field(&line[tab + 1..])?);
    }
    Ok(SimpleRemapper::new(mapping))
}

thread_local! {
    /// Whether the last class [`run`] in the `analyzer-adapter` mode on this thread had an
    /// `INVOKESPECIAL <init>` whose receiver is an initialized reference (see
    /// [`initialized_init_receiver`]).
    static INITIALIZED_INIT_RECEIVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the last class [`run`] on this thread, in the `analyzer-adapter` mode, called
/// `INVOKESPECIAL <init>` on an initialized reference (unverifiable code). ASM's `AnalyzerAdapter`
/// then replaces the receiver in its frame by reference identity, only where the very `String`
/// object that was pushed is, and oxasm's by equality, wherever an equal type is: an accepted
/// divergence (2026-10-09).
pub fn initialized_init_receiver() -> bool {
    INITIALIZED_INIT_RECEIVER.get()
}

/// oxasm's side of a commons mode: the class file written by a `ClassWriter(0)`, or the text of
/// `code-size`.
pub fn run(
    class_file: &[u8],
    commons: Commons,
    mapping: Option<&Mapping>,
) -> Result<Vec<u8>, String> {
    INITIALIZED_INIT_RECEIVER.set(false);
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    if commons == Commons::CodeSize {
        let mut code_sizes = CodeSizes::default();
        reader
            .accept(&mut code_sizes, ParsingOptions::empty())
            .map_err(|e| e.to_string())?;
        return Ok(code_sizes.text);
    }
    let flags = if commons == Commons::Jsr {
        ClassWriterFlags::COMPUTE_MAXS
    } else {
        ClassWriterFlags::empty()
    };
    let mut class_writer = ClassWriter::new(flags);
    let cw = &mut class_writer;
    let accepted = match commons {
        Commons::Svuid => {
            reader.accept(&mut SerialVersionUidAdder::new(cw), ParsingOptions::empty())
        }
        Commons::StaticInitMerger => {
            // Through a ClassNode, as the oracle (which moves ASM's merged <clinit> there).
            let mut class_node = crate::parity::class_node();
            reader
                .accept(
                    &mut StaticInitMerger::new(jstr!("clinit$"), &mut class_node),
                    ParsingOptions::empty(),
                )
                .map_err(|e| e.to_string())?;
            class_node.accept(cw);
            Ok(())
        }
        Commons::TcbSort => reader.accept(
            &mut MethodAdapters::new(cw, TcbSortWrap),
            ParsingOptions::empty(),
        ),
        Commons::Jsr => reader.accept(
            &mut MethodAdapters::new(cw, JsrWrap),
            ParsingOptions::empty(),
        ),
        Commons::InstructionAdapter => reader.accept(
            &mut MethodAdapters::new(cw, InstructionAdapterWrap),
            ParsingOptions::empty(),
        ),
        Commons::AnalyzerAdapter => reader.accept(
            &mut MethodAdapters::new(cw, AnalyzerAdapterWrap),
            ParsingOptions::EXPAND_FRAMES,
        ),
        Commons::Remap => {
            let remapper = mapping.ok_or("the remap mode needs a mapping")?;
            reader.accept(
                &mut ClassRemapper::new(cw, remapper),
                ParsingOptions::empty(),
            )
        }
        Commons::Lvs => reader.accept(
            &mut MethodAdapters::new(cw, LvsWrap),
            ParsingOptions::EXPAND_FRAMES,
        ),
        Commons::Advice => reader.accept(
            &mut MethodAdapters::new(cw, AdviceWrap),
            ParsingOptions::EXPAND_FRAMES,
        ),
        Commons::CodeSize => unreachable!("handled above"),
    };
    accepted.map_err(|e| e.to_string())?;
    class_writer.to_bytes().map_err(|e| e.to_string())
}

/// What a [`MethodAdapters`] wraps each method visitor of its delegate in.
trait MethodWrap {
    #[allow(clippy::too_many_arguments)]
    fn wrap<'a>(
        &self,
        owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a>;
}

/// A class visitor that wraps the method visitors of its delegate (the oracle's
/// `MethodAdapters`).
struct MethodAdapters<V, W> {
    class_visitor: V,
    wrap: W,
    owner: JavaString,
}

impl<V: ClassVisitor, W: MethodWrap> MethodAdapters<V, W> {
    fn new(class_visitor: V, wrap: W) -> Self {
        MethodAdapters {
            class_visitor,
            wrap,
            owner: JavaString::new(),
        }
    }
}

impl<V: ClassVisitor, W: MethodWrap> ClassVisitor for MethodAdapters<V, W> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.class_visitor)
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.owner = name.to_owned();
        self.class_visitor
            .visit(version, access, name, signature, super_name, interfaces);
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let method_visitor = self
            .class_visitor
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(self.wrap.wrap(
            &self.owner,
            access,
            name,
            descriptor,
            signature,
            exceptions,
            method_visitor,
        ))
    }
}

struct InstructionAdapterWrap;

impl MethodWrap for InstructionAdapterWrap {
    fn wrap<'a>(
        &self,
        _owner: &JavaStr,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        Box::new(oxasm_commons::InstructionAdapter::new(method_visitor))
    }
}

struct TcbSortWrap;

impl MethodWrap for TcbSortWrap {
    fn wrap<'a>(
        &self,
        _owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        Box::new(TryCatchBlockSorter::new(
            Some(method_visitor),
            access,
            name,
            descriptor,
            signature,
            exceptions,
        ))
    }
}

struct JsrWrap;

impl MethodWrap for JsrWrap {
    fn wrap<'a>(
        &self,
        _owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        Box::new(JsrInlinerAdapter::new(
            Some(method_visitor),
            access,
            name,
            descriptor,
            signature,
            exceptions,
        ))
    }
}

/// AnalyzerAdapterTest's `ClassAnalyzerAdapter`.
struct AnalyzerAdapterWrap;

impl MethodWrap for AnalyzerAdapterWrap {
    fn wrap<'a>(
        &self,
        owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        Box::new(AnalyzedFramesInserter {
            analyzer_adapter: AnalyzerAdapter::new(
                owner,
                access,
                name,
                descriptor,
                Some(method_visitor),
            ),
            has_original_frame: false,
        })
    }
}

/// AnalyzerAdapterTest's `AnalyzedFramesInserter`: inserts an `F_NEW` frame before each
/// instruction that has none, from the types computed with an `AnalyzerAdapter`.
///
/// ASM's inserter is the adapter's downstream visitor. A Rust visitor cannot reach the adapter
/// that owns it, so this one is the adapter's *upstream* visitor: it reads the same frame just
/// before it passes an instruction on, and inserts the frame into the adapter's downstream
/// visitor, past the adapter. (Inserting it *through* the adapter would reset the adapter's frame
/// from it, which changes it when the code is malformed, e.g. when a `double` lost its second
/// slot.) The writer gets the same events, except that the label an adapter creates for a `NEW`
/// comes after the inserted frame instead of before it, at the same bytecode offset.
struct AnalyzedFramesInserter<'a> {
    analyzer_adapter: AnalyzerAdapter<'a>,
    has_original_frame: bool,
}

impl AnalyzedFramesInserter<'_> {
    fn maybe_insert_frame(&mut self) {
        // Don't insert a frame if we already have one for this instruction, from the original
        // class.
        if !self.has_original_frame
            && let (Some(locals), Some(stack)) =
                (&self.analyzer_adapter.locals, &self.analyzer_adapter.stack)
        {
            let local = to_frame_types(locals);
            let stack = to_frame_types(stack);
            // Straight to the adapter's downstream visitor, as ASM's inserter (which is that
            // visitor) does: the adapter does not see the inserted frame.
            if let Some(method_visitor) = self.analyzer_adapter.delegate() {
                method_visitor.visit_frame(FrameKind::New, &local, &stack);
            }
        }
        self.has_original_frame = false;
    }
}

/// Long and double take one element in `visit_frame`, but two in an `AnalyzerAdapter`.
fn to_frame_types(analyzer_types: &[FrameValue<'static>]) -> Vec<FrameValue<'static>> {
    let mut frame_types = Vec::new();
    for (i, value) in analyzer_types.iter().enumerate() {
        if i > 0 && matches!(analyzer_types[i - 1], FrameValue::Long | FrameValue::Double) {
            continue;
        }
        frame_types.push(value.clone());
    }
    frame_types
}

impl MethodVisitor for AnalyzedFramesInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.analyzer_adapter)
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        self.analyzer_adapter.visit_frame(kind, locals, stack);
        self.has_original_frame = true;
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_insn(opcode);
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_int_insn(opcode, operand);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_var_insn(opcode, var_index);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_type_insn(opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_field_insn(opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.maybe_insert_frame();
        if opcode == INVOKESPECIAL
            && name == "<init>"
            && let Some(stack) = &self.analyzer_adapter.stack
            && let Ok(sizes) = Type::try_get_arguments_and_return_sizes_of(descriptor)
            && let Some(receiver) = stack.len().checked_sub((sizes >> 2) as usize)
            && matches!(stack[receiver], FrameValue::Object(_))
        {
            INITIALIZED_INIT_RECEIVER.set(true);
        }
        self.analyzer_adapter
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_jump_insn(opcode, label);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_ldc_insn(value);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_iinc_insn(var_index, increment);
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_table_switch_insn(min, max, dflt, labels);
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_lookup_switch_insn(dflt, keys, labels);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
    }

    fn visit_maxs(&mut self, max_stack: u16, _max_locals: u16) {
        // AnalyzerAdapter should correctly recompute maxLocals from scratch.
        self.analyzer_adapter.visit_maxs(max_stack, 0);
    }
}

/// The `code-size` mode: each method's `CodeSizeEvaluator` sizes, printed at its `visit_maxs`.
#[derive(Default)]
struct CodeSizes {
    /// Semi-UTF-8, as the oracle prints names.
    text: Vec<u8>,
}

impl ClassVisitor for CodeSizes {
    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(CodeSize {
            evaluator: CodeSizeEvaluator::new(None),
            method: [name.as_bytes(), descriptor.as_bytes()].concat(),
            text: &mut self.text,
        }))
    }
}

struct CodeSize<'a> {
    evaluator: CodeSizeEvaluator<'a>,
    method: Vec<u8>,
    text: &'a mut Vec<u8>,
}

impl MethodVisitor for CodeSize<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.evaluator)
    }

    fn visit_maxs(&mut self, _max_stack: u16, _max_locals: u16) {
        self.text.extend_from_slice(&self.method);
        self.text.extend_from_slice(
            format!(
                " {} {}\n",
                self.evaluator.get_min_size(),
                self.evaluator.get_max_size()
            )
            .as_bytes(),
        );
    }
}

struct LvsWrap;

impl MethodWrap for LvsWrap {
    fn wrap<'a>(
        &self,
        _owner: &JavaStr,
        access: u32,
        _name: &JavaStr,
        descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        Box::new(LocalVariablesSorter::new(
            access,
            descriptor,
            method_visitor,
        ))
    }
}

/// The `advice` mode's adapter (the oracle's anonymous `AdviceAdapter`).
struct AdviceWrap;

impl MethodWrap for AdviceWrap {
    fn wrap<'a>(
        &self,
        _owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
        method_visitor: Box<dyn MethodVisitor + 'a>,
    ) -> Box<dyn MethodVisitor + 'a> {
        if access & (ACC_ABSTRACT | ACC_NATIVE) != 0 {
            return method_visitor;
        }
        Box::new(AdviceAdapter::new(
            method_visitor,
            access,
            name,
            descriptor,
            BasicAdvice,
        ))
    }
}

/// AdviceAdapterTest's basic advice, visited through the adapter, and the method's access flags
/// stored in a new local.
struct BasicAdvice;

/// AdviceAdapterTest's `newBasicAdvice`.
fn basic_advice(method_visitor: &mut dyn MethodVisitor, is_after_advice: bool) {
    method_visitor.visit_field_insn(
        GETSTATIC,
        jstr!("java/lang/System"),
        jstr!("err"),
        jstr!("Ljava/io/PrintStream;"),
    );
    let message = if is_after_advice { "exit" } else { "enter" };
    method_visitor.visit_ldc_insn(&Constant::String(Cow::Borrowed(jstr!(message))));
    method_visitor.visit_method_insn(
        INVOKEVIRTUAL,
        jstr!("java/io/PrintStream"),
        jstr!("println"),
        jstr!("(Ljava/lang/String;)V"),
        false,
    );
}

impl<M: MethodVisitor> Advice<M> for BasicAdvice {
    fn on_method_enter(&mut self, adapter: &mut AdviceAdapter<M, Self>) {
        basic_advice(adapter, false);
        let local = adapter.new_local(&Type::INT);
        #[allow(clippy::cast_possible_wrap)] // Java's int access flags
        let access = adapter.method_access() as i32;
        adapter.push_int(access);
        adapter.store_local(local);
    }

    fn on_method_exit(&mut self, adapter: &mut AdviceAdapter<M, Self>, _opcode: u8) {
        basic_advice(adapter, true);
    }
}
