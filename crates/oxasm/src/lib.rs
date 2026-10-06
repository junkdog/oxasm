//! JVM class file reading, writing and transformation: a Rust port of [OW2 ASM] 9.10.1.
//!
//! This crate corresponds to ASM's `asm` module (`org.objectweb.asm` and
//! `org.objectweb.asm.signature`): the class reader and writer, the visitor traits, `Type`,
//! `Handle` and the signature parser.
//!
//! [OW2 ASM]: https://asm.ow2.io/

pub mod attribute;
pub mod constant;
pub mod constant_dynamic;
pub mod error;
pub mod handle;
pub mod hierarchy;
pub mod jstr;
pub mod label;
pub mod opcodes;
pub mod signature;
pub mod ty;
pub mod type_path;
pub mod type_reference;
pub mod visitor;

pub(crate) mod constants;
#[cfg(test)]
mod constants_test;

pub use attribute::{Attribute, CustomAttribute, UnknownAttribute};
pub use constant::{AnnotationValue, Constant, FrameValue};
pub use constant_dynamic::ConstantDynamic;
pub use handle::Handle;
pub use hierarchy::{ChainedHierarchy, ClassHierarchy, MapHierarchy, ObjectHierarchy};
pub use jstr::{JavaStr, JavaString};
pub use label::Label;
pub use opcodes::{FrameKind, HandleKind};
pub use ty::{Type, TypeSort};
pub use type_path::TypePath;
pub use type_reference::TypeReference;
pub use visitor::{
    AnnotationVisitor, ClassVisitor, FieldVisitor, MethodVisitor, ModuleVisitor,
    RecordComponentVisitor,
};
