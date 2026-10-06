//! Generic signatures (ASM's `org.objectweb.asm.signature` package): the [`SignatureReader`]
//! parser, the [`SignatureVisitor`] trait, and the [`SignatureWriter`] generator.
//!
//! See [JVMS 4.7.9.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7.9.1).

mod reader;
mod visitor;
mod writer;

pub use reader::{SignatureError, SignatureReader};
pub use visitor::{AsDynSignatureVisitor, EXTENDS, INSTANCEOF, SUPER, SignatureVisitor};
pub use writer::SignatureWriter;
