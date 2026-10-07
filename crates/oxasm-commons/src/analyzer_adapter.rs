//! [`AnalyzerAdapter`]: ASM's `AnalyzerAdapter`.

// Java's narrowing casts are the specification here (ADR-17).
#![allow(clippy::cast_possible_truncation)]

use std::borrow::Cow;

use oxasm::opcodes::*;
use oxasm::{
    Constant, FrameKind, FrameValue, Handle, JavaStr, JavaString, Label, MethodVisitor, Type,
    TypeSort,
};
use rustc_hash::FxHashMap;

use crate::support::illegal_argument;

/// A [`MethodVisitor`] that keeps track of stack map frame changes between
/// [`visit_frame`](MethodVisitor::visit_frame) calls. This adapter must be used with the
/// `EXPAND_FRAMES` option. Each `visit_x` instruction delegates to the next visitor in the chain,
/// if any, and then simulates the effect of this instruction on the stack map frame, represented
/// by [`locals`](AnalyzerAdapter::locals) and [`stack`](AnalyzerAdapter::stack). The next visitor
/// in the chain can get the state of the stack map frame *before* each instruction by reading the
/// value of these fields in its `visit_x` methods (this requires a reference to the
/// `AnalyzerAdapter` that is before it in the chain). If this adapter is used with a class that
/// does not contain stack map table attributes (i.e., pre Java 6 classes) then this adapter may
/// not be able to compute the stack map frame for each instruction. In this case no exception is
/// thrown but the [`locals`](AnalyzerAdapter::locals) and [`stack`](AnalyzerAdapter::stack) fields
/// will be `None` for these instructions.
///
/// # Reading the frames
///
/// In Rust the next visitor in the chain cannot hold a reference to this adapter, which owns it.
/// The same state is read by a visitor *before* this adapter in the chain, which owns it: since
/// every `visit_x` method delegates before it simulates the instruction, the state an upstream
/// visitor reads just before calling `visit_x` is the one ASM's downstream visitor reads during
/// the call. The one difference is the label this adapter creates (and delegates) for a `NEW`
/// instruction that follows no label: ASM's downstream visitor gets it before the `NEW`
/// instruction, i.e. after it read the state.
///
/// # Panics
///
/// Its methods panic where ASM's throw (with the `toString()` of ASM's exception, ADR-06): on a
/// compressed frame, on `JSR`/`RET`, and on instructions that are invalid in the current frame
/// (popping from an empty stack, ...).
pub struct AnalyzerAdapter<'a> {
    /// The method visitor to which this adapter delegates calls, if any.
    mv: Option<Box<dyn MethodVisitor + 'a>>,

    /// The local variable slots for the current execution frame. Primitive types are represented
    /// by [`FrameValue::Top`], [`FrameValue::Integer`], [`FrameValue::Float`],
    /// [`FrameValue::Long`], [`FrameValue::Double`], [`FrameValue::Null`] or
    /// [`FrameValue::UninitializedThis`] (long and double are represented by two elements, the
    /// second one being `Top`). Reference types are represented by [`FrameValue::Object`]
    /// (representing internal names, see [`Type::get_internal_name`]), and uninitialized types by
    /// [`FrameValue::Uninitialized`] (this label designates the NEW instruction that created this
    /// uninitialized value). This field is `None` for unreachable instructions.
    pub locals: Option<Vec<FrameValue<'static>>>,

    /// The operand stack slots for the current execution frame, in the same format as
    /// [`locals`](AnalyzerAdapter::locals). This field is `None` for unreachable instructions.
    pub stack: Option<Vec<FrameValue<'static>>>,

    /// The labels that designate the next instruction to be visited. May be `None`.
    labels: Option<Vec<Label>>,

    /// The uninitialized types in the current execution frame. This map associates the labels of
    /// NEW instructions to internal names (see [`Type::get_internal_name`]). Each label designates
    /// a NEW instruction that created the currently uninitialized types, and the associated
    /// internal name represents the NEW operand, i.e. the final, initialized type value.
    pub uninitialized_types: FxHashMap<Label, JavaString>,

    /// The maximum stack size of this method.
    max_stack: usize,

    /// The maximum number of local variables of this method.
    max_locals: usize,

    /// The owner's class name.
    owner: JavaString,
}

impl std::fmt::Debug for AnalyzerAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyzerAdapter")
            .field("locals", &self.locals)
            .field("stack", &self.stack)
            .field("labels", &self.labels)
            .field("uninitialized_types", &self.uninitialized_types)
            .field("max_stack", &self.max_stack)
            .field("max_locals", &self.max_locals)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Panics as ASM's `AssertionError` (thrown on impossible types).
#[track_caller]
fn assertion_error() -> ! {
    panic!("java.lang.AssertionError")
}

/// An object frame value (a Java `String` in ASM's lists).
fn object(internal_name: &JavaStr) -> FrameValue<'static> {
    FrameValue::Object(Cow::Owned(internal_name.to_owned()))
}

/// Java's `s.substring(begin, end)`, in UTF-16 code units.
fn java_substring(s: &JavaStr, begin: usize, end: usize) -> JavaString {
    if s.as_bytes().is_ascii() {
        return JavaStr::from_semi_utf8(&s.as_bytes()[begin..end])
            .expect("ASCII")
            .to_owned();
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    JavaString::from_utf16(&units[begin..end])
}

/// Java's `s.length()`.
fn java_length(s: &JavaStr) -> usize {
    if s.as_bytes().is_ascii() {
        s.len()
    } else {
        s.encode_utf16().count()
    }
}

/// Java's `s.charAt(0)` (as an ASCII byte, or 0 for any other character, which ASM's code never
/// distinguishes).
fn first_char(s: &JavaStr) -> u8 {
    match s.as_bytes().first() {
        Some(&b) if b.is_ascii() => b,
        Some(_) => 0,
        None => {
            panic!("java.lang.StringIndexOutOfBoundsException: Index 0 out of bounds for length 0")
        }
    }
}

impl<'a> AnalyzerAdapter<'a> {
    /// Constructs a new `AnalyzerAdapter`.
    ///
    /// - `owner`: the owner's class name.
    /// - `access`: the method's access flags (see [`oxasm::opcodes`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `method_visitor`: the method visitor to which this adapter delegates calls. May be
    ///   `None`.
    pub fn new(
        owner: &JavaStr,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        method_visitor: Option<Box<dyn MethodVisitor + 'a>>,
    ) -> Self {
        let mut locals = Vec::new();
        if access & ACC_STATIC == 0 {
            if name == "<init>" {
                locals.push(FrameValue::UninitializedThis);
            } else {
                locals.push(object(owner));
            }
        }
        for argument_type in Type::get_argument_types_of(descriptor) {
            match argument_type.get_sort() {
                TypeSort::Boolean
                | TypeSort::Char
                | TypeSort::Byte
                | TypeSort::Short
                | TypeSort::Int => locals.push(FrameValue::Integer),
                TypeSort::Float => locals.push(FrameValue::Float),
                TypeSort::Long => {
                    locals.push(FrameValue::Long);
                    locals.push(FrameValue::Top);
                }
                TypeSort::Double => {
                    locals.push(FrameValue::Double);
                    locals.push(FrameValue::Top);
                }
                TypeSort::Array => locals.push(object(&argument_type.get_descriptor())),
                TypeSort::Object => locals.push(object(argument_type.get_internal_name())),
                _ => assertion_error(),
            }
        }
        let max_locals = locals.len();
        AnalyzerAdapter {
            mv: method_visitor,
            locals: Some(locals),
            stack: Some(Vec::new()),
            labels: None,
            uninitialized_types: FxHashMap::default(),
            max_stack: 0,
            max_locals,
            owner: owner.to_owned(),
        }
    }

    fn visit_frame_types(frame_types: &[FrameValue<'_>], result: &mut Vec<FrameValue<'static>>) {
        for frame_type in frame_types {
            result.push(frame_type.to_owned());
            if matches!(frame_type, FrameValue::Long | FrameValue::Double) {
                result.push(FrameValue::Top);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Frame simulation
    // -----------------------------------------------------------------------------------------

    fn locals_mut(&mut self) -> &mut Vec<FrameValue<'static>> {
        self.locals.as_mut().expect("reachable frame")
    }

    fn stack_mut(&mut self) -> &mut Vec<FrameValue<'static>> {
        self.stack.as_mut().expect("reachable frame")
    }

    fn get(&mut self, local: usize) -> FrameValue<'static> {
        self.max_locals = self.max_locals.max(local + 1);
        let locals = self.locals_mut();
        if local < locals.len() {
            locals[local].clone()
        } else {
            FrameValue::Top
        }
    }

    fn set(&mut self, local: usize, type_: FrameValue<'static>) {
        self.max_locals = self.max_locals.max(local + 1);
        let locals = self.locals_mut();
        while local >= locals.len() {
            locals.push(FrameValue::Top);
        }
        locals[local] = type_;
    }

    fn push(&mut self, type_: FrameValue<'static>) {
        let stack = self.stack_mut();
        stack.push(type_);
        let size = stack.len();
        self.max_stack = self.max_stack.max(size);
    }

    fn push_descriptor(&mut self, field_or_method_descriptor: &JavaStr) {
        let return_descriptor;
        let descriptor = if first_char(field_or_method_descriptor) == b'(' {
            return_descriptor = Type::get_return_type_of(field_or_method_descriptor)
                .get_descriptor()
                .into_owned();
            &*return_descriptor
        } else {
            field_or_method_descriptor
        };
        match first_char(descriptor) {
            b'V' => {}
            b'Z' | b'C' | b'B' | b'S' | b'I' => self.push(FrameValue::Integer),
            b'F' => self.push(FrameValue::Float),
            b'J' => {
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            b'D' => {
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            b'[' => self.push(object(descriptor)),
            b'L' => {
                let internal_name = java_substring(descriptor, 1, java_length(descriptor) - 1);
                self.push(FrameValue::Object(Cow::Owned(internal_name)));
            }
            _ => assertion_error(),
        }
    }

    fn pop(&mut self) -> FrameValue<'static> {
        self.stack_mut().pop().unwrap_or_else(|| {
            panic!("java.lang.IndexOutOfBoundsException: Index -1 out of bounds for length 0")
        })
    }

    fn pop_slots(&mut self, num_slots: usize) {
        let stack = self.stack_mut();
        if num_slots > stack.len() {
            panic!("java.lang.IndexOutOfBoundsException: Index -1 out of bounds for length 0");
        }
        stack.truncate(stack.len() - num_slots);
    }

    fn pop_descriptor(&mut self, descriptor: &JavaStr) {
        let first_descriptor_char = first_char(descriptor);
        if first_descriptor_char == b'(' {
            let num_slots = Type::get_argument_types_of(descriptor)
                .iter()
                .map(Type::get_size)
                .sum();
            self.pop_slots(num_slots);
        } else if first_descriptor_char == b'J' || first_descriptor_char == b'D' {
            self.pop_slots(2);
        } else {
            self.pop_slots(1);
        }
    }

    /// ASM's `istore`/`lstore` and friends: after a store in `local`, a long or double in the
    /// previous slot is no longer valid.
    fn invalidate_two_word_before(&mut self, local: usize) {
        if local > 0 {
            let value2 = self.get(local - 1);
            if matches!(value2, FrameValue::Long | FrameValue::Double) {
                self.set(local - 1, FrameValue::Top);
            }
        }
    }

    #[allow(clippy::too_many_lines)] // ASM's switch
    fn execute(&mut self, opcode: u8, int_arg: i32, string_arg: Option<&JavaStr>) {
        if opcode == JSR || opcode == RET {
            illegal_argument("JSR/RET are not supported");
        }
        if self.locals.is_none() {
            self.labels = None;
            return;
        }
        let string_arg = || string_arg.expect("a string argument");
        match opcode {
            NOP | INEG | LNEG | FNEG | DNEG | I2B | I2C | I2S | GOTO | RETURN => {}
            ACONST_NULL => self.push(FrameValue::Null),
            ICONST_M1 | ICONST_0 | ICONST_1 | ICONST_2 | ICONST_3 | ICONST_4 | ICONST_5
            | BIPUSH | SIPUSH => self.push(FrameValue::Integer),
            LCONST_0 | LCONST_1 => {
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            FCONST_0 | FCONST_1 | FCONST_2 => self.push(FrameValue::Float),
            DCONST_0 | DCONST_1 => {
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            ILOAD | FLOAD | ALOAD => {
                let value = self.get(int_arg as usize);
                self.push(value);
            }
            LLOAD | DLOAD => {
                let value = self.get(int_arg as usize);
                self.push(value);
                self.push(FrameValue::Top);
            }
            LALOAD | D2L => {
                self.pop_slots(2);
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            DALOAD | L2D => {
                self.pop_slots(2);
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            AALOAD => {
                self.pop_slots(1);
                let value1 = self.pop();
                match value1 {
                    FrameValue::Object(array_descriptor) => {
                        let length = java_length(&array_descriptor);
                        if length == 0 {
                            panic!(
                                "java.lang.StringIndexOutOfBoundsException: Range [1, 0) out of \
                                 bounds for length 0"
                            );
                        }
                        let element_descriptor = java_substring(&array_descriptor, 1, length);
                        self.push_descriptor(&element_descriptor);
                    }
                    FrameValue::Null => self.push(value1),
                    _ => self.push(object(JavaStr::from_str("java/lang/Object"))),
                }
            }
            ISTORE | FSTORE | ASTORE => {
                let value1 = self.pop();
                let local = int_arg as usize;
                self.set(local, value1);
                self.invalidate_two_word_before(local);
            }
            LSTORE | DSTORE => {
                self.pop_slots(1);
                let value1 = self.pop();
                let local = int_arg as usize;
                self.set(local, value1);
                self.set(local + 1, FrameValue::Top);
                self.invalidate_two_word_before(local);
            }
            IASTORE | BASTORE | CASTORE | SASTORE | FASTORE | AASTORE => self.pop_slots(3),
            LASTORE | DASTORE => self.pop_slots(4),
            POP | IFEQ | IFNE | IFLT | IFGE | IFGT | IFLE | IRETURN | FRETURN | ARETURN
            | TABLESWITCH | LOOKUPSWITCH | ATHROW | MONITORENTER | MONITOREXIT | IFNULL
            | IFNONNULL => self.pop_slots(1),
            POP2 | IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE
            | IF_ACMPEQ | IF_ACMPNE | LRETURN | DRETURN => self.pop_slots(2),
            DUP => {
                let value1 = self.pop();
                self.push(value1.clone());
                self.push(value1);
            }
            DUP_X1 => {
                let value1 = self.pop();
                let value2 = self.pop();
                self.push(value1.clone());
                self.push(value2);
                self.push(value1);
            }
            DUP_X2 => {
                let value1 = self.pop();
                let value2 = self.pop();
                let value3 = self.pop();
                self.push(value1.clone());
                self.push(value3);
                self.push(value2);
                self.push(value1);
            }
            DUP2 => {
                let value1 = self.pop();
                let value2 = self.pop();
                self.push(value2.clone());
                self.push(value1.clone());
                self.push(value2);
                self.push(value1);
            }
            DUP2_X1 => {
                let value1 = self.pop();
                let value2 = self.pop();
                let value3 = self.pop();
                self.push(value2.clone());
                self.push(value1.clone());
                self.push(value3);
                self.push(value2);
                self.push(value1);
            }
            DUP2_X2 => {
                let value1 = self.pop();
                let value2 = self.pop();
                let value3 = self.pop();
                let t4 = self.pop();
                self.push(value2.clone());
                self.push(value1.clone());
                self.push(t4);
                self.push(value3);
                self.push(value2);
                self.push(value1);
            }
            SWAP => {
                let value1 = self.pop();
                let value2 = self.pop();
                self.push(value1);
                self.push(value2);
            }
            IALOAD | BALOAD | CALOAD | SALOAD | IADD | ISUB | IMUL | IDIV | IREM | IAND | IOR
            | IXOR | ISHL | ISHR | IUSHR | L2I | D2I | FCMPL | FCMPG => {
                self.pop_slots(2);
                self.push(FrameValue::Integer);
            }
            LADD | LSUB | LMUL | LDIV | LREM | LAND | LOR | LXOR => {
                self.pop_slots(4);
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            FALOAD | FADD | FSUB | FMUL | FDIV | FREM | L2F | D2F => {
                self.pop_slots(2);
                self.push(FrameValue::Float);
            }
            DADD | DSUB | DMUL | DDIV | DREM => {
                self.pop_slots(4);
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            LSHL | LSHR | LUSHR => {
                self.pop_slots(3);
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            IINC => self.set(int_arg as usize, FrameValue::Integer),
            I2L | F2L => {
                self.pop_slots(1);
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            I2F => {
                self.pop_slots(1);
                self.push(FrameValue::Float);
            }
            I2D | F2D => {
                self.pop_slots(1);
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            F2I | ARRAYLENGTH | INSTANCEOF => {
                self.pop_slots(1);
                self.push(FrameValue::Integer);
            }
            LCMP | DCMPL | DCMPG => {
                self.pop_slots(4);
                self.push(FrameValue::Integer);
            }
            GETSTATIC => self.push_descriptor(string_arg()),
            PUTSTATIC => self.pop_descriptor(string_arg()),
            GETFIELD => {
                self.pop_slots(1);
                self.push_descriptor(string_arg());
            }
            PUTFIELD => {
                self.pop_descriptor(string_arg());
                self.pop();
            }
            NEW => {
                let label = self
                    .labels
                    .as_ref()
                    .expect("the labels of a NEW instruction")[0];
                self.push(FrameValue::Uninitialized(label));
            }
            NEWARRAY => {
                self.pop();
                let descriptor = match int_arg {
                    T_BOOLEAN => "[Z",
                    T_CHAR => "[C",
                    T_BYTE => "[B",
                    T_SHORT => "[S",
                    T_INT => "[I",
                    T_FLOAT => "[F",
                    T_DOUBLE => "[D",
                    T_LONG => "[J",
                    _ => illegal_argument(format_args!("Invalid array type {int_arg}")),
                };
                self.push_descriptor(JavaStr::from_str(descriptor));
            }
            ANEWARRAY => {
                self.pop();
                let mut descriptor = JavaString::from("[");
                descriptor
                    .push_java_str(&crate::support::object_type(string_arg()).get_descriptor());
                self.push_descriptor(&descriptor);
            }
            CHECKCAST => {
                self.pop();
                self.push_descriptor(&crate::support::object_type(string_arg()).get_descriptor());
            }
            MULTIANEWARRAY => {
                self.pop_slots(int_arg as usize);
                self.push_descriptor(string_arg());
            }
            _ => illegal_argument(format_args!("Invalid opcode {opcode}")),
        }
        self.labels = None;
    }
}

impl MethodVisitor for AnalyzerAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        match &mut self.mv {
            Some(mv) => Some(&mut **mv),
            None => None,
        }
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        if kind != FrameKind::New {
            // Uncompressed frame.
            illegal_argument(
                "AnalyzerAdapter only accepts expanded frames (see ClassReader.EXPAND_FRAMES)",
            );
        }

        if let Some(mv) = self.delegate() {
            mv.visit_frame(kind, locals, stack);
        }

        let this_locals = self.locals.get_or_insert_with(Vec::new);
        this_locals.clear();
        Self::visit_frame_types(locals, this_locals);
        let num_locals = this_locals.len();
        let this_stack = self.stack.get_or_insert_with(Vec::new);
        this_stack.clear();
        Self::visit_frame_types(stack, this_stack);
        let num_stack = this_stack.len();
        self.max_locals = self.max_locals.max(num_locals);
        self.max_stack = self.max_stack.max(num_stack);
    }

    fn visit_insn(&mut self, opcode: u8) {
        if let Some(mv) = self.delegate() {
            mv.visit_insn(opcode);
        }
        self.execute(opcode, 0, None);
        if (IRETURN..=RETURN).contains(&opcode) || opcode == ATHROW {
            self.locals = None;
            self.stack = None;
        }
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        if let Some(mv) = self.delegate() {
            mv.visit_int_insn(opcode, operand);
        }
        self.execute(opcode, operand, None);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        if let Some(mv) = self.delegate() {
            mv.visit_var_insn(opcode, var_index);
        }
        let is_long_or_double =
            opcode == LLOAD || opcode == DLOAD || opcode == LSTORE || opcode == DSTORE;
        self.max_locals = self
            .max_locals
            .max(usize::from(var_index) + if is_long_or_double { 2 } else { 1 });
        self.execute(opcode, i32::from(var_index), None);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        if opcode == NEW {
            if self.labels.is_none() {
                let label = Label::new();
                self.labels = Some(vec![label]);
                if let Some(mv) = self.delegate() {
                    mv.visit_label(label);
                }
            }
            for &label in self.labels.iter().flatten() {
                self.uninitialized_types.insert(label, type_.to_owned());
            }
        }
        if let Some(mv) = self.delegate() {
            mv.visit_type_insn(opcode, type_);
        }
        self.execute(opcode, 0, Some(type_));
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        if let Some(mv) = self.delegate() {
            mv.visit_field_insn(opcode, owner, name, descriptor);
        }
        self.execute(opcode, 0, Some(descriptor));
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        if let Some(mv) = self.delegate() {
            mv.visit_method_insn(opcode, owner, name, descriptor, is_interface);
        }

        if self.locals.is_none() {
            self.labels = None;
            return;
        }
        self.pop_descriptor(descriptor);
        if opcode != INVOKESTATIC {
            let value = self.pop();
            if opcode == INVOKESPECIAL && name == "<init>" {
                // ASM compares the values by identity: equality is the same for the values of
                // valid code (labels and UNINITIALIZED_THIS).
                let initialized_value = if value == FrameValue::UninitializedThis {
                    object(&self.owner)
                } else {
                    object(owner)
                };
                for slot in self.locals_mut() {
                    if *slot == value {
                        *slot = initialized_value.clone();
                    }
                }
                for slot in self.stack_mut() {
                    if *slot == value {
                        *slot = initialized_value.clone();
                    }
                }
            }
        }
        self.push_descriptor(descriptor);
        self.labels = None;
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        if let Some(mv) = self.delegate() {
            mv.visit_invoke_dynamic_insn(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            );
        }
        if self.locals.is_none() {
            self.labels = None;
            return;
        }
        self.pop_descriptor(descriptor);
        self.push_descriptor(descriptor);
        self.labels = None;
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        if let Some(mv) = self.delegate() {
            mv.visit_jump_insn(opcode, label);
        }
        self.execute(opcode, 0, None);
        if opcode == GOTO {
            self.locals = None;
            self.stack = None;
        }
    }

    fn visit_label(&mut self, label: Label) {
        if let Some(mv) = self.delegate() {
            mv.visit_label(label);
        }
        self.labels.get_or_insert_with(Vec::new).push(label);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        if let Some(mv) = self.delegate() {
            mv.visit_ldc_insn(value);
        }
        if self.locals.is_none() {
            self.labels = None;
            return;
        }
        match value {
            Constant::Integer(_) => self.push(FrameValue::Integer),
            Constant::Long(_) => {
                self.push(FrameValue::Long);
                self.push(FrameValue::Top);
            }
            Constant::Float(_) => self.push(FrameValue::Float),
            Constant::Double(_) => {
                self.push(FrameValue::Double);
                self.push(FrameValue::Top);
            }
            Constant::String(_) => self.push(object(JavaStr::from_str("java/lang/String"))),
            Constant::Type(type_) => match type_.get_sort() {
                TypeSort::Object | TypeSort::Array => {
                    self.push(object(JavaStr::from_str("java/lang/Class")));
                }
                TypeSort::Method => {
                    self.push(object(JavaStr::from_str("java/lang/invoke/MethodType")));
                }
                _ => panic!("java.lang.IllegalArgumentException"),
            },
            Constant::Handle(_) => {
                self.push(object(JavaStr::from_str("java/lang/invoke/MethodHandle")));
            }
            Constant::Dynamic(constant_dynamic) => {
                self.push_descriptor(constant_dynamic.get_descriptor());
            }
        }
        self.labels = None;
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        if let Some(mv) = self.delegate() {
            mv.visit_iinc_insn(var_index, increment);
        }
        self.max_locals = self.max_locals.max(usize::from(var_index) + 1);
        self.execute(IINC, i32::from(var_index), None);
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        if let Some(mv) = self.delegate() {
            mv.visit_table_switch_insn(min, max, dflt, labels);
        }
        self.execute(TABLESWITCH, 0, None);
        self.locals = None;
        self.stack = None;
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        if let Some(mv) = self.delegate() {
            mv.visit_lookup_switch_insn(dflt, keys, labels);
        }
        self.execute(LOOKUPSWITCH, 0, None);
        self.locals = None;
        self.stack = None;
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        if let Some(mv) = self.delegate() {
            mv.visit_multi_a_new_array_insn(descriptor, num_dimensions);
        }
        self.execute(MULTIANEWARRAY, i32::from(num_dimensions), Some(descriptor));
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
        let first_descriptor_char = first_char(descriptor);
        self.max_locals = self.max_locals.max(
            usize::from(index)
                + if first_descriptor_char == b'J' || first_descriptor_char == b'D' {
                    2
                } else {
                    1
                },
        );
        if let Some(mv) = self.delegate() {
            mv.visit_local_variable(name, descriptor, signature, start, end, index);
        }
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        if self.mv.is_some() {
            self.max_stack = self.max_stack.max(usize::from(max_stack));
            self.max_locals = self.max_locals.max(usize::from(max_locals));
            let (max_stack, max_locals) = (self.max_stack as u16, self.max_locals as u16);
            if let Some(mv) = self.delegate() {
                mv.visit_maxs(max_stack, max_locals);
            }
        }
    }
}
