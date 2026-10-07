//! [`BasicValue`]: ASM's `BasicValue`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use oxasm::{JavaString, Type, TypeSort};

use crate::value::Value;

/// A [`Value`] that is represented with its type in a seven types type system, which
/// distinguishes the uninitialized, int, float, long, double, reference and return address types
/// (ASM's `BasicValue`).
///
/// ASM's static instances (`BasicValue.INT_VALUE`, ...) are the associated constants. Values are
/// equal if their types are; but, as in ASM, the [uninitialized](Self::UNINITIALIZED_VALUE),
/// [return address](Self::RETURNADDRESS_VALUE) and [reference](Self::REFERENCE_VALUE) constants
/// print as `.`, `A` and `R`, where other values (including equal ones) print as their type's
/// descriptor.
#[derive(Clone, Debug)]
pub struct BasicValue(Repr);

/// ASM's static instances, which need no allocation, and the other values. Their types are equal
/// when the instance's type is (`Of(Type::INT)` equals `Int`), but they print differently.
#[derive(Clone, Debug)]
enum Repr {
    Uninitialized,
    Int,
    Float,
    Long,
    Double,
    Reference,
    ReturnAddress,
    /// A value created with [`BasicValue::new`]. The type is shared, so that frames copy their
    /// values cheaply.
    Of(Arc<Type>),
}

/// The type of `REFERENCE_VALUE`.
static OBJECT: Type = Type::static_object_type(oxasm::jstr!("java/lang/Object"));
static INT: Type = Type::INT;
static FLOAT: Type = Type::FLOAT;
static LONG: Type = Type::LONG;
static DOUBLE: Type = Type::DOUBLE;
static VOID: Type = Type::VOID;

impl BasicValue {
    /// An uninitialized value.
    pub const UNINITIALIZED_VALUE: BasicValue = BasicValue(Repr::Uninitialized);
    /// A byte, boolean, char, short, or int value.
    pub const INT_VALUE: BasicValue = BasicValue(Repr::Int);
    /// A float value.
    pub const FLOAT_VALUE: BasicValue = BasicValue(Repr::Float);
    /// A long value.
    pub const LONG_VALUE: BasicValue = BasicValue(Repr::Long);
    /// A double value.
    pub const DOUBLE_VALUE: BasicValue = BasicValue(Repr::Double);
    /// An object or array reference value.
    pub const REFERENCE_VALUE: BasicValue = BasicValue(Repr::Reference);
    /// A return address value (produced by a `JSR` instruction).
    pub const RETURNADDRESS_VALUE: BasicValue = BasicValue(Repr::ReturnAddress);

    /// Constructs a new value of the given type, `None` for an uninitialized value.
    pub fn new(ty: Option<Type>) -> BasicValue {
        match ty {
            None => BasicValue::UNINITIALIZED_VALUE,
            Some(ty) => BasicValue(Repr::Of(Arc::new(ty))),
        }
    }

    /// The type of this value, or `None` for uninitialized values (ASM's `getType()`).
    pub fn get_type(&self) -> Option<&Type> {
        Some(match &self.0 {
            Repr::Uninitialized => return None,
            Repr::Int => &INT,
            Repr::Float => &FLOAT,
            Repr::Long => &LONG,
            Repr::Double => &DOUBLE,
            Repr::Reference => &OBJECT,
            Repr::ReturnAddress => &VOID,
            Repr::Of(ty) => ty,
        })
    }

    /// Whether this value corresponds to an object or array reference (ASM's `isReference()`).
    pub fn is_reference(&self) -> bool {
        self.get_type()
            .is_some_and(|ty| matches!(ty.get_sort(), TypeSort::Object | TypeSort::Array))
    }

    /// ASM's `toString()`, with the unpaired surrogates a type name may have.
    pub fn to_java_string(&self) -> JavaString {
        match &self.0 {
            Repr::Uninitialized => JavaString::from("."),
            Repr::ReturnAddress => JavaString::from("A"),
            Repr::Reference => JavaString::from("R"),
            _ => self
                .get_type()
                .expect("an initialized value")
                .get_descriptor()
                .into_owned(),
        }
    }
}

impl Value for BasicValue {
    fn get_size(&self) -> usize {
        match self.get_type().map(Type::get_sort) {
            Some(TypeSort::Long | TypeSort::Double) => 2,
            _ => 1,
        }
    }
}

impl PartialEq for BasicValue {
    fn eq(&self, other: &BasicValue) -> bool {
        match (&self.0, &other.0) {
            (Repr::Of(_), _) | (_, Repr::Of(_)) => self.get_type() == other.get_type(),
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

impl Eq for BasicValue {}

impl Hash for BasicValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get_type().hash(state);
    }
}

impl fmt::Display for BasicValue {
    /// ASM's `toString()` (see [`BasicValue::to_java_string`]).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_java_string(), f)
    }
}
