//! [`Analyzer`]: ASM's `Analyzer`, and [`AnalyzerHooks`], its overridable methods.

use oxasm::opcodes::*;
use oxasm::{JavaStr, Label, Type};
use oxasm_tree::{Insn, InsnId, InsnKind, InsnList, MethodNode, TryCatchBlockNode};
use rustc_hash::FxHashMap;

use crate::basic_interpreter::checked_object_type;
use crate::error::{AnalyzerError, index_out_of_bounds};
use crate::frame::{Frame, required};
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

impl<V> AnalyzerHooks<V> for () {}

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

        // For each exception handler, and each instruction within its range, record in
        // 'handlers' the fact that execution can flow from this instruction to the exception
        // handler.
        for (index, try_catch_block) in method.try_catch_blocks.iter().enumerate() {
            let start_index = java_index_of(insn_list, try_catch_block.start);
            let end_index = java_index_of(insn_list, try_catch_block.end);
            for i in start_index..end_index {
                // Like ASM, which throws an ArrayIndexOutOfBoundsException (out of `analyze`),
                // if 'start' is not in the list but 'end' is.
                let i = usize::try_from(i)
                    .map_err(|_| index_out_of_bounds(i, insn_list_size).unwrapped())?;
                self.handlers[i].push(index);
            }
        }

        let mut run = Run {
            insn_list,
            insns: insn_list
                .iter_with_ids()
                .enumerate()
                .map(|(index, (id, node))| InsnRef {
                    interner: method.interner(),
                    list: insn_list,
                    id,
                    index,
                    node,
                })
                .collect(),
            method,
            max_locals,
            interpreter: &mut self.interpreter,
            hooks: &mut self.hooks,
            frames: &mut self.frames,
            handlers: &self.handlers,
            subroutines: vec![None; insn_list_size],
            in_instructions_to_process: vec![false; insn_list_size],
            instructions_to_process: Vec::with_capacity(insn_list_size),
        };

        // Finds the method's subroutines.
        run.find_subroutines()?;

        // Initializes the data structures for the control flow analysis.
        let mut current_frame = match run.initial_frame(owner, max_stack) {
            Ok(frame) => frame,
            Err(e) if e.is_runtime() => return Err(e.at(0, insn_list.first())),
            Err(e) => return Err(e),
        };
        run.run(&mut current_frame)
    }
}

/// An analysis in progress: ASM's `Analyzer` fields.
struct Run<'a, I: Interpreter, H> {
    insn_list: &'a InsnList,
    /// The instructions, by index (ASM's `insnList.get(index)`, without a cache lookup).
    insns: Vec<InsnRef<'a>>,
    method: &'a MethodNode,
    max_locals: usize,
    interpreter: &'a mut I,
    hooks: &'a mut H,
    frames: &'a mut Vec<Option<Frame<I::V>>>,
    handlers: &'a [Vec<usize>],
    /// The subroutine of each instruction.
    subroutines: Vec<Option<Subroutine>>,
    /// Whether each instruction remains to be processed.
    in_instructions_to_process: Vec<bool>,
    /// The instructions that remain to process.
    instructions_to_process: Vec<usize>,
}

impl<'a, I: Interpreter, H: AnalyzerHooks<I::V>> Run<'a, I, H> {
    /// The index of a label, which must be in the list: where ASM's `indexOf` returns -1 for a
    /// label that is not, the analysis fails as ASM's does when it uses this index.
    fn label_index(&self, label: Label) -> Result<usize, AnalyzerError> {
        self.insn_list
            .label_index(label)
            .ok_or_else(|| index_out_of_bounds(-1, self.insn_list.len()))
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
        self.hooks
            .init(owner, self.method, self.frames, self.handlers)?;
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
            let insn = self.insns[insn_index];
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
                // instruction.
                current_frame.init(self.frame(insn_index));
                self.merge_successor(insn_index, subroutine.as_ref())?;
                self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
            }
            _ => {
                current_frame
                    .init(self.frame(insn_index))
                    .execute(insn, &mut *self.interpreter)?;
                let opcode = insn_opcode.expect("a JVM instruction");
                match &insn.insn {
                    Insn::Jump { label, .. } => {
                        if opcode != GOTO && opcode != JSR {
                            self.hooks.init_jump_target(current_frame, opcode, None);
                            self.merge(insn_index + 1, current_frame, subroutine.as_ref())?;
                            self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
                        }
                        let jump_insn_index = self.label_index(*label)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(*label));
                        if opcode == JSR {
                            let subroutine =
                                Subroutine::new(Some(*label), self.max_locals, Some(insn.id));
                            self.merge(jump_insn_index, current_frame, Some(&subroutine))?;
                        } else {
                            self.merge(jump_insn_index, current_frame, subroutine.as_ref())?;
                        }
                        self.hooks
                            .new_control_flow_edge(insn_index, jump_insn_index);
                    }
                    Insn::LookupSwitch { dflt, labels, .. } => {
                        let target_insn_index = self.label_index(*dflt)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(*dflt));
                        self.merge(target_insn_index, current_frame, subroutine.as_ref())?;
                        self.hooks
                            .new_control_flow_edge(insn_index, target_insn_index);
                        for label in labels {
                            let target_insn_index = self.label_index(*label)?;
                            self.hooks
                                .init_jump_target(current_frame, opcode, Some(*label));
                            self.merge(target_insn_index, current_frame, subroutine.as_ref())?;
                            self.hooks
                                .new_control_flow_edge(insn_index, target_insn_index);
                        }
                    }
                    Insn::TableSwitch { dflt, labels, .. } => {
                        let target_insn_index = self.label_index(*dflt)?;
                        self.hooks
                            .init_jump_target(current_frame, opcode, Some(*dflt));
                        self.merge(target_insn_index, current_frame, subroutine.as_ref())?;
                        self.hooks
                            .new_control_flow_edge(insn_index, target_insn_index);
                        for label in labels {
                            self.hooks
                                .init_jump_target(current_frame, opcode, Some(*label));
                            let target_insn_index = self.label_index(*label)?;
                            self.merge(target_insn_index, current_frame, subroutine.as_ref())?;
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
                            if let Some(frame_before_jsr) = &self.frames[jsr_insn_index] {
                                self.merge_after_ret(
                                    jsr_insn_index + 1,
                                    frame_before_jsr.clone(),
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
                        self.merge(insn_index + 1, current_frame, subroutine.as_ref())?;
                        self.hooks.new_control_flow_edge(insn_index, insn_index + 1);
                    }
                    _ => {}
                }
            }
        }

        for &try_catch_block_index in &handlers[insn_index] {
            let try_catch_block = &method.try_catch_blocks[try_catch_block_index];
            let catch_type = checked_object_type(
                try_catch_block
                    .type_
                    .map_or(THROWABLE, |t| method.resolve(t)),
            )?;
            let handler_index = self.label_index(try_catch_block.handler)?;
            if self.hooks.new_control_flow_exception_edge_of(
                insn_index,
                try_catch_block,
                handler_index,
            ) {
                // Merge the frame *before* this instruction, with its stack cleared and an
                // exception pushed, with the handler's frame.
                let mut handler = self.frame(insn_index).clone();
                handler.clear_stack();
                let exception_value =
                    self.interpreter
                        .new_exception_value(try_catch_block, &handler, &catch_type)?;
                let exception_value = required(exception_value, "newExceptionValue")?;
                handler.try_push(exception_value.clone())?;
                self.merge(handler_index, &handler, subroutine.as_ref())?;
                // Merge the frame *after* this instruction, with its stack cleared and an
                // exception pushed, with the handler's frame.
                let mut handler = current_frame.clone();
                handler.clear_stack();
                handler.try_push(exception_value)?;
                self.merge(handler_index, &handler, subroutine.as_ref())?;
            }
        }
        Ok(())
    }

    /// The frame at an instruction being processed (which has one).
    fn frame(&self, insn_index: usize) -> &Frame<I::V> {
        self.frames[insn_index]
            .as_ref()
            .expect("an instruction to process has a frame")
    }

    /// Merges the frame of a label, line number or frame node into the next instruction.
    fn merge_successor(
        &mut self,
        insn_index: usize,
        subroutine: Option<&Subroutine>,
    ) -> Result<(), AnalyzerError> {
        let successor = insn_index + 1;
        if successor >= self.frames.len() {
            return Err(index_out_of_bounds(successor, self.frames.len()));
        }
        let (before, after) = self.frames.split_at_mut(successor);
        let frame = before[insn_index]
            .as_ref()
            .expect("an instruction to process has a frame");
        let changed = match &mut after[0] {
            slot @ None => {
                *slot = Some(frame.clone());
                true
            }
            Some(old_frame) => old_frame.merge(frame, &mut *self.interpreter)?,
        };
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
        let length = self.frames.len();
        let slot = self
            .frames
            .get_mut(insn_index)
            .ok_or_else(|| index_out_of_bounds(insn_index, length))?;
        let changed = match slot {
            None => {
                *slot = Some(frame.clone());
                true
            }
            Some(old_frame) => old_frame.merge(frame, &mut *self.interpreter)?,
        };
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
                *slot = Some(subroutine.clone());
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
    /// Frame, Frame, Subroutine, boolean[])`): `frame_after_ret` is first merged with
    /// `frame_before_jsr` (the frame of the `JSR` at `jsr_insn_index`, whose subroutine is that
    /// of the instruction before the `JSR`) for the locals the subroutine does not use.
    fn merge_after_ret(
        &mut self,
        insn_index: usize,
        frame_before_jsr: Frame<I::V>,
        frame_after_ret: &mut Frame<I::V>,
        jsr_insn_index: usize,
        locals_used: &[bool],
    ) -> Result<(), AnalyzerError> {
        frame_after_ret.merge_subroutine(&frame_before_jsr, locals_used);

        let length = self.frames.len();
        let slot = self
            .frames
            .get_mut(insn_index)
            .ok_or_else(|| index_out_of_bounds(insn_index, length))?;
        let mut changed = match slot {
            None => {
                *slot = Some(frame_after_ret.clone());
                true
            }
            Some(old_frame) => old_frame.merge(frame_after_ret, &mut *self.interpreter)?,
        };
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
        // 'subroutine', and collect the jsr instructions to nested subroutines.
        let main = Subroutine::new(None, self.max_locals, None);
        let mut jsr_insns = std::collections::VecDeque::new();
        self.find_subroutine(0, &main, &mut jsr_insns)?;
        // Follow the nested subroutines, and collect their own nested subroutines, until all
        // subroutines are found.
        let mut jsr_subroutines: FxHashMap<Label, Subroutine> = FxHashMap::default();
        while let Some((jsr_insn, label)) = jsr_insns.pop_front() {
            match jsr_subroutines.get_mut(&label) {
                Some(subroutine) => subroutine.callers.push(jsr_insn),
                None => {
                    let subroutine = Subroutine::new(Some(label), self.max_locals, Some(jsr_insn));
                    let start = java_index_of(self.insn_list, label);
                    self.find_subroutine(start, &subroutine, &mut jsr_insns)?;
                    jsr_subroutines.insert(label, subroutine);
                }
            }
        }
        // Clear the main 'subroutine', which is not a real subroutine (and was used only as an
        // intermediate step above to find the real ones).
        for subroutine in &mut self.subroutines {
            if subroutine.as_ref().is_some_and(|s| s.start.is_none()) {
                *subroutine = None;
            }
        }
        Ok(())
    }

    /// Follows the control flow graph of the method from the instruction at `insn_index`, and
    /// stores a copy of `subroutine` for each instruction it reaches (ASM's `findSubroutine`).
    /// Jumps to nested subroutines are *not* followed: their instructions are put in
    /// `jsr_insns`, with their target.
    ///
    /// ASM recurses for the default target of each switch; this keeps a stack of the work lists
    /// of the "calls" instead, which processes the instructions in the same order.
    fn find_subroutine(
        &mut self,
        insn_index: i64,
        subroutine: &Subroutine,
        jsr_insns: &mut std::collections::VecDeque<(InsnId, Label)>,
    ) -> Result<(), AnalyzerError> {
        let (insn_list, method, handlers) = (self.insn_list, self.method, self.handlers);
        let insn_list_size = insn_list.len();
        let index_of = |label: Label| java_index_of(insn_list, label);
        let mut calls: Vec<Vec<i64>> = vec![vec![insn_index]];
        while let Some(instruction_indices_to_process) = calls.last_mut() {
            let Some(current_insn_index) = instruction_indices_to_process.pop() else {
                calls.pop();
                continue;
            };
            let current_insn_index = match usize::try_from(current_insn_index) {
                Ok(index) if index < insn_list_size => index,
                _ => {
                    return Err(AnalyzerError::new(
                        None,
                        "Execution can fall off the end of the code",
                    ));
                }
            };
            if self.subroutines[current_insn_index].is_some() {
                continue;
            }
            self.subroutines[current_insn_index] = Some(subroutine.clone());
            let current_insn = self.insns[current_insn_index];

            // Push the normal successors of currentInsn onto instructionIndicesToProcess.
            let mut default_target = None;
            match &current_insn.insn {
                Insn::Jump { opcode, label } => {
                    if *opcode == JSR {
                        // Do not follow a jsr, it leads to another subroutine!
                        jsr_insns.push_back((current_insn.id, *label));
                    } else {
                        instruction_indices_to_process.push(index_of(*label));
                    }
                }
                Insn::TableSwitch { dflt, labels, .. }
                | Insn::LookupSwitch { dflt, labels, .. } => {
                    // ASM recurses here, and pushes the labels after the recursive call returns;
                    // pushing them first is the same, since the call has its own work list.
                    default_target = Some(index_of(*dflt));
                    for label in labels.iter().rev() {
                        instruction_indices_to_process.push(index_of(*label));
                    }
                }
                _ => {}
            }

            // Push the exception handler successors of currentInsn onto
            // instructionIndicesToProcess.
            for &try_catch_block_index in &handlers[current_insn_index] {
                let handler = method.try_catch_blocks[try_catch_block_index].handler;
                instruction_indices_to_process.push(index_of(handler));
            }

            // Push the next instruction, if the control flow can go from currentInsn to the
            // next.
            match current_insn.opcode() {
                Some(
                    GOTO | RET | TABLESWITCH | LOOKUPSWITCH | IRETURN | LRETURN | FRETURN | DRETURN
                    | ARETURN | RETURN | ATHROW,
                ) => {}
                _ => instruction_indices_to_process.push(current_insn_index as i64 + 1),
            }

            if let Some(default_target) = default_target {
                calls.push(vec![default_target]);
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
