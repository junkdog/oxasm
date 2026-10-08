//! [`CheckFrameAnalyzer`]: ASM's `CheckFrameAnalyzer`, an analyzer that checks the stack map
//! frames of a method.

use std::fmt::Display;

use oxasm::opcodes::*;
use oxasm::{FrameKind, Interner, JavaStr, Type};
use oxasm_analysis::{Analyzer, AnalyzerError, AnalyzerHooks, Frame, InsnRef, Interpreter, Value};
use oxasm_tree::{
    FrameNode, Insn, InsnId, InsnKind, InsnList, InsnNode, MethodNode, SymFrameValue,
};

/// The internal name of `java.lang.Throwable`, the type a `finally` handler catches.
const THROWABLE: &JavaStr = oxasm::jstr!("java/lang/Throwable");

/// An [`Analyzer`] that checks that methods provide stack map frames where expected (at jump
/// targets and after instructions without an immediate successor), and that these frames are
/// valid for its interpreter (ASM's `CheckFrameAnalyzer`). They may still be invalid for the
/// JVM, if the interpreter uses a simplified type system compared to the JVM verifier.
///
/// This is done in two steps, in the analyzer's [`init`](AnalyzerHooks::init) hook:
///
/// - First, the stack map frames of the [`FrameNode`]s are expanded, and stored at their
///   instruction indices: compressed frames become full frames, whose types become values of
///   the interpreter.
/// - Next, the instructions are checked in sequence. Starting from the implicit first frame,
///   the execution of each instruction is simulated, giving a frame `f` after it. Then, where
///   the control flow cannot continue to the next instruction (after a `RETURN`, ...), a stack
///   map frame must exist after it; where it can, the frame after it, if any, must be
///   compatible with `f` (the types of `f` must be sub types of its types), and is `f`
///   otherwise; and where it can jump to another instruction, a compatible frame must exist
///   there.
///
/// `JSR` and `RET` instructions are not supported.
pub type CheckFrameAnalyzer<I> = Analyzer<I, CheckFrameHooks<I>>;

/// Constructs a new [`CheckFrameAnalyzer`] with the given interpreter (ASM's
/// `CheckFrameAnalyzer(Interpreter)`). The analyzer and its hooks each get a copy of the
/// interpreter, which ASM's share.
pub fn check_frame_analyzer<I>(interpreter: I) -> CheckFrameAnalyzer<I>
where
    I: Interpreter + Clone,
    I::V: Display,
{
    Analyzer::with_hooks(interpreter.clone(), CheckFrameHooks::new(interpreter))
}

/// The [hooks](AnalyzerHooks) of a [`CheckFrameAnalyzer`]: ASM's `CheckFrameAnalyzer` methods.
#[derive(Clone, Debug)]
pub struct CheckFrameHooks<I> {
    /// The interpreter to use to symbolically interpret the bytecode instructions.
    interpreter: I,
    /// The number of locals in the last stack map frame processed by `expand_frame`. Long and
    /// double values are represented with two elements.
    current_locals: usize,
    /// A list with one `ACONST_NULL` instruction, which the interpreter interprets to create the
    /// value of a `null` frame type.
    aconst_null: InsnList,
}

impl<I> CheckFrameHooks<I> {
    /// Constructs the hooks of a [`CheckFrameAnalyzer`] using the given interpreter.
    pub fn new(interpreter: I) -> CheckFrameHooks<I> {
        let mut aconst_null = InsnList::new();
        aconst_null.add(Insn::Simple {
            opcode: ACONST_NULL,
        });
        CheckFrameHooks {
            interpreter,
            current_locals: 0,
            aconst_null,
        }
    }
}

/// The state of `init`'s analysis of a method.
struct Check<'a, V> {
    method: &'a MethodNode,
    insns: Vec<(InsnId, &'a InsnNode)>,
    frames: &'a mut [Option<Frame<V>>],
}

/// The message of Java's `ArrayIndexOutOfBoundsException`, for a `frames[index]` out of bounds
/// (`index` -1 is a label `indexOf` did not find).
fn index_out_of_bounds(index: Option<usize>, length: usize) -> AnalyzerError {
    let index = index.map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
    AnalyzerError::runtime(format!("Index {index} out of bounds for length {length}"))
}

/// Java's `NullPointerException` message for `new Frame(frame)` with a null frame.
fn null_frame() -> AnalyzerError {
    AnalyzerError::runtime("Cannot read field \"numLocals\" because \"frame\" is null")
}

/// The value of an interpreter method that must not return `null` (Java would fail with a
/// `NullPointerException` later on).
fn required<V>(value: Option<V>, operation: &str) -> Result<V, AnalyzerError> {
    value.ok_or_else(|| AnalyzerError::runtime(format!("{operation} returned null")))
}

/// Whether the given instruction is a real JVM instruction (not a label, line number or frame
/// node).
fn is_jvm_insn_node(insn: &InsnNode) -> bool {
    insn.opcode().is_some()
}

impl<V: Clone> Check<'_, V> {
    /// The frame at `insn_index`, as Java's `getFrames()[insnIndex]`.
    fn frame(&self, insn_index: Option<usize>) -> Result<Option<&Frame<V>>, AnalyzerError> {
        match insn_index.and_then(|index| self.frames.get(index)) {
            Some(frame) => Ok(frame.as_ref()),
            None => Err(index_out_of_bounds(insn_index, self.frames.len())),
        }
    }

    /// Whether the given instruction is followed by a JVM instruction or a by stack map frame
    /// (`hasNextJvmInsnOrFrame`).
    fn has_next_jvm_insn_or_frame(&self, insn_index: usize) -> bool {
        self.insns[insn_index + 1..]
            .iter()
            .any(|(_, insn)| is_jvm_insn_node(insn) || insn.kind() == InsnKind::Frame)
    }

    /// The index of a label in the instruction list (Java's `indexOf`, `None` for -1).
    fn label_index(&self, label: oxasm::Label) -> Option<usize> {
        self.method.instructions.label_index(label)
    }
}

impl<I> CheckFrameHooks<I>
where
    I: Interpreter,
    I::V: Display,
{
    /// Expands the [`FrameNode`]s of the method and stores the frames at their indices, and at
    /// the indices of the label and line number nodes immediately preceding them
    /// (`expandFrames`).
    fn expand_frames(
        &mut self,
        owner: &JavaStr,
        check: &mut Check<'_, I::V>,
        initial_frame: &Frame<I::V>,
    ) -> Result<(), AnalyzerError> {
        let mut last_jvm_or_frame_insn_index: Option<usize> = None;
        let mut current_frame = initial_frame.clone();
        for current_insn_index in 0..check.insns.len() {
            let insn = check.insns[current_insn_index].1;
            if let Insn::Frame(frame_node) = &insn.insn {
                current_frame = self
                    .expand_frame(
                        owner,
                        check.method.interner(),
                        &check.method.instructions,
                        &current_frame,
                        frame_node,
                    )
                    .map_err(|e| {
                        if e.is_runtime() {
                            // Not an AnalyzerException: ASM does not catch it here.
                            e
                        } else {
                            prefixed(current_insn_index, e.insn, &e)
                        }
                    })?;
                let first = last_jvm_or_frame_insn_index.map_or(0, |index| index + 1);
                for index in first..=current_insn_index {
                    check.frames[index] = Some(current_frame.clone());
                }
            }
            if is_jvm_insn_node(insn) || insn.kind() == InsnKind::Frame {
                last_jvm_or_frame_insn_index = Some(current_insn_index);
            }
        }
        Ok(())
    }

    /// The expanded version of the given, possibly compressed, frame node (`expandFrame`).
    fn expand_frame(
        &mut self,
        owner: &JavaStr,
        interner: &Interner,
        insn_list: &InsnList,
        previous_frame: &Frame<I::V>,
        frame_node: &FrameNode,
    ) -> Result<Frame<I::V>, AnalyzerError> {
        let mut frame = previous_frame.clone();
        let mut current_local = self.current_locals;
        match frame_node.kind {
            FrameKind::New | FrameKind::Full | FrameKind::Append => {
                if frame_node.kind != FrameKind::Append {
                    current_local = 0;
                }
                for ty in &frame_node.local {
                    let value = self.new_frame_value(owner, interner, insn_list, ty)?;
                    let size = value.get_size();
                    if current_local + size > frame.get_locals() {
                        return Err(AnalyzerError::new(
                            None,
                            "Cannot append more locals than maxLocals",
                        ));
                    }
                    frame.try_set_local(current_local, value)?;
                    current_local += 1;
                    if size == 2 {
                        let value = required(self.interpreter.new_value(None)?, "newValue")?;
                        frame.try_set_local(current_local, value)?;
                        current_local += 1;
                    }
                }
            }
            FrameKind::Chop(count) => {
                for _ in 0..count {
                    if current_local == 0 {
                        return Err(AnalyzerError::new(
                            None,
                            "Cannot chop more locals than defined",
                        ));
                    }
                    if current_local > 1 && frame.try_get_local(current_local - 2)?.get_size() == 2
                    {
                        current_local -= 2;
                    } else {
                        current_local -= 1;
                    }
                }
            }
            FrameKind::Same | FrameKind::Same1 => {}
            FrameKind::Insert => {
                return Err(AnalyzerError::new(
                    None,
                    format!("Illegal frame type {}", frame_node.kind.to_i32()),
                ));
            }
        }
        self.current_locals = current_local;
        while current_local < frame.get_locals() {
            let value = required(self.interpreter.new_value(None)?, "newValue")?;
            frame.try_set_local(current_local, value)?;
            current_local += 1;
        }

        frame.clear_stack();
        for ty in &frame_node.stack {
            let value = self.new_frame_value(owner, interner, insn_list, ty)?;
            frame.try_push(value)?;
        }
        Ok(frame)
    }

    /// The value that represents the given stack map frame type (`newFrameValue`).
    fn new_frame_value(
        &mut self,
        owner: &JavaStr,
        interner: &Interner,
        insn_list: &InsnList,
        ty: &SymFrameValue,
    ) -> Result<I::V, AnalyzerError> {
        let value = match ty {
            SymFrameValue::Top => self.interpreter.new_value(None)?,
            SymFrameValue::Integer => self.interpreter.new_value(Some(&Type::INT))?,
            SymFrameValue::Float => self.interpreter.new_value(Some(&Type::FLOAT))?,
            SymFrameValue::Long => self.interpreter.new_value(Some(&Type::LONG))?,
            SymFrameValue::Double => self.interpreter.new_value(Some(&Type::DOUBLE))?,
            SymFrameValue::Null => {
                let insn = InsnRef::at(interner, &self.aconst_null, 0)
                    .expect("an ACONST_NULL instruction");
                self.interpreter.new_operation(insn)?
            }
            SymFrameValue::UninitializedThis => self
                .interpreter
                .new_value(Some(&Type::get_object_type(owner)))?,
            SymFrameValue::Object(internal_name) => self.interpreter.new_value(Some(
                &Type::get_object_type(interner.resolve(*internal_name)),
            ))?,
            SymFrameValue::Uninitialized(label) => {
                let new_insn = insn_list.label_index(*label).and_then(|index| {
                    insn_list
                        .iter()
                        .skip(index)
                        .find(|insn| is_jvm_insn_node(insn))
                });
                match new_insn.map(|insn| &insn.insn) {
                    Some(Insn::Type { opcode: NEW, desc }) => self
                        .interpreter
                        .new_value(Some(&Type::get_object_type(interner.resolve(*desc))))?,
                    _ => {
                        return Err(AnalyzerError::new(
                            None,
                            "LabelNode does not designate a NEW instruction",
                        ));
                    }
                }
            }
        };
        required(value, "newValue")
    }

    /// Checks that the given frame is compatible with the frame at the given instruction index,
    /// if any. If there is none and none is required, sets it to the given frame (`checkFrame`).
    fn check_frame(
        &mut self,
        check: &mut Check<'_, I::V>,
        insn_index: Option<usize>,
        frame: Option<&Frame<I::V>>,
        require_frame: bool,
    ) -> Result<(), AnalyzerError> {
        let old_frame = check.frame(insn_index)?;
        let insn_index = insn_index.expect("checked by frame()");
        match old_frame {
            None => {
                if require_frame {
                    return Err(AnalyzerError::new(
                        None,
                        format!("Expected stack map frame at instruction {insn_index}"),
                    ));
                }
                let frame = frame.ok_or_else(null_frame)?.clone();
                check.frames[insn_index] = Some(frame);
            }
            Some(old_frame) => {
                let frame = frame.ok_or_else(null_frame)?;
                if let Some(error) = self.check_merge(frame, old_frame)? {
                    return Err(AnalyzerError::new(
                        None,
                        format!(
                            "Stack map frame incompatible with frame at instruction {insn_index} \
                             ({error})"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks that merging the two given frames would not change the destination frame, i.e.
    /// that the types of the source frame are sub types of those of the destination frame
    /// (`checkMerge`). Returns an error message if not.
    fn check_merge(
        &mut self,
        src_frame: &Frame<I::V>,
        dst_frame: &Frame<I::V>,
    ) -> Result<Option<String>, AnalyzerError> {
        let num_locals = src_frame.get_locals();
        // An AssertionError in ASM.
        assert_eq!(
            num_locals,
            dst_frame.get_locals(),
            "frames with different locals"
        );
        for i in 0..num_locals {
            let (src, dst) = (src_frame.get_local(i), dst_frame.get_local(i));
            let merged = self.interpreter.merge(src, dst)?;
            if merged.as_ref().unwrap_or(src) != dst {
                return Ok(Some(format!(
                    "incompatible types at local {i}: {src} and {dst}"
                )));
            }
        }
        let num_stack = src_frame.get_stack_size();
        if num_stack != dst_frame.get_stack_size() {
            return Ok(Some("incompatible stack heights".to_owned()));
        }
        for i in 0..num_stack {
            let (src, dst) = (src_frame.get_stack(i), dst_frame.get_stack(i));
            let merged = self.interpreter.merge(src, dst)?;
            if merged.as_ref().unwrap_or(src) != dst {
                return Ok(Some(format!(
                    "incompatible types at stack item {i}: {src} and {dst}"
                )));
            }
        }
        Ok(None)
    }

    /// Ends the control flow at the given instruction: checks that there is a frame at the next
    /// instruction, if any (`endControlFlow`).
    fn end_control_flow(
        &self,
        check: &Check<'_, I::V>,
        insn_index: usize,
    ) -> Result<(), AnalyzerError> {
        if check.has_next_jvm_insn_or_frame(insn_index)
            && check.frame(Some(insn_index + 1))?.is_none()
        {
            return Err(AnalyzerError::new(
                None,
                format!("Expected stack map frame at instruction {}", insn_index + 1),
            ));
        }
        Ok(())
    }

    /// Checks the instruction at `insn_index`: the body of `init`'s loop.
    fn check_insn(
        &mut self,
        check: &mut Check<'_, I::V>,
        handlers: &[Vec<usize>],
        current_frame: &mut Frame<I::V>,
        insn_index: usize,
    ) -> Result<(), AnalyzerError> {
        let old_frame = check.frames[insn_index].clone();
        let (id, insn) = check.insns[insn_index];
        let insn_opcode = insn.opcode();

        match insn.kind() {
            InsnKind::Label | InsnKind::Line | InsnKind::Frame => {
                self.check_frame(check, Some(insn_index + 1), old_frame.as_ref(), false)?;
            }
            _ => {
                let old_frame_ref = old_frame.as_ref().ok_or_else(|| {
                    AnalyzerError::runtime(
                        "Cannot read field \"returnValue\" because \"frame\" is null",
                    )
                })?;
                let insn_ref = InsnRef {
                    interner: check.method.interner(),
                    list: &check.method.instructions,
                    id,
                    index: insn_index,
                    node: insn,
                };
                current_frame
                    .init(old_frame_ref)
                    .execute(insn_ref, &mut self.interpreter)?;

                match &insn.insn {
                    Insn::Jump { opcode, label } => {
                        if *opcode == JSR {
                            return Err(AnalyzerError::new(
                                Some(id),
                                "JSR instructions are unsupported",
                            ));
                        }
                        let target_insn_index = check.label_index(*label);
                        self.check_frame(check, target_insn_index, Some(current_frame), true)?;
                        if *opcode == GOTO {
                            self.end_control_flow(check, insn_index)?;
                        } else {
                            self.check_frame(
                                check,
                                Some(insn_index + 1),
                                Some(current_frame),
                                false,
                            )?;
                        }
                    }
                    Insn::LookupSwitch(insn) => {
                        let target_insn_index = check.label_index(insn.dflt);
                        self.check_frame(check, target_insn_index, Some(current_frame), true)?;
                        for label in &insn.labels {
                            let target_insn_index = check.label_index(*label);
                            self.check_frame(check, target_insn_index, Some(current_frame), true)?;
                        }
                        self.end_control_flow(check, insn_index)?;
                    }
                    Insn::TableSwitch(insn) => {
                        let target_insn_index = check.label_index(insn.dflt);
                        self.check_frame(check, target_insn_index, Some(current_frame), true)?;
                        for label in &insn.labels {
                            let target_insn_index = check.label_index(*label);
                            self.check_frame(check, target_insn_index, Some(current_frame), true)?;
                        }
                        self.end_control_flow(check, insn_index)?;
                    }
                    _ if insn_opcode == Some(RET) => {
                        return Err(AnalyzerError::new(
                            Some(id),
                            "RET instructions are unsupported",
                        ));
                    }
                    _ if insn_opcode != Some(ATHROW)
                        && !matches!(insn_opcode, Some(IRETURN..=RETURN)) =>
                    {
                        self.check_frame(check, Some(insn_index + 1), Some(current_frame), false)?;
                    }
                    _ => self.end_control_flow(check, insn_index)?,
                }
            }
        }

        for &try_catch_block_index in &handlers[insn_index] {
            let try_catch_block = &check.method.try_catch_blocks[try_catch_block_index];
            let catch_type = Type::get_object_type(
                try_catch_block
                    .type_
                    .map_or(THROWABLE, |t| check.method.resolve(t)),
            );
            let mut handler = old_frame.clone().ok_or_else(null_frame)?;
            handler.clear_stack();
            let exception =
                self.interpreter
                    .new_exception_value(try_catch_block, &handler, &catch_type)?;
            handler.try_push(required(exception, "newExceptionValue")?)?;
            let handler_index = check.label_index(try_catch_block.handler);
            self.check_frame(check, handler_index, Some(&handler), true)?;
        }
        Ok(())
    }
}

/// An error at the given instruction index, as `CheckFrameAnalyzer` wraps it.
fn prefixed(insn_index: usize, insn: Option<InsnId>, e: &AnalyzerError) -> AnalyzerError {
    let mut message = oxasm::JavaString::from(format!("Error at instruction {insn_index}: "));
    message.push_java_str(&e.message);
    AnalyzerError::new(insn, message)
}

impl<I> AnalyzerHooks<I::V> for CheckFrameHooks<I>
where
    I: Interpreter,
    I::V: Display,
{
    fn init(
        &mut self,
        owner: &JavaStr,
        method: &MethodNode,
        frames: &mut [Option<Frame<I::V>>],
        handlers: &[Vec<usize>],
    ) -> Result<(), AnalyzerError> {
        // The descriptor is valid: the analyzer parsed it to compute the first frame.
        let arguments_size = usize::try_from(
            Type::get_arguments_and_return_sizes_of(method.resolve(method.desc)) >> 2,
        )
        .expect("a u32 fits in a usize");
        self.current_locals = if method.access & ACC_STATIC != 0 {
            arguments_size - 1
        } else {
            arguments_size
        };

        let mut check = Check {
            method,
            insns: method.instructions.iter_with_ids().collect(),
            frames,
        };
        let first_frame = check.frames[0].clone().ok_or_else(null_frame)?;
        let mut current_frame = first_frame.clone();
        self.expand_frames(owner, &mut check, &first_frame)?;
        for insn_index in 0..check.insns.len() {
            let id = check.insns[insn_index].0;
            self.check_insn(&mut check, handlers, &mut current_frame, insn_index)
                .map_err(|e| {
                    // ASM wraps both its AnalyzerExceptions (at their instruction) and its
                    // RuntimeExceptions (at this instruction).
                    let insn = if e.is_runtime() { Some(id) } else { e.insn };
                    prefixed(insn_index, insn, &e)
                })?;
            if !check.has_next_jvm_insn_or_frame(insn_index) {
                break;
            }
        }
        Ok(())
    }
}
