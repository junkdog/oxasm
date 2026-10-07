//! [`InstructionAdapter`]: ASM's `InstructionAdapter`, a [`MethodVisitor`] providing a more
//! detailed API to generate and transform instructions.
//!
//! ASM's class is two things at once (ADR-13):
//!
//! - **An emitter**: typed methods such as `iconst(5)`, `load(1, Type::INT)` or `add(Type::LONG)`
//!   that generate the matching instruction on the downstream visitor (`mv`). Here they are the
//!   default methods of the [`InstructionVisitor`] trait, which only asks for the downstream
//!   visitor ([`InstructionVisitor::mv`]).
//! - **A reverse dispatcher**: its `visitXxx` methods decode each visited instruction and call
//!   the matching typed method (`visitInsn(IADD)` calls `add(Type.INT_TYPE)`), which subclasses
//!   override to transform the instructions of a certain kind. Here this is the
//!   [`MethodVisitor`] implementation of [`InstructionAdapter`] (which overrides nothing, like a
//!   plain ASM `InstructionAdapter`) and of [`InstructionDispatcher`], which dispatches to any
//!   [`InstructionVisitor`]: ASM's subclass of `InstructionAdapter` is a type implementing
//!   [`InstructionVisitor`], wrapped in an [`InstructionDispatcher`].
//!
//! In both cases, every other event (frames, try catch blocks, annotations, ...) is forwarded
//! to the downstream visitor unchanged.
//!
//! ```
//! use oxasm::{JavaStr, MethodVisitor, Type};
//! use oxasm_commons::{InstructionDispatcher, InstructionVisitor};
//!
//! /// Replaces every `int` addition with a subtraction.
//! struct AddToSub<M>(M);
//!
//! impl<M: MethodVisitor> InstructionVisitor for AddToSub<M> {
//!     fn mv(&mut self) -> &mut dyn MethodVisitor {
//!         &mut self.0
//!     }
//!
//!     fn add(&mut self, type_: &Type) {
//!         if *type_ == Type::INT {
//!             self.sub(type_);
//!         } else {
//!             self.mv().visit_insn(type_.get_opcode(oxasm::opcodes::IADD));
//!         }
//!     }
//! }
//!
//! # fn adapt<'a>(mv: Box<dyn MethodVisitor + 'a>) -> Box<dyn MethodVisitor + 'a> {
//! Box::new(InstructionDispatcher::new(AddToSub(mv)))
//! # }
//! ```

#![allow(clippy::cast_possible_truncation)] // ADR-17: Java's narrowing casts

use oxasm::opcodes::*;
use oxasm::{
    Constant, ConstantDynamic, Handle, JavaStr, Label, MethodVisitor, Type, TypeSort, jstr,
};

/// The type of the java.lang.Object class (ASM's `InstructionAdapter.OBJECT_TYPE`).
pub const OBJECT_TYPE: Type = Type::static_object_type(jstr!("java/lang/Object"));

/// Panics as ASM throws an `IllegalArgumentException` without a message.
#[track_caller]
fn illegal_argument() -> ! {
    panic!("java.lang.IllegalArgumentException")
}

// -------------------------------------------------------------------------------------------------
// The emitter
// -------------------------------------------------------------------------------------------------

/// The typed instruction methods of ASM's `InstructionAdapter`: each one generates the matching
/// instruction(s) on [`mv`](InstructionVisitor::mv) by default.
///
/// Override some of them, and wrap the implementation in an [`InstructionDispatcher`], to
/// transform the instructions of a method (ASM's subclasses of `InstructionAdapter`); or call
/// them on an [`InstructionAdapter`] to generate code.
pub trait InstructionVisitor {
    /// The method visitor to which the generated instructions are sent (ASM's `mv`).
    fn mv(&mut self) -> &mut dyn MethodVisitor;

    /// Generates a nop instruction.
    fn nop(&mut self) {
        self.mv().visit_insn(NOP);
    }

    /// Generates the instruction to push the given value on the stack: `ACONST_NULL` for
    /// `None`, otherwise an `LDC` of the constant.
    ///
    /// - `value`: the constant to be pushed on the stack. This parameter must be an `Integer`,
    ///   a `Float`, a `Long`, a `Double`, a `String`, a [`Type`] of OBJECT or ARRAY sort for
    ///   `.class` constants, for classes whose version is 49, a [`Type`] of METHOD sort for
    ///   MethodType, a [`Handle`] for MethodHandle constants, for classes whose version is 51 or
    ///   a [`ConstantDynamic`] for a constant dynamic for classes whose version is 55.
    fn aconst(&mut self, value: Option<&Constant<'_>>) {
        match value {
            None => self.mv().visit_insn(ACONST_NULL),
            Some(value) => self.mv().visit_ldc_insn(value),
        }
    }

    /// Generates the instruction to push the given value on the stack.
    fn iconst(&mut self, int_value: i32) {
        if (-1..=5).contains(&int_value) {
            self.mv()
                .visit_insn((i32::from(ICONST_0) + int_value) as u8);
        } else if i32::from(i8::MIN) <= int_value && int_value <= i32::from(i8::MAX) {
            self.mv().visit_int_insn(BIPUSH, int_value);
        } else if i32::from(i16::MIN) <= int_value && int_value <= i32::from(i16::MAX) {
            self.mv().visit_int_insn(SIPUSH, int_value);
        } else {
            self.mv().visit_ldc_insn(&Constant::Integer(int_value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    fn lconst(&mut self, long_value: i64) {
        if long_value == 0 || long_value == 1 {
            self.mv().visit_insn(LCONST_0 + long_value as u8);
        } else {
            self.mv().visit_ldc_insn(&Constant::Long(long_value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    fn fconst(&mut self, float_value: f32) {
        let bits = float_value.to_bits();
        if bits == 0 || bits == 0x3F80_0000 || bits == 0x4000_0000 {
            // 0..2
            self.mv().visit_insn(FCONST_0 + float_value as u8);
        } else {
            self.mv().visit_ldc_insn(&Constant::Float(float_value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    fn dconst(&mut self, double_value: f64) {
        let bits = double_value.to_bits();
        if bits == 0 || bits == 0x3FF0_0000_0000_0000 {
            // +0.0d and 1.0d
            self.mv().visit_insn(DCONST_0 + double_value as u8);
        } else {
            self.mv().visit_ldc_insn(&Constant::Double(double_value));
        }
    }

    /// Generates the instruction to push the given type on the stack.
    fn tconst(&mut self, type_: &Type) {
        self.mv().visit_ldc_insn(&Constant::Type(type_.clone()));
    }

    /// Generates the instruction to push the given handle on the stack.
    fn hconst(&mut self, handle: &Handle) {
        self.mv().visit_ldc_insn(&Constant::Handle(handle.clone()));
    }

    /// Generates the instruction to push the given constant dynamic on the stack.
    fn cconst(&mut self, constant_dynamic: &ConstantDynamic) {
        self.mv()
            .visit_ldc_insn(&Constant::Dynamic(Box::new(constant_dynamic.clone())));
    }

    /// Generates the instruction to load the given local variable, of the given type.
    fn load(&mut self, var_index: u16, type_: &Type) {
        self.mv().visit_var_insn(type_.get_opcode(ILOAD), var_index);
    }

    /// Generates the instruction to load an element of the given type from an array.
    fn aload(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IALOAD));
    }

    /// Generates the instruction to store the given local variable, of the given type.
    fn store(&mut self, var_index: u16, type_: &Type) {
        self.mv()
            .visit_var_insn(type_.get_opcode(ISTORE), var_index);
    }

    /// Generates the instruction to store an element of the given type in an array.
    fn astore(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IASTORE));
    }

    /// Generates a POP instruction.
    fn pop(&mut self) {
        self.mv().visit_insn(POP);
    }

    /// Generates a POP2 instruction.
    fn pop2(&mut self) {
        self.mv().visit_insn(POP2);
    }

    /// Generates a DUP instruction.
    fn dup(&mut self) {
        self.mv().visit_insn(DUP);
    }

    /// Generates a DUP2 instruction.
    fn dup2(&mut self) {
        self.mv().visit_insn(DUP2);
    }

    /// Generates a DUP_X1 instruction.
    fn dup_x1(&mut self) {
        self.mv().visit_insn(DUP_X1);
    }

    /// Generates a DUP_X2 instruction.
    fn dup_x2(&mut self) {
        self.mv().visit_insn(DUP_X2);
    }

    /// Generates a DUP2_X1 instruction.
    fn dup2_x1(&mut self) {
        self.mv().visit_insn(DUP2_X1);
    }

    /// Generates a DUP2_X2 instruction.
    fn dup2_x2(&mut self) {
        self.mv().visit_insn(DUP2_X2);
    }

    /// Generates a SWAP instruction.
    fn swap(&mut self) {
        self.mv().visit_insn(SWAP);
    }

    /// Generates the addition instruction for the given type.
    fn add(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IADD));
    }

    /// Generates the subtraction instruction for the given type.
    fn sub(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(ISUB));
    }

    /// Generates the multiplication instruction for the given type.
    fn mul(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IMUL));
    }

    /// Generates the division instruction for the given type.
    fn div(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IDIV));
    }

    /// Generates the remainder instruction for the given type.
    fn rem(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IREM));
    }

    /// Generates the negation instruction for the given type.
    fn neg(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(INEG));
    }

    /// Generates the shift left instruction for the given type.
    fn shl(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(ISHL));
    }

    /// Generates the arithmetic shift right instruction for the given type.
    fn shr(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(ISHR));
    }

    /// Generates the logical shift right instruction for the given type.
    fn ushr(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IUSHR));
    }

    /// Generates the bitwise and instruction for the given type.
    fn and(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IAND));
    }

    /// Generates the bitwise or instruction for the given type.
    fn or(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IOR));
    }

    /// Generates the bitwise xor instruction for the given type.
    fn xor(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IXOR));
    }

    /// Generates an IINC instruction.
    fn iinc(&mut self, var_index: u16, increment: i16) {
        self.mv().visit_iinc_insn(var_index, increment);
    }

    /// Generates the instruction to cast from the first given type to the other.
    fn cast(&mut self, from: &Type, to: &Type) {
        cast(self.mv(), from, to);
    }

    /// Generates an LCMP instruction.
    fn lcmp(&mut self) {
        self.mv().visit_insn(LCMP);
    }

    /// Generates an FCMPL instruction if the given type is `float`, a DCMPL instruction
    /// otherwise.
    fn cmpl(&mut self, type_: &Type) {
        self.mv()
            .visit_insn(if *type_ == Type::FLOAT { FCMPL } else { DCMPL });
    }

    /// Generates an FCMPG instruction if the given type is `float`, a DCMPG instruction
    /// otherwise.
    fn cmpg(&mut self, type_: &Type) {
        self.mv()
            .visit_insn(if *type_ == Type::FLOAT { FCMPG } else { DCMPG });
    }

    /// Generates an IFEQ instruction.
    fn ifeq(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFEQ, label);
    }

    /// Generates an IFNE instruction.
    fn ifne(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFNE, label);
    }

    /// Generates an IFLT instruction.
    fn iflt(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFLT, label);
    }

    /// Generates an IFGE instruction.
    fn ifge(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFGE, label);
    }

    /// Generates an IFGT instruction.
    fn ifgt(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFGT, label);
    }

    /// Generates an IFLE instruction.
    fn ifle(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFLE, label);
    }

    /// Generates an IF_ICMPEQ instruction.
    fn ificmpeq(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPEQ, label);
    }

    /// Generates an IF_ICMPNE instruction.
    fn ificmpne(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPNE, label);
    }

    /// Generates an IF_ICMPLT instruction.
    fn ificmplt(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPLT, label);
    }

    /// Generates an IF_ICMPGE instruction.
    fn ificmpge(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPGE, label);
    }

    /// Generates an IF_ICMPGT instruction.
    fn ificmpgt(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPGT, label);
    }

    /// Generates an IF_ICMPLE instruction.
    fn ificmple(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ICMPLE, label);
    }

    /// Generates an IF_ACMPEQ instruction.
    fn ifacmpeq(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ACMPEQ, label);
    }

    /// Generates an IF_ACMPNE instruction.
    fn ifacmpne(&mut self, label: Label) {
        self.mv().visit_jump_insn(IF_ACMPNE, label);
    }

    /// Generates a GOTO instruction (ASM's `goTo`).
    fn go_to(&mut self, label: Label) {
        self.mv().visit_jump_insn(GOTO, label);
    }

    /// Generates a JSR instruction.
    fn jsr(&mut self, label: Label) {
        self.mv().visit_jump_insn(JSR, label);
    }

    /// Generates a RET instruction.
    fn ret(&mut self, var_index: u16) {
        self.mv().visit_var_insn(RET, var_index);
    }

    /// Generates a TABLESWITCH instruction.
    fn tableswitch(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.mv().visit_table_switch_insn(min, max, dflt, labels);
    }

    /// Generates a LOOKUPSWITCH instruction.
    fn lookupswitch(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.mv().visit_lookup_switch_insn(dflt, keys, labels);
    }

    /// Generates the return instruction for the given type (`RETURN` for `void`).
    fn areturn(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IRETURN));
    }

    /// Generates a GETSTATIC instruction.
    fn getstatic(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) {
        self.mv()
            .visit_field_insn(GETSTATIC, owner, name, descriptor);
    }

    /// Generates a PUTSTATIC instruction.
    fn putstatic(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) {
        self.mv()
            .visit_field_insn(PUTSTATIC, owner, name, descriptor);
    }

    /// Generates a GETFIELD instruction.
    fn getfield(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) {
        self.mv()
            .visit_field_insn(GETFIELD, owner, name, descriptor);
    }

    /// Generates a PUTFIELD instruction.
    fn putfield(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) {
        self.mv()
            .visit_field_insn(PUTFIELD, owner, name, descriptor);
    }

    /// Generates the instruction to call the given virtual method.
    ///
    /// - `owner`: the internal name of the method's owner class (see
    ///   [`Type::get_internal_name`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `is_interface`: if the method's owner class is an interface.
    fn invokevirtual(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.mv()
            .visit_method_insn(INVOKEVIRTUAL, owner, name, descriptor, is_interface);
    }

    /// Generates the instruction to call the given special method.
    ///
    /// - `owner`: the internal name of the method's owner class (see
    ///   [`Type::get_internal_name`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `is_interface`: if the method's owner class is an interface.
    fn invokespecial(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.mv()
            .visit_method_insn(INVOKESPECIAL, owner, name, descriptor, is_interface);
    }

    /// Generates the instruction to call the given static method.
    ///
    /// - `owner`: the internal name of the method's owner class (see
    ///   [`Type::get_internal_name`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `is_interface`: if the method's owner class is an interface.
    fn invokestatic(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.mv()
            .visit_method_insn(INVOKESTATIC, owner, name, descriptor, is_interface);
    }

    /// Generates the instruction to call the given interface method.
    ///
    /// - `owner`: the internal name of the method's owner class (see
    ///   [`Type::get_internal_name`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    fn invokeinterface(&mut self, owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr) {
        self.mv()
            .visit_method_insn(INVOKEINTERFACE, owner, name, descriptor, true);
    }

    /// Generates the instruction to call the given dynamic method.
    ///
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `bootstrap_method_handle`: the bootstrap method.
    /// - `bootstrap_method_arguments`: the bootstrap method constant arguments.
    fn invokedynamic(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.mv().visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    /// Generates a NEW instruction.
    fn anew(&mut self, type_: &Type) {
        self.mv().visit_type_insn(NEW, type_.get_internal_name());
    }

    /// Generates the instruction to create and push on the stack an array of the given type.
    ///
    /// - `type_`: the element type of the array (`NEWARRAY` for a primitive type, `ANEWARRAY`
    ///   otherwise).
    fn newarray(&mut self, type_: &Type) {
        newarray(self.mv(), type_);
    }

    /// Generates an ARRAYLENGTH instruction.
    fn arraylength(&mut self) {
        self.mv().visit_insn(ARRAYLENGTH);
    }

    /// Generates an ATHROW instruction.
    fn athrow(&mut self) {
        self.mv().visit_insn(ATHROW);
    }

    /// Generates a CHECKCAST instruction.
    fn checkcast(&mut self, type_: &Type) {
        self.mv()
            .visit_type_insn(CHECKCAST, type_.get_internal_name());
    }

    /// Generates an INSTANCEOF instruction (ASM's `instanceOf`).
    fn instance_of(&mut self, type_: &Type) {
        self.mv()
            .visit_type_insn(INSTANCEOF, type_.get_internal_name());
    }

    /// Generates a MONITORENTER instruction.
    fn monitorenter(&mut self) {
        self.mv().visit_insn(MONITORENTER);
    }

    /// Generates a MONITOREXIT instruction.
    fn monitorexit(&mut self) {
        self.mv().visit_insn(MONITOREXIT);
    }

    /// Generates a MULTIANEWARRAY instruction.
    fn multianewarray(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.mv()
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
    }

    /// Generates an IFNULL instruction.
    fn ifnull(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFNULL, label);
    }

    /// Generates an IFNONNULL instruction.
    fn ifnonnull(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFNONNULL, label);
    }

    /// Visits the given label.
    fn mark(&mut self, label: Label) {
        self.mv().visit_label(label);
    }
}

/// Generates the instruction to cast from the first given type to the other (ASM's static
/// `InstructionAdapter.cast`).
pub(crate) fn cast(method_visitor: &mut dyn MethodVisitor, from: &Type, to: &Type) {
    if from != to {
        if *from == Type::DOUBLE {
            if *to == Type::FLOAT {
                method_visitor.visit_insn(D2F);
            } else if *to == Type::LONG {
                method_visitor.visit_insn(D2L);
            } else {
                method_visitor.visit_insn(D2I);
                cast(method_visitor, &Type::INT, to);
            }
        } else if *from == Type::FLOAT {
            if *to == Type::DOUBLE {
                method_visitor.visit_insn(F2D);
            } else if *to == Type::LONG {
                method_visitor.visit_insn(F2L);
            } else {
                method_visitor.visit_insn(F2I);
                cast(method_visitor, &Type::INT, to);
            }
        } else if *from == Type::LONG {
            if *to == Type::DOUBLE {
                method_visitor.visit_insn(L2D);
            } else if *to == Type::FLOAT {
                method_visitor.visit_insn(L2F);
            } else {
                method_visitor.visit_insn(L2I);
                cast(method_visitor, &Type::INT, to);
            }
        } else if *to == Type::BYTE {
            method_visitor.visit_insn(I2B);
        } else if *to == Type::CHAR {
            method_visitor.visit_insn(I2C);
        } else if *to == Type::DOUBLE {
            method_visitor.visit_insn(I2D);
        } else if *to == Type::FLOAT {
            method_visitor.visit_insn(I2F);
        } else if *to == Type::LONG {
            method_visitor.visit_insn(I2L);
        } else if *to == Type::SHORT {
            method_visitor.visit_insn(I2S);
        }
    }
}

/// Generates the instruction to create and push on the stack an array of the given element type
/// (ASM's static `InstructionAdapter.newarray`).
pub(crate) fn newarray(method_visitor: &mut dyn MethodVisitor, type_: &Type) {
    let array_type = match type_.get_sort() {
        TypeSort::Boolean => T_BOOLEAN,
        TypeSort::Char => T_CHAR,
        TypeSort::Byte => T_BYTE,
        TypeSort::Short => T_SHORT,
        TypeSort::Int => T_INT,
        TypeSort::Float => T_FLOAT,
        TypeSort::Long => T_LONG,
        TypeSort::Double => T_DOUBLE,
        _ => {
            method_visitor.visit_type_insn(ANEWARRAY, type_.get_internal_name());
            return;
        }
    };
    method_visitor.visit_int_insn(NEWARRAY, array_type);
}

// -------------------------------------------------------------------------------------------------
// The reverse dispatch
// -------------------------------------------------------------------------------------------------

/// ASM's `InstructionAdapter.visitInsn`.
fn dispatch_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, opcode: u8) {
    match opcode {
        NOP => iv.nop(),
        ACONST_NULL => iv.aconst(None),
        ICONST_M1..=ICONST_5 => iv.iconst(i32::from(opcode) - i32::from(ICONST_0)),
        LCONST_0 | LCONST_1 => iv.lconst(i64::from(opcode - LCONST_0)),
        FCONST_0..=FCONST_2 => iv.fconst(f32::from(opcode - FCONST_0)),
        DCONST_0 | DCONST_1 => iv.dconst(f64::from(opcode - DCONST_0)),
        IALOAD => iv.aload(&Type::INT),
        LALOAD => iv.aload(&Type::LONG),
        FALOAD => iv.aload(&Type::FLOAT),
        DALOAD => iv.aload(&Type::DOUBLE),
        AALOAD => iv.aload(&OBJECT_TYPE),
        BALOAD => iv.aload(&Type::BYTE),
        CALOAD => iv.aload(&Type::CHAR),
        SALOAD => iv.aload(&Type::SHORT),
        IASTORE => iv.astore(&Type::INT),
        LASTORE => iv.astore(&Type::LONG),
        FASTORE => iv.astore(&Type::FLOAT),
        DASTORE => iv.astore(&Type::DOUBLE),
        AASTORE => iv.astore(&OBJECT_TYPE),
        BASTORE => iv.astore(&Type::BYTE),
        CASTORE => iv.astore(&Type::CHAR),
        SASTORE => iv.astore(&Type::SHORT),
        POP => iv.pop(),
        POP2 => iv.pop2(),
        DUP => iv.dup(),
        DUP_X1 => iv.dup_x1(),
        DUP_X2 => iv.dup_x2(),
        DUP2 => iv.dup2(),
        DUP2_X1 => iv.dup2_x1(),
        DUP2_X2 => iv.dup2_x2(),
        SWAP => iv.swap(),
        IADD => iv.add(&Type::INT),
        LADD => iv.add(&Type::LONG),
        FADD => iv.add(&Type::FLOAT),
        DADD => iv.add(&Type::DOUBLE),
        ISUB => iv.sub(&Type::INT),
        LSUB => iv.sub(&Type::LONG),
        FSUB => iv.sub(&Type::FLOAT),
        DSUB => iv.sub(&Type::DOUBLE),
        IMUL => iv.mul(&Type::INT),
        LMUL => iv.mul(&Type::LONG),
        FMUL => iv.mul(&Type::FLOAT),
        DMUL => iv.mul(&Type::DOUBLE),
        IDIV => iv.div(&Type::INT),
        LDIV => iv.div(&Type::LONG),
        FDIV => iv.div(&Type::FLOAT),
        DDIV => iv.div(&Type::DOUBLE),
        IREM => iv.rem(&Type::INT),
        LREM => iv.rem(&Type::LONG),
        FREM => iv.rem(&Type::FLOAT),
        DREM => iv.rem(&Type::DOUBLE),
        INEG => iv.neg(&Type::INT),
        LNEG => iv.neg(&Type::LONG),
        FNEG => iv.neg(&Type::FLOAT),
        DNEG => iv.neg(&Type::DOUBLE),
        ISHL => iv.shl(&Type::INT),
        LSHL => iv.shl(&Type::LONG),
        ISHR => iv.shr(&Type::INT),
        LSHR => iv.shr(&Type::LONG),
        IUSHR => iv.ushr(&Type::INT),
        LUSHR => iv.ushr(&Type::LONG),
        IAND => iv.and(&Type::INT),
        LAND => iv.and(&Type::LONG),
        IOR => iv.or(&Type::INT),
        LOR => iv.or(&Type::LONG),
        IXOR => iv.xor(&Type::INT),
        LXOR => iv.xor(&Type::LONG),
        I2L => iv.cast(&Type::INT, &Type::LONG),
        I2F => iv.cast(&Type::INT, &Type::FLOAT),
        I2D => iv.cast(&Type::INT, &Type::DOUBLE),
        L2I => iv.cast(&Type::LONG, &Type::INT),
        L2F => iv.cast(&Type::LONG, &Type::FLOAT),
        L2D => iv.cast(&Type::LONG, &Type::DOUBLE),
        F2I => iv.cast(&Type::FLOAT, &Type::INT),
        F2L => iv.cast(&Type::FLOAT, &Type::LONG),
        F2D => iv.cast(&Type::FLOAT, &Type::DOUBLE),
        D2I => iv.cast(&Type::DOUBLE, &Type::INT),
        D2L => iv.cast(&Type::DOUBLE, &Type::LONG),
        D2F => iv.cast(&Type::DOUBLE, &Type::FLOAT),
        I2B => iv.cast(&Type::INT, &Type::BYTE),
        I2C => iv.cast(&Type::INT, &Type::CHAR),
        I2S => iv.cast(&Type::INT, &Type::SHORT),
        LCMP => iv.lcmp(),
        FCMPL => iv.cmpl(&Type::FLOAT),
        FCMPG => iv.cmpg(&Type::FLOAT),
        DCMPL => iv.cmpl(&Type::DOUBLE),
        DCMPG => iv.cmpg(&Type::DOUBLE),
        IRETURN => iv.areturn(&Type::INT),
        LRETURN => iv.areturn(&Type::LONG),
        FRETURN => iv.areturn(&Type::FLOAT),
        DRETURN => iv.areturn(&Type::DOUBLE),
        ARETURN => iv.areturn(&OBJECT_TYPE),
        RETURN => iv.areturn(&Type::VOID),
        ARRAYLENGTH => iv.arraylength(),
        ATHROW => iv.athrow(),
        MONITORENTER => iv.monitorenter(),
        MONITOREXIT => iv.monitorexit(),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitIntInsn`.
fn dispatch_int_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, opcode: u8, operand: i32) {
    match opcode {
        BIPUSH | SIPUSH => iv.iconst(operand),
        NEWARRAY => match operand {
            T_BOOLEAN => iv.newarray(&Type::BOOLEAN),
            T_CHAR => iv.newarray(&Type::CHAR),
            T_BYTE => iv.newarray(&Type::BYTE),
            T_SHORT => iv.newarray(&Type::SHORT),
            T_INT => iv.newarray(&Type::INT),
            T_FLOAT => iv.newarray(&Type::FLOAT),
            T_LONG => iv.newarray(&Type::LONG),
            T_DOUBLE => iv.newarray(&Type::DOUBLE),
            _ => illegal_argument(),
        },
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitVarInsn`.
fn dispatch_var_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, opcode: u8, var_index: u16) {
    match opcode {
        ILOAD => iv.load(var_index, &Type::INT),
        LLOAD => iv.load(var_index, &Type::LONG),
        FLOAD => iv.load(var_index, &Type::FLOAT),
        DLOAD => iv.load(var_index, &Type::DOUBLE),
        ALOAD => iv.load(var_index, &OBJECT_TYPE),
        ISTORE => iv.store(var_index, &Type::INT),
        LSTORE => iv.store(var_index, &Type::LONG),
        FSTORE => iv.store(var_index, &Type::FLOAT),
        DSTORE => iv.store(var_index, &Type::DOUBLE),
        ASTORE => iv.store(var_index, &OBJECT_TYPE),
        RET => iv.ret(var_index),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitTypeInsn`.
fn dispatch_type_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, opcode: u8, type_: &JavaStr) {
    let object_type = crate::support::object_type(type_);
    match opcode {
        NEW => iv.anew(&object_type),
        ANEWARRAY => iv.newarray(&object_type),
        CHECKCAST => iv.checkcast(&object_type),
        INSTANCEOF => iv.instance_of(&object_type),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitFieldInsn`.
fn dispatch_field_insn<V: InstructionVisitor + ?Sized>(
    iv: &mut V,
    opcode: u8,
    owner: &JavaStr,
    name: &JavaStr,
    descriptor: &JavaStr,
) {
    match opcode {
        GETSTATIC => iv.getstatic(owner, name, descriptor),
        PUTSTATIC => iv.putstatic(owner, name, descriptor),
        GETFIELD => iv.getfield(owner, name, descriptor),
        PUTFIELD => iv.putfield(owner, name, descriptor),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitMethodInsn`.
fn dispatch_method_insn<V: InstructionVisitor + ?Sized>(
    iv: &mut V,
    opcode: u8,
    owner: &JavaStr,
    name: &JavaStr,
    descriptor: &JavaStr,
    is_interface: bool,
) {
    match opcode {
        INVOKESPECIAL => iv.invokespecial(owner, name, descriptor, is_interface),
        INVOKEVIRTUAL => iv.invokevirtual(owner, name, descriptor, is_interface),
        INVOKESTATIC => iv.invokestatic(owner, name, descriptor, is_interface),
        INVOKEINTERFACE => iv.invokeinterface(owner, name, descriptor),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitJumpInsn`.
fn dispatch_jump_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, opcode: u8, label: Label) {
    match opcode {
        IFEQ => iv.ifeq(label),
        IFNE => iv.ifne(label),
        IFLT => iv.iflt(label),
        IFGE => iv.ifge(label),
        IFGT => iv.ifgt(label),
        IFLE => iv.ifle(label),
        IF_ICMPEQ => iv.ificmpeq(label),
        IF_ICMPNE => iv.ificmpne(label),
        IF_ICMPLT => iv.ificmplt(label),
        IF_ICMPGE => iv.ificmpge(label),
        IF_ICMPGT => iv.ificmpgt(label),
        IF_ICMPLE => iv.ificmple(label),
        IF_ACMPEQ => iv.ifacmpeq(label),
        IF_ACMPNE => iv.ifacmpne(label),
        GOTO => iv.go_to(label),
        JSR => iv.jsr(label),
        IFNULL => iv.ifnull(label),
        IFNONNULL => iv.ifnonnull(label),
        _ => illegal_argument(),
    }
}

/// ASM's `InstructionAdapter.visitLdcInsn`. Java's boxed `Byte`, `Character`, `Short` and
/// `Boolean` constants, which ASM turns into `iconst`s, are `Integer` constants here (ADR-05).
fn dispatch_ldc_insn<V: InstructionVisitor + ?Sized>(iv: &mut V, value: &Constant<'_>) {
    match value {
        Constant::Integer(value) => iv.iconst(*value),
        Constant::Float(value) => iv.fconst(*value),
        Constant::Long(value) => iv.lconst(*value),
        Constant::Double(value) => iv.dconst(*value),
        Constant::String(_) => iv.aconst(Some(value)),
        Constant::Type(type_) => iv.tconst(type_),
        Constant::Handle(handle) => iv.hconst(handle),
        Constant::Dynamic(constant_dynamic) => iv.cconst(constant_dynamic),
    }
}

/// Implements [`MethodVisitor`] for a type implementing [`InstructionVisitor`]: the instruction
/// events are dispatched to the typed methods, and the others are forwarded to `mv()`.
macro_rules! dispatching_method_visitor {
    () => {
        fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
            Some(self.instruction_visitor().mv())
        }

        fn visit_insn(&mut self, opcode: u8) {
            dispatch_insn(self.instruction_visitor(), opcode);
        }

        fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
            dispatch_int_insn(self.instruction_visitor(), opcode, operand);
        }

        fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
            dispatch_var_insn(self.instruction_visitor(), opcode, var_index);
        }

        fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
            dispatch_type_insn(self.instruction_visitor(), opcode, type_);
        }

        fn visit_field_insn(
            &mut self,
            opcode: u8,
            owner: &JavaStr,
            name: &JavaStr,
            descriptor: &JavaStr,
        ) {
            dispatch_field_insn(self.instruction_visitor(), opcode, owner, name, descriptor);
        }

        fn visit_method_insn(
            &mut self,
            opcode: u8,
            owner: &JavaStr,
            name: &JavaStr,
            descriptor: &JavaStr,
            is_interface: bool,
        ) {
            dispatch_method_insn(
                self.instruction_visitor(),
                opcode,
                owner,
                name,
                descriptor,
                is_interface,
            );
        }

        fn visit_invoke_dynamic_insn(
            &mut self,
            name: &JavaStr,
            descriptor: &JavaStr,
            bootstrap_method_handle: &Handle,
            bootstrap_method_arguments: &[Constant<'_>],
        ) {
            self.instruction_visitor().invokedynamic(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            );
        }

        fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
            dispatch_jump_insn(self.instruction_visitor(), opcode, label);
        }

        fn visit_label(&mut self, label: Label) {
            self.instruction_visitor().mark(label);
        }

        fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
            dispatch_ldc_insn(self.instruction_visitor(), value);
        }

        fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
            self.instruction_visitor().iinc(var_index, increment);
        }

        fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
            self.instruction_visitor()
                .tableswitch(min, max, dflt, labels);
        }

        fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
            self.instruction_visitor().lookupswitch(dflt, keys, labels);
        }

        fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
            self.instruction_visitor()
                .multianewarray(descriptor, num_dimensions);
        }
    };
}

// -------------------------------------------------------------------------------------------------
// InstructionAdapter
// -------------------------------------------------------------------------------------------------

/// A [`MethodVisitor`] providing a more detailed API to generate and transform instructions
/// (ASM's `InstructionAdapter`, used as is).
///
/// As an [`InstructionVisitor`], it generates instructions on its downstream visitor `M`. As a
/// [`MethodVisitor`], it decodes each visited instruction into the matching typed method, which
/// regenerates it: the result is the same instruction, or an equivalent one (an `LDC` of a small
/// `int` becomes an `ICONST_n`, `BIPUSH` or `SIPUSH`, as in ASM). To override some of the typed
/// methods (ASM's subclasses), implement [`InstructionVisitor`] and wrap it in an
/// [`InstructionDispatcher`].
///
/// # Panics
///
/// Its visit methods panic where ASM's throw an `IllegalArgumentException`: on an opcode that
/// is not one of the visit method's opcodes, or a `NEWARRAY` operand that is not an array type.
#[derive(Debug)]
pub struct InstructionAdapter<M> {
    mv: M,
}

impl<M: MethodVisitor> InstructionAdapter<M> {
    /// Constructs a new adapter that generates and forwards its events to `method_visitor`.
    pub fn new(method_visitor: M) -> Self {
        InstructionAdapter { mv: method_visitor }
    }

    /// The method visitor to which this adapter delegates calls.
    pub fn get_ref(&self) -> &M {
        &self.mv
    }

    /// The method visitor to which this adapter delegates calls.
    pub fn get_mut(&mut self) -> &mut M {
        &mut self.mv
    }

    /// Returns the method visitor to which this adapter delegates calls.
    pub fn into_inner(self) -> M {
        self.mv
    }

    fn instruction_visitor(&mut self) -> &mut Self {
        self
    }
}

impl<M: MethodVisitor> InstructionVisitor for InstructionAdapter<M> {
    fn mv(&mut self) -> &mut dyn MethodVisitor {
        &mut self.mv
    }
}

impl<M: MethodVisitor> MethodVisitor for InstructionAdapter<M> {
    dispatching_method_visitor!();
}

// -------------------------------------------------------------------------------------------------
// InstructionDispatcher
// -------------------------------------------------------------------------------------------------

/// A [`MethodVisitor`] that decodes each visited instruction and calls the matching typed
/// method of an [`InstructionVisitor`] (`visit_insn(IADD)` calls `add(&Type::INT)`), and
/// forwards the other events to its [`mv`](InstructionVisitor::mv): the visit methods of ASM's
/// `InstructionAdapter`, for its subclasses, which override some of the typed methods.
///
/// # Panics
///
/// As [`InstructionAdapter`]'s visit methods.
#[derive(Debug)]
pub struct InstructionDispatcher<V> {
    instruction_visitor: V,
}

impl<V: InstructionVisitor> InstructionDispatcher<V> {
    /// Constructs a new dispatcher to `instruction_visitor`.
    pub fn new(instruction_visitor: V) -> Self {
        InstructionDispatcher {
            instruction_visitor,
        }
    }

    /// The instruction visitor to which this dispatcher dispatches.
    pub fn get_ref(&self) -> &V {
        &self.instruction_visitor
    }

    /// The instruction visitor to which this dispatcher dispatches.
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.instruction_visitor
    }

    /// Returns the instruction visitor to which this dispatcher dispatches.
    pub fn into_inner(self) -> V {
        self.instruction_visitor
    }

    fn instruction_visitor(&mut self) -> &mut V {
        &mut self.instruction_visitor
    }
}

impl<V: InstructionVisitor> MethodVisitor for InstructionDispatcher<V> {
    dispatching_method_visitor!();
}
