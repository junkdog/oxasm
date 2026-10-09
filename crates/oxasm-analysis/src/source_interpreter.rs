//! [`SourceInterpreter`]: ASM's `SourceInterpreter`.

use oxasm::opcodes::*;
use oxasm::{Type, TypeSort};
use oxasm_tree::{Insn, SymConstant};

use crate::basic_interpreter::{assertion_error, get_type, return_type};
use crate::error::AnalyzerError;
use crate::frame::cast_error;
use crate::interpreter::{InsnRef, Interpreter};
use crate::set_arena::SetArena;
use crate::source_value::{Producer, SourceValue};

/// An [`Interpreter`] for [`SourceValue`] values (ASM's `SourceInterpreter`).
///
/// Its operations are also associated functions, for the interpreters that extend it: an
/// interpreter of [`SourceValue`]s that overrides some operations calls these for the others
/// (`SourceInterpreter::copy_operation(insn, value)` is `super.copyOperation(insn, value)`). They
/// are generic over the [`Producer`] type, so that an extension can record producers that are no
/// instructions; this interpreter itself records [`InsnId`](oxasm_tree::InsnId)s.
///
/// The sets of two or more producers that its merges make are allocated in a [`SetArena`],
/// which its values borrow for `'s`.
#[derive(Clone, Copy, Debug)]
pub struct SourceInterpreter<'s> {
    sets: &'s SetArena,
}

impl<'s> SourceInterpreter<'s> {
    /// Constructs a new interpreter, whose values' sets are allocated in `sets`.
    pub fn new(sets: &'s SetArena) -> SourceInterpreter<'s> {
        SourceInterpreter { sets }
    }

    /// The arena of this interpreter's sets.
    pub fn sets(&self) -> &'s SetArena {
        self.sets
    }

    /// ASM's `SourceInterpreter.newValue`.
    ///
    /// # Errors
    ///
    /// For a method type (ASM's `AssertionError`).
    pub fn new_value<'v, P: Producer>(
        ty: Option<&Type>,
    ) -> Result<Option<SourceValue<'v, P>>, AnalyzerError> {
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
    pub fn new_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
    ) -> Result<SourceValue<'v, P>, AnalyzerError> {
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
    pub fn copy_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
        value: &SourceValue<'v, P>,
    ) -> SourceValue<'v, P> {
        SourceValue::of(value.size, insn.id.into())
    }

    /// ASM's `SourceInterpreter.unaryOperation`.
    ///
    /// # Errors
    ///
    /// For a malformed field descriptor.
    pub fn unary_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
        _value: &SourceValue<'v, P>,
    ) -> Result<SourceValue<'v, P>, AnalyzerError> {
        let size = match insn.opcode() {
            Some(LNEG | DNEG | I2L | I2D | L2D | F2L | F2D | D2L) => 2,
            Some(GETFIELD) => field_size(insn)?,
            _ => 1,
        };
        Ok(SourceValue::of(size, insn.id.into()))
    }

    /// ASM's `SourceInterpreter.binaryOperation`.
    pub fn binary_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
        _value1: &SourceValue<'v, P>,
        _value2: &SourceValue<'v, P>,
    ) -> SourceValue<'v, P> {
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
    pub fn ternary_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
        _value1: &SourceValue<'v, P>,
        _value2: &SourceValue<'v, P>,
        _value3: &SourceValue<'v, P>,
    ) -> SourceValue<'v, P> {
        SourceValue::of(1, insn.id.into())
    }

    /// ASM's `SourceInterpreter.naryOperation`.
    ///
    /// # Errors
    ///
    /// For a malformed method descriptor.
    pub fn nary_operation<'v, P: Producer>(
        insn: InsnRef<'_>,
        _values: &[SourceValue<'v, P>],
    ) -> Result<SourceValue<'v, P>, AnalyzerError> {
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
    /// (`None` if that is `value1`). A union of two or more producers is allocated in `sets`.
    #[inline]
    pub fn merge<P: Producer>(
        sets: &'s SetArena,
        value1: &SourceValue<'s, P>,
        value2: &SourceValue<'s, P>,
    ) -> Option<SourceValue<'s, P>> {
        // Most merges are of a value with a copy of itself: inline, and without a call.
        if is_identical(value1, value2) {
            return None;
        }
        let mut merged = *value1;
        SourceInterpreter::merge_different(sets, &mut merged, value2).then_some(merged)
    }

    /// [`Interpreter::merge_values`] for the values of a `SourceInterpreter` (or of an
    /// interpreter whose [`merge`](Interpreter::merge) is [`merge`](Self::merge)): `values[i]`
    /// becomes the merge of `values[i]` and `other[i]`. Returns whether `values` changed.
    ///
    /// A merged value always differs from the old one (it has a smaller size, or more
    /// producers), so there is no comparison of the two, and most values, which are identical
    /// copies, cost an inline test.
    #[inline]
    pub fn merge_values<P: Producer>(
        sets: &'s SetArena,
        values: &mut [SourceValue<'s, P>],
        other: &[SourceValue<'s, P>],
    ) -> bool {
        let mut changed = false;
        for (value, other) in values.iter_mut().zip(other) {
            if !is_identical(value, other) {
                changed |= SourceInterpreter::merge_different(sets, value, other);
            }
        }
        changed
    }

    /// Merges `value2` into `value1`, which is not an identical copy of it. Returns whether
    /// `value1` changed.
    #[inline(never)]
    fn merge_different<P: Producer>(
        sets: &'s SetArena,
        value1: &mut SourceValue<'s, P>,
        value2: &SourceValue<'s, P>,
    ) -> bool {
        let size = value1.size.min(value2.size);
        match value1.insns.union_if_grown(&value2.insns, sets) {
            Some(insns) => *value1 = SourceValue::with_insns(size, insns),
            None if size == value1.size => return false,
            None => value1.size = size,
        }
        true
    }
}

impl<'s> Interpreter for SourceInterpreter<'s> {
    type V = SourceValue<'s>;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        SourceInterpreter::new_value(ty)
    }

    fn new_operation(
        &mut self,
        insn: InsnRef<'_>,
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        SourceInterpreter::new_operation(insn).map(Some)
    }

    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &SourceValue<'s>,
    ) -> Result<SourceValue<'s>, AnalyzerError> {
        Ok(SourceInterpreter::copy_operation(insn, value))
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &SourceValue<'s>,
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        SourceInterpreter::unary_operation(insn, value).map(Some)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &SourceValue<'s>,
        value2: &SourceValue<'s>,
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        Ok(Some(SourceInterpreter::binary_operation(
            insn, value1, value2,
        )))
    }

    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &SourceValue<'s>,
        value2: &SourceValue<'s>,
        value3: &SourceValue<'s>,
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        Ok(Some(SourceInterpreter::ternary_operation(
            insn, value1, value2, value3,
        )))
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[SourceValue<'s>],
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        SourceInterpreter::nary_operation(insn, values).map(Some)
    }

    fn return_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value: &SourceValue<'s>,
        _expected: Option<&SourceValue<'s>>,
    ) -> Result<(), AnalyzerError> {
        Ok(())
    }

    #[inline]
    fn merge(
        &mut self,
        value1: &SourceValue<'s>,
        value2: &SourceValue<'s>,
    ) -> Result<Option<SourceValue<'s>>, AnalyzerError> {
        Ok(SourceInterpreter::merge(self.sets, value1, value2))
    }

    #[inline]
    fn merge_values(
        &mut self,
        values: &mut [SourceValue<'s>],
        other: &[SourceValue<'s>],
    ) -> Result<bool, AnalyzerError> {
        Ok(SourceInterpreter::merge_values(self.sets, values, other))
    }
}

/// Whether two values are identical copies (`SmallSet::is_identical`): their merge is the
/// first one.
#[inline]
fn is_identical<P: Producer>(value1: &SourceValue<'_, P>, value2: &SourceValue<'_, P>) -> bool {
    value1.size == value2.size && value1.insns.is_identical(&value2.insns)
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
