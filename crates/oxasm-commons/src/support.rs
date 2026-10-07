//! Helpers shared by the adapters: the panics that stand for ASM's exceptions (ADR-06).

#![allow(dead_code)] // each adapter uses part of it

use std::fmt::Display;

use oxasm::{JavaStr, Type};

/// Panics as ASM throws an `IllegalArgumentException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn illegal_argument(message: impl Display) -> ! {
    panic!("java.lang.IllegalArgumentException: {message}")
}

/// Panics as ASM throws an `IllegalStateException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn illegal_state(message: impl Display) -> ! {
    panic!("java.lang.IllegalStateException: {message}")
}

/// Panics as ASM throws an `UnsupportedOperationException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn unsupported_operation(message: impl Display) -> ! {
    panic!("java.lang.UnsupportedOperationException: {message}")
}

/// ASM's `Type.getObjectType`, which throws on an empty internal name (its `charAt(0)`), where
/// oxasm's [`Type::get_object_type`] returns an object type with an empty name.
#[track_caller]
pub(crate) fn object_type(internal_name: &JavaStr) -> Type {
    if internal_name.is_empty() {
        panic!("java.lang.StringIndexOutOfBoundsException: Index 0 out of bounds for length 0");
    }
    Type::get_object_type(internal_name)
}
