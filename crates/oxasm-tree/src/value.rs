//! The values a tree stores with interned strings: [`SymConstant`], [`SymHandle`],
//! [`SymConstantDynamic`] and [`SymFrameValue`], the counterparts of [`Constant`], [`Handle`],
//! [`ConstantDynamic`] and [`FrameValue`] (ADR-04's P9 amendment).
//!
//! The visitor traits pass the [`oxasm`] values, with borrowed strings; a node interns them on
//! visit ([`SymConstant::intern`], ...) and resolves them on `accept` ([`SymConstant::resolve`],
//! ...). A class type constant is stored as the [`Sym`] of its descriptor, which determines it
//! (ADR-15), and a method type constant as that of its method descriptor, kept as is.

use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use oxasm::constant::{Constant, FrameValue};
use oxasm::{ConstantDynamic, Handle, HandleKind, Interner, Label, Sym, Type, TypeSort};

use crate::strings::Strings;

/// A reference to a field or a method (a [`Handle`] whose strings are interned).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymHandle {
    /// The kind of field or method designated by this handle.
    pub tag: HandleKind,
    /// The internal name of the class that owns the field or method.
    pub owner: Sym,
    /// The name of the field or method.
    pub name: Sym,
    /// The descriptor of the field or method.
    pub desc: Sym,
    /// Whether the owner is an interface.
    pub is_interface: bool,
}

impl SymHandle {
    /// The given handle, with its strings interned.
    pub fn intern(handle: &Handle, interner: &Interner) -> SymHandle {
        SymHandle::from_visited(handle, &mut Strings::Direct(interner))
    }

    pub(crate) fn from_visited(handle: &Handle, strings: &mut Strings<'_>) -> SymHandle {
        SymHandle {
            tag: handle.get_tag(),
            owner: strings.get(handle.get_owner()),
            name: strings.get(handle.get_name()),
            desc: strings.get(handle.get_desc()),
            is_interface: handle.is_interface(),
        }
    }

    /// This handle, with its strings resolved.
    pub fn resolve(&self, interner: &Interner) -> Handle {
        Handle::new(
            self.tag,
            interner.resolve(self.owner),
            interner.resolve(self.name),
            interner.resolve(self.desc),
            self.is_interface,
        )
    }
}

/// A constant whose value is computed at runtime, with a bootstrap method (a [`ConstantDynamic`]
/// whose strings are interned).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymConstantDynamic {
    /// The constant name.
    pub name: Sym,
    /// The constant type, a field descriptor.
    pub desc: Sym,
    /// The bootstrap method that computes the constant value.
    pub bsm: SymHandle,
    /// The arguments of the bootstrap method.
    pub bsm_args: Vec<SymConstant>,
}

impl SymConstantDynamic {
    /// The given constant, with its strings interned.
    pub fn intern(constant: &ConstantDynamic, interner: &Interner) -> SymConstantDynamic {
        SymConstantDynamic::from_visited(constant, &mut Strings::Direct(interner))
    }

    pub(crate) fn from_visited(
        constant: &ConstantDynamic,
        strings: &mut Strings<'_>,
    ) -> SymConstantDynamic {
        SymConstantDynamic {
            name: strings.get(constant.get_name()),
            desc: strings.get(constant.get_descriptor()),
            bsm: SymHandle::from_visited(constant.get_bootstrap_method(), strings),
            bsm_args: constant
                .get_bootstrap_method_arguments()
                .iter()
                .map(|arg| SymConstant::from_visited(arg, strings))
                .collect(),
        }
    }

    /// This constant, with its strings resolved.
    pub fn resolve(&self, interner: &Interner) -> ConstantDynamic {
        ConstantDynamic::new(
            interner.resolve(self.name),
            interner.resolve(self.desc),
            self.bsm.resolve(interner),
            self.bsm_args
                .iter()
                .map(|arg| arg.resolve(interner).into_owned())
                .collect(),
        )
    }

    /// The size of this constant's values: 2 for a `long` or `double`, 1 otherwise.
    pub fn get_size(&self, interner: &Interner) -> usize {
        match interner.resolve(self.desc).as_bytes().first() {
            Some(b'J' | b'D') => 2,
            _ => 1,
        }
    }
}

/// A constant pool value (a [`Constant`] whose strings are interned): the operand of `LDC`, the
/// initial value of a field, or a bootstrap method argument.
///
/// Floating point values compare and hash like Java's `Float.equals`/`Double.equals`, as
/// [`Constant`]'s do.
#[derive(Clone, Debug)]
pub enum SymConstant {
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    String(Sym),
    /// A class type constant (a [`Type`] of sort `OBJECT` or `ARRAY`), as its descriptor.
    Type(Sym),
    /// A method type constant (a [`Type`] of sort `METHOD`), as its method descriptor. It is kept
    /// apart from [`Type`](Self::Type) because `Type::get_method_type` keeps any string as is,
    /// which `Type::get_type` would not give back (an empty `CONSTANT_MethodType`, say).
    MethodType(Sym),
    Handle(SymHandle),
    Dynamic(Box<SymConstantDynamic>),
}

impl SymConstant {
    /// The given constant, with its strings interned.
    pub fn intern(constant: &Constant<'_>, interner: &Interner) -> SymConstant {
        SymConstant::from_visited(constant, &mut Strings::Direct(interner))
    }

    pub(crate) fn from_visited(constant: &Constant<'_>, strings: &mut Strings<'_>) -> SymConstant {
        match constant {
            Constant::Integer(v) => SymConstant::Integer(*v),
            Constant::Float(v) => SymConstant::Float(*v),
            Constant::Long(v) => SymConstant::Long(*v),
            Constant::Double(v) => SymConstant::Double(*v),
            Constant::String(s) => SymConstant::String(strings.get(s)),
            Constant::Type(t) if t.get_sort() == TypeSort::Method => {
                SymConstant::MethodType(strings.get(&t.get_descriptor()))
            }
            Constant::Type(t) => SymConstant::Type(strings.get(&t.get_descriptor())),
            Constant::Handle(h) => SymConstant::Handle(SymHandle::from_visited(h, strings)),
            Constant::Dynamic(d) => {
                SymConstant::Dynamic(Box::new(SymConstantDynamic::from_visited(d, strings)))
            }
        }
    }

    /// This constant, with its strings resolved (a string borrows from the interner).
    pub fn resolve<'i>(&self, interner: &'i Interner) -> Constant<'i> {
        match self {
            SymConstant::Integer(v) => Constant::Integer(*v),
            SymConstant::Float(v) => Constant::Float(*v),
            SymConstant::Long(v) => Constant::Long(*v),
            SymConstant::Double(v) => Constant::Double(*v),
            SymConstant::String(s) => Constant::String(Cow::Borrowed(interner.resolve(*s))),
            SymConstant::Type(t) => Constant::Type(resolve_type(interner, *t)),
            SymConstant::MethodType(t) => {
                Constant::Type(Type::get_method_type(interner.resolve(*t)))
            }
            SymConstant::Handle(h) => Constant::Handle(h.resolve(interner)),
            SymConstant::Dynamic(d) => Constant::Dynamic(Box::new(d.resolve(interner))),
        }
    }

    /// Whether this constant takes two stack slots: `long`, `double`, or a dynamic constant of
    /// one of these types.
    pub fn is_two_words(&self, interner: &Interner) -> bool {
        match self {
            SymConstant::Long(_) | SymConstant::Double(_) => true,
            SymConstant::Dynamic(d) => d.get_size(interner) == 2,
            _ => false,
        }
    }
}

/// The type of a stored descriptor (of a class type constant, or a class annotation value). The
/// descriptor of a type that isn't a method type gives it back: `Type::get_type` takes the whole
/// descriptor as the type's value, without parsing it further, so even a malformed internal name
/// (whose descriptor is `L<name>;`) comes back intact.
pub(crate) fn resolve_type(interner: &Interner, descriptor: Sym) -> Type {
    Type::get_type(interner.resolve(descriptor))
}

/// Java's `floatToIntBits`.
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

impl PartialEq for SymConstant {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SymConstant::Integer(a), SymConstant::Integer(b)) => a == b,
            (SymConstant::Float(a), SymConstant::Float(b)) => float_bits(*a) == float_bits(*b),
            (SymConstant::Long(a), SymConstant::Long(b)) => a == b,
            (SymConstant::Double(a), SymConstant::Double(b)) => double_bits(*a) == double_bits(*b),
            (SymConstant::String(a), SymConstant::String(b))
            | (SymConstant::Type(a), SymConstant::Type(b))
            | (SymConstant::MethodType(a), SymConstant::MethodType(b)) => a == b,
            (SymConstant::Handle(a), SymConstant::Handle(b)) => a == b,
            (SymConstant::Dynamic(a), SymConstant::Dynamic(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for SymConstant {}

impl Hash for SymConstant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            SymConstant::Integer(v) => v.hash(state),
            SymConstant::Float(v) => float_bits(*v).hash(state),
            SymConstant::Long(v) => v.hash(state),
            SymConstant::Double(v) => double_bits(*v).hash(state),
            SymConstant::String(s) | SymConstant::Type(s) | SymConstant::MethodType(s) => {
                s.hash(state);
            }
            SymConstant::Handle(h) => h.hash(state),
            SymConstant::Dynamic(d) => d.hash(state),
        }
    }
}

impl From<i32> for SymConstant {
    fn from(v: i32) -> Self {
        SymConstant::Integer(v)
    }
}

impl From<f32> for SymConstant {
    fn from(v: f32) -> Self {
        SymConstant::Float(v)
    }
}

impl From<i64> for SymConstant {
    fn from(v: i64) -> Self {
        SymConstant::Long(v)
    }
}

impl From<f64> for SymConstant {
    fn from(v: f64) -> Self {
        SymConstant::Double(v)
    }
}

impl From<SymHandle> for SymConstant {
    fn from(v: SymHandle) -> Self {
        SymConstant::Handle(v)
    }
}

impl From<SymConstantDynamic> for SymConstant {
    fn from(v: SymConstantDynamic) -> Self {
        SymConstant::Dynamic(Box::new(v))
    }
}

/// A local variable or operand stack entry of a stack map frame (a [`FrameValue`] whose internal
/// name is interned).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymFrameValue {
    Top,
    Integer,
    Float,
    Double,
    Long,
    Null,
    UninitializedThis,
    /// An object or array type, as an internal name.
    Object(Sym),
    /// An uninitialized value, created by the `NEW` instruction just after the label.
    Uninitialized(Label),
}

impl SymFrameValue {
    /// The given value, with its internal name interned.
    pub fn intern(value: &FrameValue<'_>, interner: &Interner) -> SymFrameValue {
        SymFrameValue::from_visited(value, &mut Strings::Direct(interner))
    }

    pub(crate) fn from_visited(value: &FrameValue<'_>, strings: &mut Strings<'_>) -> SymFrameValue {
        match value {
            FrameValue::Top => SymFrameValue::Top,
            FrameValue::Integer => SymFrameValue::Integer,
            FrameValue::Float => SymFrameValue::Float,
            FrameValue::Double => SymFrameValue::Double,
            FrameValue::Long => SymFrameValue::Long,
            FrameValue::Null => SymFrameValue::Null,
            FrameValue::UninitializedThis => SymFrameValue::UninitializedThis,
            FrameValue::Object(s) => SymFrameValue::Object(strings.get(s)),
            FrameValue::Uninitialized(label) => SymFrameValue::Uninitialized(*label),
        }
    }

    /// This value, with its internal name resolved (borrowed from the interner).
    pub fn resolve<'i>(&self, interner: &'i Interner) -> FrameValue<'i> {
        match self {
            SymFrameValue::Top => FrameValue::Top,
            SymFrameValue::Integer => FrameValue::Integer,
            SymFrameValue::Float => FrameValue::Float,
            SymFrameValue::Double => FrameValue::Double,
            SymFrameValue::Long => FrameValue::Long,
            SymFrameValue::Null => FrameValue::Null,
            SymFrameValue::UninitializedThis => FrameValue::UninitializedThis,
            SymFrameValue::Object(s) => FrameValue::Object(Cow::Borrowed(interner.resolve(*s))),
            SymFrameValue::Uninitialized(label) => FrameValue::Uninitialized(*label),
        }
    }
}

impl From<Label> for SymFrameValue {
    fn from(label: Label) -> Self {
        SymFrameValue::Uninitialized(label)
    }
}

impl From<Sym> for SymFrameValue {
    fn from(internal_name: Sym) -> Self {
        SymFrameValue::Object(internal_name)
    }
}
