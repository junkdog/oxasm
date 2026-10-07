//! The class writer (ASM's `ClassWriter` and the package-private classes behind it): a
//! [`ClassVisitor`](crate::ClassVisitor) that generates a class file.
//!
//! Ownership follows ADR-08: the [`ClassWriter`] owns the symbol table and the state of every
//! field, method, record component and module it writes; the visitors it hands out
//! (`visit_method`, `visit_field`, ...) borrow the symbol table and their own state.

mod annotation_writer;
mod attributes;
mod byte_vector;
mod class_writer;
mod field_writer;
mod frame;
mod handler;
mod label_table;
mod method_writer;
mod module_writer;
mod record_component_writer;
mod symbol_table;

pub use attributes::AttributeWriteContext;
pub use byte_vector::ByteVector;
pub use class_writer::{ClassWriter, ClassWriterFlags};
