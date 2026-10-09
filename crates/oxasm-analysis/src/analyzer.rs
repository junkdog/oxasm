//! [`Analyzer`]: ASM's `Analyzer`, and [`AnalyzerHooks`], its overridable methods.

use oxasm::opcodes::*;
use oxasm::{Interner, JavaStr, Label, Type};
use oxasm_tree::{Insn, InsnId, InsnKind, InsnList, InsnNode, MethodNode, TryCatchBlockNode};
use rustc_hash::FxHashMap;

use crate::basic_interpreter::checked_object_type;
use crate::error::{AnalyzerError, index_out_of_bounds};
use crate::frame::{Frame, required};
use crate::frame_arena::FrameArena;
use crate::interpreter::{InsnRef, Interpreter};
use crate::subroutine::Subroutine;
use crate::value::Value;

/// The internal name of `java.lang.Throwable`, the type a `finally` handler catches.
const THROWABLE: &JavaStr = oxasm::jstr!("java/lang/Throwable");

/// The overridable methods of ASM's `Analyzer` (`init`, `newControlFlowEdge`, ...) and `Frame`
/// (`initJumpTarget`), with ASM's default behavior. `()` is the hooks of a plain analyzer.
///
/// ASM's `newFrame` methods, which create frames of a `Frame` subclass, have no counterpart: the
/// behavior such a subclass would override is [`init_jump_target`](Self::init_jump_target).
#[allow(unused_variables)]
pub trait AnalyzerHooks<V> {
    /// Called just before the control flow analysis loop, after the first frame is computed
    /// and stored at instruction 0 (ASM's `init`). Does nothing by default.
    ///
    /// `frames` and `handlers` are the analyzer's state, as ASM's `getFrames()` and
    /// `getHandlers(int)` give it to a subclass: the frame of each instruction (which this hook
    /// may set; the analysis then merges into them), and the exception handlers of each
    /// instruction, as indices in the method's `try_catch_blocks`.
    ///
    /// # Errors
    ///
    /// To make the analysis fail. A [runtime](AnalyzerError::runtime) error is reported at the
    /// first instruction, with an `Error at instruction 0: ` prefix; others as they are.
    fn init(
        &mut self,
        owner: &JavaStr,
        method: &MethodNode,
        frames: &mut [Option<Frame<V>>],
        handlers: &[Vec<usize>],
    ) -> Result<(), AnalyzerError> {
        Ok(())
    }

    /// Whether [`init`](Self::init) is known to do nothing, so that the analyzer need not give
    /// it the frames. `false` by default; the `()` hooks are the only ones that say `true`.
    #[doc(hidden)]
    fn init_does_nothing(&self) -> bool {
        false
    }

    /// Initializes a frame for the target, or the successor, of a jump instruction (ASM's
    /// `Frame.initJumpTarget`): called once for each target of the jump instruction (`target`
    /// is its label), and once for its successor (`target` is `None`, except for `GOTO` and
    /// `JSR`), before the frame is merged with the existing frame at this location. Changing the
    /// frame's values gives a branch-sensitive analysis. Does nothing by default.
    ///
    /// `opcode` is `IFEQ` ... `IF_ACMPNE`, `GOTO`, `JSR`, `IFNULL`, `IFNONNULL`, `TABLESWITCH` or
    /// `LOOKUPSWITCH`.
    fn init_jump_target(&mut self, frame: &mut Frame<V>, opcode: u8, target: Option<Label>) {}

    /// Creates a control flow graph edge (ASM's `newControlFlowEdge`). Does nothing by default.
    fn new_control_flow_edge(&mut self, insn_index: usize, successor_index: usize) {}

    /// Creates a control flow graph edge to an exception handler (ASM's
    /// `newControlFlowExceptionEdge(int, int)`). Returns whether the edge must be considered in
    /// the data flow analysis: `true` by default.
    fn new_control_flow_exception_edge(
        &mut self,
        insn_index: usize,
        successor_index: usize,
    ) -> bool {
        true
    }

    /// Creates a control flow graph edge to the handler of `try_catch_block`, whose index is
    /// `successor_index` (ASM's `newControlFlowExceptionEdge(int, TryCatchBlockNode)`). Calls
    /// [`new_control_flow_exception_edge`](Self::new_control_flow_exception_edge) by default.
    fn new_control_flow_exception_edge_of(
        &mut self,
        insn_index: usize,
        try_catch_block: &TryCatchBlockNode,
        successor_index: usize,
    ) -> bool {
        self.new_control_flow_exception_edge(insn_index, successor_index)
    }
}

impl<V> AnalyzerHooks<V> for () {
    fn init_does_nothing(&self) -> bool {
        true
    }
}

impl<V, H: AnalyzerHooks<V> + ?Sized> AnalyzerHooks<V> for &mut H {
    fn init(
        &mut self,
        owner: &JavaStr,
        method: &MethodNode,
        frames: &mut [Option<Frame<V>>],
        handlers: &[Vec<usize>],
    ) -> Result<(), AnalyzerError> {
        (**self).init(owner, method, frames, handlers)
    }
    fn init_does_nothing(&self) -> bool {
        (**self).init_does_nothing()
    }
    fn init_jump_target(&mut self, frame: &mut Frame<V>, opcode: u8, target: Option<Label>) {
        (**self).init_jump_target(frame, opcode, target);
    }
    fn new_control_flow_edge(&mut self, insn_index: usize, successor_index: usize) {
        (**self).new_control_flow_edge(insn_index, successor_index);
    }
    fn new_control_flow_exception_edge(
        &mut self,
        insn_index: usize,
        successor_index: usize,
    ) -> bool {
        (**self).new_control_flow_exception_edge(insn_index, successor_index)
    }
    fn new_control_flow_exception_edge_of(
        &mut self,
        insn_index: usize,
        try_catch_block: &TryCatchBlockNode,
        successor_index: usize,
    ) -> bool {
        (**self).new_control_flow_exception_edge_of(insn_index, try_catch_block, successor_index)
    }
}

/// A semantic bytecode analyzer (ASM's `Analyzer`). *It does not fully check that `JSR` and
/// `RET` instructions are valid.*
///
/// It computes, with an [`Interpreter`], the symbolic state of the frame at each instruction of
/// a method, by iterating until a fixed point. The [hooks](AnalyzerHooks) are ASM's overridable
/// methods.
#[derive(Debug)]
pub struct Analyzer<I: Interpreter, H = ()> {
    interpreter: I,
    hooks: H,
    /// The frames of the last analyzed method, one per instruction (`None` if unreachable).
    frames: Vec<Option<Frame<I::V>>>,
    /// The exception handlers of the last analyzed method, one list per instruction: indices in
    /// its `try_catch_blocks`.
    handlers: Vec<Vec<usize>>,
    /// The frames during an analysis, which end up in `frames`.
    arena: FrameArena<I::V>,
}

impl<I: Interpreter> Analyzer<I> {
    /// Constructs a new analyzer (ASM's `Analyzer(Interpreter)`).
    pub fn new(interpreter: I) -> Analyzer<I> {
        Analyzer::with_hooks(interpreter, ())
    }
}

impl<I: Interpreter, H: AnalyzerHooks<I::V>> Analyzer<I, H> {
    /// Constructs a new analyzer with the given hooks (what overriding ASM's `Analyzer` methods
    /// does).
    pub fn with_hooks(interpreter: I, hooks: H) -> Analyzer<I, H> {
        Analyzer {
            interpreter,
            hooks,
            frames: Vec::new(),
            handlers: Vec::new(),
            arena: FrameArena::default(),
        }
    }

    /// The interpreter.
    pub fn interpreter(&self) -> &I {
        &self.interpreter
    }

    /// The interpreter.
    pub fn interpreter_mut(&mut self) -> &mut I {
        &mut self.interpreter
    }

    /// The hooks.
    pub fn hooks(&self) -> &H {
        &self.hooks
    }

    /// The hooks.
    pub fn hooks_mut(&mut self) -> &mut H {
        &mut self.hooks
    }

    /// The interpreter and the hooks.
    pub fn into_parts(self) -> (I, H) {
        (self.interpreter, self.hooks)
    }

    /// The frames of the last analyzed method (ASM's `getFrames()`): one per instruction (and
    /// label, ...), `None` for an instruction that cannot be reached, or that the analysis did
    /// not reach before it failed.
    pub fn get_frames(&self) -> &[Option<Frame<I::V>>] {
        &self.frames
    }

    /// The frames of the last analyzed method, taken out of this analyzer.
    pub fn take_frames(&mut self) -> Vec<Option<Frame<I::V>>> {
        std::mem::take(&mut self.frames)
    }

    /// The exception handlers of the instruction at `insn_index` of the last analyzed method,
    /// as indices in its `try_catch_blocks` (ASM's `getHandlers`, which returns `null` for
    /// none).
    ///
    /// # Panics
    ///
    /// If `insn_index` is not an instruction index of the last analyzed method.
    pub fn get_handlers(&self, insn_index: usize) -> &[usize] {
        &self.handlers[insn_index]
    }

    /// Analyzes the given method (ASM's `analyze`). `owner` is the internal name of its class,
    /// and its `max_stack` and `max_locals` must be correct.
    ///
    /// Returns the frame at each instruction (see [`get_frames`](Self::get_frames)), or none
    /// for an abstract or native method.
    ///
    /// # Errors
    ///
    /// If a problem occurs during the analysis, with ASM's message.
    pub fn analyze(
        &mut self,
        owner: &JavaStr,
        method: &MethodNode,
    ) -> Result<&[Option<Frame<I::V>>], AnalyzerError> {
        self.analyze_with_maxs(
            owner,
            method,
            usize::from(method.max_locals),
            Some(usize::from(method.max_stack)),
        )?;
        Ok(&self.frames)
    }

    /// Analyzes the given method, and computes and sets its `max_stack` and `max_locals` (ASM's
    /// `analyzeAndComputeMaxs`). See [`analyze`](Self::analyze). Values that do not fit in a
    /// `u16` are stored as `u16::MAX`.
    ///
    /// # Errors
    ///
    /// If a problem occurs during the analysis, with ASM's message.
    pub fn analyze_and_compute_maxs(
        &mut self,
        owner: &JavaStr,
        method: &mut MethodNode,
    ) -> Result<&[Option<Frame<I::V>>], AnalyzerError> {
        let max_locals = compute_max_locals(method)?;
        method.max_locals = u16::try_from(max_locals).unwrap_or(u16::MAX);
        let result = self.analyze_with_maxs(owner, method, max_locals, None);
        // ASM sets maxStack to -1 for the analysis, and computes it only if it succeeds.
        result?;
        let max_stack = compute_max_stack(&self.frames);
        method.max_stack = u16::try_from(max_stack).unwrap_or(u16::MAX);
        Ok(&self.frames)
    }

    fn analyze_with_maxs(
        &mut self,
        owner: &JavaStr,
        method: &MethodNode,
        max_locals: usize,
        max_stack: Option<usize>,
    ) -> Result<(), AnalyzerError> {
        self.frames.clear();
        self.handlers.clear();
        if method.access & (ACC_ABSTRACT | ACC_NATIVE) != 0 {
            return Ok(());
        }
        let insn_list = &method.instructions;
        let insn_list_size = insn_list.len();
        self.frames.resize_with(insn_list_size, || None);
        self.handlers.resize_with(insn_list_size, Vec::new);

        // The instructions, and the index of each label and jump target.
        let mut insns = Vec::with_capacity(insn_list_size);
        let mut label_nodes = Vec::new();
        let mut jumps = Vec::new();
        for (index, (id, node)) in insn_list.iter_with_ids().enumerate() {
            match node.insn {
                Insn::Label(label) => label_nodes.push((label, index)),
                Insn::Jump { label, .. } => jumps.push((index, label)),
                _ => {}
            }
            insns.push(InsnEntry {
                id,
                jump_target: -1,
                node,
            });
        }
        let labels = LabelIndices::new(insn_list, &label_nodes);
        for (index, label) in jumps {
            insns[index].jump_target = labels.index_of(label);
        }

        // For each exception handler, and each instruction within its range, record in
        // 'handlers' the fact that execution can flow from this instruction to the exception
        // handler.
        for (index, try_catch_block) in method.try_catch_blocks.iter().enumerate() {
            let start_index = labels.index_of(try_catch_block.start);
            let end_index = labels.index_of(try_catch_block.end);
            for i in start_index..end_index {
                // Like ASM, which throws an ArrayIndexOutOfBoundsException (out of `analyze`),
                // if 'start' is not in the list but 'end' is.
                let i = usize::try_from(i)
                    .map_err(|_| index_out_of_bounds(i, insn_list_size).unwrapped())?;
                self.handlers[i].push(index);
            }
        }

        self.arena.reset(insn_list_size);
        let mut run = Run {
            insn_list,
            interner: method.interner(),
            insns,
            catch_types: vec![None; method.try_catch_blocks.len()],
            handler_targets: method
                .try_catch_blocks
                .iter()
                .map(|try_catch_block| labels.index_of(try_catch_block.handler))
                .collect(),
            labels,
            method,
            max_locals,
            interpreter: &mut self.interpreter,
            hooks: &mut self.hooks,
            frames: &mut self.frames,
            arena: &mut self.arena,
            handler_frame: None,
            handlers: &self.handlers,
            subroutines: vec![None; insn_list_size],
            in_instructions_to_process: vec![false; insn_list_size],
            instructions_to_process: Vec::with_capacity(insn_list_size),
        };

        let result = run.analyze(owner, max_stack);
        run.arena.finish(run.frames);
        result
    }
}

/// The index of each label of an instruction list, as [`java_index_of`] gives it: from a table
/// indexed by label id, if the ids of its labels are close enough (a class reader creates those
/// of a method one after the other), or else from the list.
struct LabelIndices<'a> {
    insn_list: &'a InsnList,
    /// The id of the label at `indices[0]`.
    first_id: u64,
    /// The index of the label of each id from `first_id` on, [`NO_INDEX`] for a label that is not
    /// in the list; `None` if the list gives them.
    indices: Option<Vec<u32>>,
}

/// An id of [`LabelIndices::indices`] that is no label of the list.
const NO_INDEX: u32 = u32::MAX;

impl<'a> LabelIndices<'a> {
    /// The indices of the label nodes of `insn_list`, given with their label in list order.
    fn new(insn_list: &'a InsnList, label_nodes: &[(Label, usize)]) -> LabelIndices<'a> {
        let ids = label_nodes.iter().map(|(label, _)| label.id());
        let (Some(first_id), Some(last_id)) = (ids.clone().min(), ids.max()) else {
            return LabelIndices {
                insn_list,
                first_id: 0,
                indices: Some(Vec::new()),
            };
        };
        let span = last_id - first_id + 1;
        let dense = span <= 4 * label_nodes.len() as u64 + 64
            && u32::try_from(insn_list.len()).is_ok_and(|len| len < NO_INDEX);
        let indices = dense.then(|| {
            let mut indices = vec![NO_INDEX; span as usize];
            for &(label, index) in label_nodes {
                // A label's node is the first one with this label (see `InsnList::label_position`).
                let slot = &mut indices[(label.id() - first_id) as usize];
                if *slot == NO_INDEX {
                    *slot = index as u32;
                }
            }
            indices
        });
        LabelIndices {
            insn_list,
            first_id,
            indices,
        }
    }

    /// The index of `label` in the list, or -1 if it is not in the list (see [`java_index_of`]).
    #[inline]
    fn index_of(&self, label: Label) -> i64 {
        let Some(indices) = &self.indices else {
            return java_index_of(self.insn_list, label);
        };
        match indices.get(label.id().wrapping_sub(self.first_id) as usize) {
            Some(&index) if index != NO_INDEX => i64::from(index),
            _ => -1,
        }
    }
}

/// An instruction of the method being analyzed, without what all its [`InsnRef`]s share.
#[derive(Clone, Copy)]
struct InsnEntry<'a> {
    id: InsnId,
    /// For a jump instruction, the index of its label, or -1 if the label is not in the list
    /// (ASM's `insnList.indexOf(jumpInsn.label)`).
    jump_target: i64,
    node: &'a InsnNode,
}

/// An analysis in progress: ASM's `Analyzer` fields.
struct Run<'a, I: Interpreter, H> {
    insn_list: &'a InsnList,
    interner: &'a Interner,
    /// The instructions, by index (ASM's `insnList.get(index)`, without a cache lookup).
    insns: Vec<InsnEntry<'a>>,
    /// The type each exception handler catches, once computed.
    catch_types: Vec<Option<Type<'a>>>,
    /// The index of each exception handler, or -1 if its label is not in the list.
    handler_targets: Vec<i64>,
    /// The index of each label.
    labels: LabelIndices<'a>,
    method: &'a MethodNode,
    max_locals: usize,
    interpreter: &'a mut I,
    hooks: &'a mut H,
    /// The analyzer's frames, which the `init` hook gets, and which get the frames of the arena
    /// at the end.
    frames: &'a mut Vec<Option<Frame<I::V>>>,
    /// The frames during the analysis.
    arena: &'a mut FrameArena<I::V>,
    /// A frame for the exception handlers' merges, kept for its storage.
    handler_frame: Option<Frame<I::V>>,
    handlers: &'a [Vec<usize>],
    /// The subroutine of each instruction (few methods have any).
    subroutines: Vec<Option<Box<Subroutine>>>,
    /// Whether each instruction remains to be processed.
    in_instructions_to_process: Vec<bool>,
    /// The instructions that remain to process.
    instructions_to_process: Vec<usize>,
}

impl<'a, I: Interpreter, H: AnalyzerHooks<I::V>> Run<'a, I, H> {
    /// The analysis, after the exception handlers are recorded.
    fn analyze(&mut self, owner: &JavaStr, max_stack: Option<usize>) -> Result<(), AnalyzerError> {
        // Finds the method's subroutines.
        self.find_subroutines()?;

        // Initializes the data structures for the control flow analysis.
        let mut current_frame = match self.initial_frame(owner, max_stack) {
            Ok(frame) => frame,
            Err(e) if e.is_runtime() => return Err(e.at(0, self.insn_list.first())),
            Err(e) => return Err(e),
        };
        self.run(&mut current_frame)
    }

    /// The index of a label, which must be in the list: where ASM's `indexOf` returns -1 for a
    /// label that is not, the analysis fails as ASM's does when it uses this index.
    fn label_index(&self, label: Label) -> Result<usize, AnalyzerError> {
        self.checked_index(self.labels.index_of(label))
    }

    /// A label's index from [`java_index_of`], which must not be -1 (see
    /// [`label_index`](Self::label_index)).
    fn checked_index(&self, index: i64) -> Result<usize, AnalyzerError> {
        usize::try_from(index).map_err(|_| index_out_of_bounds(-1, self.insn_list.len()))
    }

    /// The instruction at `index`.
    fn insn(&self, index: usize) -> InsnRef<'a> {
        let entry = self.insns[index];
        InsnRef {
            interner: self.interner,
            list: self.insn_list,
            id: entry.id,
            index,
            node: entry.node,
        }
    }

    /// The type that the exception handler of the given try catch block catches, and the index
    /// of its handler (what ASM computes for each instruction in the block).
    fn handler_of(
        &mut self,
        try_catch_block_index: usize,
    ) -> Result<(Type<'a>, usize), AnalyzerError> {
        let catch_type = match &self.catch_types[try_catch_block_index] {
            Some(catch_type) => catch_type.clone(),
            None => {
                let method = self.method;
                let catch_type = checked_object_type(
                    method.try_catch_blocks[try_catch_block_index]
                        .type_
                        .map_or(THROWABLE, |t| method.resolve(t)),
                )?;
                self.catch_types[try_catch_block_index] = Some(catch_type.clone());
                catch_type
            }
        };
        let handler_index = self.checked_index(self.handler_targets[try_catch_block_index])?;
        Ok((catch_type, handler_index))
    }

    /// Computes the first frame, merges it at instruction 0, and calls the `init` hook: what
    /// ASM does in a `try` that wraps runtime exceptions at instruction 0. Returns the first
    /// frame, which becomes the current frame of the analysis.
    fn initial_frame(
        &mut self,
        owner: &JavaStr,
        max_stack: Option<usize>,
    ) -> Result<Frame<I::V>, AnalyzerError> {
        let frame = self.compute_initial_frame(owner, max_stack)?;
        self.merge(0, &frame, None)?;
        if self.hooks.init_does_nothing() {
            return Ok(frame);
        }
        // The hook gets the frames as ASM's `getFrames()`, and may set some.
        self.arena.export(self.frames);
        let result = self
            .hooks
            .init(owner, self.method, self.frames, self.handlers);
        self.arena.import(self.frames);
        result?;
        Ok(frame)
    }

    /// The initial frame of the method (ASM's `computeInitialFrame`).
    fn compute_initial_frame(
        &mut self,
        owner: &JavaStr,
        max_stack: Option<usize>,
    ) -> Result<Frame<I::V>, AnalyzerError> {
        let max_locals = self.max_locals;
        let mut locals = Vec::with_capacity(max_locals);
        let set_local = |locals: &mut Vec<I::V>, value: Option<I::V>, operation: &str| {
            let value = required(value, operation)?;
            if locals.len() >= max_locals {
                return Err(AnalyzerError::runtime(format!(
                    "Trying to set an inexistant local variable {}",
                    locals.len()
                )));
            }
            locals.push(value);
            Ok(())
        };
        let is_instance_method = self.method.access & ACC_STATIC == 0;
        if is_instance_method {
            let owner_type = checked_object_type(owner)?;
            let value = self
                .interpreter
                .new_parameter_value(is_instance_method, 0, &owner_type)?;
            set_local(&mut locals, value, "newParameterValue")?;
        }
        let argument_types = Type::try_get_argument_types_of(self.method.resolve(self.method.desc))
            .map_err(|e| AnalyzerError::runtime(e.message()))?;
        for argument_type in &argument_types {
            let local = locals.len();
            let value =
                self.interpreter
                    .new_parameter_value(is_instance_method, local, argument_type)?;
            set_local(&mut locals, value, "newParameterValue")?;
            if argument_type.get_size() == 2 {
                let value = self.interpreter.new_empty_value(local + 1)?;
                set_local(&mut locals, value, "newEmptyValue")?;
            }
        }
        while locals.len() < max_locals {
            let value = self.interpreter.new_empty_value(locals.len())?;
            set_local(&mut locals, value, "newEmptyValue")?;
        }
        let return_type = Type::try_get_return_type_of(self.method.resolve(self.method.desc))
            .map_err(|e| AnalyzerError::runtime(e.message()))?;
        let mut frame = Frame::new(locals, max_stack);
        frame.set_return(self.interpreter.new_return_type_value(&return_type)?);
        Ok(frame)
    }

    /// The control flow analysis loop.
    fn run(&mut self, current_frame: &mut Frame<I::V>) -> Result<(), AnalyzerError> {
        while let Some(insn_index) = self.instructions_to_process.pop() {
            self.in_instructions_to_process[insn_index] = false;
            let insn = self.insn(insn_index);
            self.process(insn, current_frame)
                .map_err(|e| e.at(insn_index, Some(insn.id)))?;
        }
        Ok(())
    }

    /// Simulates the execution of an instruction, and merges the resulting frames into its
    /// successors.
    fn process(
        &mut self,
        insn: InsnRef<'a>,
        current_frame: &mut Frame<I::V>,
    ) -> Result<(), AnalyzerError> {
        let (method, handlers) = (self.method, self.handlers);
        let insn_index = insn.index;
        let mut subroutine = self.subroutines[insn_index].clone();
        let insn_opcode = insn.opcode();

        match insn.kind() {
            InsnKind::Label | InsnKind::Line | InsnKind::Frame => {
                // Update the current frame, so it can be used during processing for this
                // instruction. Only the exception handlers below use it: otherwise the next
                // instruction processed updates it first, and only what accumulates in it, its
                // capacity, is updated.
                if handlers[insn_index].is_empty() {
                    self.arena.init_capacity(insn_index, current_frame);
                } else {
                    self.arena.init(insn_index, current_frame);
                }
                self.merge_successor(insn_index, subroutine.as_deref())?;
                self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
            }
            _ => {
                self.arena
                    .init(insn_index, current_frame)
                    .execute(insn, &mut *self.interpreter)?;
                let opcode = insn_opcode.expect("a JVM instruction");
                match &insn.insn {
                    Insn::Jump { label, .. } => {
                        if opcode != GOTO && opcode != JSR {
                            self.hooks.init_jump_target(current_frame, opcode, None);
                            self.merge(insn_index + 1, current_frame, subroutine.as_deref())?;
                            self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
                        }
                        let jump_insn_index =
                            self.checked_index(self.insns[insn_index].jump_target)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(*label));
                        if opcode == JSR {
                            let subroutine =
                                Subroutine::new(Some(*label), self.max_locals, Some(insn.id));
                            self.merge(jump_insn_index, current_frame, Some(&subroutine))?;
                        } else {
                            self.merge(jump_insn_index, current_frame, subroutine.as_deref())?;
                        }
                        self.hooks
                            .new_control_flow_edge(insn_index, jump_insn_index);
                    }
                    Insn::LookupSwitch(insn) => {
                        let target_insn_index = self.label_index(insn.dflt)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(insn.dflt));
                        self.merge(target_insn_index, current_frame, subroutine.as_deref())?;
                        self.hooks
                            .new_control_flow_edge(insn_index, target_insn_index);
                        for label in &insn.labels {
                            let target_insn_index = self.label_index(*label)?;
                            self.hooks
                                .init_jump_target(current_frame, opcode, Some(*label));
                            self.merge(target_insn_index, current_frame, subroutine.as_deref())?;
                            self.hooks
                                .new_control_flow_edge(insn_index, target_insn_index);
                        }
                    }
                    Insn::TableSwitch(insn) => {
                        let target_insn_index = self.label_index(insn.dflt)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(insn.dflt));
                        self.merge(target_insn_index, current_frame, subroutine.as_deref())?;
                        self.hooks
                            .new_control_flow_edge(insn_index, target_insn_index);
                        for label in &insn.labels {
                            self.hooks
                                .init_jump_target(current_frame, opcode, Some(*label));
                            let target_insn_index = self.label_index(*label)?;
                            self.merge(target_insn_index, current_frame, subroutine.as_deref())?;
                            self.hooks
                                .new_control_flow_edge(insn_index, target_insn_index);
                        }
                    }
                    _ if opcode == RET => {
                        let Some(subroutine) = &subroutine else {
                            return Err(AnalyzerError::new(
                                Some(insn.id),
                                "RET instruction outside of a subroutine",
                            ));
                        };
                        for caller in &subroutine.callers {
                            let jsr_insn_index = self
                                .insn_list
                                .index_of(*caller)
                                .ok_or_else(|| index_out_of_bounds(-1, self.insn_list.len()))?;
                            if self.arena.is_set(jsr_insn_index) {
                                self.merge_after_ret(
                                    jsr_insn_index + 1,
                                    current_frame,
                                    jsr_insn_index,
                                    &subroutine.locals_used,
                                )?;
                                self.hooks
                                    .new_control_flow_edge(insn_index, jsr_insn_index + 1);
                            }
                        }
                    }
                    _ if opcode != ATHROW && !(IRETURN..=RETURN).contains(&opcode) => {
                        if let Some(subroutine) = &mut subroutine {
                            let var_index = match insn.insn {
                                Insn::Var { var, .. } => Some(usize::from(var)),
                                Insn::Iinc { var, .. } => Some(usize::from(var)),
                                _ => None,
                            };
                            if let Some(var_index) = var_index {
                                mark_used(&mut subroutine.locals_used, var_index)?;
                                if matches!(opcode, LLOAD | DLOAD | LSTORE | DSTORE) {
                                    mark_used(&mut subroutine.locals_used, var_index + 1)?;
                                }
                            }
                        }
                        self.merge(insn_index + 1, current_frame, subroutine.as_deref())?;
                        self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
                    }
                    _ => {}
                }
            }
        }

        for &try_catch_block_index in &handlers[insn_index] {
            let try_catch_block = &method.try_catch_blocks[try_catch_block_index];
            let (catch_type, handler_index) = self.handler_of(try_catch_block_index)?;
            if self.hooks.new_control_flow_exception_edge_of(
                insn_index,
                try_catch_block,
                handler_index,
            ) {
                // Merge the frame *before* this instruction, with its stack cleared and an
                // exception pushed, with the handler's frame.
                let mut handler = self
                    .handler_frame
                    .take()
                    .unwrap_or_else(|| Frame::new(Vec::new(), None));
                self.arena.clone_into(insn_index, &mut handler);
                let result = self.merge_handler(
                    &mut handler,
                    current_frame,
                    try_catch_block,
                    &catch_type,
                    handler_index,
                    subroutine.as_deref(),
                );
                self.handler_frame = Some(handler);
                result?;
            }
        }
        Ok(())
    }

    /// Merges into the frame of an exception handler `handler`, which holds a clone of the frame
    /// before the instruction, then `current_frame`, the frame after it: each with its stack
    /// cleared and an exception pushed.
    fn merge_handler(
        &mut self,
        handler: &mut Frame<I::V>,
        current_frame: &Frame<I::V>,
        try_catch_block: &TryCatchBlockNode,
        catch_type: &Type,
        handler_index: usize,
        subroutine: Option<&Subroutine>,
    ) -> Result<(), AnalyzerError> {
        handler.clear_stack();
        let exception_value =
            self.interpreter
                .new_exception_value(try_catch_block, handler, catch_type)?;
        let exception_value = required(exception_value, "newExceptionValue")?;
        handler.try_push(exception_value.clone())?;
        self.merge(handler_index, handler, subroutine)?;
        handler.assign(
            current_frame.return_value(),
            current_frame.values(),
            current_frame.shape(),
        );
        handler.clear_stack();
        handler.try_push(exception_value)?;
        self.merge(handler_index, handler, subroutine)
    }

    /// Merges the frame of a label, line number or frame node into the next instruction.
    fn merge_successor(
        &mut self,
        insn_index: usize,
        subroutine: Option<&Subroutine>,
    ) -> Result<(), AnalyzerError> {
        let successor = insn_index + 1;
        if successor >= self.arena.len() {
            return Err(index_out_of_bounds(successor, self.arena.len()));
        }
        let changed = self
            .arena
            .merge_into_next(insn_index, &mut *self.interpreter)?;
        self.merge_subroutine_and_enqueue(successor, subroutine, changed);
        Ok(())
    }

    /// Merges the given frame and subroutine into those at the given instruction, and adds the
    /// instruction to the instructions to process if this changes them (ASM's
    /// `merge(int, Frame, Subroutine)`).
    fn merge(
        &mut self,
        insn_index: usize,
        frame: &Frame<I::V>,
        subroutine: Option<&Subroutine>,
    ) -> Result<(), AnalyzerError> {
        let length = self.arena.len();
        if insn_index >= length {
            return Err(index_out_of_bounds(insn_index, length));
        }
        let changed = self
            .arena
            .merge(insn_index, frame, &mut *self.interpreter)?;
        self.merge_subroutine_and_enqueue(insn_index, subroutine, changed);
        Ok(())
    }

    fn merge_subroutine_and_enqueue(
        &mut self,
        insn_index: usize,
        subroutine: Option<&Subroutine>,
        mut changed: bool,
    ) {
        match (&mut self.subroutines[insn_index], subroutine) {
            (slot @ None, Some(subroutine)) => {
                *slot = Some(Box::new(subroutine.clone()));
                changed = true;
            }
            (Some(old_subroutine), Some(subroutine)) => {
                changed |= old_subroutine.merge(subroutine);
            }
            (_, None) => {}
        }
        self.enqueue(insn_index, changed);
    }

    fn enqueue(&mut self, insn_index: usize, changed: bool) {
        if changed && !self.in_instructions_to_process[insn_index] {
            self.in_instructions_to_process[insn_index] = true;
            self.instructions_to_process.push(insn_index);
        }
    }

    /// Merges the frame after a `RET` into the instruction after a `JSR` (ASM's `merge(int,
    /// Frame, Frame, Subroutine, boolean[])`): `frame_after_ret` is first merged with the frame
    /// before the `JSR` at `jsr_insn_index` (whose subroutine is that of the instruction before
    /// the `JSR`) for the locals the subroutine does not use.
    fn merge_after_ret(
        &mut self,
        insn_index: usize,
        frame_after_ret: &mut Frame<I::V>,
        jsr_insn_index: usize,
        locals_used: &[bool],
    ) -> Result<(), AnalyzerError> {
        frame_after_ret.merge_subroutine_values(self.arena.values(jsr_insn_index), locals_used);

        let length = self.arena.len();
        if insn_index >= length {
            return Err(index_out_of_bounds(insn_index, length));
        }
        let mut changed = self
            .arena
            .merge(insn_index, frame_after_ret, &mut *self.interpreter)?;
        if let Some(subroutine_before_jsr) = self.subroutines[jsr_insn_index].clone()
            && let Some(old_subroutine) = &mut self.subroutines[insn_index]
        {
            changed |= old_subroutine.merge(&subroutine_before_jsr);
        }
        self.enqueue(insn_index, changed);
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Subroutines
    // ---------------------------------------------------------------------------------------------

    /// Finds the subroutines of the method, and stores them in `subroutines` (ASM's
    /// `findSubroutines`).
    fn find_subroutines(&mut self) -> Result<(), AnalyzerError> {
        // For each instruction, compute the subroutine to which it belongs. Follow the main
        // 'subroutine', and collect the jsr instructions to nested subroutines. ASM stores a copy
        // of the main 'subroutine' at each instruction it reaches, and clears them at the end;
        // here the instructions are only marked as reached.
        let mut reached_by_main = vec![false; self.insns.len()];
        let mut jsr_insns = std::collections::VecDeque::new();
        let mut work = Vec::new();
        self.find_subroutine(0, None, &mut reached_by_main, &mut jsr_insns, &mut work)?;
        // Follow the nested subroutines, and collect their own nested subroutines, until all
        // subroutines are found.
        let mut jsr_subroutines: FxHashMap<Label, Subroutine> = FxHashMap::default();
        while let Some((jsr_insn, label)) = jsr_insns.pop_front() {
            match jsr_subroutines.get_mut(&label) {
                Some(subroutine) => subroutine.callers.push(jsr_insn),
                None => {
                    let subroutine = Subroutine::new(Some(label), self.max_locals, Some(jsr_insn));
                    let start = self.labels.index_of(label);
                    self.find_subroutine(
                        start,
                        Some(&subroutine),
                        &mut reached_by_main,
                        &mut jsr_insns,
                        &mut work,
                    )?;
                    jsr_subroutines.insert(label, subroutine);
                }
            }
        }
        Ok(())
    }

    /// Follows the control flow graph of the method from the instruction at `insn_index`, and
    /// stores a copy of `subroutine` for each instruction it reaches, or marks it in
    /// `reached_by_main` for the main 'subroutine', `None` (ASM's `findSubroutine`).
    /// Jumps to nested subroutines are *not* followed: their instructions are put in
    /// `jsr_insns`, with their target.
    ///
    /// ASM recurses for the default target of each switch, with a new list of the instructions
    /// to process. Here `work` holds the lists of the "calls" one above the other: since the
    /// instructions are taken from its top, those of a call are all processed before those of its
    /// caller, in the same order.
    fn find_subroutine(
        &mut self,
        insn_index: i64,
        subroutine: Option<&Subroutine>,
        reached_by_main: &mut [bool],
        jsr_insns: &mut std::collections::VecDeque<(InsnId, Label)>,
        work: &mut Vec<i64>,
    ) -> Result<(), AnalyzerError> {
        let (insn_list, handlers) = (self.insn_list, self.handlers);
        let insn_list_size = insn_list.len();
        let labels = &self.labels;
        let index_of = |label: Label| labels.index_of(label);
        let instruction_indices_to_process = work;
        instruction_indices_to_process.clear();
        instruction_indices_to_process.push(insn_index);
        while let Some(current_insn_index) = instruction_indices_to_process.pop() {
            let current_insn_index = match usize::try_from(current_insn_index) {
                Ok(index) if index < insn_list_size => index,
                _ => {
                    return Err(AnalyzerError::new(
                        None,
                        "Execution can fall off the end of the code",
                    ));
                }
            };
            if self.subroutines[current_insn_index].is_some() || reached_by_main[current_insn_index]
            {
                continue;
            }
            match subroutine {
                Some(subroutine) => {
                    self.subroutines[current_insn_index] = Some(Box::new(subroutine.clone()));
                }
                None => reached_by_main[current_insn_index] = true,
            }
            let current_insn = self.insns[current_insn_index];

            // Push the normal successors of currentInsn onto instructionIndicesToProcess.
            let mut default_target = None;
            match current_insn.node.insn {
                Insn::Jump { opcode, label } => {
                    if opcode == JSR {
                        // Do not follow a jsr, it leads to another subroutine!
                        jsr_insns.push_back((current_insn.id, label));
                    } else {
                        instruction_indices_to_process.push(current_insn.jump_target);
                    }
                }
                Insn::TableSwitch(_) | Insn::LookupSwitch(_) => {
                    let (dflt, labels) = current_insn.node.insn.switch_labels().expect("a switch");
                    // ASM recurses here, and pushes the labels after the recursive call returns;
                    // pushing them first is the same, since the call has its own work list.
                    default_target = Some(index_of(dflt));
                    for label in labels.iter().rev() {
                        instruction_indices_to_process.push(index_of(*label));
                    }
                }
                _ => {}
            }

            // Push the exception handler successors of currentInsn onto
            // instructionIndicesToProcess.
            for &try_catch_block_index in &handlers[current_insn_index] {
                instruction_indices_to_process.push(self.handler_targets[try_catch_block_index]);
            }

            // Push the next instruction, if the control flow can go from currentInsn to the
            // next.
            match current_insn.node.opcode() {
                Some(
                    GOTO | RET | TABLESWITCH | LOOKUPSWITCH | IRETURN | LRETURN | FRETURN | DRETURN
                    | ARETURN | RETURN | ATHROW,
                ) => {}
                _ => instruction_indices_to_process.push(current_insn_index as i64 + 1),
            }

            // The recursive call, whose list is the top of the stack.
            if let Some(default_target) = default_target {
                instruction_indices_to_process.push(default_target);
            }
        }
        Ok(())
    }
}

/// The index of a label in an instruction list, or -1 if it is not in the list (ASM's
/// `indexOf`, for a label node that was never added to a list).
fn java_index_of(insn_list: &InsnList, label: Label) -> i64 {
    insn_list
        .label_index(label)
        .map_or(-1, |index| index as i64)
}

/// Marks a local variable as used by a subroutine (ASM's `localsUsed[varIndex] = true`).
fn mark_used(locals_used: &mut [bool], var_index: usize) -> Result<(), AnalyzerError> {
    let length = locals_used.len();
    *locals_used
        .get_mut(var_index)
        .ok_or_else(|| index_out_of_bounds(var_index, length))? = true;
    Ok(())
}

/// The maximum number of local variables used in the given method (ASM's `computeMaxLocals`).
fn compute_max_locals(method: &MethodNode) -> Result<usize, AnalyzerError> {
    let sizes = Type::try_get_arguments_and_return_sizes_of(method.resolve(method.desc))
        .map_err(|e| AnalyzerError::new(None, e.message()))?;
    let mut max_locals = (sizes >> 2) as usize;
    if method.access & ACC_STATIC != 0 {
        max_locals = max_locals.saturating_sub(1);
    }
    for insn_node in &method.instructions {
        match insn_node.insn {
            Insn::Var { opcode, var } => {
                let size = if matches!(opcode, LLOAD | DLOAD | LSTORE | DSTORE) {
                    2
                } else {
                    1
                };
                max_locals = max_locals.max(usize::from(var) + size);
            }
            Insn::Iinc { var, .. } => max_locals = max_locals.max(usize::from(var) + 1),
            _ => {}
        }
    }
    Ok(max_locals)
}

/// The maximum stack size of a method, given its frames (ASM's `computeMaxStack`).
fn compute_max_stack<V: Value>(frames: &[Option<Frame<V>>]) -> usize {
    frames
        .iter()
        .flatten()
        .map(|frame| frame.stack().iter().map(Value::get_size).sum())
        .max()
        .unwrap_or(0)
}
