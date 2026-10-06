//! Non standard class, field, method and code attributes (ADR-10): what the `visit_attribute`
//! methods of the visitor traits carry.
//!
//! A class reader decodes an attribute with the [`AttributeCodec`] registered for its name (ASM's
//! attribute "prototypes"), or keeps it as an [`UnknownAttribute`] otherwise. The write half of
//! ADR-10 (`write`, the write context) lands with the class writer (P5).

use std::any::Any;
use std::fmt::Debug;

use dyn_clone::DynClone;

use crate::error::ReadError;
use crate::jstr::{JavaStr, JavaString};
use crate::label::Label;
use crate::reader::AttributeReadContext;

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

    /// A human readable representation of this attribute, which the Textifier prints after the
    /// attribute name (ASM's `TextifierSupport.textify`); `None` (the default) prints it as
    /// `unknown`. `label_name` returns the Textifier's name of a label, naming it first if it
    /// has none yet, as the `labelNames` map passed to Java's `textify` does.
    fn textify(&self, _label_name: &mut dyn FnMut(Label) -> String) -> Option<String> {
        None
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

/// A decoder of one kind of non standard attribute (ASM's attribute "prototype", i.e. the
/// `Attribute` instance passed to `ClassReader.accept` whose `read` method is a factory).
pub trait AttributeCodec: Send + Sync {
    /// The name of the attributes this codec decodes.
    fn name(&self) -> &JavaStr;

    /// Whether the attributes this codec decodes are Code attributes.
    fn is_code_attribute(&self) -> bool {
        false
    }

    /// Decodes the attribute at `cx.offset()` (ASM's `Attribute.read`).
    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError>;
}
