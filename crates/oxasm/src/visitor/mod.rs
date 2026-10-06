//! The visitor traits (ADR-02): [`ClassVisitor`], [`MethodVisitor`], [`FieldVisitor`],
//! [`AnnotationVisitor`], [`ModuleVisitor`] and [`RecordComponentVisitor`].
//!
//! # Delegation
//!
//! ASM's visitors are classes with a nullable delegate (`cv`, `mv`, ...) to which every method
//! forwards by default. Here each trait has a [`delegate`](ClassVisitor::delegate) method,
//! returning `None` by default, and every other method has a default implementation that forwards
//! to it. A visitor that only overrides a few methods keeps forwarding the others, exactly like a
//! Java subclass of `ClassVisitor`:
//!
//! ```
//! use oxasm::{ClassVisitor, JavaStr, MethodVisitor};
//!
//! /// Drops every method named `toString`.
//! struct DropToString<V>(V);
//!
//! impl<V: ClassVisitor> ClassVisitor for DropToString<V> {
//!     fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
//!         Some(&mut self.0)
//!     }
//!
//!     fn visit_method<'a>(
//!         &'a mut self,
//!         access: u32,
//!         name: &JavaStr,
//!         descriptor: &JavaStr,
//!         signature: Option<&JavaStr>,
//!         exceptions: &[&JavaStr],
//!     ) -> Option<Box<dyn MethodVisitor + 'a>> {
//!         if name == "toString" {
//!             return None;
//!         }
//!         self.0.visit_method(access, name, descriptor, signature, exceptions)
//!     }
//! }
//! ```
//!
//! `delegate()` must return the *same* downstream visitor every time (Java's `getDelegate()` has
//! the same contract).
//!
//! # Child visitors
//!
//! Methods that start a nested structure (`visit_method`, `visit_annotation`, ...) return
//! `Option<Box<dyn XVisitor + 'a>>`: `None` when the visitor is not interested (which lets a
//! reader skip the content), or a child visitor that borrows its parent mutably until it is
//! dropped. Producers drive one child at a time, finishing (and dropping) it before calling the
//! parent again. An adapter wraps its downstream's child through disjoint field borrows:
//!
//! ```ignore
//! let mv = self.next.visit_method(access, name, descriptor, signature, exceptions)?;
//! Some(Box::new(MethodRemapper::new(mv, &self.remapper)))
//! ```
//!
//! One consequence of the borrow: while a child is alive, its parent cannot be visited. A
//! transformation that must emit a new method *while* another one is being visited buffers it
//! (e.g. in a tree node) and emits it afterwards, typically from `visit_end`.
//!
//! # Arguments
//!
//! Arguments are borrowed for the duration of the call; a visitor that keeps a value copies it
//! (`to_owned`, `into_owned`). Strings are [`JavaStr`](crate::jstr::JavaStr) (ADR-04); Java's
//! `Object` arguments are the enums of [`crate::constant`] (ADR-05); integers have the width of
//! their class file field (a local variable index is a `u16`, an `iinc` increment an `i16`).
//! Java's `null` arrays are empty slices.
//!
//! `&mut V` and `Box<V>` are visitors whenever `V` is, forwarding *every* method (not only
//! `delegate`) to `V`, so `&mut dyn ClassVisitor` and boxed visitors work wherever a visitor is
//! expected.

mod annotation;
mod class;
mod field;
mod method;
mod module;
mod record_component;

pub use annotation::AnnotationVisitor;
pub use class::ClassVisitor;
pub use field::FieldVisitor;
#[doc(hidden)]
pub use method::MethodCopySource;
pub use method::MethodVisitor;
pub use module::ModuleVisitor;
pub use record_component::RecordComponentVisitor;

#[cfg(test)]
mod tests;
