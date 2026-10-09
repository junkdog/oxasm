//! Data-flow analysis over `oxasm-tree` methods: a Rust port of ASM's `asm-analysis` module
//! (`org.objectweb.asm.tree.analysis`).
//!
//! An [`Analyzer`] computes the symbolic [`Frame`] at each instruction of a
//! [`MethodNode`](oxasm_tree::MethodNode), with an [`Interpreter`] that gives the values their
//! meaning: [`BasicInterpreter`] (seven basic types), [`BasicVerifier`] and [`SimpleVerifier`]
//! (which also check that instructions get the values they expect), and [`SourceInterpreter`]
//! (which instructions produce each value). The design is ADR-12: generic interpreters and
//! values, ASM's overridable analyzer methods as [`AnalyzerHooks`], and stock interpreters that
//! other interpreters extend by delegation.

// ASM's frame and interpreter code is the spec: its int arithmetic and casts are kept.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

mod analyzer;
mod basic_interpreter;
mod basic_value;
mod basic_verifier;
mod error;
mod frame;
mod frames;
mod interpreter;
mod simple_verifier;
mod small_set;
mod source_interpreter;
mod source_value;
mod subroutine;
mod value;

pub use analyzer::{Analyzer, AnalyzerHooks};
pub use basic_interpreter::BasicInterpreter;
pub use basic_value::BasicValue;
pub use basic_verifier::{BasicVerifier, Verifier};
pub use error::AnalyzerError;
pub use frame::{Frame, FrameRef};
pub use frames::{Frames, FramesIter};
pub use interpreter::{InsnRef, Interpreter};
pub use simple_verifier::SimpleVerifier;
pub use small_set::SmallSet;
pub use source_interpreter::SourceInterpreter;
pub use source_value::{Producer, SourceValue};
pub use value::Value;
