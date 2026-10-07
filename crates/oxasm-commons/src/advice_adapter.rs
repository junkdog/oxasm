//! [`AdviceAdapter`] and [`Advice`]: ASM's `AdviceAdapter`, whose abstract methods are the
//! [`Advice`] hooks (ADR-13).

use std::ops::{Deref, DerefMut};

use oxasm::constant::{Constant, FrameValue};
use oxasm::opcodes::*;
use oxasm::{
    AnnotationVisitor, FrameKind, Handle, JavaStr, JavaString, Label, MethodVisitor, Type,
    TypePath, TypeReference,
};
use rustc_hash::FxHashMap;

use crate::generator_adapter::GeneratorAdapter;
use crate::support::illegal_argument;

/// The code an [`AdviceAdapter`] inserts at the beginning of a method or constructor, and before
/// each of its exits (ASM's `onMethodEnter` and `onMethodExit`, which subclasses of
/// `AdviceAdapter` override).
///
/// The hooks get the adapter itself, which is both a [`MethodVisitor`] and (by `Deref`) a
/// [`GeneratorAdapter`], exactly like `this` in an ASM subclass:
///
/// - the visit methods of the adapter (e.g. `adapter.visit_insn(...)`) go through the adapter: its
///   local variables remapping, and its tracking of the stack of a constructor (but not the
///   hooks, which are not called again while one of them runs);
/// - the code generation methods of the generator (e.g. `adapter.push_int(...)`), and the
///   downstream visitor (`adapter.mv()`), emit directly to the downstream visitor, as ASM's
///   `mv.visitXxx` calls do.
///
/// `()` is the advice that inserts nothing.
pub trait Advice<M>: Sized {
    /// Generates the "before" advice for the visited method. The default implementation of this
    /// method does nothing. Subclasses can use or change all the local variables, but should not
    /// change state of the stack. This method is called at the beginning of the method or after
    /// super class constructor has been called (in constructors).
    fn on_method_enter(&mut self, adapter: &mut AdviceAdapter<M, Self>) {
        let _ = adapter;
    }

    /// Generates the "after" advice for the visited method. The default implementation of this
    /// method does nothing. Subclasses can use or change all the local variables, but should not
    /// change state of the stack. This method is called at the end of the method, just before
    /// return and athrow instructions. The top element on the stack contains the return value or
    /// the exception instance. For example:
    ///
    /// ```java
    /// public void onMethodExit(final int opcode) {
    ///   if (opcode == RETURN) {
    ///     visitInsn(ACONST_NULL);
    ///   } else if (opcode == ARETURN || opcode == ATHROW) {
    ///     dup();
    ///   } else {
    ///     if (opcode == LRETURN || opcode == DRETURN) {
    ///       dup2();
    ///     } else {
    ///       dup();
    ///     }
    ///     box(Type.getReturnType(this.methodDesc));
    ///   }
    ///   visitIntInsn(SIPUSH, opcode);
    ///   visitMethodInsn(INVOKESTATIC, owner, "onExit", "(Ljava/lang/Object;I)V");
    /// }
    ///
    /// // An actual call back method.
    /// public static void onExit(final Object exitValue, final int opcode) {
    ///   ...
    /// }
    /// ```
    ///
    /// - `opcode`: one of RETURN, IRETURN, FRETURN, ARETURN, LRETURN, DRETURN or ATHROW.
    fn on_method_exit(&mut self, adapter: &mut AdviceAdapter<M, Self>, opcode: u8) {
        let _ = (adapter, opcode);
    }
}

impl<M> Advice<M> for () {}

/// A value of the stack of a constructor, before its super class constructor is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StackValue {
    /// The uninitialized `this`.
    UninitializedThis,
    /// Any other value.
    Other,
}

/// ASM's `ArrayList` operations on the simulated stack, which throw (here panic, with Java's
/// message) on an invalid index.
mod stack {
    use super::StackValue;

    #[track_caller]
    fn index_out_of_bounds(index: isize, length: usize) -> ! {
        panic!(
            "java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {length}"
        )
    }

    #[track_caller]
    pub(super) fn get(stack: &[StackValue], index: isize) -> StackValue {
        match usize::try_from(index).ok().and_then(|i| stack.get(i)) {
            Some(&value) => value,
            None => index_out_of_bounds(index, stack.len()),
        }
    }

    #[track_caller]
    pub(super) fn add(stack: &mut Vec<StackValue>, index: isize, value: StackValue) {
        match usize::try_from(index) {
            Ok(i) if i <= stack.len() => stack.insert(i, value),
            _ => panic!(
                "java.lang.IndexOutOfBoundsException: Index: {index}, Size: {}",
                stack.len()
            ),
        }
    }

    #[track_caller]
    pub(super) fn remove(stack: &mut Vec<StackValue>, index: isize) -> StackValue {
        match usize::try_from(index) {
            Ok(i) if i < stack.len() => stack.remove(i),
            _ => index_out_of_bounds(index, stack.len()),
        }
    }
}

/// A [`MethodVisitor`] to insert before, after and around advices in methods and constructors.
/// For constructors, the code keeps track of the elements on the stack in order to detect when
/// the super class constructor is called (note that there can be multiple such calls in
/// different branches). `on_method_enter` is called after each super class constructor call,
/// because the object cannot be used before it is properly initialized.
///
/// The advices are the [`Advice`] `A` (ASM's abstract methods, ADR-13). The adapter dereferences
/// to its [`GeneratorAdapter`] (ASM's super class), whose code generation methods the advices
/// use.
///
/// # Panics
///
/// Its methods panic where ASM's throw: on an invalid opcode, or a return of a value from a
/// constructor ("Invalid return in constructor"), before its super class constructor is called.
pub struct AdviceAdapter<M, A> {
    /// The generator adapter (ASM's super class).
    generator: GeneratorAdapter<M>,
    /// The advice, or `None` while one of its hooks is running.
    advice: Option<A>,
    /// The access flags of the visited method.
    method_access: u32,
    /// The descriptor of the visited method.
    method_desc: JavaString,
    /// Whether the visited method is a constructor.
    is_constructor: bool,
    /// Whether the super class constructor has been called (if the visited method is a
    /// constructor), at the current instruction. There can be multiple call sites to the super
    /// constructor (e.g. for Java code such as `super(expr ? value1 : value2);`), in different
    /// branches. When scanning the bytecode linearly, we can move from one branch where the
    /// super constructor has been called to another where it has not been called yet. Therefore,
    /// this value can change from false to true, and vice-versa.
    super_class_constructor_called: bool,
    /// The values on the JVM execution stack frame (if the visited method is a constructor), at
    /// the current instruction. `UninitializedThis` represents the uninitialized value of 'this',
    /// and `Other` any other value.
    stack_frame: Vec<StackValue>,
    /// The stack map frames corresponding to the labels of the forward jumps made *before* the
    /// super class constructor has been called (note that the Java Virtual Machine forbids
    /// backward jumps before the super class constructor is called). Note that by definition
    /// (cf. the 'before'), when we reach a label from this map, `super_class_constructor_called`
    /// must be reset to false. This map is only used for constructors (`None` until the code is
    /// visited).
    forward_jump_stack_frames: Option<FxHashMap<Label, Vec<StackValue>>>,
}

impl<M: MethodVisitor, A: Advice<M>> AdviceAdapter<M, A> {
    /// Constructs a new [`AdviceAdapter`].
    ///
    /// - `method_visitor`: the method visitor to which this adapter delegates calls.
    /// - `access`: the method's access flags (see [`oxasm::opcodes`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `advice`: the advices to insert.
    #[track_caller]
    pub fn new(
        method_visitor: M,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        advice: A,
    ) -> Self {
        AdviceAdapter {
            generator: GeneratorAdapter::new(method_visitor, access, name, descriptor),
            advice: Some(advice),
            method_access: access,
            method_desc: descriptor.to_owned(),
            is_constructor: name == "<init>",
            super_class_constructor_called: false,
            stack_frame: Vec::new(),
            forward_jump_stack_frames: None,
        }
    }

    /// The access flags of the visited method.
    #[must_use]
    pub fn method_access(&self) -> u32 {
        self.method_access
    }

    /// The descriptor of the visited method.
    #[must_use]
    pub fn method_desc(&self) -> &JavaStr {
        &self.method_desc
    }

    /// The advice of this adapter, or `None` while one of its hooks is running.
    pub fn advice(&mut self) -> Option<&mut A> {
        self.advice.as_mut()
    }

    /// The generator adapter of this adapter (ASM's super class).
    pub fn generator(&mut self) -> &mut GeneratorAdapter<M> {
        &mut self.generator
    }

    /// Returns the downstream visitor and the advice of this adapter.
    ///
    /// # Panics
    ///
    /// If it is called from one of the advice's hooks.
    pub fn into_parts(self) -> (M, A) {
        let advice = self.advice.expect("into_parts called from an advice");
        (self.generator.into_inner(), advice)
    }

    fn on_method_enter(&mut self) {
        if let Some(mut advice) = self.advice.take() {
            advice.on_method_enter(self);
            self.advice = Some(advice);
        }
    }

    fn on_method_exit(&mut self, opcode: u8) {
        if let Some(mut advice) = self.advice.take() {
            advice.on_method_exit(self, opcode);
            self.advice = Some(advice);
        }
    }

    fn tracks_constructor_stack(&self) -> bool {
        self.is_constructor && !self.super_class_constructor_called
    }

    fn do_visit_method_insn(&mut self, opcode: u8, name: &JavaStr, descriptor: &JavaStr) {
        if self.tracks_constructor_stack() {
            for argument_type in Type::get_argument_types_of(descriptor) {
                self.pop_value();
                if argument_type.get_size() == 2 {
                    self.pop_value();
                }
            }
            match opcode {
                INVOKEINTERFACE | INVOKEVIRTUAL => {
                    self.pop_value();
                }
                INVOKESPECIAL => {
                    let value = self.pop_value();
                    if value == StackValue::UninitializedThis
                        && !self.super_class_constructor_called
                        && name == "<init>"
                    {
                        self.super_class_constructor_called = true;
                        self.on_method_enter();
                    }
                }
                _ => {}
            }

            let return_type = Type::get_return_type_of(descriptor);
            if return_type != Type::VOID {
                self.push_value(StackValue::Other);
                if return_type.get_size() == 2 {
                    self.push_value(StackValue::Other);
                }
            }
        }
    }

    fn add_forward_jumps(&mut self, dflt: Label, labels: &[Label]) {
        self.add_forward_jump(dflt);
        for &label in labels {
            self.add_forward_jump(label);
        }
    }

    fn add_forward_jump(&mut self, label: Label) {
        let stack_frame = &self.stack_frame;
        self.forward_jump_stack_frames
            .get_or_insert_with(FxHashMap::default)
            .entry(label)
            .or_insert_with(|| stack_frame.clone());
    }

    fn end_constructor_basic_block_without_successor(&mut self) {
        // The next instruction is not reachable from this instruction. If it is dead code, we
        // should not try to simulate stack operations, and there is no need to insert advices
        // here. If it is reachable with a backward jump, the only possible case is that the super
        // class constructor has already been called (backward jumps are forbidden before it is
        // called). If it is reachable with a forward jump, there are two sub-cases. Either the
        // super class constructor has already been called when reaching the next instruction, or
        // it has not been called. But in this case there must be a forwardJumpStackFrames entry
        // for a Label designating the next instruction, and superClassConstructorCalled will be
        // reset to false there. We can therefore always reset this field to true here.
        self.super_class_constructor_called = true;
    }

    #[track_caller]
    fn pop_value(&mut self) -> StackValue {
        let index = self.stack_frame.len().cast_signed() - 1;
        stack::remove(&mut self.stack_frame, index)
    }

    #[track_caller]
    fn peek_value(&self) -> StackValue {
        stack::get(&self.stack_frame, self.stack_frame.len().cast_signed() - 1)
    }

    fn push_value(&mut self, value: StackValue) {
        self.stack_frame.push(value);
    }

    /// `stackFrame.add(stackSize - offset, stackFrame.get(stackSize - 1))`.
    #[track_caller]
    fn insert_top_at(&mut self, stack_size: isize, offset: isize) {
        let top = stack::get(&self.stack_frame, stack_size - 1);
        stack::add(&mut self.stack_frame, stack_size - offset, top);
    }

    /// Simulates the effect of a zero operand instruction on the stack of a constructor, before
    /// its super class constructor is called.
    #[track_caller]
    fn track_insn(&mut self, opcode: u8) {
        let stack_size = self.stack_frame.len().cast_signed();
        match opcode {
            IRETURN | FRETURN | ARETURN | LRETURN | DRETURN => {
                illegal_argument("Invalid return in constructor");
            }
            RETURN => {
                // empty stack
                self.on_method_exit(opcode);
                self.end_constructor_basic_block_without_successor();
            }
            ATHROW => {
                // 1 before n/a after
                self.pop_value();
                self.on_method_exit(opcode);
                self.end_constructor_basic_block_without_successor();
            }
            NOP | LALOAD | DALOAD | LNEG | DNEG | FNEG | INEG | L2D | D2L | F2I | I2B | I2C
            | I2S | I2F | ARRAYLENGTH => {}
            ACONST_NULL | ICONST_M1 | ICONST_0 | ICONST_1 | ICONST_2 | ICONST_3 | ICONST_4
            | ICONST_5 | FCONST_0 | FCONST_1 | FCONST_2 | F2L | F2D | I2L | I2D => {
                self.push_value(StackValue::Other);
            }
            LCONST_0 | LCONST_1 | DCONST_0 | DCONST_1 => {
                self.push_value(StackValue::Other);
                self.push_value(StackValue::Other);
            }
            IALOAD | FALOAD | AALOAD | BALOAD | CALOAD | SALOAD | POP | IADD | FADD | ISUB
            | LSHL | LSHR | LUSHR | L2I | L2F | D2I | D2F | FSUB | FMUL | FDIV | FREM | FCMPL
            | FCMPG | IMUL | IDIV | IREM | ISHL | ISHR | IUSHR | IAND | IOR | IXOR
            | MONITORENTER | MONITOREXIT => {
                self.pop_value();
            }
            POP2 | LSUB | LMUL | LDIV | LREM | LADD | LAND | LOR | LXOR | DADD | DMUL | DSUB
            | DDIV | DREM => {
                self.pop_value();
                self.pop_value();
            }
            IASTORE | FASTORE | AASTORE | BASTORE | CASTORE | SASTORE | LCMP | DCMPL | DCMPG => {
                self.pop_value();
                self.pop_value();
                self.pop_value();
            }
            LASTORE | DASTORE => {
                self.pop_value();
                self.pop_value();
                self.pop_value();
                self.pop_value();
            }
            DUP => {
                let value = self.peek_value();
                self.push_value(value);
            }
            DUP_X1 => self.insert_top_at(stack_size, 2),
            DUP_X2 => self.insert_top_at(stack_size, 3),
            DUP2 => {
                self.insert_top_at(stack_size, 2);
                self.insert_top_at(stack_size, 2);
            }
            DUP2_X1 => {
                self.insert_top_at(stack_size, 3);
                self.insert_top_at(stack_size, 3);
            }
            DUP2_X2 => {
                self.insert_top_at(stack_size, 4);
                self.insert_top_at(stack_size, 4);
            }
            SWAP => {
                self.insert_top_at(stack_size, 2);
                stack::remove(&mut self.stack_frame, stack_size);
            }
            _ => illegal_argument(format_args!("Invalid opcode {opcode}")),
        }
    }
}

impl<M: MethodVisitor, A> Deref for AdviceAdapter<M, A> {
    type Target = GeneratorAdapter<M>;

    fn deref(&self) -> &GeneratorAdapter<M> {
        &self.generator
    }
}

impl<M: MethodVisitor, A> DerefMut for AdviceAdapter<M, A> {
    fn deref_mut(&mut self) -> &mut GeneratorAdapter<M> {
        &mut self.generator
    }
}

impl<M, A> std::fmt::Debug for AdviceAdapter<M, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdviceAdapter")
            .field("method_access", &self.method_access)
            .field("method_desc", &self.method_desc)
            .field("is_constructor", &self.is_constructor)
            .field(
                "super_class_constructor_called",
                &self.super_class_constructor_called,
            )
            .field("stack_frame", &self.stack_frame)
            .finish_non_exhaustive()
    }
}

impl<M: MethodVisitor, A: Advice<M>> MethodVisitor for AdviceAdapter<M, A> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(self.generator.mv())
    }

    fn visit_code(&mut self) {
        self.generator.visit_code();
        if self.is_constructor {
            self.stack_frame = Vec::new();
            self.forward_jump_stack_frames = Some(FxHashMap::default());
        } else {
            self.on_method_enter();
        }
    }

    fn visit_label(&mut self, label: Label) {
        self.generator.visit_label(label);
        if self.is_constructor
            && let Some(forward_jump_stack_frames) = &mut self.forward_jump_stack_frames
            && let Some(label_stack_frame) = forward_jump_stack_frames.remove(&label)
        {
            self.stack_frame = label_stack_frame;
            self.super_class_constructor_called = false;
        }
    }

    fn visit_insn(&mut self, opcode: u8) {
        if self.tracks_constructor_stack() {
            self.track_insn(opcode);
        } else {
            match opcode {
                RETURN | IRETURN | FRETURN | ARETURN | LRETURN | DRETURN | ATHROW => {
                    self.on_method_exit(opcode);
                }
                _ => {}
            }
        }
        self.generator.visit_insn(opcode);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.generator.visit_var_insn(opcode, var_index);
        if self.tracks_constructor_stack() {
            match opcode {
                ILOAD | FLOAD => self.push_value(StackValue::Other),
                LLOAD | DLOAD => {
                    self.push_value(StackValue::Other);
                    self.push_value(StackValue::Other);
                }
                ALOAD => self.push_value(if var_index == 0 {
                    StackValue::UninitializedThis
                } else {
                    StackValue::Other
                }),
                ASTORE | ISTORE | FSTORE => {
                    self.pop_value();
                }
                LSTORE | DSTORE => {
                    self.pop_value();
                    self.pop_value();
                }
                RET => self.end_constructor_basic_block_without_successor(),
                _ => illegal_argument(format_args!("Invalid opcode {opcode}")),
            }
        }
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.generator
            .visit_field_insn(opcode, owner, name, descriptor);
        if self.tracks_constructor_stack() {
            let Some(&first_descriptor_char) = descriptor.as_bytes().first() else {
                panic!(
                    "java.lang.StringIndexOutOfBoundsException: Index 0 out of bounds for length 0"
                );
            };
            let long_or_double = first_descriptor_char == b'J' || first_descriptor_char == b'D';
            match opcode {
                GETSTATIC => {
                    self.push_value(StackValue::Other);
                    if long_or_double {
                        self.push_value(StackValue::Other);
                    }
                }
                PUTSTATIC => {
                    self.pop_value();
                    if long_or_double {
                        self.pop_value();
                    }
                }
                PUTFIELD => {
                    self.pop_value();
                    self.pop_value();
                    if long_or_double {
                        self.pop_value();
                    }
                }
                GETFIELD => {
                    if long_or_double {
                        self.push_value(StackValue::Other);
                    }
                }
                _ => illegal_argument(format_args!("Invalid opcode {opcode}")),
            }
        }
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.generator.visit_int_insn(opcode, operand);
        if self.tracks_constructor_stack() && opcode != NEWARRAY {
            self.push_value(StackValue::Other);
        }
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.generator.visit_ldc_insn(value);
        if self.tracks_constructor_stack() {
            self.push_value(StackValue::Other);
            if value.is_two_words() {
                self.push_value(StackValue::Other);
            }
        }
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.generator
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
        if self.tracks_constructor_stack() {
            for _ in 0..num_dimensions {
                self.pop_value();
            }
            self.push_value(StackValue::Other);
        }
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.generator.visit_type_insn(opcode, type_);
        // ANEWARRAY, CHECKCAST or INSTANCEOF don't change stack.
        if self.tracks_constructor_stack() && opcode == NEW {
            self.push_value(StackValue::Other);
        }
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.generator
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
        self.do_visit_method_insn(opcode, name, descriptor);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.generator.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        self.do_visit_method_insn(INVOKEDYNAMIC, name, descriptor);
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.generator.visit_jump_insn(opcode, label);
        if self.tracks_constructor_stack() {
            match opcode {
                IFEQ | IFNE | IFLT | IFGE | IFGT | IFLE | IFNULL | IFNONNULL => {
                    self.pop_value();
                }
                IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE
                | IF_ACMPEQ | IF_ACMPNE => {
                    self.pop_value();
                    self.pop_value();
                }
                JSR => self.push_value(StackValue::Other),
                GOTO => self.end_constructor_basic_block_without_successor(),
                _ => {}
            }
            self.add_forward_jump(label);
        }
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.generator.visit_lookup_switch_insn(dflt, keys, labels);
        if self.tracks_constructor_stack() {
            self.pop_value();
            self.add_forward_jumps(dflt, labels);
            self.end_constructor_basic_block_without_successor();
        }
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.generator
            .visit_table_switch_insn(min, max, dflt, labels);
        if self.tracks_constructor_stack() {
            self.pop_value();
            self.add_forward_jumps(dflt, labels);
            self.end_constructor_basic_block_without_successor();
        }
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.generator
            .visit_try_catch_block(start, end, handler, type_);
        // By definition of 'forwardJumpStackFrames', 'handler' should be pushed only if there is
        // an instruction between 'start' and 'end' at which the super class constructor is not
        // yet called. Unfortunately, try catch blocks must be visited before their labels, so we
        // have no way to know this at this point. Instead, we suppose that the super class
        // constructor has not been called at the start of *any* exception handler. If this is
        // wrong, normally there should not be a second super class constructor call in the
        // exception handler (an object can't be initialized twice), so this is not issue (in the
        // sense that there is no risk to emit a wrong 'onMethodEnter').
        if self.is_constructor {
            self.forward_jump_stack_frames
                .get_or_insert_with(FxHashMap::default)
                .entry(handler)
                .or_insert_with(|| vec![StackValue::Other]);
        }
    }

    // The methods of the LocalVariablesSorter, which AdviceAdapter does not override.

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.generator.visit_iinc_insn(var_index, increment);
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.generator.visit_maxs(max_stack, max_locals);
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
        self.generator
            .visit_local_variable(name, descriptor, signature, start, end, index);
    }

    fn visit_local_variable_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.generator.visit_local_variable_annotation(
            type_ref, type_path, start, end, index, descriptor, visible,
        )
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        self.generator.visit_frame(kind, local, stack);
    }
}
