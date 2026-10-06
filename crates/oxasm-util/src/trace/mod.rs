//! The tracing visitors: visitors that print the events they receive with a [`Printer`]
//! (ASM's `Trace*Visitor`s), and [`TraceSignatureVisitor`].
//!
//! Each tracing visitor forwards every event to its printer, then to an optional downstream
//! visitor. Unlike most visitors, it always returns a (tracing) child visitor, whether or not the
//! downstream visitor returns one, so that everything is printed. A child's printer is the one
//! that the parent's printer returns for the nested structure, borrowed from it.
//!
//! [`Printer`]: crate::printer::Printer

mod annotation;
mod class;
mod field;
mod method;
mod module;
mod record_component;
mod signature;

pub use annotation::TraceAnnotationVisitor;
pub use class::{NoDelegate, TraceClassVisitor};
pub use field::TraceFieldVisitor;
pub use method::TraceMethodVisitor;
pub use module::TraceModuleVisitor;
pub use record_component::TraceRecordComponentVisitor;
pub use signature::TraceSignatureVisitor;
