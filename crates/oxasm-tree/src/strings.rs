//! Where the strings a node visits are interned.
//!
//! A class reader visits the same owners, names and descriptors many times, from its cache of
//! constant pool strings. A [`ClassNode`](crate::ClassNode) interns them through an
//! [`InternMemo`], which finds a string seen before at the same address without hashing it; a
//! member node visited on its own interns them directly.

use oxasm::sym::InternMemo;
use oxasm::{Interner, JavaStr, Sym};

/// Interns visited strings: directly, or through the memo of the class being visited.
pub(crate) enum Strings<'a> {
    Direct(&'a Interner),
    Memo(&'a mut InternMemo),
}

impl Strings<'_> {
    /// The symbol of a visited string.
    #[inline]
    pub(crate) fn get(&mut self, s: &JavaStr) -> Sym {
        match self {
            Strings::Direct(interner) => interner.intern(s),
            Strings::Memo(memo) => memo.intern(s),
        }
    }

    /// The symbol of an optional visited string.
    #[inline]
    pub(crate) fn get_opt(&mut self, s: Option<&JavaStr>) -> Option<Sym> {
        s.map(|s| self.get(s))
    }

    /// The same strings, for a shorter borrow (a sub-visitor's).
    pub(crate) fn reborrow(&mut self) -> Strings<'_> {
        match self {
            Strings::Direct(interner) => Strings::Direct(interner),
            Strings::Memo(memo) => Strings::Memo(memo),
        }
    }
}

/// The strings of the given symbols, as the `&[&JavaStr]` a visitor takes.
pub(crate) fn resolve_all<'i>(interner: &'i Interner, syms: &[Sym]) -> Vec<&'i JavaStr> {
    syms.iter().map(|&s| interner.resolve(s)).collect()
}
