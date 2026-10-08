//! [`GeneratorAdapter`] and [`TableSwitchGenerator`]: ASM's `GeneratorAdapter` and
//! `TableSwitchGenerator`, with ASM's `GeneratorAdapter` constants for [`GeneratorAdapter::math`]
//! and [`GeneratorAdapter::if_cmp`].

// Java's narrowing casts are the specification here (ADR-17): local variable indices are ints in
// ASM, written as 16 bit values.
#![allow(clippy::cast_possible_truncation)]

use std::borrow::Cow;

use oxasm::constant::{Constant, FrameValue};
use oxasm::opcodes::*;
use oxasm::{
    AnnotationVisitor, ClassVisitor, ConstantDynamic, FrameKind, Handle, JavaStr, JavaString,
    Label, MethodVisitor, Type, TypePath, TypeReference, TypeSort, jstr,
};

use crate::instruction_adapter;
use crate::local_variables_sorter::{LocalVariablesSorter, LocalVariablesSorterHooks};
use crate::method::Method;
use crate::support::{illegal_argument, illegal_state};

const CLASS_DESCRIPTOR: &str = "Ljava/lang/Class;";

const BYTE_TYPE: Type = Type::static_object_type(jstr!("java/lang/Byte"));

const BOOLEAN_TYPE: Type = Type::static_object_type(jstr!("java/lang/Boolean"));

const SHORT_TYPE: Type = Type::static_object_type(jstr!("java/lang/Short"));

const CHARACTER_TYPE: Type = Type::static_object_type(jstr!("java/lang/Character"));

const INTEGER_TYPE: Type = Type::static_object_type(jstr!("java/lang/Integer"));

const FLOAT_TYPE: Type = Type::static_object_type(jstr!("java/lang/Float"));

const LONG_TYPE: Type = Type::static_object_type(jstr!("java/lang/Long"));

const DOUBLE_TYPE: Type = Type::static_object_type(jstr!("java/lang/Double"));

const NUMBER_TYPE: Type = Type::static_object_type(jstr!("java/lang/Number"));

const OBJECT_TYPE: Type = Type::static_object_type(jstr!("java/lang/Object"));

/// Constant for the [`GeneratorAdapter::math`] method.
pub const ADD: u8 = IADD;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const SUB: u8 = ISUB;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const MUL: u8 = IMUL;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const DIV: u8 = IDIV;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const REM: u8 = IREM;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const NEG: u8 = INEG;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const SHL: u8 = ISHL;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const SHR: u8 = ISHR;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const USHR: u8 = IUSHR;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const AND: u8 = IAND;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const OR: u8 = IOR;

/// Constant for the [`GeneratorAdapter::math`] method.
pub const XOR: u8 = IXOR;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const EQ: u8 = IFEQ;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const NE: u8 = IFNE;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const LT: u8 = IFLT;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const GE: u8 = IFGE;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const GT: u8 = IFGT;

/// Constant for the [`GeneratorAdapter::if_cmp`] method.
pub const LE: u8 = IFLE;

/// A code generator for switch statements (ASM's `TableSwitchGenerator`), used by
/// [`GeneratorAdapter::table_switch`]. It gets the generator adapter that emits the switch, to
/// generate the code of each case.
///
/// A pair of closures `(case, default)` is a `TableSwitchGenerator`.
pub trait TableSwitchGenerator<M> {
    /// Generates the code for a switch case.
    ///
    /// - `key`: the switch case key.
    /// - `end`: a label that corresponds to the end of the switch statement.
    fn generate_case(&mut self, generator: &mut GeneratorAdapter<M>, key: i32, end: Label);

    /// Generates the code for the default switch case.
    fn generate_default(&mut self, generator: &mut GeneratorAdapter<M>);
}

impl<M, C, D> TableSwitchGenerator<M> for (C, D)
where
    C: FnMut(&mut GeneratorAdapter<M>, i32, Label),
    D: FnMut(&mut GeneratorAdapter<M>),
{
    fn generate_case(&mut self, generator: &mut GeneratorAdapter<M>, key: i32, end: Label) {
        (self.0)(generator, key, end);
    }

    fn generate_default(&mut self, generator: &mut GeneratorAdapter<M>) {
        (self.1)(generator);
    }
}

/// The hooks of the local variables sorter of a [`GeneratorAdapter`]: ASM's override of
/// `setLocalType`, which records the types of the local variables.
#[derive(Debug)]
struct LocalTypes {
    /// The index of the first local variable, after formal parameters.
    first_local: u32,
    /// The types of the local variables of the method visited by this adapter, from
    /// `first_local` on.
    local_types: Vec<Option<Type<'static>>>,
}

impl LocalVariablesSorterHooks for LocalTypes {
    fn set_local_type(&mut self, local: u32, type_: &Type<'_>) {
        let index = i64::from(local) - i64::from(self.first_local);
        let Ok(index) = usize::try_from(index) else {
            // ASM's List.set throws.
            panic!(
                "java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {}",
                self.local_types.len()
            );
        };
        if self.local_types.len() < index + 1 {
            self.local_types.resize(index + 1, None);
        }
        self.local_types[index] = Some(type_.clone().into_owned());
    }
}

/// A [`MethodVisitor`] with convenient methods to generate code. For example, using this adapter,
/// the class below
///
/// ```java
/// public class Example {
///   public static void main(String[] args) {
///     System.out.println("Hello world!");
///   }
/// }
/// ```
///
/// can be generated as follows:
///
/// ```
/// use oxasm::opcodes::*;
/// use oxasm::{ClassVisitor, ClassWriter, ClassWriterFlags, Type, jstr};
/// use oxasm_commons::{GeneratorAdapter, Method};
///
/// let mut cw = ClassWriter::new(ClassWriterFlags::empty());
/// cw.visit(V1_1, ACC_PUBLIC, jstr!("Example"), None, Some(jstr!("java/lang/Object")), &[]);
///
/// let m = Method::get_method("void <init> ()");
/// let mut mg = GeneratorAdapter::with_class_visitor(ACC_PUBLIC, &m, None, &[], &mut cw).unwrap();
/// mg.load_this();
/// mg.invoke_constructor(&Type::get_type("Ljava/lang/Object;"), &m);
/// mg.return_value();
/// mg.end_method();
/// drop(mg);
///
/// let m = Method::get_method("void main (String[])");
/// let mut mg =
///     GeneratorAdapter::with_class_visitor(ACC_PUBLIC + ACC_STATIC, &m, None, &[], &mut cw)
///         .unwrap();
/// mg.get_static(
///     &Type::get_type("Ljava/lang/System;"),
///     jstr!("out"),
///     &Type::get_type("Ljava/io/PrintStream;"),
/// );
/// mg.push_string(Some(jstr!("Hello world!")));
/// mg.invoke_virtual(
///     &Type::get_type("Ljava/io/PrintStream;"),
///     &Method::get_method("void println (String)"),
/// );
/// mg.return_value();
/// mg.end_method();
/// drop(mg);
///
/// cw.visit_end();
/// let bytes = cw.to_bytes().unwrap();
/// # assert!(!bytes.is_empty());
/// ```
///
/// Like ASM's, this adapter renumbers the local variables it *visits* (it contains a
/// [`LocalVariablesSorter`], ADR-13), while its code generation methods emit their
/// instructions directly to the downstream visitor `M`: the local variables they use are the
/// arguments, and the new locals returned by [`GeneratorAdapter::new_local`], which are already
/// remapped.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the methods).
pub struct GeneratorAdapter<M> {
    /// The local variables sorter, which holds the downstream visitor.
    sorter: LocalVariablesSorter<M, LocalTypes>,
    /// The access flags of the visited method.
    access: u32,
    /// The name of the visited method.
    name: JavaString,
    /// The return type of the visited method.
    return_type: Type<'static>,
    /// The argument types of the visited method.
    argument_types: Vec<Type<'static>>,
}

impl<M: MethodVisitor> GeneratorAdapter<M> {
    /// Constructs a new [`GeneratorAdapter`].
    ///
    /// - `method_visitor`: the method visitor to which this adapter delegates calls.
    /// - `access`: the method's access flags (see [`oxasm::opcodes`]).
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    #[track_caller]
    pub fn new(method_visitor: M, access: u32, name: &JavaStr, descriptor: &JavaStr) -> Self {
        let sorter = LocalVariablesSorter::with_hooks(
            access,
            descriptor,
            method_visitor,
            LocalTypes {
                first_local: 0,
                local_types: Vec::new(),
            },
        );
        // One copy of the descriptor, shared by the return and argument types.
        let method_type = Type::get_method_type(descriptor).into_owned();
        let mut generator = GeneratorAdapter {
            sorter,
            access,
            name: name.to_owned(),
            return_type: method_type.get_return_type(),
            argument_types: method_type.get_argument_types(),
        };
        generator.sorter.hooks.first_local = generator.sorter.first_local();
        generator
    }

    /// Constructs a new [`GeneratorAdapter`].
    ///
    /// - `access`: access flags of the adapted method.
    /// - `method`: the adapted method.
    /// - `method_visitor`: the method visitor to which this adapter delegates calls.
    #[track_caller]
    pub fn from_method(access: u32, method: &Method, method_visitor: M) -> Self {
        GeneratorAdapter::new(
            method_visitor,
            access,
            method.get_name(),
            method.get_descriptor(),
        )
    }

    /// The method visitor to which this adapter delegates calls (ASM's `mv`).
    pub fn mv(&mut self) -> &mut M {
        self.sorter.mv()
    }

    /// Returns the method visitor to which this adapter delegates calls.
    pub fn into_inner(self) -> M {
        self.sorter.into_inner()
    }

    /// Returns the access flags of the method visited by this adapter.
    #[must_use]
    pub fn get_access(&self) -> u32 {
        self.access
    }

    /// Returns the name of the method visited by this adapter.
    #[must_use]
    pub fn get_name(&self) -> &JavaStr {
        &self.name
    }

    /// Returns the return type of the method visited by this adapter.
    #[must_use]
    pub fn get_return_type(&self) -> &Type<'static> {
        &self.return_type
    }

    /// Returns the argument types of the method visited by this adapter.
    #[must_use]
    pub fn get_argument_types(&self) -> &[Type<'static>] {
        &self.argument_types
    }

    /// The index of the first local variable, after formal parameters.
    #[must_use]
    pub fn first_local(&self) -> u32 {
        self.sorter.first_local()
    }

    /// The index of the next local variable to be created by [`GeneratorAdapter::new_local`].
    #[must_use]
    pub fn next_local(&self) -> u32 {
        self.sorter.next_local()
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to push constants on the stack
    // ---------------------------------------------------------------------------------------------

    /// Generates the instruction to push the given value on the stack.
    pub fn push_bool(&mut self, value: bool) {
        self.push_int(i32::from(value));
    }

    /// Generates the instruction to push the given value on the stack.
    pub fn push_int(&mut self, value: i32) {
        let mv = self.mv();
        if (-1..=5).contains(&value) {
            mv.visit_insn((i32::from(ICONST_0) + value) as u8);
        } else if (i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(&value) {
            mv.visit_int_insn(BIPUSH, value);
        } else if (i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&value) {
            mv.visit_int_insn(SIPUSH, value);
        } else {
            mv.visit_ldc_insn(&Constant::Integer(value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    pub fn push_long(&mut self, value: i64) {
        let mv = self.mv();
        if value == 0 || value == 1 {
            mv.visit_insn(LCONST_0 + value as u8);
        } else {
            mv.visit_ldc_insn(&Constant::Long(value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    pub fn push_float(&mut self, value: f32) {
        let mv = self.mv();
        let bits = value.to_bits();
        if bits == 0 || bits == 0x3F80_0000 || bits == 0x4000_0000 {
            // 0..2
            mv.visit_insn(FCONST_0 + value as u8);
        } else {
            mv.visit_ldc_insn(&Constant::Float(value));
        }
    }

    /// Generates the instruction to push the given value on the stack.
    pub fn push_double(&mut self, value: f64) {
        let mv = self.mv();
        let bits = value.to_bits();
        if bits == 0 || bits == 0x3FF0_0000_0000_0000 {
            // +0.0d and 1.0d
            mv.visit_insn(DCONST_0 + value as u8);
        } else {
            mv.visit_ldc_insn(&Constant::Double(value));
        }
    }

    /// Generates the instruction to push the given value on the stack (`ACONST_NULL` for
    /// `None`).
    pub fn push_string(&mut self, value: Option<&JavaStr>) {
        let mv = self.mv();
        match value {
            None => mv.visit_insn(ACONST_NULL),
            Some(value) => mv.visit_ldc_insn(&Constant::String(Cow::Borrowed(value))),
        }
    }

    /// Generates the instruction to push the given value on the stack (`ACONST_NULL` for
    /// `None`).
    ///
    /// - `value`: the value to be pushed on the stack. A primitive type is pushed as its
    ///   `java.lang.Class` object (e.g. `Integer.TYPE`).
    pub fn push_type(&mut self, value: Option<&Type>) {
        let mv = self.mv();
        let Some(value) = value else {
            mv.visit_insn(ACONST_NULL);
            return;
        };
        let owner = match value.get_sort() {
            TypeSort::Void => "java/lang/Void",
            TypeSort::Boolean => "java/lang/Boolean",
            TypeSort::Char => "java/lang/Character",
            TypeSort::Byte => "java/lang/Byte",
            TypeSort::Short => "java/lang/Short",
            TypeSort::Int => "java/lang/Integer",
            TypeSort::Float => "java/lang/Float",
            TypeSort::Long => "java/lang/Long",
            TypeSort::Double => "java/lang/Double",
            _ => {
                mv.visit_ldc_insn(&Constant::Type(value.clone()));
                return;
            }
        };
        mv.visit_field_insn(
            GETSTATIC,
            jstr!(owner),
            jstr!("TYPE"),
            jstr!(CLASS_DESCRIPTOR),
        );
    }

    /// Generates the instruction to push a handle on the stack (`ACONST_NULL` for `None`).
    pub fn push_handle(&mut self, handle: Option<&Handle>) {
        let mv = self.mv();
        match handle {
            None => mv.visit_insn(ACONST_NULL),
            Some(handle) => mv.visit_ldc_insn(&Constant::Handle(handle.clone())),
        }
    }

    /// Generates the instruction to push a constant dynamic on the stack (`ACONST_NULL` for
    /// `None`).
    pub fn push_constant_dynamic(&mut self, constant_dynamic: Option<&ConstantDynamic>) {
        let mv = self.mv();
        match constant_dynamic {
            None => mv.visit_insn(ACONST_NULL),
            Some(constant_dynamic) => {
                mv.visit_ldc_insn(&Constant::Dynamic(Box::new(constant_dynamic.clone())));
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to load and store method arguments
    // ---------------------------------------------------------------------------------------------

    /// Returns the index of the given method argument in the frame's local variables array.
    fn get_arg_index(&self, arg: usize) -> u32 {
        let mut index = u32::from(self.access & ACC_STATIC == 0);
        for argument_type in &self.argument_types[..arg] {
            index += argument_type.get_size() as u32;
        }
        index
    }

    /// Generates the instruction to push a local variable on the stack.
    fn load_insn(&mut self, type_: &Type, index: u32) {
        self.mv()
            .visit_var_insn(type_.get_opcode(ILOAD), index as u16);
    }

    /// Generates the instruction to store the top stack value in a local variable.
    fn store_insn(&mut self, type_: &Type, index: u32) {
        self.mv()
            .visit_var_insn(type_.get_opcode(ISTORE), index as u16);
    }

    /// Generates the instruction to load 'this' on the stack.
    ///
    /// # Panics
    ///
    /// If the method is static (ASM's `IllegalStateException`).
    #[track_caller]
    pub fn load_this(&mut self) {
        if self.access & ACC_STATIC != 0 {
            illegal_state("no 'this' pointer within static method");
        }
        self.mv().visit_var_insn(ALOAD, 0);
    }

    /// Generates the instruction to load the given method argument on the stack.
    ///
    /// - `arg`: the index of a method argument.
    #[track_caller]
    pub fn load_arg(&mut self, arg: usize) {
        let argument_type = self.argument_types[arg].clone();
        let index = self.get_arg_index(arg);
        self.load_insn(&argument_type, index);
    }

    /// Generates the instructions to load the given method arguments on the stack.
    ///
    /// - `arg`: the index of the first method argument to be loaded.
    /// - `count`: the number of method arguments to be loaded.
    #[track_caller]
    pub fn load_args_range(&mut self, arg: usize, count: usize) {
        let mut index = self.get_arg_index(arg);
        for i in 0..count {
            let argument_type = self.argument_types[arg + i].clone();
            self.load_insn(&argument_type, index);
            index += argument_type.get_size() as u32;
        }
    }

    /// Generates the instructions to load all the method arguments on the stack.
    pub fn load_args(&mut self) {
        self.load_args_range(0, self.argument_types.len());
    }

    /// Generates the instructions to load all the method arguments on the stack, as a single
    /// object array.
    pub fn load_arg_array(&mut self) {
        let length = self.argument_types.len();
        self.push_int(length as i32);
        self.new_array(&OBJECT_TYPE);
        for i in 0..length {
            self.dup();
            self.push_int(i as i32);
            self.load_arg(i);
            let argument_type = self.argument_types[i].clone();
            self.box_(&argument_type);
            self.array_store(&OBJECT_TYPE);
        }
    }

    /// Generates the instruction to store the top stack value in the given method argument.
    ///
    /// - `arg`: the index of a method argument.
    #[track_caller]
    pub fn store_arg(&mut self, arg: usize) {
        let argument_type = self.argument_types[arg].clone();
        let index = self.get_arg_index(arg);
        self.store_insn(&argument_type, index);
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to load and store local variables
    // ---------------------------------------------------------------------------------------------

    /// Constructs a new local variable of the given type, and returns its identifier.
    ///
    /// # Panics
    ///
    /// If `type_` is `void` or a method type (ASM's `AssertionError`).
    #[track_caller]
    pub fn new_local(&mut self, type_: &Type) -> u16 {
        self.sorter.new_local(type_)
    }

    /// Returns the type of the given local variable, or `None` if it has none (ASM's `null`).
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    ///
    /// # Panics
    ///
    /// If `local` is not a local variable created by this adapter (ASM's
    /// `IndexOutOfBoundsException`).
    #[must_use]
    #[track_caller]
    pub fn get_local_type(&self, local: u16) -> Option<Type<'static>> {
        let local_types = &self.sorter.hooks.local_types;
        let index = i64::from(local) - i64::from(self.sorter.first_local());
        match usize::try_from(index)
            .ok()
            .and_then(|index| local_types.get(index))
        {
            Some(type_) => type_.clone(),
            None => panic!(
                "java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {}",
                local_types.len()
            ),
        }
    }

    /// The type of a local variable, which must have one (ASM throws a `NullPointerException`
    /// on `null`).
    #[track_caller]
    fn local_type(&self, local: u16) -> Type<'static> {
        self.get_local_type(local)
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// Generates the instruction to load the given local variable on the stack.
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    #[track_caller]
    pub fn load_local(&mut self, local: u16) {
        let type_ = self.local_type(local);
        self.load_insn(&type_, u32::from(local));
    }

    /// Generates the instruction to load the given local variable on the stack, and sets its
    /// type.
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    /// - `type_`: the type of this local variable.
    #[track_caller]
    pub fn load_local_with_type(&mut self, local: u16, type_: &Type) {
        self.sorter.hooks.set_local_type(u32::from(local), type_);
        self.load_insn(type_, u32::from(local));
    }

    /// Generates the instruction to store the top stack value in the given local variable.
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    #[track_caller]
    pub fn store_local(&mut self, local: u16) {
        let type_ = self.local_type(local);
        self.store_insn(&type_, u32::from(local));
    }

    /// Generates the instruction to store the top stack value in the given local variable, and
    /// sets its type.
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    /// - `type_`: the type of this local variable.
    #[track_caller]
    pub fn store_local_with_type(&mut self, local: u16, type_: &Type) {
        self.sorter.hooks.set_local_type(u32::from(local), type_);
        self.store_insn(type_, u32::from(local));
    }

    /// Generates the instruction to load an element from an array.
    ///
    /// - `type_`: the type of the array element to be loaded.
    #[track_caller]
    pub fn array_load(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IALOAD));
    }

    /// Generates the instruction to store an element in an array.
    ///
    /// - `type_`: the type of the array element to be stored.
    #[track_caller]
    pub fn array_store(&mut self, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(IASTORE));
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to manage the stack
    // ---------------------------------------------------------------------------------------------

    /// Generates a POP instruction.
    pub fn pop(&mut self) {
        self.mv().visit_insn(POP);
    }

    /// Generates a POP2 instruction.
    pub fn pop2(&mut self) {
        self.mv().visit_insn(POP2);
    }

    /// Generates a DUP instruction.
    pub fn dup(&mut self) {
        self.mv().visit_insn(DUP);
    }

    /// Generates a DUP2 instruction.
    pub fn dup2(&mut self) {
        self.mv().visit_insn(DUP2);
    }

    /// Generates a DUP_X1 instruction.
    pub fn dup_x1(&mut self) {
        self.mv().visit_insn(DUP_X1);
    }

    /// Generates a DUP_X2 instruction.
    pub fn dup_x2(&mut self) {
        self.mv().visit_insn(DUP_X2);
    }

    /// Generates a DUP2_X1 instruction.
    pub fn dup2_x1(&mut self) {
        self.mv().visit_insn(DUP2_X1);
    }

    /// Generates a DUP2_X2 instruction.
    pub fn dup2_x2(&mut self) {
        self.mv().visit_insn(DUP2_X2);
    }

    /// Generates a SWAP instruction.
    pub fn swap(&mut self) {
        self.mv().visit_insn(SWAP);
    }

    /// Generates the instructions to swap the top two stack values.
    ///
    /// - `prev`: type of the top - 1 stack value.
    /// - `type_`: type of the top stack value.
    pub fn swap_types(&mut self, prev: &Type, type_: &Type) {
        if type_.get_size() == 1 {
            if prev.get_size() == 1 {
                self.swap(); // Same as dupX1 pop.
            } else {
                self.dup_x2();
                self.pop();
            }
        } else if prev.get_size() == 1 {
            self.dup2_x1();
            self.pop2();
        } else {
            self.dup2_x2();
            self.pop2();
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to do mathematical and logical operations
    // ---------------------------------------------------------------------------------------------

    /// Generates the instruction to do the specified mathematical or logical operation.
    ///
    /// - `op`: a mathematical or logical operation. Must be one of [`ADD`], [`SUB`], [`MUL`],
    ///   [`DIV`], [`REM`], [`NEG`], [`SHL`], [`SHR`], [`USHR`], [`AND`], [`OR`], [`XOR`].
    /// - `type_`: the type of the operand(s) for this operation.
    #[track_caller]
    pub fn math(&mut self, op: u8, type_: &Type) {
        self.mv().visit_insn(type_.get_opcode(op));
    }

    /// Generates the instructions to compute the bitwise negation of the top stack value.
    pub fn not(&mut self) {
        let mv = self.mv();
        mv.visit_insn(ICONST_1);
        mv.visit_insn(IXOR);
    }

    /// Generates the instruction to increment the given local variable.
    ///
    /// - `local`: the local variable to be incremented.
    /// - `amount`: the amount by which the local variable must be incremented.
    pub fn iinc(&mut self, local: u16, amount: i16) {
        self.mv().visit_iinc_insn(local, amount);
    }

    /// Generates the instructions to cast a numerical value from one type to another.
    ///
    /// - `from`: the type of the top stack value.
    /// - `to`: the type in which this value must be cast.
    ///
    /// # Panics
    ///
    /// If one of the types is not a numerical primitive type (ASM's
    /// `IllegalArgumentException`).
    #[track_caller]
    pub fn cast(&mut self, from: &Type, to: &Type) {
        if from != to {
            // The sorts from BOOLEAN to DOUBLE.
            let is_numeric = |t: &Type| (1..=8).contains(&(t.get_sort() as i32));
            if !is_numeric(from) || !is_numeric(to) {
                illegal_argument(format_args!("Cannot cast from {from} to {to}"));
            }
            instruction_adapter::cast(self.mv(), from, to);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to do boxing and unboxing operations
    // ---------------------------------------------------------------------------------------------

    fn get_boxed_type<'a>(type_: &Type<'a>) -> Type<'a> {
        match type_.get_sort() {
            TypeSort::Byte => BYTE_TYPE,
            TypeSort::Boolean => BOOLEAN_TYPE,
            TypeSort::Short => SHORT_TYPE,
            TypeSort::Char => CHARACTER_TYPE,
            TypeSort::Int => INTEGER_TYPE,
            TypeSort::Float => FLOAT_TYPE,
            TypeSort::Long => LONG_TYPE,
            TypeSort::Double => DOUBLE_TYPE,
            _ => type_.clone(),
        }
    }

    /// Generates the instructions to box the top stack value. This value is replaced by its
    /// boxed equivalent on top of the stack (ASM's `box`).
    ///
    /// - `type_`: the type of the top stack value.
    pub fn box_(&mut self, type_: &Type) {
        if matches!(type_.get_sort(), TypeSort::Object | TypeSort::Array) {
            return;
        }
        if *type_ == Type::VOID {
            self.push_string(None);
        } else {
            let boxed_type = Self::get_boxed_type(type_);
            self.new_instance(&boxed_type);
            if type_.get_size() == 2 {
                // Pp -> Ppo -> oPpo -> ooPpo -> ooPp -> o
                self.dup_x2();
                self.dup_x2();
                self.pop();
            } else {
                // p -> po -> opo -> oop -> o
                self.dup_x1();
                self.swap();
            }
            self.invoke_constructor(
                &boxed_type,
                &Method::from_types("<init>", &Type::VOID, std::slice::from_ref(type_)),
            );
        }
    }

    /// Generates the instructions to box the top stack value using Java 5's valueOf() method.
    /// This value is replaced by its boxed equivalent on top of the stack.
    ///
    /// - `type_`: the type of the top stack value.
    pub fn value_of(&mut self, type_: &Type) {
        if matches!(type_.get_sort(), TypeSort::Object | TypeSort::Array) {
            return;
        }
        if *type_ == Type::VOID {
            self.push_string(None);
        } else {
            let boxed_type = Self::get_boxed_type(type_);
            self.invoke_static(
                &boxed_type,
                &Method::from_types("valueOf", &boxed_type, std::slice::from_ref(type_)),
            );
        }
    }

    /// Generates the instructions to unbox the top stack value. This value is replaced by its
    /// unboxed equivalent on top of the stack.
    ///
    /// - `type_`: the type of the top stack value.
    pub fn unbox(&mut self, type_: &Type) {
        let mut boxed_type = NUMBER_TYPE;
        let unbox_method = match type_.get_sort() {
            TypeSort::Void => return,
            TypeSort::Char => {
                boxed_type = CHARACTER_TYPE;
                Some("char charValue()")
            }
            TypeSort::Boolean => {
                boxed_type = BOOLEAN_TYPE;
                Some("boolean booleanValue()")
            }
            TypeSort::Double => Some("double doubleValue()"),
            TypeSort::Float => Some("float floatValue()"),
            TypeSort::Long => Some("long longValue()"),
            TypeSort::Int | TypeSort::Short | TypeSort::Byte => Some("int intValue()"),
            _ => None,
        };
        match unbox_method {
            None => self.check_cast(type_),
            Some(unbox_method) => {
                self.check_cast(&boxed_type);
                self.invoke_virtual(&boxed_type, &Method::get_method(unbox_method));
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to jump to other instructions
    // ---------------------------------------------------------------------------------------------

    /// Constructs a new [`Label`].
    #[must_use]
    pub fn new_label(&self) -> Label {
        Label::new()
    }

    /// Marks the current code position with the given label.
    pub fn mark(&mut self, label: Label) {
        self.mv().visit_label(label);
    }

    /// Marks the current code position with a new label, and returns it.
    pub fn mark_new(&mut self) -> Label {
        let label = Label::new();
        self.mv().visit_label(label);
        label
    }

    /// Generates the instructions to jump to a label based on the comparison of the top two
    /// stack values.
    ///
    /// - `type_`: the type of the top two stack values.
    /// - `mode`: how these values must be compared. One of [`EQ`], [`NE`], [`LT`], [`GE`],
    ///   [`GT`], [`LE`].
    /// - `label`: where to jump if the comparison result is `true`.
    ///
    /// # Panics
    ///
    /// On a comparison mode that does not apply to the type (ASM's `IllegalArgumentException`).
    #[track_caller]
    pub fn if_cmp(&mut self, type_: &Type, mode: u8, label: Label) {
        let mv = self.mv();
        match type_.get_sort() {
            TypeSort::Long => mv.visit_insn(LCMP),
            TypeSort::Double => {
                mv.visit_insn(if mode == GE || mode == GT {
                    DCMPL
                } else {
                    DCMPG
                });
            }
            TypeSort::Float => {
                mv.visit_insn(if mode == GE || mode == GT {
                    FCMPL
                } else {
                    FCMPG
                });
            }
            TypeSort::Array | TypeSort::Object => {
                if mode == EQ {
                    mv.visit_jump_insn(IF_ACMPEQ, label);
                    return;
                } else if mode == NE {
                    mv.visit_jump_insn(IF_ACMPNE, label);
                    return;
                }
                illegal_argument(format_args!("Bad comparison for type {type_}"));
            }
            _ => {
                let int_op = match mode {
                    EQ => IF_ICMPEQ,
                    NE => IF_ICMPNE,
                    GE => IF_ICMPGE,
                    LT => IF_ICMPLT,
                    LE => IF_ICMPLE,
                    GT => IF_ICMPGT,
                    _ => illegal_argument(format_args!("Bad comparison mode {mode}")),
                };
                mv.visit_jump_insn(int_op, label);
                return;
            }
        }
        mv.visit_jump_insn(mode, label);
    }

    /// Generates the instructions to jump to a label based on the comparison of the top two
    /// integer stack values.
    ///
    /// - `mode`: how these values must be compared. One of [`EQ`], [`NE`], [`LT`], [`GE`],
    ///   [`GT`], [`LE`].
    /// - `label`: where to jump if the comparison result is `true`.
    #[track_caller]
    pub fn if_i_cmp(&mut self, mode: u8, label: Label) {
        self.if_cmp(&Type::INT, mode, label);
    }

    /// Generates the instructions to jump to a label based on the comparison of the top integer
    /// stack value with zero.
    ///
    /// - `mode`: how these values must be compared. One of [`EQ`], [`NE`], [`LT`], [`GE`],
    ///   [`GT`], [`LE`].
    /// - `label`: where to jump if the comparison result is `true`.
    pub fn if_z_cmp(&mut self, mode: u8, label: Label) {
        self.mv().visit_jump_insn(mode, label);
    }

    /// Generates the instruction to jump to the given label if the top stack value is null.
    pub fn if_null(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFNULL, label);
    }

    /// Generates the instruction to jump to the given label if the top stack value is not null.
    pub fn if_non_null(&mut self, label: Label) {
        self.mv().visit_jump_insn(IFNONNULL, label);
    }

    /// Generates the instruction to jump to the given label.
    pub fn go_to(&mut self, label: Label) {
        self.mv().visit_jump_insn(GOTO, label);
    }

    /// Generates a RET instruction.
    ///
    /// - `local`: a local variable identifier, as returned by [`GeneratorAdapter::new_local`].
    pub fn ret(&mut self, local: u16) {
        self.mv().visit_var_insn(RET, local);
    }

    /// Generates the instructions for a switch statement, with a TABLESWITCH if the keys are
    /// dense enough, and a LOOKUPSWITCH otherwise.
    ///
    /// - `keys`: the switch case keys.
    /// - `generator`: a generator to generate the code for the switch cases.
    ///
    /// # Panics
    ///
    /// If the keys are not sorted in ascending order (ASM's `IllegalArgumentException`).
    #[track_caller]
    pub fn table_switch(&mut self, keys: &[i32], generator: &mut impl TableSwitchGenerator<M>) {
        let density = if keys.is_empty() {
            0.0
        } else {
            // Java's int arithmetic, then a float division.
            let range = keys[keys.len() - 1].wrapping_sub(keys[0]).wrapping_add(1);
            keys.len() as f32 / range as f32
        };
        self.table_switch_with(keys, generator, density >= 0.5);
    }

    /// Generates the instructions for a switch statement.
    ///
    /// - `keys`: the switch case keys.
    /// - `generator`: a generator to generate the code for the switch cases.
    /// - `use_table`: `true` to use a TABLESWITCH instruction, or `false` to use a LOOKUPSWITCH
    ///   instruction.
    ///
    /// # Panics
    ///
    /// If the keys are not sorted in ascending order (ASM's `IllegalArgumentException`).
    #[track_caller]
    pub fn table_switch_with(
        &mut self,
        keys: &[i32],
        generator: &mut impl TableSwitchGenerator<M>,
        use_table: bool,
    ) {
        for i in 1..keys.len() {
            if keys[i] < keys[i - 1] {
                illegal_argument("keys must be sorted in ascending order");
            }
        }
        let default_label = self.new_label();
        let end_label = self.new_label();
        if !keys.is_empty() {
            let num_keys = keys.len();
            if use_table {
                let min = keys[0];
                let max = keys[num_keys - 1];
                let range = max.wrapping_sub(min).wrapping_add(1);
                let Ok(range) = usize::try_from(range) else {
                    panic!("java.lang.NegativeArraySizeException: {range}");
                };
                let mut labels = vec![default_label; range];
                for &key in keys {
                    labels[key.wrapping_sub(min).cast_unsigned() as usize] = self.new_label();
                }
                self.mv()
                    .visit_table_switch_insn(min, max, default_label, &labels);
                for (i, &label) in labels.iter().enumerate() {
                    if label != default_label {
                        self.mark(label);
                        generator.generate_case(self, (i as i32).wrapping_add(min), end_label);
                    }
                }
            } else {
                let labels: Vec<Label> = (0..num_keys).map(|_| self.new_label()).collect();
                self.mv()
                    .visit_lookup_switch_insn(default_label, keys, &labels);
                for (&key, &label) in keys.iter().zip(&labels) {
                    self.mark(label);
                    generator.generate_case(self, key, end_label);
                }
            }
        }
        self.mark(default_label);
        generator.generate_default(self);
        self.mark(end_label);
    }

    /// Generates the instruction to return the top stack value to the caller.
    #[track_caller]
    pub fn return_value(&mut self) {
        let opcode = self.return_type.get_opcode(IRETURN);
        self.mv().visit_insn(opcode);
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to load and store fields
    // ---------------------------------------------------------------------------------------------

    /// Generates a get field or set field instruction.
    fn field_insn(&mut self, opcode: u8, owner_type: &Type, name: &JavaStr, field_type: &Type) {
        self.mv().visit_field_insn(
            opcode,
            owner_type.get_internal_name(),
            name,
            &field_type.get_descriptor(),
        );
    }

    /// Generates the instruction to push the value of a static field on the stack.
    ///
    /// - `owner`: the class in which the field is defined.
    /// - `name`: the name of the field.
    /// - `type_`: the type of the field.
    pub fn get_static(&mut self, owner: &Type, name: &JavaStr, type_: &Type) {
        self.field_insn(GETSTATIC, owner, name, type_);
    }

    /// Generates the instruction to store the top stack value in a static field.
    ///
    /// - `owner`: the class in which the field is defined.
    /// - `name`: the name of the field.
    /// - `type_`: the type of the field.
    pub fn put_static(&mut self, owner: &Type, name: &JavaStr, type_: &Type) {
        self.field_insn(PUTSTATIC, owner, name, type_);
    }

    /// Generates the instruction to push the value of a non static field on the stack.
    ///
    /// - `owner`: the class in which the field is defined.
    /// - `name`: the name of the field.
    /// - `type_`: the type of the field.
    pub fn get_field(&mut self, owner: &Type, name: &JavaStr, type_: &Type) {
        self.field_insn(GETFIELD, owner, name, type_);
    }

    /// Generates the instruction to store the top stack value in a non static field.
    ///
    /// - `owner`: the class in which the field is defined.
    /// - `name`: the name of the field.
    /// - `type_`: the type of the field.
    pub fn put_field(&mut self, owner: &Type, name: &JavaStr, type_: &Type) {
        self.field_insn(PUTFIELD, owner, name, type_);
    }

    // ---------------------------------------------------------------------------------------------
    // Instructions to invoke methods
    // ---------------------------------------------------------------------------------------------

    /// Generates an invoke method instruction.
    fn invoke_insn(&mut self, opcode: u8, type_: &Type, method: &Method, is_interface: bool) {
        let owner = if type_.get_sort() == TypeSort::Array {
            type_.get_descriptor()
        } else {
            Cow::Borrowed(type_.get_internal_name())
        };
        self.mv().visit_method_insn(
            opcode,
            &owner,
            method.get_name(),
            method.get_descriptor(),
            is_interface,
        );
    }

    /// Generates the instruction to invoke a normal method.
    ///
    /// - `owner`: the class in which the method is defined.
    /// - `method`: the method to be invoked.
    pub fn invoke_virtual(&mut self, owner: &Type, method: &Method) {
        self.invoke_insn(INVOKEVIRTUAL, owner, method, false);
    }

    /// Generates the instruction to invoke a constructor.
    ///
    /// - `type_`: the class in which the constructor is defined.
    /// - `method`: the constructor to be invoked.
    pub fn invoke_constructor(&mut self, type_: &Type, method: &Method) {
        self.invoke_insn(INVOKESPECIAL, type_, method, false);
    }

    /// Generates the instruction to invoke a static method.
    ///
    /// - `owner`: the class in which the method is defined.
    /// - `method`: the method to be invoked.
    pub fn invoke_static(&mut self, owner: &Type, method: &Method) {
        self.invoke_insn(INVOKESTATIC, owner, method, false);
    }

    /// Generates the instruction to invoke an interface method.
    ///
    /// - `owner`: the class in which the method is defined.
    /// - `method`: the method to be invoked.
    pub fn invoke_interface(&mut self, owner: &Type, method: &Method) {
        self.invoke_insn(INVOKEINTERFACE, owner, method, true);
    }

    /// Generates an invokedynamic instruction.
    ///
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `bootstrap_method_handle`: the bootstrap method.
    /// - `bootstrap_method_arguments`: the bootstrap method constant arguments.
    pub fn invoke_dynamic(
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

    // ---------------------------------------------------------------------------------------------
    // Instructions to create objects and arrays
    // ---------------------------------------------------------------------------------------------

    /// Generates a type dependent instruction.
    fn type_insn(&mut self, opcode: u8, type_: &Type) {
        self.mv().visit_type_insn(opcode, type_.get_internal_name());
    }

    /// Generates the instruction to create a new object.
    ///
    /// - `type_`: the class of the object to be created.
    pub fn new_instance(&mut self, type_: &Type) {
        self.type_insn(NEW, type_);
    }

    /// Generates the instruction to create a new array.
    ///
    /// - `type_`: the type of the array elements.
    pub fn new_array(&mut self, type_: &Type) {
        instruction_adapter::newarray(self.mv(), type_);
    }

    // ---------------------------------------------------------------------------------------------
    // Miscellaneous instructions
    // ---------------------------------------------------------------------------------------------

    /// Generates the instruction to compute the length of an array.
    pub fn array_length(&mut self) {
        self.mv().visit_insn(ARRAYLENGTH);
    }

    /// Generates the instruction to throw an exception.
    pub fn throw_exception(&mut self) {
        self.mv().visit_insn(ATHROW);
    }

    /// Generates the instructions to create and throw an exception. The exception class must
    /// have a constructor with a single String argument.
    ///
    /// - `type_`: the class of the exception to be thrown.
    /// - `message`: the detailed message of the exception.
    pub fn throw_new_exception(&mut self, type_: &Type, message: Option<&JavaStr>) {
        self.new_instance(type_);
        self.dup();
        self.push_string(message);
        self.invoke_constructor(type_, &Method::get_method("void <init> (String)"));
        self.throw_exception();
    }

    /// Generates the instruction to check that the top stack value is of the given type.
    ///
    /// - `type_`: a class or interface type.
    pub fn check_cast(&mut self, type_: &Type) {
        if *type_ != OBJECT_TYPE {
            self.type_insn(CHECKCAST, type_);
        }
    }

    /// Generates the instruction to test if the top stack value is of the given type.
    ///
    /// - `type_`: a class or interface type.
    pub fn instance_of(&mut self, type_: &Type) {
        self.type_insn(INSTANCEOF, type_);
    }

    /// Generates the instruction to get the monitor of the top stack value.
    pub fn monitor_enter(&mut self) {
        self.mv().visit_insn(MONITORENTER);
    }

    /// Generates the instruction to release the monitor of the top stack value.
    pub fn monitor_exit(&mut self) {
        self.mv().visit_insn(MONITOREXIT);
    }

    // ---------------------------------------------------------------------------------------------
    // Non instructions
    // ---------------------------------------------------------------------------------------------

    /// Marks the end of the visited method.
    pub fn end_method(&mut self) {
        let access = self.access;
        let mv = self.mv();
        if access & ACC_ABSTRACT == 0 {
            mv.visit_maxs(0, 0);
        }
        mv.visit_end();
    }

    /// Marks the start of an exception handler.
    ///
    /// - `start`: beginning of the exception handler's scope (inclusive).
    /// - `end`: end of the exception handler's scope (exclusive).
    /// - `exception`: internal name of the type of exceptions handled by the handler (`None` to
    ///   catch any exceptions).
    pub fn catch_exception(&mut self, start: Label, end: Label, exception: Option<&Type>) {
        let catch_label = Label::new();
        self.mv().visit_try_catch_block(
            start,
            end,
            catch_label,
            exception.map(Type::get_internal_name),
        );
        self.mark(catch_label);
    }
}

impl<'a> GeneratorAdapter<Box<dyn MethodVisitor + 'a>> {
    /// Constructs a new [`GeneratorAdapter`] for a new method of a class: calls
    /// `class_visitor.visit_method`, and returns an adapter of the method visitor it returns, or
    /// `None` if it returns none (where ASM's adapter would throw a `NullPointerException` on
    /// its first instruction).
    ///
    /// - `access`: access flags of the adapted method.
    /// - `method`: the adapted method.
    /// - `signature`: the signature of the adapted method (may be `None`).
    /// - `exceptions`: the exceptions thrown by the adapted method.
    /// - `class_visitor`: the class visitor to which this adapter's method belongs.
    #[track_caller]
    pub fn with_class_visitor<C: ClassVisitor + ?Sized>(
        access: u32,
        method: &Method,
        signature: Option<&JavaStr>,
        exceptions: &[Type],
        class_visitor: &'a mut C,
    ) -> Option<Self> {
        let exceptions: Vec<&JavaStr> = exceptions.iter().map(Type::get_internal_name).collect();
        let method_visitor = class_visitor.visit_method(
            access,
            method.get_name(),
            method.get_descriptor(),
            signature,
            &exceptions,
        )?;
        Some(GeneratorAdapter::from_method(
            access,
            method,
            method_visitor,
        ))
    }
}

impl<M> std::fmt::Debug for GeneratorAdapter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratorAdapter")
            .field("access", &self.access)
            .field("name", &self.name)
            .field("return_type", &self.return_type)
            .field("argument_types", &self.argument_types)
            .finish_non_exhaustive()
    }
}

/// The visit methods are those of the [`LocalVariablesSorter`]: the local variables of the
/// visited code are renumbered.
impl<M: MethodVisitor> MethodVisitor for GeneratorAdapter<M> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(self.sorter.mv())
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.sorter.visit_var_insn(opcode, var_index);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.sorter.visit_iinc_insn(var_index, increment);
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.sorter.visit_maxs(max_stack, max_locals);
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
        self.sorter
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
        self.sorter.visit_local_variable_annotation(
            type_ref, type_path, start, end, index, descriptor, visible,
        )
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        self.sorter.visit_frame(kind, local, stack);
    }
}
