//! Visitors that check that their methods are properly used: ASM's `Check*Adapter`s and
//! `CheckFrameAnalyzer`.
//!
//! - [`CheckClassAdapter`] checks a class, with a [`CheckMethodAdapter`] for each method (with
//!   basic data flow checks by default), and so on for the other members. [`verify`] checks a
//!   class file with a [`SimpleVerifier`](oxasm_analysis::SimpleVerifier).
//! - [`CheckSignatureAdapter`] checks the events of a signature visitor;
//!   [`check_class_signature`], [`check_method_signature`] and [`check_field_signature`] check
//!   signature strings.
//! - [`CheckFrameAnalyzer`] checks the stack map frames of a method.
//!
//! # Panics
//!
//! The check adapters are assertion tools: where ASM's throw an exception, they **panic**
//! (ADR-06), with the `toString()` of ASM's exception as the message, such as
//! `java.lang.IllegalArgumentException: Invalid descriptor: I;`, or
//! `java.lang.IllegalStateException: Cannot visit instructions before visitCode has been called.`
//!
//! The checks that ASM makes on arguments that Rust's types rule out (a `null` name or label, a
//! local variable index that is not an unsigned short, an unknown handle tag, ...) have no
//! counterpart.
//!
//! # Divergences
//!
//! - Below Java 5, ASM checks names with `Character.isJavaIdentifierStart` and
//!   `isJavaIdentifierPart`, whose results depend on the Unicode version of the JVM: here they
//!   are those of Java 25 (Unicode 16).
//! - ASM's signature checks recurse once per nested type argument, without bound: here, type
//!   arguments may nest 256 levels, as `SignatureReader`'s do (ADR-06), and array types are
//!   checked iteratively.

mod annotation;
mod class;
mod field;
mod frame_analyzer;
mod java_chars;
mod method;
mod module;
mod signature;
mod support;

pub use annotation::CheckAnnotationAdapter;
pub use class::{CheckClassAdapter, main, verify};
pub use field::{CheckFieldAdapter, CheckRecordComponentAdapter};
pub use frame_analyzer::{CheckFrameAnalyzer, CheckFrameHooks, check_frame_analyzer};
pub use method::CheckMethodAdapter;
pub use module::CheckModuleAdapter;
pub use signature::{CheckSignatureAdapter, SignatureKind};
pub use support::{check_class_signature, check_field_signature, check_method_signature};
