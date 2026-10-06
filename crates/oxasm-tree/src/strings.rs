//! The sharing of the strings a node stores.
//!
//! A class reader visits the same owners, names and descriptors many times, from its cache of
//! constant pool strings; ASM's nodes keep references to these `String`s, so each one is
//! allocated once per class. The visitor traits pass borrowed strings instead. A [`JavaArcStr`]
//! of up to 23 bytes is stored inline, and costs no allocation; a [`ClassNode`](crate::ClassNode)
//! shares the longer ones itself, through a [`StringPool`], so that each is allocated once per
//! visited class.

use std::fmt;

use oxasm::{JavaArcStr, JavaStr};
use rustc_hash::FxHashSet;

/// Turns visited strings into the strings a node stores.
pub(crate) trait Strings {
    /// The string to store for the given visited string.
    fn get(&mut self, s: &JavaStr) -> JavaArcStr;

    /// The string to store for the given optional visited string.
    fn get_opt(&mut self, s: Option<&JavaStr>) -> Option<JavaArcStr> {
        s.map(|s| self.get(s))
    }
}

/// The strings of a node that visits without a pool (a member node visited on its own): each
/// visited string is a new allocation.
pub(crate) struct Unshared;

impl Strings for Unshared {
    fn get(&mut self, s: &JavaStr) -> JavaArcStr {
        JavaArcStr::from(s)
    }
}

/// The distinct long (not inline) strings visited so far: a visited string equal to one of them
/// is stored as another reference to it.
#[derive(Clone, Default)]
pub(crate) struct StringPool {
    strings: FxHashSet<JavaArcStr>,
}

impl Strings for StringPool {
    #[inline]
    fn get(&mut self, s: &JavaStr) -> JavaArcStr {
        if JavaArcStr::fits_inline(s) {
            return JavaArcStr::new(s);
        }
        self.get_shared(s)
    }
}

impl StringPool {
    /// The pooled string equal to the given long string, which is pooled if there is none.
    fn get_shared(&mut self, s: &JavaStr) -> JavaArcStr {
        if let Some(shared) = self.strings.get(s) {
            return shared.clone();
        }
        let shared = JavaArcStr::new(s);
        self.strings.insert(shared.clone());
        shared
    }
}

impl fmt::Debug for StringPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StringPool")
            .field("len", &self.strings.len())
            .finish()
    }
}
