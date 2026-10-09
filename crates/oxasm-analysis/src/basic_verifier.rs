//! [`BasicVerifier`] and [`Verifier`]: ASM's `BasicVerifier`.

use oxasm::opcodes::*;
use oxasm::{JavaStr, JavaString, Type};
use oxasm_tree::Insn;

use crate::basic_interpreter::{BasicInterpreter, assertion_error, checked_object_type, get_type};
use crate::basic_value::BasicValue;
use crate::error::AnalyzerError;
use crate::frame::cast_error;
use crate::interpreter::{InsnRef, Interpreter};

/// The methods of ASM's `BasicVerifier` that its subclasses (`SimpleVerifier`) override, with
/// `BasicVerifier`'s behavior as defaults.
///
/// The verification functions of [`BasicVerifier`] take a `this: &mut impl Verifier`, and call
/// these methods (and [`Interpreter::new_value`]) on it, as ASM's methods do on `this`.
pub trait Verifier: Interpreter<V = BasicValue> {
    /// Whether the given value corresponds to an array reference (ASM's `isArrayValue`).
    fn is_array_value(&mut self, value: &BasicValue) -> bool {
        value.is_reference()
    }

    /// The value of the elements of the given array reference value (ASM's
    /// `getElementValue`).
    ///
    /// # Errors
    ///
    /// If `object_array_value` does not correspond to an array type.
    fn get_element_value(
        &mut self,
        object_array_value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        let _ = object_array_value;
        Ok(BasicValue::REFERENCE_VALUE)
    }

    /// Whether the type of `value` is a subtype of the type of `expected` (ASM's
    /// `isSubTypeOf`).
    ///
    /// # Errors
    ///
    /// What ASM throws as a `RuntimeException` (a class a `SimpleVerifier`'s hierarchy does not
    /// know).
    fn is_sub_type_of(
        &mut self,
        value: &BasicValue,
        expected: &BasicValue,
    ) -> Result<bool, AnalyzerError> {
        Ok(value == expected)
    }
}

/// An extended [`BasicInterpreter`] that checks that bytecode instructions are correctly used
/// (ASM's `BasicVerifier`).
///
/// Like [`BasicInterpreter`], its operations are associated functions taking the interpreter as
/// `this`, for interpreters that extend it (see [`Verifier`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct BasicVerifier;

impl BasicVerifier {
    /// Constructs a new verifier.
    pub fn new() -> BasicVerifier {
        BasicVerifier
    }

    /// ASM's `BasicVerifier.copyOperation`.
    ///
    /// # Errors
    ///
    /// If the value does not have the type the instruction expects.
    pub fn copy_operation(
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        let expected = match opcode {
            ILOAD | ISTORE => BasicValue::INT_VALUE,
            FLOAD | FSTORE => BasicValue::FLOAT_VALUE,
            LLOAD | LSTORE => BasicValue::LONG_VALUE,
            DLOAD | DSTORE => BasicValue::DOUBLE_VALUE,
            ALOAD => {
                if !value.is_reference() {
                    return Err(expected_error(
                        insn,
                        None,
                        oxasm::jstr!("an object reference"),
                        value,
                    ));
                }
                return Ok(*value);
            }
            ASTORE => {
                if !value.is_reference() && BasicValue::RETURNADDRESS_VALUE != *value {
                    return Err(expected_error(
                        insn,
                        None,
                        oxasm::jstr!("an object reference or a return address"),
                        value,
                    ));
                }
                return Ok(*value);
            }
            _ => return Ok(*value),
        };
        if expected != *value {
            return Err(expected_value_error(insn, None, Some(&expected), value));
        }
        Ok(*value)
    }

    /// ASM's `BasicVerifier.unaryOperation`.
    ///
    /// # Errors
    ///
    /// If the value does not have the type the instruction expects.
    pub fn unary_operation<I: Verifier + ?Sized>(
        this: &mut I,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        let expected = match opcode {
            INEG | IINC | I2F | I2L | I2D | I2B | I2C | I2S | IFEQ | IFNE | IFLT | IFGE | IFGT
            | IFLE | TABLESWITCH | LOOKUPSWITCH | IRETURN | NEWARRAY | ANEWARRAY => {
                Some(BasicValue::INT_VALUE)
            }
            FNEG | F2I | F2L | F2D | FRETURN => Some(BasicValue::FLOAT_VALUE),
            LNEG | L2I | L2F | L2D | LRETURN => Some(BasicValue::LONG_VALUE),
            DNEG | D2I | D2F | D2L | DRETURN => Some(BasicValue::DOUBLE_VALUE),
            GETFIELD => {
                let Insn::Field { owner, .. } = &insn.insn else {
                    return Err(cast_error(insn, "FieldInsnNode"));
                };
                this.new_value(Some(&checked_object_type(insn.resolve(*owner))?))?
            }
            ARRAYLENGTH => {
                if !this.is_array_value(value) {
                    return Err(expected_error(
                        insn,
                        None,
                        oxasm::jstr!("an array reference"),
                        value,
                    ));
                }
                return BasicInterpreter::unary_operation(this, insn, value);
            }
            CHECKCAST | ARETURN | ATHROW | INSTANCEOF | MONITORENTER | MONITOREXIT | IFNULL
            | IFNONNULL => {
                if !value.is_reference() {
                    return Err(expected_error(
                        insn,
                        None,
                        oxasm::jstr!("an object reference"),
                        value,
                    ));
                }
                return BasicInterpreter::unary_operation(this, insn, value);
            }
            PUTSTATIC => {
                let Insn::Field { desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "FieldInsnNode"));
                };
                this.new_value(Some(&get_type(insn.resolve(*desc))?))?
            }
            _ => return Err(assertion_error()),
        };
        if !is_sub_type_of(this, value, expected.as_ref())? {
            return Err(expected_value_error(insn, None, expected.as_ref(), value));
        }
        BasicInterpreter::unary_operation(this, insn, value)
    }

    /// ASM's `BasicVerifier.binaryOperation`.
    ///
    /// # Errors
    ///
    /// If a value does not have the type the instruction expects.
    pub fn binary_operation<I: Verifier + ?Sized>(
        this: &mut I,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        let array = |this: &mut I, descriptor: &str| {
            this.new_value(Some(&Type::get_type(oxasm::jstr!(descriptor))))
        };
        let (expected1, expected2) = match opcode {
            IALOAD => (array(this, "[I")?, Some(BasicValue::INT_VALUE)),
            BALOAD => {
                let boolean_array = array(this, "[Z")?;
                if is_sub_type_of(this, value1, boolean_array.as_ref())? {
                    (array(this, "[Z")?, Some(BasicValue::INT_VALUE))
                } else {
                    (array(this, "[B")?, Some(BasicValue::INT_VALUE))
                }
            }
            CALOAD => (array(this, "[C")?, Some(BasicValue::INT_VALUE)),
            SALOAD => (array(this, "[S")?, Some(BasicValue::INT_VALUE)),
            LALOAD => (array(this, "[J")?, Some(BasicValue::INT_VALUE)),
            FALOAD => (array(this, "[F")?, Some(BasicValue::INT_VALUE)),
            DALOAD => (array(this, "[D")?, Some(BasicValue::INT_VALUE)),
            AALOAD => (
                array(this, "[Ljava/lang/Object;")?,
                Some(BasicValue::INT_VALUE),
            ),
            IADD | ISUB | IMUL | IDIV | IREM | ISHL | ISHR | IUSHR | IAND | IOR | IXOR
            | IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE => {
                (Some(BasicValue::INT_VALUE), Some(BasicValue::INT_VALUE))
            }
            FADD | FSUB | FMUL | FDIV | FREM | FCMPL | FCMPG => {
                (Some(BasicValue::FLOAT_VALUE), Some(BasicValue::FLOAT_VALUE))
            }
            LADD | LSUB | LMUL | LDIV | LREM | LAND | LOR | LXOR | LCMP => {
                (Some(BasicValue::LONG_VALUE), Some(BasicValue::LONG_VALUE))
            }
            LSHL | LSHR | LUSHR => (Some(BasicValue::LONG_VALUE), Some(BasicValue::INT_VALUE)),
            DADD | DSUB | DMUL | DDIV | DREM | DCMPL | DCMPG => (
                Some(BasicValue::DOUBLE_VALUE),
                Some(BasicValue::DOUBLE_VALUE),
            ),
            IF_ACMPEQ | IF_ACMPNE => (
                Some(BasicValue::REFERENCE_VALUE),
                Some(BasicValue::REFERENCE_VALUE),
            ),
            PUTFIELD => {
                let Insn::Field { owner, desc, .. } = &insn.insn else {
                    return Err(cast_error(insn, "FieldInsnNode"));
                };
                (
                    this.new_value(Some(&checked_object_type(insn.resolve(*owner))?))?,
                    this.new_value(Some(&get_type(insn.resolve(*desc))?))?,
                )
            }
            _ => return Err(assertion_error()),
        };
        if !is_sub_type_of(this, value1, expected1.as_ref())? {
            return Err(expected_value_error(
                insn,
                Some("First argument"),
                expected1.as_ref(),
                value1,
            ));
        } else if !is_sub_type_of(this, value2, expected2.as_ref())? {
            return Err(expected_value_error(
                insn,
                Some("Second argument"),
                expected2.as_ref(),
                value2,
            ));
        }
        if opcode == AALOAD {
            Ok(Some(this.get_element_value(value1)?))
        } else {
            BasicInterpreter::binary_operation(insn, value1, value2)
        }
    }

    /// ASM's `BasicVerifier.ternaryOperation`.
    ///
    /// # Errors
    ///
    /// If a value does not have the type the instruction expects.
    pub fn ternary_operation<I: Verifier + ?Sized>(
        this: &mut I,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
        value3: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        let array = |this: &mut I, descriptor: &str| {
            this.new_value(Some(&Type::get_type(oxasm::jstr!(descriptor))))
        };
        let (expected1, expected3) = match opcode {
            IASTORE => (array(this, "[I")?, BasicValue::INT_VALUE),
            BASTORE => {
                let boolean_array = array(this, "[Z")?;
                if is_sub_type_of(this, value1, boolean_array.as_ref())? {
                    (array(this, "[Z")?, BasicValue::INT_VALUE)
                } else {
                    (array(this, "[B")?, BasicValue::INT_VALUE)
                }
            }
            CASTORE => (array(this, "[C")?, BasicValue::INT_VALUE),
            SASTORE => (array(this, "[S")?, BasicValue::INT_VALUE),
            LASTORE => (array(this, "[J")?, BasicValue::LONG_VALUE),
            FASTORE => (array(this, "[F")?, BasicValue::FLOAT_VALUE),
            DASTORE => (array(this, "[D")?, BasicValue::DOUBLE_VALUE),
            AASTORE => (Some(*value1), BasicValue::REFERENCE_VALUE),
            _ => return Err(assertion_error()),
        };
        if !is_sub_type_of(this, value1, expected1.as_ref())? {
            let mut expected = JavaString::from("a ");
            expected.push_java_str(&value_text(expected1.as_ref()));
            expected.push_str(" array reference");
            return Err(expected_error(
                insn,
                Some("First argument"),
                &expected,
                value1,
            ));
        } else if BasicValue::INT_VALUE != *value2 {
            return Err(expected_value_error(
                insn,
                Some("Second argument"),
                Some(&BasicValue::INT_VALUE),
                value2,
            ));
        } else if !this.is_sub_type_of(value3, &expected3)? {
            return Err(expected_value_error(
                insn,
                Some("Third argument"),
                Some(&expected3),
                value3,
            ));
        }
        Ok(None)
    }

    /// ASM's `BasicVerifier.naryOperation`.
    ///
    /// # Errors
    ///
    /// If a value does not have the type the instruction expects.
    pub fn nary_operation<I: Verifier + ?Sized>(
        this: &mut I,
        insn: InsnRef<'_>,
        values: &[BasicValue],
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let opcode = insn.opcode().ok_or_else(assertion_error)?;
        if opcode == MULTIANEWARRAY {
            for value in values {
                if BasicValue::INT_VALUE != *value {
                    return Err(expected_value_error(
                        insn,
                        None,
                        Some(&BasicValue::INT_VALUE),
                        value,
                    ));
                }
            }
        } else {
            let mut i = 0;
            let method_descriptor: &JavaStr = match &insn.insn {
                Insn::InvokeDynamic(invoke_dynamic) if opcode == INVOKEDYNAMIC => {
                    insn.resolve(invoke_dynamic.desc)
                }
                Insn::Method { owner, desc, .. } => {
                    if opcode != INVOKESTATIC {
                        let owner =
                            this.new_value(Some(&checked_object_type(insn.resolve(*owner))?))?;
                        let value = values.first().ok_or_else(|| index_out_of_range(0))?;
                        i += 1;
                        if !is_sub_type_of(this, value, owner.as_ref())? {
                            return Err(expected_value_error(
                                insn,
                                Some("Method owner"),
                                owner.as_ref(),
                                value,
                            ));
                        }
                    }
                    insn.resolve(*desc)
                }
                _ => return Err(cast_error(insn, "MethodInsnNode")),
            };
            let args = Type::try_get_argument_types_of(method_descriptor)
                .map_err(|e| AnalyzerError::runtime(e.message()))?;
            let mut j = 0;
            while i < values.len() {
                let argument = args.get(j).ok_or_else(|| index_out_of_range(j))?;
                j += 1;
                let expected = this.new_value(Some(argument))?;
                let actual = &values[i];
                i += 1;
                if !is_sub_type_of(this, actual, expected.as_ref())? {
                    return Err(expected_value_error(
                        insn,
                        Some(&format!("Argument {j}")),
                        expected.as_ref(),
                        actual,
                    ));
                }
            }
        }
        BasicInterpreter::nary_operation(this, insn, values)
    }

    /// ASM's `BasicVerifier.returnOperation`.
    ///
    /// # Errors
    ///
    /// If the value is not a subtype of the method's return type.
    pub fn return_operation<I: Verifier + ?Sized>(
        this: &mut I,
        insn: InsnRef<'_>,
        value: &BasicValue,
        expected: Option<&BasicValue>,
    ) -> Result<(), AnalyzerError> {
        if !is_sub_type_of(this, value, expected)? {
            return Err(expected_value_error(
                insn,
                Some("Incompatible return type"),
                expected,
                value,
            ));
        }
        Ok(())
    }
}

impl Verifier for BasicVerifier {}

impl Interpreter for BasicVerifier {
    type V = BasicValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::new_value(ty)
    }

    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::new_operation(self, insn)
    }

    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        BasicVerifier::copy_operation(insn, value)
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::unary_operation(self, insn, value)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::binary_operation(self, insn, value1, value2)
    }

    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
        value3: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::ternary_operation(self, insn, value1, value2, value3)
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[BasicValue],
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::nary_operation(self, insn, values)
    }

    fn return_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
        expected: Option<&BasicValue>,
    ) -> Result<(), AnalyzerError> {
        BasicVerifier::return_operation(self, insn, value, expected)
    }

    fn merge(
        &mut self,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        Ok(BasicInterpreter::merge(value1, value2))
    }
}

/// `isSubTypeOf(value, expected)` where `expected` may be ASM's `null` (the value of the `void`
/// type): never a subtype, as with `BasicVerifier.isSubTypeOf`'s `equals`.
fn is_sub_type_of<I: Verifier + ?Sized>(
    this: &mut I,
    value: &BasicValue,
    expected: Option<&BasicValue>,
) -> Result<bool, AnalyzerError> {
    match expected {
        Some(expected) => this.is_sub_type_of(value, expected),
        None => Ok(false),
    }
}

/// The text of a value in a message: its `toString()`, or `null`.
fn value_text(value: Option<&BasicValue>) -> JavaString {
    value.map_or_else(|| JavaString::from("null"), BasicValue::to_java_string)
}

/// ASM's `AnalyzerException(insn, message, expected, actual)`, with a text as `expected`.
fn expected_error(
    insn: InsnRef<'_>,
    message: Option<&str>,
    expected: &JavaStr,
    actual: &BasicValue,
) -> AnalyzerError {
    AnalyzerError::expected(Some(insn.id), message, expected, &actual.to_java_string())
}

/// ASM's `AnalyzerException(insn, message, expected, actual)`, with a value as `expected`.
fn expected_value_error(
    insn: InsnRef<'_>,
    message: Option<&str>,
    expected: Option<&BasicValue>,
    actual: &BasicValue,
) -> AnalyzerError {
    AnalyzerError::expected(
        Some(insn.id),
        message,
        &value_text(expected),
        &actual.to_java_string(),
    )
}

/// Java's `IndexOutOfBoundsException` of a list access, which only an inconsistent instruction
/// (more values than its descriptor has arguments) causes.
fn index_out_of_range(index: usize) -> AnalyzerError {
    AnalyzerError::runtime(format!("Index {index} out of bounds"))
}
