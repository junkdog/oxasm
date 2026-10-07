//! [`AnalyzerError`]: ASM's `AnalyzerException`.

use std::fmt::{self, Write};

use oxasm::{JavaStr, JavaString};
use oxasm_tree::InsnId;

/// A problem found during the analysis of a method (ASM's `AnalyzerException`).
///
/// The message is ASM's, including the `Error at instruction <index>: ` prefix that
/// [`Analyzer::analyze`](crate::Analyzer::analyze) adds. It is a [`JavaString`] because it may
/// contain names from the class file, which may have unpaired surrogates.
///
/// ASM's analyzer also turns the `RuntimeException`s of its frames and interpreters (an index
/// out of bounds, a class a `SimpleVerifier` cannot load, ...) into `AnalyzerException`s, at the
/// instruction being interpreted. Those are the errors made with [`AnalyzerError::runtime`]:
/// the analyzer sets their instruction, where ASM's `AnalyzerException`s keep the one they were
/// made with (possibly none).
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AnalyzerError {
    /// The instruction where the analysis failed (ASM's `node`), if known.
    pub insn: Option<InsnId>,
    /// The reason why the analysis failed.
    pub message: JavaString,
    /// Whether this is a Java `RuntimeException`, to which the analyzer gives the instruction
    /// being interpreted.
    runtime: bool,
}

impl AnalyzerError {
    /// An error at the given instruction (ASM's `AnalyzerException(insn, message)`).
    pub fn new(insn: Option<InsnId>, message: impl Into<JavaString>) -> AnalyzerError {
        AnalyzerError {
            insn,
            message: message.into(),
            runtime: false,
        }
    }

    /// An error saying that a value was expected and another found (ASM's
    /// `AnalyzerException(insn, message, expected, actual)`): `<message>: expected <expected>,
    /// but found <actual>`, or `Expected <expected>, but found <actual>` without a message.
    pub fn expected(
        insn: Option<InsnId>,
        message: Option<&str>,
        expected: &JavaStr,
        actual: &JavaStr,
    ) -> AnalyzerError {
        let mut text = JavaString::new();
        match message {
            None => text.push_str("Expected "),
            Some(message) => {
                text.push_str(message);
                text.push_str(": expected ");
            }
        }
        text.push_java_str(expected);
        text.push_str(", but found ");
        text.push_java_str(actual);
        AnalyzerError::new(insn, text)
    }

    /// An error that ASM signals with a `RuntimeException` (or another `Throwable` its analyzer
    /// catches): the analyzer reports it at the instruction it is interpreting.
    pub fn runtime(message: impl Into<JavaString>) -> AnalyzerError {
        AnalyzerError {
            insn: None,
            message: message.into(),
            runtime: true,
        }
    }

    /// Whether this error was made with [`AnalyzerError::runtime`] and not reported by the
    /// analyzer yet.
    pub fn is_runtime(&self) -> bool {
        self.runtime
    }

    /// This error, as it escapes ASM's analyzer without being wrapped: not a runtime error any
    /// more.
    pub(crate) fn unwrapped(self) -> AnalyzerError {
        AnalyzerError {
            runtime: false,
            ..self
        }
    }

    /// This error, as the analyzer reports it when interpreting the instruction at `index`
    /// (`insn`): with the `Error at instruction <index>: ` prefix, and at `insn` if it is a
    /// runtime error.
    pub(crate) fn at(self, index: usize, insn: Option<InsnId>) -> AnalyzerError {
        let mut message = JavaString::new();
        write!(message, "Error at instruction {index}: ").expect("writing to a string");
        message.push_java_str(&self.message);
        AnalyzerError {
            insn: if self.runtime { insn } else { self.insn },
            message,
            runtime: false,
        }
    }
}

impl fmt::Debug for AnalyzerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalyzerError")
            .field("insn", &self.insn)
            .field("message", &self.message)
            .finish()
    }
}

/// The message of Java's `IndexOutOfBoundsException` for an index out of an array.
pub(crate) fn index_out_of_bounds(index: impl fmt::Display, length: usize) -> AnalyzerError {
    AnalyzerError::runtime(format!("Index {index} out of bounds for length {length}"))
}
