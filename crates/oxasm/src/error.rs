//! The error types of this crate (ADR-06), in one place.
//!
use thiserror::Error;

use crate::jstr::JavaString;

pub use crate::hierarchy::{HierarchyError, MalformedHierarchy};
pub use crate::jstr::Utf8Error;
pub use crate::signature::SignatureError;
pub use crate::ty::InvalidDescriptor;
pub use crate::type_path::InvalidTypePath;

/// A malformed or unsupported class file: what ASM's `ClassReader` throws as an
/// `IllegalArgumentException` or an `ArrayIndexOutOfBoundsException`.
///
/// When [`ClassReader::accept`](crate::ClassReader::accept) fails, the visitor has seen a prefix
/// of the events of the class (as in ASM, which throws in the middle of a visit).
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{kind} (at offset {offset})")]
pub struct ReadError {
    /// The offset in the class file of the structure that could not be read.
    pub offset: usize,
    /// What is wrong.
    pub kind: ReadErrorKind,
}

impl ReadError {
    /// An error at the given offset, as computed by the reader: a negative offset (which a
    /// malformed class can produce) is reported as 0.
    pub(crate) fn new(offset: i64, kind: ReadErrorKind) -> ReadError {
        ReadError {
            offset: usize::try_from(offset).unwrap_or(0),
            kind,
        }
    }
}

/// What is wrong with a class file that a [`ReadError`] reports.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ReadErrorKind {
    /// A structure extends past the end of the class file.
    #[error("unexpected end of class file")]
    UnexpectedEnd,
    /// The class file major version is newer than the supported one (`V27`).
    #[error("unsupported class file major version {0}")]
    UnsupportedVersion(i16),
    /// A constant pool entry has an unknown tag.
    #[error("invalid constant pool tag {0}")]
    InvalidConstantPoolTag(u8),
    /// A constant pool index is out of the constant pool.
    #[error("invalid constant pool index {0}")]
    InvalidConstantPoolIndex(u16),
    /// A constant pool entry does not have the expected kind (e.g. a `ldc` of a `CONSTANT_Utf8`).
    #[error("invalid constant pool entry {0}")]
    InvalidConstant(u16),
    /// A bytecode offset is outside of the method code.
    #[error("invalid bytecode offset {0}")]
    InvalidBytecodeOffset(i64),
    /// An instruction has an unknown opcode.
    #[error("invalid opcode {0}")]
    InvalidOpcode(u8),
    /// A `wide` instruction has an opcode that cannot be widened.
    #[error("invalid wide opcode {0}")]
    InvalidWideOpcode(u8),
    /// An annotation element value has an unknown tag.
    #[error("invalid element value tag {0}")]
    InvalidElementValueTag(u8),
    /// A type annotation has an unknown or misplaced target type.
    #[error("invalid type annotation target type {0:#x}")]
    InvalidTypeAnnotationTarget(u8),
    /// A stack map frame has an unknown frame type.
    #[error("invalid stack map frame type {0}")]
    InvalidFrameType(u8),
    /// A stack map frame has an unknown verification type tag.
    #[error("invalid verification type tag {0}")]
    InvalidVerificationType(u8),
    /// The constant pool has `CONSTANT_Dynamic` or `CONSTANT_InvokeDynamic` entries, but the class
    /// has no `BootstrapMethods` attribute.
    #[error("missing BootstrapMethods attribute")]
    MissingBootstrapMethods,
    /// A method handle has a reference kind outside of 1 to 9 (ADR-09).
    #[error("invalid method handle reference kind {0}")]
    InvalidHandleKind(u8),
    /// A length or count is invalid (e.g. a `SourceDebugExtension` longer than the class file).
    #[error("invalid length {0}")]
    InvalidLength(i64),
    /// The input nests deeper than oxasm's bound (ADR-06), where ASM 9.10.1 would overflow its
    /// stack.
    #[error("recursion limit exceeded")]
    RecursionLimit,
    /// A custom attribute codec rejected its attribute.
    #[error("invalid attribute: {0}")]
    InvalidAttribute(String),
}

/// An error that prevents a [`ClassWriter`](crate::ClassWriter) from producing a class file: what
/// ASM's `ClassWriter`, its method writers and its symbol table throw.
///
/// The visitor methods of a class writer cannot fail (ADR-06): an error ASM throws in the middle of
/// a visit is recorded where ASM throws it, the visit goes on, and
/// [`ClassWriter::to_bytes`](crate::ClassWriter::to_bytes) returns the first recorded error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WriteError {
    /// The constant pool of the class has more than 65535 entries (ASM's
    /// `ClassTooLargeException`).
    #[error("Class too large: {class_name}")]
    ClassTooLarge {
        /// The internal name of the class.
        class_name: JavaString,
        /// The number of constant pool items of the class (plus 1).
        constant_pool_count: usize,
    },
    /// The Code attribute of a method has more than 65535 bytes of bytecode (ASM's
    /// `MethodTooLargeException`).
    #[error("Method too large: {class_name}.{method_name} {descriptor}")]
    MethodTooLarge {
        /// The internal name of the owner class.
        class_name: JavaString,
        /// The name of the method.
        method_name: JavaString,
        /// The descriptor of the method.
        descriptor: JavaString,
        /// The size of the method's Code attribute, in bytes.
        code_size: usize,
    },
    /// A string has more than 65535 UTF-16 units, or its modified UTF-8 encoding has more than
    /// 65535 bytes.
    #[error("UTF8 string too large")]
    Utf8TooLarge,
    /// An invalid argument was passed to a visitor method (ASM's `IllegalArgumentException`).
    #[error("{0}")]
    InvalidArgument(String),
    /// A visitor method was called in an invalid state (ASM's `IllegalStateException`).
    #[error("{0}")]
    IllegalState(String),
    /// An index is out of bounds: ASM's `ArrayIndexOutOfBoundsException` (e.g. a parameter
    /// annotation for a parameter the method descriptor does not have) and
    /// `StringIndexOutOfBoundsException` (e.g. an empty descriptor).
    #[error("{0}")]
    IndexOutOfBounds(String),
    /// A malformed method descriptor.
    #[error(transparent)]
    InvalidDescriptor(#[from] InvalidDescriptor),
    /// The class hierarchy could not answer a question that the computation of stack map frames
    /// needed.
    #[error(transparent)]
    Hierarchy(#[from] HierarchyError),
    /// A feature of ASM's class writer that oxasm does not have yet: computing the maximum stack
    /// size, the number of locals or the stack map frames (P6).
    #[doc(hidden)]
    #[error("not yet ported: {0}")]
    NotYetPorted(&'static str),
    /// The class file written before replacing the ASM specific instructions could not be read
    /// back (this would be an oxasm bug).
    #[error("rereading the class: {0}")]
    Reread(ReadError),
}
