//! [`SourceInterpreter`]: ASM's `SourceInterpreter`.

use oxasm::opcodes::*;
use oxasm::{Type, TypeSort};
use oxasm_tree::{Insn, SymConstant};

use crate::basic_interpreter::{assertion_error, get_type, return_type};
use crate::error::AnalyzerError;
use crate::frame::cast_error;
use crate::interpreter::{InsnRef, Interpreter};
use crate::source_value::{Producer, SourceValue};

/// An [`Interpreter`] for [`SourceValue`] values (ASM's `SourceInterpreter`).
///
/// Its operations are also associated functions, for the interpreters that extend it: an
/// interpreter of [`SourceValue`]s that overrides some operations calls these for the others
/// (`SourceInterpreter::copy_operation(insn, value)` is `super.copyOperation(insn, value)`). They
/// are generic over the [`Producer`] type, so that an extension can record producers that are no
/// instructions; this interpreter itself records [`InsnId`](oxasm_tree::InsnId)s.
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceInterpreter;

impl SourceInterpreter {
    /// Constructs a new interpreter.
    pub fn new() -> SourceInterpreter {
        SourceInterpreter
    }

    /// ASM's `SourceInterpreter.newValue`.
    ///
    /// # Errors
    ///
    /// For a method type (ASM's `AssertionError`).
    pub fn new_value<P: Producer>(
        ty: Option<&Type>,
    ) -> Result<Option<SourceValue<P>>, AnalyzerError> {
        match ty {
            Some(ty) if ty.get_sort() == TypeSort::Void => Ok(None),
            Some(ty) => Ok(Some(SourceValue::new(size(ty)?))),
            None => Ok(Some(SourceValue::new(1))),
        }
    }

    /// ASM's `SourceInterpreter.newOperation`.
    ///
    /// # Errors
    ///
    /// For a malformed field descriptor.
    pub fn new_operation<P: Producer>(insn: InsnRef<'_>) -> Result<SourceValue<P>, AnalyzerError> {
        let size = match insn.opcode() {
            Some(LCONST_0 | LCONST_1 | DCONST_0 | DCONST_1) => 2,
            Some(LDC) => {
                // Values able to be pushed by LDC:
                //   - int, float, string (object), type (Class, object), type (MethodType,
                //       object), handle (MethodHandle, object): one word
                //   - long, double, ConstantDynamic (can produce either single word values, or
                //       double word values): (up to) two words
                let Insn::Ldc(value) = &insn.insn else {
                    return Err(cast_error(insn, "LdcInsnNode"));
                };
                match value {
                    // two words guaranteed
                    SymConstant::Long(_) | SymConstant::Double(_) => 2,
                    // might yield two words
                    SymConstant::Dynamic(constant_dynamic) => {
                        if insn.resolve(constant_dynamic.desc).is_empty() {
                            return Err(AnalyzerError::runtime(
                                "Index 0 out of bounds for length 0",
                            ));
                        }
                        constant_dynamic.get_size(insn.interner)
                    }
                    // one word guaranteed
                    _ => 1,
                }
            }
            Some(GETSTATIC) => field_size(insn)?,
            _ => 1,
        };
        Ok(SourceValue::of(size, insn.id.into()))
    }

    /// ASM's `SourceInterpreter.copyOperation`: a value of the same size, produced by `insn`.
    pub fn copy_operation<P: Producer>(
        insn: InsnRef<'_>,
        value: &SourceValue<P>,
    ) -> SourceValue<P> {
        SourceValue::of(value.size, insn.id.into())
    }

    /// ASM's `SourceInterpreter.unaryOperation`.
    ///
    /// # Errors
    ///
    /// For a malformed field descriptor.
    pub fn unary_operation<P: Producer>(
        insn: InsnRef<'_>,
        _value: &SourceValue<P>,
    ) -> Result<SourceValue<P>, AnalyzerError> {
        let size = match insn.opcode() {
            Some(LNEG | DNEG | I2L | I2D | L2D | F2L | F2D | D2L) => 2,
            Some(GETFIELD) => field_size(insn)?,
            _ => 1,
        };
        Ok(SourceValue::of(size, insn.id.into()))
    }

    /// ASM's `SourceInterpreter.binaryOperation`.
    pub fn binary_operation<P: Producer>(
        insn: InsnRef<'_>,
        _value1: &SourceValue<P>,
        _value2: &SourceValue<P>,
    ) -> SourceValue<P> {
        let size = match insn.opcode() {
            Some(
                LALOAD | DALOAD | LADD | DADD | LSUB | DSUB | LMUL | DMUL | LDIV | DDIV | LREM
                | DREM | LSHL | LSHR | LUSHR | LAND | LOR | LXOR,
            ) => 2,
            _ => 1,
        };
        SourceValue::of(size, insn.id.into())
    }

    /// ASM's `SourceInterpreter.ternaryOperation`: a value of size 1, produced by `insn`.
    pub fn ternary_operation<P: Producer>(
        insn: InsnRef<'_>,
        _value1: &SourceValue<P>,
        _value2: &SourceValue<P>,
        _value3: &SourceValue<P>,
    ) -> SourceValue<P> {
        SourceValue::of(1, insn.id.into())
    }

    /// ASM's `SourceInterpreter.naryOperation`.
    ///
    /// # Errors
    ///
    /// For a malformed method descriptor.
    pub fn nary_operation<P: Producer>(
        insn: InsnRef<'_>,
        _values: &[SourceValue<P>],
    ) -> Result<SourceValue<P>, AnalyzerError> {
        let size = match &insn.insn {
            Insn::MultiANewArray { .. } => 1,
            Insn::InvokeDynamic(invoke_dynamic) => {
                size(&return_type(insn.resolve(invoke_dynamic.desc))?)?
            }
            Insn::Method { desc, .. } => size(&return_type(insn.resolve(*desc))?)?,
            _ => return Err(cast_error(insn, "MethodInsnNode")),
        };
        Ok(SourceValue::of(size, insn.id.into()))
    }

    /// ASM's `SourceInterpreter.merge`: the union of the producers, with the smallest size
    /// (`None` if that is `value1`).
    pub fn merge<P: Producer>(
        value1: &SourceValue<P>,
        value2: &SourceValue<P>,
    ) -> Option<SourceValue<P>> {
        if value1.size <= value2.size && value1.insns.is_superset(&value2.insns) {
            return None;
        }
        Some(SourceValue::with_insns(
            value1.size.min(value2.size),
            value1.insns.union(&value2.insns),
        ))
    }
}

impl Interpreter for SourceInterpreter {
    type V = SourceValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<SourceValue>, AnalyzerError> {
        SourceInterpreter::new_value(ty)
    }

    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<SourceValue>, AnalyzerError> {
        SourceInterpreter::new_operation(insn).map(Some)
    }

    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &SourceValue,
    ) -> Result<SourceValue, AnalyzerError> {
        Ok(SourceInterpreter::copy_operation(insn, value))
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &SourceValue,
    ) -> Result<Option<SourceValue>, AnalyzerError> {
        SourceInterpreter::unary_operation(insn, value).map(Some)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &SourceValue,
        value2: &SourceValue,
    ) -> Result<Option<SourceValue>, AnalyzerError> {
        Ok(Some(SourceInterpreter::binary_operation(
            insn, value1, value2,
        )))
    }

    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &SourceValue,
        value2: &SourceValue,
        value3: &SourceValue,
    ) -> Result<Option<SourceValue>, AnalyzerError> {
        Ok(Some(SourceInterpreter::ternary_operation(
            insn, value1, value2, value3,
        )))
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[SourceValue],
    ) -> Result<Option<SourceValue>, AnalyzerError> {
        SourceInterpreter::nary_operation(insn, values).map(Some)
    }

    fn return_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value: &SourceValue,
        _expected: Option<&SourceValue>,
    ) -> Result<(), AnalyzerError> {
        Ok(())
    }

    fn merge(
        &mut self,
        value1: &SourceValue,
        value2: &SourceValue,
    ) -> Result<Option<SourceValue>, AnalyzerError> {
        Ok(SourceInterpreter::merge(value1, value2))
    }
}

/// `type.getSize()`, whose `AssertionError` for a method type is an error here.
fn size(ty: &Type) -> Result<usize, AnalyzerError> {
    if ty.get_sort() == TypeSort::Method {
        return Err(assertion_error());
    }
    Ok(ty.get_size())
}

/// The size of the field of a field instruction.
fn field_size(insn: InsnRef<'_>) -> Result<usize, AnalyzerError> {
    let Insn::Field { desc, .. } = &insn.insn else {
        return Err(cast_error(insn, "FieldInsnNode"));
    };
    size(&get_type(insn.resolve(*desc))?)
}
