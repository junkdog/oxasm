//! [`Frame`]: ASM's `Frame`, a symbolic execution stack frame.

use std::fmt;

use oxasm::opcodes::*;
use oxasm::{JavaStr, Type, TypeSort};
use oxasm_tree::Insn;

use crate::error::AnalyzerError;
use crate::interpreter::{InsnRef, Interpreter};
use crate::value::Value;

/// The maximum size of the operand stack of any method.
const MAX_STACK_SIZE: usize = 65536;

/// A symbolic execution stack frame (ASM's `Frame`): local variable slots and an operand stack.
///
/// Long and double values take *two* slots in the local variables, and *one* in the operand
/// stack.
///
/// ASM's frames hold `null` in the local variables until they are set; here, a frame is created
/// with the values of its local variables ([`Frame::new`]). ASM's `Frame` subclasses override
/// `initJumpTarget`, which is a method of [`AnalyzerHooks`](crate::AnalyzerHooks) here.
#[derive(Clone, Debug)]
pub struct Frame<V> {
    /// The expected return type of the analyzed method, or `None` if it returns `void`.
    return_value: Option<V>,
    /// The local variables, then the operand stack.
    values: Vec<V>,
    /// The number of local variables.
    num_locals: usize,
    /// The maximum number of elements in the operand stack.
    max_stack: usize,
    /// The size of ASM's `values` array, which bounds the stack in Java's peculiar way (see
    /// [`Frame::try_push`]).
    capacity: usize,
}

impl<V> Frame<V> {
    /// This frame, borrowed as a [`FrameRef`].
    pub fn as_frame_ref(&self) -> FrameRef<'_, V> {
        FrameRef::new(&self.return_value, &self.values, self.shape())
    }

    /// The sizes of this frame.
    pub(crate) fn shape(&self) -> FrameShape {
        FrameShape {
            num_locals: self.num_locals,
            max_stack: self.max_stack,
            capacity: self.capacity,
        }
    }
}

impl<V: Value> Frame<V> {
    /// Constructs a new frame with the given local variables and an empty stack of at most
    /// `max_stack` elements, or 65536 if `None` (ASM's `Frame(numLocals,
    /// maxStack)`, with `-1` for `None`, followed by `setLocal` of each local).
    pub fn new(locals: Vec<V>, max_stack: Option<usize>) -> Frame<V> {
        let num_locals = locals.len();
        Frame {
            return_value: None,
            values: locals,
            num_locals,
            max_stack: max_stack.unwrap_or(MAX_STACK_SIZE),
            capacity: num_locals + max_stack.unwrap_or(4),
        }
    }

    /// Copies the state of the given frame into this frame (ASM's `init`).
    pub fn init(&mut self, frame: &Frame<V>) -> &mut Frame<V> {
        self.init_from(&frame.return_value, &frame.values, frame.shape())
    }

    /// [`init`](Self::init) from the parts of a frame.
    pub(crate) fn init_from(
        &mut self,
        return_value: &Option<V>,
        values: &[V],
        shape: FrameShape,
    ) -> &mut Frame<V> {
        self.return_value.clone_from(return_value);
        values.clone_into(&mut self.values);
        self.num_locals = shape.num_locals;
        self.max_stack = shape.max_stack;
        self.accumulate_capacity(shape);
        self
    }

    /// What [`init_from`](Self::init_from) keeps of the previous state: the largest capacity.
    pub(crate) fn accumulate_capacity(&mut self, shape: FrameShape) {
        self.capacity = self.capacity.max(shape.capacity);
    }

    /// Makes this frame a clone of the frame with the given parts, reusing its storage.
    pub(crate) fn assign(&mut self, return_value: &Option<V>, values: &[V], shape: FrameShape) {
        self.init_from(return_value, values, shape);
        self.capacity = shape.capacity;
    }

    /// The local variables, then the operand stack.
    pub(crate) fn values(&self) -> &[V] {
        &self.values
    }

    /// The expected return type of the analyzed method, as stored.
    pub(crate) fn return_value(&self) -> &Option<V> {
        &self.return_value
    }

    /// Sets the expected return type of the analyzed method, `None` if it returns `void` (ASM's
    /// `setReturn`).
    pub fn set_return(&mut self, value: Option<V>) {
        self.return_value = value;
    }

    /// The expected return type of the analyzed method, `None` if it returns `void`.
    pub fn get_return(&self) -> Option<&V> {
        self.return_value.as_ref()
    }

    /// The number of local variables of this frame (ASM's `getLocals()`). Long and double values
    /// take two variables.
    pub fn get_locals(&self) -> usize {
        self.num_locals
    }

    /// The maximum number of elements in the operand stack of this frame (ASM's
    /// `getMaxStackSize()`).
    pub fn get_max_stack_size(&self) -> usize {
        self.max_stack
    }

    /// The local variables of this frame.
    pub fn locals(&self) -> &[V] {
        &self.values[..self.num_locals]
    }

    /// The operand stack of this frame, from bottom to top.
    pub fn stack(&self) -> &[V] {
        &self.values[self.num_locals..]
    }

    /// The value of the given local variable (ASM's `getLocal`).
    ///
    /// # Panics
    ///
    /// If the variable does not exist (ASM throws `IndexOutOfBoundsException`).
    #[track_caller]
    pub fn get_local(&self, index: usize) -> &V {
        self.try_get_local(index).unwrap_or_else(|e| panic!("{e}"))
    }

    /// The value of the given local variable.
    ///
    /// # Errors
    ///
    /// If the variable does not exist, with ASM's message.
    pub fn try_get_local(&self, index: usize) -> Result<&V, AnalyzerError> {
        self.as_frame_ref().try_get_local(index)
    }

    /// Sets the value of the given local variable (ASM's `setLocal`).
    ///
    /// # Panics
    ///
    /// If the variable does not exist (ASM throws `IndexOutOfBoundsException`).
    #[track_caller]
    pub fn set_local(&mut self, index: usize, value: V) {
        self.try_set_local(index, value)
            .unwrap_or_else(|e| panic!("{e}"));
    }

    /// Sets the value of the given local variable.
    ///
    /// # Errors
    ///
    /// If the variable does not exist, with ASM's message.
    pub fn try_set_local(&mut self, index: usize, value: V) -> Result<(), AnalyzerError> {
        if index >= self.num_locals {
            return Err(AnalyzerError::runtime(format!(
                "Trying to set an inexistant local variable {index}"
            )));
        }
        self.values[index] = value;
        Ok(())
    }

    /// The number of elements in the operand stack of this frame (ASM's `getStackSize()`). Long
    /// and double values take one element.
    pub fn get_stack_size(&self) -> usize {
        self.values.len() - self.num_locals
    }

    /// The value of the given operand stack slot, 0 being the bottom (ASM's `getStack`).
    ///
    /// # Panics
    ///
    /// If the slot does not exist.
    #[track_caller]
    pub fn get_stack(&self, index: usize) -> &V {
        &self.stack()[index]
    }

    /// Sets the value of the given operand stack slot (ASM's `setStack`).
    ///
    /// # Panics
    ///
    /// If the slot does not exist.
    #[track_caller]
    pub fn set_stack(&mut self, index: usize, value: V) {
        let num_locals = self.num_locals;
        self.values[num_locals..][index] = value;
    }

    /// Clears the operand stack of this frame (ASM's `clearStack`).
    pub fn clear_stack(&mut self) {
        let num_locals = self.num_locals;
        self.values.truncate(num_locals);
    }

    /// Pops a value from the operand stack of this frame (ASM's `pop`).
    ///
    /// # Panics
    ///
    /// If the operand stack is empty (ASM throws `IndexOutOfBoundsException`).
    #[track_caller]
    pub fn pop(&mut self) -> V {
        self.try_pop().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Pops a value from the operand stack of this frame.
    ///
    /// # Errors
    ///
    /// If the operand stack is empty, with ASM's message.
    pub fn try_pop(&mut self) -> Result<V, AnalyzerError> {
        if self.values.len() == self.num_locals {
            return Err(AnalyzerError::runtime(
                "Cannot pop operand off an empty stack.",
            ));
        }
        Ok(self.values.pop().expect("the stack is not empty"))
    }

    /// Pushes a value onto the operand stack of this frame (ASM's `push`).
    ///
    /// # Panics
    ///
    /// If the operand stack is full (ASM throws `IndexOutOfBoundsException`).
    #[track_caller]
    pub fn push(&mut self, value: V) {
        self.try_push(value).unwrap_or_else(|e| panic!("{e}"));
    }

    /// Pushes a value onto the operand stack of this frame.
    ///
    /// # Errors
    ///
    /// If the operand stack is full, with ASM's message. As in ASM, a frame with a maximum stack
    /// size is full when its stack has that many elements; one without (see [`Frame::new`])
    /// grows its capacity by doubling, and is full when it is out of capacity and the locals and
    /// the stack have at least 65536 elements.
    pub fn try_push(&mut self, value: V) -> Result<(), AnalyzerError> {
        let size = self.values.len();
        if size >= self.capacity {
            if size >= self.max_stack {
                return Err(AnalyzerError::runtime("Insufficient maximum stack size."));
            }
            self.capacity *= 2;
        }
        self.values.push(value);
        Ok(())
    }

    /// Pushes the result of an interpreter operation, which must be a value.
    fn push_result(&mut self, value: Option<V>, operation: &str) -> Result<(), AnalyzerError> {
        self.try_push(required(value, operation)?)
    }

    /// Simulates the execution of the given instruction on this frame (ASM's `execute`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be executed on this frame (e.g. a `POP` on an empty operand
    /// stack), or the interpreter fails.
    pub fn execute<I>(
        &mut self,
        insn: InsnRef<'_>,
        interpreter: &mut I,
    ) -> Result<(), AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        let Some(opcode) = insn.opcode() else {
            return Err(AnalyzerError::new(Some(insn.id), "Illegal opcode -1"));
        };
        match opcode {
            NOP => {}
            ACONST_NULL | ICONST_M1 | ICONST_0 | ICONST_1 | ICONST_2 | ICONST_3 | ICONST_4
            | ICONST_5 | LCONST_0 | LCONST_1 | FCONST_0 | FCONST_1 | FCONST_2 | DCONST_0
            | DCONST_1 | BIPUSH | SIPUSH | LDC => {
                let value = interpreter.new_operation(insn)?;
                self.push_result(value, "newOperation")?;
            }
            ILOAD | LLOAD | FLOAD | DLOAD | ALOAD => {
                let value = interpreter.copy_operation(insn, self.try_get_local(var(insn)?)?)?;
                self.try_push(value)?;
            }
            ISTORE | LSTORE | FSTORE | DSTORE | ASTORE => {
                let value1 = interpreter.copy_operation(insn, &self.try_pop()?)?;
                let var_index = var(insn)?;
                let size = value1.get_size();
                self.try_set_local(var_index, value1)?;
                if size == 2 {
                    let empty = interpreter.new_empty_value(var_index + 1)?;
                    self.try_set_local(var_index + 1, required(empty, "newEmptyValue")?)?;
                }
                if var_index > 0 {
                    let local = self.try_get_local(var_index - 1)?;
                    if local.get_size() == 2 {
                        let empty = interpreter.new_empty_value(var_index - 1)?;
                        self.try_set_local(var_index - 1, required(empty, "newEmptyValue")?)?;
                    }
                }
            }
            IASTORE | LASTORE | FASTORE | DASTORE | AASTORE | BASTORE | CASTORE | SASTORE => {
                let value3 = self.try_pop()?;
                let value2 = self.try_pop()?;
                let value1 = self.try_pop()?;
                interpreter.ternary_operation(insn, &value1, &value2, &value3)?;
            }
            POP => {
                if self.try_pop()?.get_size() == 2 {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of POP"));
                }
            }
            POP2 => {
                if self.try_pop()?.get_size() == 1 && self.try_pop()?.get_size() != 1 {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of POP2"));
                }
            }
            DUP => {
                let value1 = self.try_pop()?;
                if value1.get_size() != 1 {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP"));
                }
                self.push_copies(insn, interpreter, &[&value1, &value1])?;
            }
            DUP_X1 => {
                let value1 = self.try_pop()?;
                let value2 = self.try_pop()?;
                if value1.get_size() != 1 || value2.get_size() != 1 {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP_X1"));
                }
                self.push_copies(insn, interpreter, &[&value1, &value2, &value1])?;
            }
            DUP_X2 => {
                let value1 = self.try_pop()?;
                if !(value1.get_size() == 1 && self.execute_dup_x2(insn, &value1, interpreter)?) {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP_X2"));
                }
            }
            DUP2 => {
                let value1 = self.try_pop()?;
                if value1.get_size() == 1 {
                    let value2 = self.try_pop()?;
                    if value2.get_size() != 1 {
                        return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP2"));
                    }
                    self.push_copies(insn, interpreter, &[&value2, &value1, &value2, &value1])?;
                } else {
                    self.push_copies(insn, interpreter, &[&value1, &value1])?;
                }
            }
            DUP2_X1 => {
                let value1 = self.try_pop()?;
                let done = if value1.get_size() == 1 {
                    let value2 = self.try_pop()?;
                    if value2.get_size() == 1 {
                        let value3 = self.try_pop()?;
                        if value3.get_size() == 1 {
                            self.push_copies(
                                insn,
                                interpreter,
                                &[&value2, &value1, &value3, &value2, &value1],
                            )?;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    let value2 = self.try_pop()?;
                    if value2.get_size() == 1 {
                        self.push_copies(insn, interpreter, &[&value1, &value2, &value1])?;
                        true
                    } else {
                        false
                    }
                };
                if !done {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP2_X1"));
                }
            }
            DUP2_X2 => {
                let value1 = self.try_pop()?;
                let done = if value1.get_size() == 1 {
                    let value2 = self.try_pop()?;
                    if value2.get_size() == 1 {
                        let value3 = self.try_pop()?;
                        if value3.get_size() == 1 {
                            let value4 = self.try_pop()?;
                            if value4.get_size() == 1 {
                                self.push_copies(
                                    insn,
                                    interpreter,
                                    &[&value2, &value1, &value4, &value3, &value2, &value1],
                                )?;
                                true
                            } else {
                                false
                            }
                        } else {
                            self.push_copies(
                                insn,
                                interpreter,
                                &[&value2, &value1, &value3, &value2, &value1],
                            )?;
                            true
                        }
                    } else {
                        false
                    }
                } else {
                    self.execute_dup_x2(insn, &value1, interpreter)?
                };
                if !done {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of DUP2_X2"));
                }
            }
            SWAP => {
                let value2 = self.try_pop()?;
                let value1 = self.try_pop()?;
                if value1.get_size() != 1 || value2.get_size() != 1 {
                    return Err(AnalyzerError::new(Some(insn.id), "Illegal use of SWAP"));
                }
                self.push_copies(insn, interpreter, &[&value2, &value1])?;
            }
            IALOAD | LALOAD | FALOAD | DALOAD | AALOAD | BALOAD | CALOAD | SALOAD | IADD | LADD
            | FADD | DADD | ISUB | LSUB | FSUB | DSUB | IMUL | LMUL | FMUL | DMUL | IDIV | LDIV
            | FDIV | DDIV | IREM | LREM | FREM | DREM | ISHL | LSHL | ISHR | LSHR | IUSHR
            | LUSHR | IAND | LAND | IOR | LOR | IXOR | LXOR | LCMP | FCMPL | FCMPG | DCMPL
            | DCMPG => {
                let value2 = self.try_pop()?;
                let value1 = self.try_pop()?;
                let value = interpreter.binary_operation(insn, &value1, &value2)?;
                self.push_result(value, "binaryOperation")?;
            }
            INEG | LNEG | FNEG | DNEG | I2L | I2F | I2D | L2I | L2F | L2D | F2I | F2L | F2D
            | D2I | D2L | D2F | I2B | I2C | I2S | GETFIELD | NEWARRAY | ANEWARRAY | ARRAYLENGTH
            | CHECKCAST | INSTANCEOF => {
                let value1 = self.try_pop()?;
                let value = interpreter.unary_operation(insn, &value1)?;
                self.push_result(value, "unaryOperation")?;
            }
            IINC => {
                let Insn::Iinc { var, .. } = insn.insn else {
                    return Err(cast_error(insn, "IincInsnNode"));
                };
                let var_index = usize::from(var);
                let value = interpreter.unary_operation(insn, self.try_get_local(var_index)?)?;
                self.try_set_local(var_index, required(value, "unaryOperation")?)?;
            }
            IFEQ | IFNE | IFLT | IFGE | IFGT | IFLE | TABLESWITCH | LOOKUPSWITCH | PUTSTATIC
            | ATHROW | MONITORENTER | MONITOREXIT | IFNULL | IFNONNULL => {
                let value1 = self.try_pop()?;
                interpreter.unary_operation(insn, &value1)?;
            }
            IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE | IF_ACMPEQ
            | IF_ACMPNE | PUTFIELD => {
                let value2 = self.try_pop()?;
                let value1 = self.try_pop()?;
                interpreter.binary_operation(insn, &value1, &value2)?;
            }
            GOTO | RET => {}
            JSR | GETSTATIC | NEW => {
                let value = interpreter.new_operation(insn)?;
                self.push_result(value, "newOperation")?;
            }
            IRETURN | LRETURN | FRETURN | DRETURN | ARETURN => {
                let value1 = self.try_pop()?;
                interpreter.unary_operation(insn, &value1)?;
                interpreter.return_operation(insn, &value1, self.return_value.as_ref())?;
            }
            RETURN => {
                if self.return_value.is_some() {
                    return Err(AnalyzerError::new(
                        Some(insn.id),
                        "Incompatible return type",
                    ));
                }
            }
            INVOKEVIRTUAL | INVOKESPECIAL | INVOKESTATIC | INVOKEINTERFACE => {
                let Insn::Method { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "MethodInsnNode"));
                };
                self.execute_invoke_insn(insn, insn.resolve(*desc), interpreter)?;
            }
            INVOKEDYNAMIC => {
                let Insn::InvokeDynamic(invoke_dynamic) = &insn.insn else {
                    return Err(cast_error(insn, "InvokeDynamicInsnNode"));
                };
                self.execute_invoke_insn(insn, insn.resolve(invoke_dynamic.desc), interpreter)?;
            }
            MULTIANEWARRAY => {
                let Insn::MultiANewArray { dims, .. } = insn.insn else {
                    return Err(cast_error(insn, "MultiANewArrayInsnNode"));
                };
                let start = self.stack_top_start(usize::from(dims))?;
                let value = interpreter.nary_operation(insn, &self.values[start..], None);
                self.values.truncate(start);
                self.push_result(value?, "naryOperation")?;
            }
            _ => {
                return Err(AnalyzerError::new(
                    Some(insn.id),
                    format!("Illegal opcode {opcode}"),
                ));
            }
        }
        Ok(())
    }

    /// Pushes the copies (`copyOperation`) of the given values, in order.
    fn push_copies<I>(
        &mut self,
        insn: InsnRef<'_>,
        interpreter: &mut I,
        values: &[&V],
    ) -> Result<(), AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        for value in values {
            let copy = interpreter.copy_operation(insn, value)?;
            self.try_push(copy)?;
        }
        Ok(())
    }

    fn execute_dup_x2<I>(
        &mut self,
        insn: InsnRef<'_>,
        value1: &V,
        interpreter: &mut I,
    ) -> Result<bool, AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        let value2 = self.try_pop()?;
        if value2.get_size() == 1 {
            let value3 = self.try_pop()?;
            if value3.get_size() == 1 {
                self.push_copies(insn, interpreter, &[value1, &value3, &value2, value1])?;
                return Ok(true);
            }
        } else {
            self.push_copies(insn, interpreter, &[value1, &value2, value1])?;
            return Ok(true);
        }
        Ok(false)
    }

    fn execute_invoke_insn<I>(
        &mut self,
        insn: InsnRef<'_>,
        method_descriptor: &JavaStr,
        interpreter: &mut I,
    ) -> Result<(), AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        // One walk over the argument types; the return type's error, if any, comes after the
        // operands are popped, as in ASM.
        let (argument_count, return_type) =
            Type::try_get_argument_count_and_return_type_of(method_descriptor)
                .map_err(|e| AnalyzerError::runtime(e.message()))?;
        let opcode = insn.opcode();
        let receiver = usize::from(opcode != Some(INVOKESTATIC) && opcode != Some(INVOKEDYNAMIC));
        // The receiver, if any, is just below the arguments: the operands are the top of the
        // stack, in order, which the interpreter reads in place before they are popped.
        let start = self.stack_top_start(argument_count + receiver)?;
        let return_type = match return_type {
            Ok(return_type) => return_type,
            Err(e) => {
                self.values.truncate(start);
                return Err(AnalyzerError::runtime(e.message()));
            }
        };
        let value = interpreter.nary_operation(insn, &self.values[start..], Some(&return_type));
        self.values.truncate(start);
        let value = value?;
        if return_type.get_sort() != TypeSort::Void {
            self.push_result(value, "naryOperation")?;
        }
        Ok(())
    }

    /// The index in `values` of the deepest of the `count` top stack values, which the caller
    /// pops by truncating `values` there.
    ///
    /// # Errors
    ///
    /// If the stack has fewer than `count` values. Java pops one value at a time: the same
    /// error, after popping the whole stack.
    fn stack_top_start(&mut self, count: usize) -> Result<usize, AnalyzerError> {
        if count > self.get_stack_size() {
            self.clear_stack();
            return Err(AnalyzerError::runtime(
                "Cannot pop operand off an empty stack.",
            ));
        }
        Ok(self.values.len() - count)
    }

    /// Merges the given frame into this frame (ASM's `merge(Frame, Interpreter)`). Returns
    /// whether this frame changed.
    ///
    /// # Errors
    ///
    /// If the frames have different stack heights, or the interpreter fails.
    pub fn merge<I>(&mut self, frame: &Frame<V>, interpreter: &mut I) -> Result<bool, AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        if self.get_stack_size() != frame.get_stack_size() {
            return Err(incompatible_stack_heights());
        }
        interpreter.merge_values(&mut self.values, &frame.values)
    }

    /// Merges the given frame into this frame, in the case of a subroutine (ASM's
    /// `merge(Frame, boolean[])`): the operand stacks are not merged, and only the local
    /// variables that the subroutine does not use are. Returns whether this frame changed.
    pub fn merge_subroutine(&mut self, frame: &Frame<V>, locals_used: &[bool]) -> bool {
        self.merge_subroutine_values(&frame.values, locals_used)
    }

    /// [`merge_subroutine`](Self::merge_subroutine) with the values of the merged frame.
    pub(crate) fn merge_subroutine_values(&mut self, values: &[V], locals_used: &[bool]) -> bool {
        let mut changed = false;
        let num_locals = self.num_locals;
        for ((value, other), &used) in self
            .values
            .iter_mut()
            .zip(values)
            .zip(locals_used)
            .take(num_locals)
        {
            if !used && *value != *other {
                value.clone_from(other);
                changed = true;
            }
        }
        changed
    }
}

impl<V: PartialEq> PartialEq for Frame<V> {
    /// Frames are equal if they have the same return value, local variables, stack, and
    /// maximum stack size.
    fn eq(&self, other: &Frame<V>) -> bool {
        self.as_frame_ref() == other.as_frame_ref()
    }
}

impl<V: fmt::Display> fmt::Display for Frame<V> {
    /// ASM's `toString()`: the local variables, a space, then the stack, each value with its
    /// `Display`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_frame_ref(), f)
    }
}

/// A frame of an analyzed method, borrowed from its [`Frames`](crate::Frames), or from a
/// [`Frame`] ([`Frame::as_frame_ref`]): the read-only methods of [`Frame`], and
/// [`to_frame`](Self::to_frame) for a frame of its own.
pub struct FrameRef<'a, V> {
    return_value: &'a Option<V>,
    /// The local variables, then the operand stack.
    values: &'a [V],
    shape: FrameShape,
}

impl<'a, V> FrameRef<'a, V> {
    /// The frame with the given parts.
    pub(crate) fn new(return_value: &'a Option<V>, values: &'a [V], shape: FrameShape) -> Self {
        FrameRef {
            return_value,
            values,
            shape,
        }
    }

    /// A copy of this frame, as a [`Frame`] (what ASM's `new Frame<>(frame)` makes).
    pub fn to_frame(&self) -> Frame<V>
    where
        V: Clone,
    {
        Frame {
            return_value: self.return_value.clone(),
            values: self.values.to_vec(),
            num_locals: self.shape.num_locals,
            max_stack: self.shape.max_stack,
            capacity: self.shape.capacity,
        }
    }

    /// The expected return type of the analyzed method, `None` if it returns `void`.
    pub fn get_return(&self) -> Option<&'a V> {
        self.return_value.as_ref()
    }

    /// The number of local variables of this frame (ASM's `getLocals()`). Long and double values
    /// take two variables.
    pub fn get_locals(&self) -> usize {
        self.shape.num_locals
    }

    /// The maximum number of elements in the operand stack of this frame (ASM's
    /// `getMaxStackSize()`).
    pub fn get_max_stack_size(&self) -> usize {
        self.shape.max_stack
    }

    /// The local variables of this frame.
    pub fn locals(&self) -> &'a [V] {
        &self.values[..self.shape.num_locals]
    }

    /// The operand stack of this frame, from bottom to top.
    pub fn stack(&self) -> &'a [V] {
        &self.values[self.shape.num_locals..]
    }

    /// The value of the given local variable (ASM's `getLocal`).
    ///
    /// # Panics
    ///
    /// If the variable does not exist (ASM throws `IndexOutOfBoundsException`).
    #[track_caller]
    pub fn get_local(&self, index: usize) -> &'a V {
        self.try_get_local(index).unwrap_or_else(|e| panic!("{e}"))
    }

    /// The value of the given local variable.
    ///
    /// # Errors
    ///
    /// If the variable does not exist, with ASM's message.
    pub fn try_get_local(&self, index: usize) -> Result<&'a V, AnalyzerError> {
        if index >= self.shape.num_locals {
            return Err(AnalyzerError::runtime(format!(
                "Trying to get an inexistant local variable {index}"
            )));
        }
        Ok(&self.values[index])
    }

    /// The number of elements in the operand stack of this frame (ASM's `getStackSize()`). Long
    /// and double values take one element.
    pub fn get_stack_size(&self) -> usize {
        self.values.len() - self.shape.num_locals
    }

    /// The value of the given operand stack slot, 0 being the bottom (ASM's `getStack`).
    ///
    /// # Panics
    ///
    /// If the slot does not exist.
    #[track_caller]
    pub fn get_stack(&self, index: usize) -> &'a V {
        &self.stack()[index]
    }
}

impl<V> Clone for FrameRef<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for FrameRef<'_, V> {}

impl<V: PartialEq> PartialEq for FrameRef<'_, V> {
    /// Frames are equal if they have the same return value, local variables, stack, and
    /// maximum stack size.
    fn eq(&self, other: &Self) -> bool {
        self.return_value == other.return_value
            && self.shape.num_locals == other.shape.num_locals
            && self.values == other.values
            && self.shape.max_stack == other.shape.max_stack
    }
}

impl<V: PartialEq> PartialEq<Frame<V>> for FrameRef<'_, V> {
    fn eq(&self, other: &Frame<V>) -> bool {
        *self == other.as_frame_ref()
    }
}

impl<V: PartialEq> PartialEq<FrameRef<'_, V>> for Frame<V> {
    fn eq(&self, other: &FrameRef<'_, V>) -> bool {
        self.as_frame_ref() == *other
    }
}

impl<V: fmt::Display> fmt::Display for FrameRef<'_, V> {
    /// ASM's `toString()`: the local variables, a space, then the stack, each value with its
    /// `Display`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for value in self.locals() {
            write!(f, "{value}")?;
        }
        f.write_str(" ")?;
        for value in self.stack() {
            write!(f, "{value}")?;
        }
        Ok(())
    }
}

impl<V: fmt::Debug> fmt::Debug for FrameRef<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameRef")
            .field("return_value", self.return_value)
            .field("values", &self.values)
            .field("num_locals", &self.shape.num_locals)
            .field("max_stack", &self.shape.max_stack)
            .field("capacity", &self.shape.capacity)
            .finish()
    }
}

/// The sizes of a frame: what it has besides its values and return value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameShape {
    pub(crate) num_locals: usize,
    pub(crate) max_stack: usize,
    pub(crate) capacity: usize,
}

/// The error of a merge of frames with different stack heights.
pub(crate) fn incompatible_stack_heights() -> AnalyzerError {
    AnalyzerError::new(None, "Incompatible stack heights")
}

/// A value that an interpreter `operation` returned, which must not be `None` (where ASM would
/// store a `null`).
pub(crate) fn required<V>(value: Option<V>, operation: &str) -> Result<V, AnalyzerError> {
    value.ok_or_else(|| AnalyzerError::runtime(format!("{operation} returned null")))
}

/// The local variable of a `VarInsnNode`.
fn var(insn: InsnRef<'_>) -> Result<usize, AnalyzerError> {
    match insn.insn {
        Insn::Var { var, .. } => Ok(usize::from(var)),
        _ => Err(cast_error(insn, "VarInsnNode")),
    }
}

/// The error of ASM's cast of an instruction node to the node class its opcode implies, for an
/// instruction of another kind (which only a hand-built instruction list has).
pub(crate) fn cast_error(insn: InsnRef<'_>, node_class: &str) -> AnalyzerError {
    AnalyzerError::runtime(format!(
        "a {:?} instruction cannot be cast to {node_class}",
        insn.kind()
    ))
}
