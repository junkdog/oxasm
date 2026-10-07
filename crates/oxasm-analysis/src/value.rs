//! [`Value`]: ASM's `Value`.

/// An immutable symbolic value for the semantic interpretation of bytecode (ASM's `Value`).
///
/// Values are compared with `==` where ASM uses `equals`: an [`Analyzer`](crate::Analyzer)
/// iterates until merging changes no value of a frame.
pub trait Value: Clone + PartialEq {
    /// The size of this value in 32 bits words: 1 for `byte`, `boolean`, `char`, `short`, `int`,
    /// `float`, object and array types, and 2 for `long` and `double`.
    fn get_size(&self) -> usize;
}
