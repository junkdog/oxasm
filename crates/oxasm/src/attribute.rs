//! Non standard class, field, method and code attributes (ADR-10): what the `visit_attribute`
//! methods of the visitor traits carry.
//!
//! This is the P1 cut: the [`Attribute`] value and the [`CustomAttribute`] trait as visitors see
//! them. The codec half of ADR-10 (`AttributeCodec`, read and write contexts) lands with the
//! class reader (P2) and writer (P5), as defaulted additions to these types.

use std::any::Any;
use std::fmt::Debug;

use dyn_clone::DynClone;

use crate::jstr::{JavaStr, JavaString};
use crate::label::Label;

/// A non standard attribute.
#[derive(Clone, Debug)]
pub enum Attribute {
    /// An attribute no codec was registered for: its name and raw content (ASM's `Attribute`
    /// read without a matching prototype, for which `isUnknown()` is true).
    Unknown(UnknownAttribute),
    /// An attribute decoded by a registered codec, or created by the user.
    Custom(Box<dyn CustomAttribute>),
}

impl Attribute {
    /// The name of this attribute.
    #[must_use]
    pub fn name(&self) -> &JavaStr {
        match self {
            Attribute::Unknown(a) => &a.name,
            Attribute::Custom(a) => a.name(),
        }
    }

    /// Whether this attribute is unknown, i.e. read without a codec (ASM's `isUnknown`).
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Attribute::Unknown(_))
    }

    /// Whether this attribute is a Code attribute, i.e. an attribute of a method's `Code`
    /// attribute (ASM's `isCodeAttribute`). Unknown attributes never are.
    #[must_use]
    pub fn is_code_attribute(&self) -> bool {
        match self {
            Attribute::Unknown(_) => false,
            Attribute::Custom(a) => a.is_code_attribute(),
        }
    }

    /// The labels of this attribute, if it is a Code attribute that refers to bytecode offsets
    /// (ASM's `getLabels`).
    #[must_use]
    pub fn labels(&self) -> Vec<Label> {
        match self {
            Attribute::Unknown(_) => Vec::new(),
            Attribute::Custom(a) => a.labels(),
        }
    }
}

/// The name and raw content of an attribute that was read without a codec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownAttribute {
    /// The attribute name.
    pub name: JavaString,
    /// The attribute content (the bytes after `attribute_length`).
    pub content: Vec<u8>,
}

/// A decoded non standard attribute (the "subclass of `Attribute`" of ASM).
///
/// Implementations are `Clone` (via [`DynClone`]) and `Any`, so a visitor can recognize a
/// specific attribute type with `downcast_ref` (on `dyn CustomAttribute`).
pub trait CustomAttribute: Any + Debug + Send + Sync + DynClone {
    /// The attribute name.
    fn name(&self) -> &JavaStr;

    /// Whether this is a Code attribute, i.e. an attribute of a method's `Code` attribute.
    fn is_code_attribute(&self) -> bool {
        false
    }

    /// The labels corresponding to this attribute's bytecode offsets, if any.
    fn labels(&self) -> Vec<Label> {
        Vec::new()
    }
}

dyn_clone::clone_trait_object!(CustomAttribute);

impl dyn CustomAttribute {
    /// This attribute as a `T`, if it is one.
    #[must_use]
    pub fn downcast_ref<T: CustomAttribute>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}
