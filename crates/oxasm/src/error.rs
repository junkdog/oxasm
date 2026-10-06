//! The error types of this crate (ADR-06), in one place.
//!
//! Reading and writing errors (`ReadError`, `WriteError`) arrive with the class reader (P2) and
//! the class writer (P5).

pub use crate::hierarchy::{HierarchyError, MalformedHierarchy};
pub use crate::jstr::Utf8Error;
pub use crate::signature::SignatureError;
pub use crate::ty::InvalidDescriptor;
pub use crate::type_path::InvalidTypePath;
