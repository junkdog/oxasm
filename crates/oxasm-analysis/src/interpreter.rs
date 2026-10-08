//! [`Interpreter`]: ASM's `Interpreter`, and [`InsnRef`], the instruction it interprets.

use std::ops::Deref;

use oxasm::{Interner, JavaStr, Sym, Type};
use oxasm_tree::{InsnId, InsnList, InsnNode, TryCatchBlockNode};

use crate::error::AnalyzerError;
use crate::frame::Frame;
use crate::value::Value;

/// An instruction of a method being analyzed: what ASM's interpreters get as an
/// `AbstractInsnNode`, which knows its list and its identity.
///
/// It dereferences to the [`InsnNode`]. Its [`id`](Self::id) is the identity of the instruction
/// (what a `SourceValue` records), [`list`](Self::list) its instruction list (for ASM's
/// `getNext()`, `getPrevious()`, ...), and [`interner`](Self::interner) that of its method's
/// strings, which [`resolve`](Self::resolve) uses.
#[derive(Clone, Copy, Debug)]
pub struct InsnRef<'a> {
    /// The interner of the method's strings.
    pub interner: &'a Interner,
    /// The instruction list of the method.
    pub list: &'a InsnList,
    /// The handle of the instruction in [`list`](Self::list).
    pub id: InsnId,
    /// The index of the instruction in [`list`](Self::list).
    pub index: usize,
    /// The instruction.
    pub node: &'a InsnNode,
}

impl<'a> InsnRef<'a> {
    /// The instruction designated by `id` in `list`, or `None` if it is not an instruction of
    /// this list.
    pub fn new(interner: &'a Interner, list: &'a InsnList, id: InsnId) -> Option<InsnRef<'a>> {
        Some(InsnRef {
            interner,
            list,
            id,
            index: list.index_of(id)?,
            node: list.node(id)?,
        })
    }

    /// The instruction at `index` in `list`, or `None` if `index >= list.len()`.
    pub fn at(interner: &'a Interner, list: &'a InsnList, index: usize) -> Option<InsnRef<'a>> {
        let id = list.id_at(index)?;
        Some(InsnRef {
            interner,
            list,
            id,
            index,
            node: &list[id],
        })
    }
}

impl<'a> InsnRef<'a> {
    /// The string of a symbol of this instruction.
    #[inline]
    pub fn resolve(&self, sym: Sym) -> &'a JavaStr {
        self.interner.resolve(sym)
    }
}

impl Deref for InsnRef<'_> {
    type Target = InsnNode;

    fn deref(&self) -> &InsnNode {
        self.node
    }
}

/// A semantic bytecode interpreter (ASM's `Interpreter`): it computes values from other values,
/// and leaves their transfer to and from the stack and the local variables to a [`Frame`]. This
/// lets a generic [`Analyzer`](crate::Analyzer) work with various semantic interpreters.
///
/// # Differences with ASM
///
/// - **Nullable results are `Option`s.** ASM's methods may return `null`: `new_value` does for
///   the `void` type, and the operations do for the instructions that push nothing (an `IFEQ`'s
///   `unaryOperation`, ...). Where a frame needs a value that is `None`, the analysis fails
///   (where ASM would store a `null`, and usually fail later).
/// - **Every method can fail.** ASM's analyzer turns the `RuntimeException`s of an interpreter
///   into `AnalyzerException`s; here, a method returns an [`AnalyzerError`] (made with
///   [`AnalyzerError::runtime`] for what ASM throws as a `RuntimeException`). Panics are not
///   caught.
/// - **[`merge`](Self::merge) returns `None` when the merged value is `value1`,** which ASM's
///   contract requires to be returned as is: this saves a clone.
/// - **Extension by delegation.** ASM's stock interpreters are extended by subclassing. Here, they
///   expose their operations as inherent methods, so that a wrapper can override some of them and
///   delegate the rest (ADR-12).
#[allow(unused_variables)]
pub trait Interpreter {
    /// The type of the values of the analysis.
    type V: Value;

    /// Creates a new value that represents the given type, or an uninitialized value for `None`
    /// (ASM's `newValue`).
    ///
    /// It is called for method parameters (including `this`), exception handler variables, and
    /// with `None` for the variables reserved by `long` and `double` types; the `new_*_value`
    /// methods let an interpreter distinguish these cases. The size of the returned value must
    /// be the size of the type. Returns `None` for the `void` type.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException`.
    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<Self::V>, AnalyzerError>;

    /// Creates a new value that represents the given parameter type, to initialize the local
    /// variable `local` of the first frame (ASM's `newParameterValue`). Calls
    /// [`new_value`](Self::new_value) by default.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException`.
    fn new_parameter_value(
        &mut self,
        is_instance_method: bool,
        local: usize,
        ty: &Type,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        self.new_value(Some(ty))
    }

    /// Creates a new value that represents the given return type, the expected return type
    /// value of the frames (ASM's `newReturnTypeValue`). Calls [`new_value`](Self::new_value) by
    /// default.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException`.
    fn new_return_type_value(&mut self, ty: &Type) -> Result<Option<Self::V>, AnalyzerError> {
        self.new_value(Some(ty))
    }

    /// Creates a new uninitialized value for a local variable: one that does not correspond to
    /// a method parameter, or one half of a size 2 value whose other half is assigned a size 1
    /// value (ASM's `newEmptyValue`). Its size must be 1. Calls `new_value(None)` by default.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException`.
    fn new_empty_value(&mut self, local: usize) -> Result<Option<Self::V>, AnalyzerError> {
        self.new_value(None)
    }

    /// Creates a new value that represents the given exception type, pushed on the stack at the
    /// entry of an exception handler (ASM's `newExceptionValue`). Its size must be 1. Calls
    /// [`new_value`](Self::new_value) by default.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException`.
    fn new_exception_value(
        &mut self,
        try_catch_block: &TryCatchBlockNode,
        handler_frame: &Frame<Self::V>,
        exception_type: &Type,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        self.new_value(Some(exception_type))
    }

    /// Interprets an instruction without arguments: `ACONST_NULL`, `ICONST_M1` ... `ICONST_5`,
    /// `LCONST_0`, `LCONST_1`, `FCONST_0` ... `FCONST_2`, `DCONST_0`, `DCONST_1`, `BIPUSH`,
    /// `SIPUSH`, `LDC`, `JSR`, `GETSTATIC` and `NEW` (ASM's `newOperation`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<Self::V>, AnalyzerError>;

    /// Interprets an instruction that moves a value on the stack or to or from local variables:
    /// `ILOAD` ... `ALOAD`, `ISTORE` ... `ASTORE`, `DUP`, `DUP_X1`, `DUP_X2`, `DUP2`, `DUP2_X1`,
    /// `DUP2_X2` and `SWAP` (ASM's `copyOperation`). The returned value must be equal to the
    /// given one.
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
    ) -> Result<Self::V, AnalyzerError>;

    /// Interprets an instruction with a single argument: `INEG`, `LNEG`, `FNEG`, `DNEG`, `IINC`,
    /// `I2L` ... `I2S`, `IFEQ` ... `IFLE`, `TABLESWITCH`, `LOOKUPSWITCH`, `IRETURN` ...
    /// `ARETURN`, `PUTSTATIC`, `GETFIELD`, `NEWARRAY`, `ANEWARRAY`, `ARRAYLENGTH`, `ATHROW`,
    /// `CHECKCAST`, `INSTANCEOF`, `MONITORENTER`, `MONITOREXIT`, `IFNULL` and `IFNONNULL`
    /// (ASM's `unaryOperation`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError>;

    /// Interprets an instruction with two arguments: `IALOAD` ... `SALOAD`, the arithmetic,
    /// shift, logical and comparison instructions, `IF_ICMPEQ` ... `IF_ACMPNE` and `PUTFIELD`
    /// (ASM's `binaryOperation`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &Self::V,
        value2: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError>;

    /// Interprets an instruction with three arguments: `IASTORE` ... `SASTORE` (ASM's
    /// `ternaryOperation`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &Self::V,
        value2: &Self::V,
        value3: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError>;

    /// Interprets an instruction with a variable number of arguments: `INVOKEVIRTUAL`,
    /// `INVOKESPECIAL`, `INVOKESTATIC`, `INVOKEINTERFACE`, `MULTIANEWARRAY` and `INVOKEDYNAMIC`
    /// (ASM's `naryOperation`).
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[Self::V],
    ) -> Result<Option<Self::V>, AnalyzerError>;

    /// Interprets a return instruction: `IRETURN` ... `ARETURN` (ASM's `returnOperation`).
    /// `expected` is the expected return type value of the method, `None` if it returns `void`.
    ///
    /// # Errors
    ///
    /// If the instruction cannot be interpreted.
    fn return_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
        expected: Option<&Self::V>,
    ) -> Result<(), AnalyzerError>;

    /// Merges two values (ASM's `merge`): returns a value that represents both (a common super
    /// type of two types, an interval containing two intervals, ...), or `None` if that is
    /// `value1`.
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException` (a `SimpleVerifier` whose hierarchy does not know
    /// a class).
    fn merge(
        &mut self,
        value1: &Self::V,
        value2: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError>;
}

impl<I: Interpreter + ?Sized> Interpreter for &mut I {
    type V = I::V;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_value(ty)
    }
    fn new_parameter_value(
        &mut self,
        is_instance_method: bool,
        local: usize,
        ty: &Type,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_parameter_value(is_instance_method, local, ty)
    }
    fn new_return_type_value(&mut self, ty: &Type) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_return_type_value(ty)
    }
    fn new_empty_value(&mut self, local: usize) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_empty_value(local)
    }
    fn new_exception_value(
        &mut self,
        try_catch_block: &TryCatchBlockNode,
        handler_frame: &Frame<Self::V>,
        exception_type: &Type,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_exception_value(try_catch_block, handler_frame, exception_type)
    }
    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).new_operation(insn)
    }
    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
    ) -> Result<Self::V, AnalyzerError> {
        (**self).copy_operation(insn, value)
    }
    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).unary_operation(insn, value)
    }
    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &Self::V,
        value2: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).binary_operation(insn, value1, value2)
    }
    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &Self::V,
        value2: &Self::V,
        value3: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).ternary_operation(insn, value1, value2, value3)
    }
    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[Self::V],
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).nary_operation(insn, values)
    }
    fn return_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &Self::V,
        expected: Option<&Self::V>,
    ) -> Result<(), AnalyzerError> {
        (**self).return_operation(insn, value, expected)
    }
    fn merge(
        &mut self,
        value1: &Self::V,
        value2: &Self::V,
    ) -> Result<Option<Self::V>, AnalyzerError> {
        (**self).merge(value1, value2)
    }
}
