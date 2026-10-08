//! [`BasicInterpreter`]: ASM's `BasicInterpreter`.

use oxasm::opcodes::*;
use oxasm::{JavaStr, Type, TypeSort};
use oxasm_tree::{Insn, SymConstant};

use crate::basic_value::BasicValue;
use crate::error::AnalyzerError;
use crate::frame::cast_error;
use crate::interpreter::{InsnRef, Interpreter};

/// An [`Interpreter`] for [`BasicValue`] values (ASM's `BasicInterpreter`).
///
/// # Extension
///
/// ASM's subclasses of `BasicInterpreter` override some of its methods, and call `super` for
/// the others. Here, an interpreter of [`BasicValue`]s implements [`Interpreter`] itself, and
/// calls the associated functions of `BasicInterpreter` for the operations it does not
/// override: `BasicInterpreter::new_operation(self, insn)` is `super.newOperation(insn)`. These
/// functions take the interpreter as `this`, and call its [`Interpreter::new_value`] where ASM
/// calls the (possibly overridden) `newValue`.
#[derive(Clone, Copy, Debug, Default)]
pub struct BasicInterpreter;

/// The internal name of the type of the `null` literal.
const NULL: &JavaStr = oxasm::jstr!("null");

impl BasicInterpreter {
    /// Special type used for the `null` literal: an object reference type with descriptor
    /// `Lnull;`.
    pub const NULL_TYPE: Type = Type::static_object_type(NULL);

    /// Constructs a new interpreter.
    pub fn new() -> BasicInterpreter {
        BasicInterpreter
    }

    /// ASM's `BasicInterpreter.newValue`.
    ///
    /// # Errors
    ///
    /// For a method type (ASM's `AssertionError`).
    pub fn new_value(ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        let Some(ty) = ty else {
            return Ok(Some(BasicValue::UNINITIALIZED_VALUE));
        };
        Ok(Some(match ty.get_sort() {
            TypeSort::Void => return Ok(None),
            TypeSort::Boolean
            | TypeSort::Char
            | TypeSort::Byte
            | TypeSort::Short
            | TypeSort::Int => BasicValue::INT_VALUE,
            TypeSort::Float => BasicValue::FLOAT_VALUE,
            TypeSort::Long => BasicValue::LONG_VALUE,
            TypeSort::Double => BasicValue::DOUBLE_VALUE,
            TypeSort::Array | TypeSort::Object => BasicValue::REFERENCE_VALUE,
            TypeSort::Method => return Err(assertion_error()),
        }))
    }

    /// ASM's `BasicInterpreter.newOperation`, which calls `this.new_value`.
    ///
    /// # Errors
    ///
    /// For an `LDC` of an illegal value, or a malformed descriptor.
    pub fn new_operation<I>(
        this: &mut I,
        insn: InsnRef<'_>,
    ) -> Result<Option<BasicValue>, AnalyzerError>
    where
        I: Interpreter<V = BasicValue> + ?Sized,
    {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        Ok(Some(match opcode {
            ACONST_NULL => return this.new_value(Some(&BasicInterpreter::NULL_TYPE)),
            ICONST_M1 | ICONST_0 | ICONST_1 | ICONST_2 | ICONST_3 | ICONST_4 | ICONST_5 => {
                BasicValue::INT_VALUE
            }
            LCONST_0 | LCONST_1 => BasicValue::LONG_VALUE,
            FCONST_0 | FCONST_1 | FCONST_2 => BasicValue::FLOAT_VALUE,
            DCONST_0 | DCONST_1 => BasicValue::DOUBLE_VALUE,
            BIPUSH | SIPUSH => BasicValue::INT_VALUE,
            LDC => {
                let Insn::Ldc(value) = &insn.insn else {
                    return Err(cast_error(insn, "LdcInsnNode"));
                };
                match value {
                    SymConstant::Integer(_) => BasicValue::INT_VALUE,
                    SymConstant::Float(_) => BasicValue::FLOAT_VALUE,
                    SymConstant::Long(_) => BasicValue::LONG_VALUE,
                    SymConstant::Double(_) => BasicValue::DOUBLE_VALUE,
                    SymConstant::String(_) => {
                        return this.new_value(Some(&object_type("java/lang/String")));
                    }
                    SymConstant::Type(descriptor) => {
                        match get_type(insn.resolve(*descriptor))?.get_sort() {
                            TypeSort::Object | TypeSort::Array => {
                                return this.new_value(Some(&object_type("java/lang/Class")));
                            }
                            TypeSort::Method => {
                                return this
                                    .new_value(Some(&object_type("java/lang/invoke/MethodType")));
                            }
                            _ => {
                                let mut message = oxasm::JavaString::from("Illegal LDC value ");
                                message.push_java_str(insn.resolve(*descriptor));
                                return Err(AnalyzerError::new(Some(insn.id), message));
                            }
                        }
                    }
                    SymConstant::MethodType(_) => {
                        return this.new_value(Some(&object_type("java/lang/invoke/MethodType")));
                    }
                    SymConstant::Handle(_) => {
                        return this.new_value(Some(&object_type("java/lang/invoke/MethodHandle")));
                    }
                    SymConstant::Dynamic(constant_dynamic) => {
                        return this
                            .new_value(Some(&get_type(insn.resolve(constant_dynamic.desc))?));
                    }
                }
            }
            JSR => BasicValue::RETURNADDRESS_VALUE,
            GETSTATIC => {
                let Insn::Field { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "FieldInsnNode"));
                };
                return this.new_value(Some(&get_type(insn.resolve(*desc))?));
            }
            NEW => {
                let Insn::Type { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "TypeInsnNode"));
                };
                return this.new_value(Some(&checked_object_type(insn.resolve(*desc))?));
            }
            _ => return Err(assertion_error()),
        }))
    }

    /// ASM's `BasicInterpreter.copyOperation`: the value itself.
    pub fn copy_operation(value: &BasicValue) -> BasicValue {
        value.clone()
    }

    /// ASM's `BasicInterpreter.unaryOperation`, which calls `this.new_value`.
    ///
    /// # Errors
    ///
    /// For a `NEWARRAY` of an invalid array type, or a malformed descriptor.
    pub fn unary_operation<I>(
        this: &mut I,
        insn: InsnRef<'_>,
        _value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError>
    where
        I: Interpreter<V = BasicValue> + ?Sized,
    {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        Ok(Some(match opcode {
            INEG | IINC | L2I | F2I | D2I | I2B | I2C | I2S => BasicValue::INT_VALUE,
            FNEG | I2F | L2F | D2F => BasicValue::FLOAT_VALUE,
            LNEG | I2L | F2L | D2L => BasicValue::LONG_VALUE,
            DNEG | I2D | L2D | F2D => BasicValue::DOUBLE_VALUE,
            IFEQ | IFNE | IFLT | IFGE | IFGT | IFLE | TABLESWITCH | LOOKUPSWITCH | IRETURN
            | LRETURN | FRETURN | DRETURN | ARETURN | PUTSTATIC => return Ok(None),
            GETFIELD => {
                let Insn::Field { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "FieldInsnNode"));
                };
                return this.new_value(Some(&get_type(insn.resolve(*desc))?));
            }
            NEWARRAY => {
                let Insn::Int { operand, .. } = insn.insn else {
                    return Err(cast_error(insn, "IntInsnNode"));
                };
                let descriptor = match operand {
                    T_BOOLEAN => "[Z",
                    T_CHAR => "[C",
                    T_BYTE => "[B",
                    T_SHORT => "[S",
                    T_INT => "[I",
                    T_FLOAT => "[F",
                    T_DOUBLE => "[D",
                    T_LONG => "[J",
                    _ => return Err(AnalyzerError::new(Some(insn.id), "Invalid array type")),
                };
                return this.new_value(Some(&Type::get_type(oxasm::jstr!(descriptor))));
            }
            ANEWARRAY => {
                let Insn::Type { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "TypeInsnNode"));
                };
                let mut descriptor = oxasm::JavaString::from("[");
                checked_object_type(insn.resolve(*desc))?.append_descriptor(&mut descriptor);
                return this.new_value(Some(&get_type(&descriptor)?));
            }
            ARRAYLENGTH => BasicValue::INT_VALUE,
            ATHROW => return Ok(None),
            CHECKCAST => {
                let Insn::Type { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "TypeInsnNode"));
                };
                return this.new_value(Some(&checked_object_type(insn.resolve(*desc))?));
            }
            INSTANCEOF => BasicValue::INT_VALUE,
            MONITORENTER | MONITOREXIT | IFNULL | IFNONNULL => return Ok(None),
            _ => return Err(assertion_error()),
        }))
    }

    /// ASM's `BasicInterpreter.binaryOperation`.
    ///
    /// # Errors
    ///
    /// For an instruction that is not a binary operation (ASM's `AssertionError`).
    pub fn binary_operation(
        insn: InsnRef<'_>,
        _value1: &BasicValue,
        _value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        Ok(Some(match opcode {
            IALOAD | BALOAD | CALOAD | SALOAD | IADD | ISUB | IMUL | IDIV | IREM | ISHL | ISHR
            | IUSHR | IAND | IOR | IXOR => BasicValue::INT_VALUE,
            FALOAD | FADD | FSUB | FMUL | FDIV | FREM => BasicValue::FLOAT_VALUE,
            LALOAD | LADD | LSUB | LMUL | LDIV | LREM | LSHL | LSHR | LUSHR | LAND | LOR | LXOR => {
                BasicValue::LONG_VALUE
            }
            DALOAD | DADD | DSUB | DMUL | DDIV | DREM => BasicValue::DOUBLE_VALUE,
            AALOAD => BasicValue::REFERENCE_VALUE,
            LCMP | FCMPL | FCMPG | DCMPL | DCMPG => BasicValue::INT_VALUE,
            IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE | IF_ACMPEQ
            | IF_ACMPNE | PUTFIELD => return Ok(None),
            _ => return Err(assertion_error()),
        }))
    }

    /// ASM's `BasicInterpreter.ternaryOperation`: `None`.
    pub fn ternary_operation() -> Option<BasicValue> {
        None
    }

    /// ASM's `BasicInterpreter.naryOperation`, which calls `this.new_value`.
    ///
    /// # Errors
    ///
    /// For a malformed descriptor.
    pub fn nary_operation<I>(
        this: &mut I,
        insn: InsnRef<'_>,
        _values: &[BasicValue],
    ) -> Result<Option<BasicValue>, AnalyzerError>
    where
        I: Interpreter<V = BasicValue> + ?Sized,
    {
        let ty = match &insn.insn {
            Insn::MultiANewArray { desc, .. } => get_type(insn.resolve(*desc))?,
            Insn::InvokeDynamic(invoke_dynamic) => return_type(insn.resolve(invoke_dynamic.desc))?,
            Insn::Method { desc, .. } => return_type(insn.resolve(*desc))?,
            _ => return Err(cast_error(insn, "MethodInsnNode")),
        };
        this.new_value(Some(&ty))
    }

    /// ASM's `BasicInterpreter.merge`: `value1` if the values are equal, an uninitialized value
    /// otherwise (`None` if that is `value1`, see [`Interpreter::merge`]).
    pub fn merge(value1: &BasicValue, value2: &BasicValue) -> Option<BasicValue> {
        (value1 != value2).then_some(BasicValue::UNINITIALIZED_VALUE)
    }
}

impl Interpreter for BasicInterpreter {
    type V = BasicValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::new_value(ty)
    }

    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::new_operation(self, insn)
    }

    fn copy_operation(
        &mut self,
        _insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        Ok(BasicInterpreter::copy_operation(value))
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::unary_operation(self, insn, value)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::binary_operation(insn, value1, value2)
    }

    fn ternary_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value1: &BasicValue,
        _value2: &BasicValue,
        _value3: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        Ok(BasicInterpreter::ternary_operation())
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[BasicValue],
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::nary_operation(self, insn, values)
    }

    fn return_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value: &BasicValue,
        _expected: Option<&BasicValue>,
    ) -> Result<(), AnalyzerError> {
        Ok(())
    }

    fn merge(
        &mut self,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        Ok(BasicInterpreter::merge(value1, value2))
    }
}

/// The object type of an internal name.
pub(crate) fn object_type(internal_name: &str) -> Type {
    Type::get_object_type(oxasm::jstr!(internal_name))
}

/// `Type.getObjectType(internalName)`, which throws for an empty name (`charAt(0)`).
pub(crate) fn checked_object_type(internal_name: &JavaStr) -> Result<Type, AnalyzerError> {
    if internal_name.is_empty() {
        return Err(AnalyzerError::runtime("Index 0 out of bounds for length 0"));
    }
    Ok(Type::get_object_type(internal_name))
}

/// `Type.getType(descriptor)`, whose `IllegalArgumentException` is a runtime error.
pub(crate) fn get_type(descriptor: &JavaStr) -> Result<Type, AnalyzerError> {
    Type::try_get_type(descriptor).map_err(|e| AnalyzerError::runtime(e.message()))
}

/// `Type.getReturnType(methodDescriptor)`, whose exceptions are runtime errors.
pub(crate) fn return_type(method_descriptor: &JavaStr) -> Result<Type, AnalyzerError> {
    Type::try_get_return_type_of(method_descriptor).map_err(|e| AnalyzerError::runtime(e.message()))
}

/// ASM's `AssertionError`, for a case its code deems impossible. Java's analyzer does not catch
/// it (it is not a `RuntimeException`); here it fails the analysis like any error.
pub(crate) fn assertion_error() -> AnalyzerError {
    AnalyzerError::runtime("java.lang.AssertionError")
}
