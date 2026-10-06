//! Printers, tracers and checkers for [`oxasm`]: a Rust port of ASM's `asm-util` module
//! (`org.objectweb.asm.util`).
//!
//! - [`printer::Printer`] and [`textifier::Textifier`]: convert visit events to text, the latter
//!   to ASM's disassembled view, byte for byte.
//! - [`trace`]: visitors that print what they visit with a printer, such as
//!   [`trace::TraceClassVisitor`].
//! - [`java_fmt`]: Java's `Float.toString` and `Double.toString`.
//!
//! [`oxasm`]: https://docs.rs/oxasm

pub mod java_fmt;
pub mod printer;
pub mod textifier;
pub mod trace;

pub use printer::{Printer, TextNode};
pub use textifier::Textifier;
pub use trace::{
    TraceAnnotationVisitor, TraceClassVisitor, TraceFieldVisitor, TraceMethodVisitor,
    TraceModuleVisitor, TraceRecordComponentVisitor, TraceSignatureVisitor,
};
