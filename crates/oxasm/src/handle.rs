//! [`Handle`]: a reference to a field or a method.

use std::fmt;

use crate::jstr::{JavaStr, JavaString};
use crate::opcodes::HandleKind;

/// A reference to a field or a method (a `CONSTANT_MethodHandle_info`).
///
/// ASM's deprecated 4-argument constructor, which derived `is_interface` from the tag, is not
/// ported.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    /// The kind of field or method designated by this handle.
    tag: HandleKind,
    /// The internal name of the class that owns the field or method designated by this handle.
    owner: JavaString,
    /// The name of the field or method designated by this handle.
    name: JavaString,
    /// The descriptor of the field or method designated by this handle.
    descriptor: JavaString,
    /// Whether the owner is an interface or not.
    is_interface: bool,
}

impl Handle {
    /// Constructs a new field or method handle.
    ///
    /// - `owner`: the internal name of the class that owns the field or method designated by this
    ///   handle (see [`Type::get_internal_name`](crate::Type::get_internal_name)).
    /// - `name`: the name of the field or method designated by this handle.
    /// - `descriptor`: the descriptor of the field or method designated by this handle.
    /// - `is_interface`: whether the owner is an interface or not.
    #[must_use]
    pub fn new(
        tag: HandleKind,
        owner: impl Into<JavaString>,
        name: impl Into<JavaString>,
        descriptor: impl Into<JavaString>,
        is_interface: bool,
    ) -> Handle {
        Handle {
            tag,
            owner: owner.into(),
            name: name.into(),
            descriptor: descriptor.into(),
            is_interface,
        }
    }

    /// Returns the kind of field or method designated by this handle.
    #[inline]
    #[must_use]
    pub fn get_tag(&self) -> HandleKind {
        self.tag
    }

    /// Returns the internal name of the class that owns the field or method designated by this
    /// handle.
    #[inline]
    #[must_use]
    pub fn get_owner(&self) -> &JavaStr {
        &self.owner
    }

    /// Returns the name of the field or method designated by this handle.
    #[inline]
    #[must_use]
    pub fn get_name(&self) -> &JavaStr {
        &self.name
    }

    /// Returns the descriptor of the field or method designated by this handle.
    #[inline]
    #[must_use]
    pub fn get_desc(&self) -> &JavaStr {
        &self.descriptor
    }

    /// Returns true if the owner of the field or method designated by this handle is an
    /// interface.
    #[inline]
    #[must_use]
    pub fn is_interface(&self) -> bool {
        self.is_interface
    }
}

/// The textual representation of this handle: `owner "." name descriptor " (" tag ")"`, or
/// `owner "." name descriptor " (" tag " itf)"` for a reference to an interface (the tag in
/// decimal).
impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let itf = if self.is_interface { " itf" } else { "" };
        write!(
            f,
            "{}.{}{} ({}{itf})",
            self.owner,
            self.name,
            self.descriptor,
            self.tag.tag()
        )
    }
}
