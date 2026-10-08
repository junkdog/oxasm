//! The values ASM passes as `Object` (ADR-05): [`Constant`] (`visitLdcInsn`, field initial
//! values, bootstrap method arguments), [`AnnotationValue`] (`AnnotationVisitor.visit`) and
//! [`FrameValue`] (`visitFrame` locals and stack).
//!
//! Each has a lifetime for zero-copy strings borrowed from the class file during a visit;
//! `'static` values (via `into_owned`) are for storage. A `&Constant<'static>` can be passed
//! wherever a `&Constant<'_>` is expected.
//!
//! Floating point values compare and hash like Java's `Float.equals`/`Double.equals`: by bits,
//! with every NaN equal to every other NaN (`floatToIntBits`), and `0.0 != -0.0`.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::constant_dynamic::ConstantDynamic;
use crate::handle::Handle;
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::ty::Type;

/// A constant pool value: the operand of `LDC`, the initial value of a field, or a bootstrap
/// method argument.
///
/// Java's `Byte`, `Character`, `Short` and `Boolean` inputs, which ASM's `SymbolTable.addConstant`
/// widens to an `int`, are not modeled: callers widen them to [`Constant::Integer`].
#[derive(Clone, Debug)]
pub enum Constant<'a> {
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    String(Cow<'a, JavaStr>),
    /// A class (`OBJECT` or `ARRAY` sort) or method type (`METHOD` sort) constant.
    Type(Type<'a>),
    Handle(Handle),
    Dynamic(Box<ConstantDynamic>),
}

impl Constant<'_> {
    /// Converts into a constant that owns its data.
    #[must_use]
    pub fn into_owned(self) -> Constant<'static> {
        match self {
            Constant::Integer(v) => Constant::Integer(v),
            Constant::Float(v) => Constant::Float(v),
            Constant::Long(v) => Constant::Long(v),
            Constant::Double(v) => Constant::Double(v),
            Constant::String(s) => Constant::String(Cow::Owned(s.into_owned())),
            Constant::Type(t) => Constant::Type(t.into_owned()),
            Constant::Handle(h) => Constant::Handle(h),
            Constant::Dynamic(d) => Constant::Dynamic(d),
        }
    }

    /// Copies into a constant that owns its data.
    #[must_use]
    pub fn to_owned(&self) -> Constant<'static> {
        self.clone().into_owned()
    }

    /// Whether this constant takes two stack slots: `long`, `double`, or a dynamic constant of
    /// one of these types.
    #[must_use]
    pub fn is_two_words(&self) -> bool {
        match self {
            Constant::Long(_) | Constant::Double(_) => true,
            Constant::Dynamic(d) => d.get_size() == 2,
            _ => false,
        }
    }
}

impl<'a> From<&'a str> for Constant<'a> {
    fn from(s: &'a str) -> Self {
        Constant::String(Cow::Borrowed(JavaStr::from_str(s)))
    }
}

impl<'a> From<&'a JavaStr> for Constant<'a> {
    fn from(s: &'a JavaStr) -> Self {
        Constant::String(Cow::Borrowed(s))
    }
}

macro_rules! impl_from {
    ($($ty:ty => $variant:ident),* $(,)?) => {$(
        impl From<$ty> for Constant<'_> {
            fn from(v: $ty) -> Self {
                Constant::$variant(v.into())
            }
        }
    )*};
}

impl_from!(i32 => Integer, f32 => Float, i64 => Long, f64 => Double, Handle => Handle);

impl<'a> From<Type<'a>> for Constant<'a> {
    fn from(v: Type<'a>) -> Self {
        Constant::Type(v)
    }
}

impl From<ConstantDynamic> for Constant<'_> {
    fn from(v: ConstantDynamic) -> Self {
        Constant::Dynamic(Box::new(v))
    }
}

/// Java's `floatToIntBits`: the raw bits, with every NaN collapsed to the canonical one.
fn float_bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}

/// Java's `doubleToLongBits`.
fn double_bits(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits()
    }
}

impl PartialEq for Constant<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Constant::Integer(a), Constant::Integer(b)) => a == b,
            (Constant::Float(a), Constant::Float(b)) => float_bits(*a) == float_bits(*b),
            (Constant::Long(a), Constant::Long(b)) => a == b,
            (Constant::Double(a), Constant::Double(b)) => double_bits(*a) == double_bits(*b),
            (Constant::String(a), Constant::String(b)) => a == b,
            (Constant::Type(a), Constant::Type(b)) => a == b,
            (Constant::Handle(a), Constant::Handle(b)) => a == b,
            (Constant::Dynamic(a), Constant::Dynamic(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Constant<'_> {}

impl Hash for Constant<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Constant::Integer(v) => v.hash(state),
            Constant::Float(v) => float_bits(*v).hash(state),
            Constant::Long(v) => v.hash(state),
            Constant::Double(v) => double_bits(*v).hash(state),
            Constant::String(s) => s.hash(state),
            Constant::Type(t) => t.hash(state),
            Constant::Handle(h) => h.hash(state),
            Constant::Dynamic(d) => d.hash(state),
        }
    }
}

/// The value's `toString()` as Java would print it, except that floating point values use Rust's
/// shortest representation (Java's exact `Float.toString` formatting is `oxasm-util`'s concern).
impl fmt::Display for Constant<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constant::Integer(v) => write!(f, "{v}"),
            Constant::Float(v) => write!(f, "{v:?}"),
            Constant::Long(v) => write!(f, "{v}"),
            Constant::Double(v) => write!(f, "{v:?}"),
            Constant::String(s) => write!(f, "{s}"),
            Constant::Type(t) => write!(f, "{t}"),
            Constant::Handle(h) => write!(f, "{h}"),
            Constant::Dynamic(d) => write!(f, "{d}"),
        }
    }
}

/// A value of an annotation element, as passed to `AnnotationVisitor::visit`: a primitive, a
/// `String`, a class [`Type`], or an array of primitives. (Enum values, nested annotations and
/// arrays of those have their own visit methods.)
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationValue<'a> {
    Byte(i8),
    Char(u16),
    Short(i16),
    Boolean(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(Cow<'a, JavaStr>),
    /// A class value: a [`Type`] of sort `OBJECT` or `ARRAY` (or a primitive type, for
    /// `int.class` and the like).
    Type(Type<'a>),
    ByteArray(Cow<'a, [i8]>),
    BooleanArray(Cow<'a, [bool]>),
    CharArray(Cow<'a, [u16]>),
    ShortArray(Cow<'a, [i16]>),
    IntArray(Cow<'a, [i32]>),
    LongArray(Cow<'a, [i64]>),
    FloatArray(Cow<'a, [f32]>),
    DoubleArray(Cow<'a, [f64]>),
}

impl AnnotationValue<'_> {
    /// Converts into a value that owns its data.
    #[must_use]
    pub fn into_owned(self) -> AnnotationValue<'static> {
        fn own<T: Clone>(c: Cow<'_, [T]>) -> Cow<'static, [T]> {
            Cow::Owned(c.into_owned())
        }
        match self {
            AnnotationValue::Byte(v) => AnnotationValue::Byte(v),
            AnnotationValue::Char(v) => AnnotationValue::Char(v),
            AnnotationValue::Short(v) => AnnotationValue::Short(v),
            AnnotationValue::Boolean(v) => AnnotationValue::Boolean(v),
            AnnotationValue::Int(v) => AnnotationValue::Int(v),
            AnnotationValue::Long(v) => AnnotationValue::Long(v),
            AnnotationValue::Float(v) => AnnotationValue::Float(v),
            AnnotationValue::Double(v) => AnnotationValue::Double(v),
            AnnotationValue::String(s) => AnnotationValue::String(Cow::Owned(s.into_owned())),
            AnnotationValue::Type(t) => AnnotationValue::Type(t.into_owned()),
            AnnotationValue::ByteArray(a) => AnnotationValue::ByteArray(own(a)),
            AnnotationValue::BooleanArray(a) => AnnotationValue::BooleanArray(own(a)),
            AnnotationValue::CharArray(a) => AnnotationValue::CharArray(own(a)),
            AnnotationValue::ShortArray(a) => AnnotationValue::ShortArray(own(a)),
            AnnotationValue::IntArray(a) => AnnotationValue::IntArray(own(a)),
            AnnotationValue::LongArray(a) => AnnotationValue::LongArray(own(a)),
            AnnotationValue::FloatArray(a) => AnnotationValue::FloatArray(own(a)),
            AnnotationValue::DoubleArray(a) => AnnotationValue::DoubleArray(own(a)),
        }
    }

    /// Copies into a value that owns its data.
    #[must_use]
    pub fn to_owned(&self) -> AnnotationValue<'static> {
        self.clone().into_owned()
    }
}

impl<'a> From<&'a str> for AnnotationValue<'a> {
    fn from(s: &'a str) -> Self {
        AnnotationValue::String(Cow::Borrowed(JavaStr::from_str(s)))
    }
}

impl<'a> From<&'a JavaStr> for AnnotationValue<'a> {
    fn from(s: &'a JavaStr) -> Self {
        AnnotationValue::String(Cow::Borrowed(s))
    }
}

/// A local variable or operand stack entry of a stack map frame, as passed to
/// `MethodVisitor::visit_frame`. The first seven variants are ASM's `Opcodes.TOP` ...
/// `Opcodes.UNINITIALIZED_THIS`; `Object` is ASM's `String` (an internal name) and
/// `Uninitialized` its `Label` (the `NEW` instruction that created the value).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FrameValue<'a> {
    Top,
    Integer,
    Float,
    Double,
    Long,
    Null,
    UninitializedThis,
    /// An object or array type, as an internal name.
    Object(Cow<'a, JavaStr>),
    /// An uninitialized value, created by the `NEW` instruction just after the label.
    Uninitialized(Label),
}

impl FrameValue<'_> {
    /// Converts into a value that owns its data.
    #[must_use]
    pub fn into_owned(self) -> FrameValue<'static> {
        match self {
            FrameValue::Top => FrameValue::Top,
            FrameValue::Integer => FrameValue::Integer,
            FrameValue::Float => FrameValue::Float,
            FrameValue::Double => FrameValue::Double,
            FrameValue::Long => FrameValue::Long,
            FrameValue::Null => FrameValue::Null,
            FrameValue::UninitializedThis => FrameValue::UninitializedThis,
            FrameValue::Object(s) => FrameValue::Object(Cow::Owned(s.into_owned())),
            FrameValue::Uninitialized(l) => FrameValue::Uninitialized(l),
        }
    }

    /// Copies into a value that owns its data.
    #[must_use]
    pub fn to_owned(&self) -> FrameValue<'static> {
        self.clone().into_owned()
    }
}

impl<'a> From<&'a str> for FrameValue<'a> {
    fn from(internal_name: &'a str) -> Self {
        FrameValue::Object(Cow::Borrowed(JavaStr::from_str(internal_name)))
    }
}

impl<'a> From<&'a JavaStr> for FrameValue<'a> {
    fn from(internal_name: &'a JavaStr) -> Self {
        FrameValue::Object(Cow::Borrowed(internal_name))
    }
}

impl From<Label> for FrameValue<'_> {
    fn from(label: Label) -> Self {
        FrameValue::Uninitialized(label)
    }
}
