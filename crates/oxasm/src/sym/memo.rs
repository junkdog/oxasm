//! [`InternMemo`]: a cache in front of an interner for the strings of one class.

use std::collections::hash_map::Entry;
use std::fmt;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::{Interner, Sym};
use crate::jstr::JavaStr;

/// An [`Interner`] with a cache keyed by the address and length of the strings it interns.
///
/// A class reader passes the same `&JavaStr` for every reference to a constant pool entry, so
/// while one class is visited, most strings are repeats of an earlier one at the same address:
/// the memo finds those without hashing them. A hit is checked against the content (the address
/// of a temporary string can be reused within a class), using the address of the interned bytes,
/// which the memo keeps, so a hit doesn't touch the interner at all. [`clear`](Self::clear) the
/// memo between classes, to keep it small.
pub struct InternMemo {
    interner: Arc<Interner>,
    /// `(address, length)` of a visited string → its symbol, and its interned bytes.
    memo: FxHashMap<(usize, usize), (Sym, Interned)>,
}

/// The address of interned bytes, which live as long as their interner.
#[derive(Clone, Copy)]
struct Interned(*const u8);

// SAFETY: the address of immutable bytes, owned by an `Interner`, which is `Send + Sync`.
unsafe impl Send for Interned {}
// SAFETY: as above.
unsafe impl Sync for Interned {}

impl InternMemo {
    /// Constructs an empty memo in front of the given interner.
    #[must_use]
    pub fn new(interner: Arc<Interner>) -> InternMemo {
        InternMemo {
            interner,
            memo: FxHashMap::default(),
        }
    }

    /// The interner.
    #[inline]
    #[must_use]
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// The symbol of the given string, which is interned if it isn't yet.
    ///
    /// # Panics
    ///
    /// As [`Interner::intern`].
    #[inline]
    pub fn intern(&mut self, s: &JavaStr) -> Sym {
        let bytes = s.as_bytes();
        match self.memo.entry((bytes.as_ptr() as usize, bytes.len())) {
            Entry::Occupied(mut entry) => {
                let (sym, interned) = *entry.get();
                // SAFETY: `interned` is the address of `bytes.len()` (the key's length) bytes
                // interned by `self.interner`, which never move nor change while it lives, and
                // `self` keeps it alive.
                let interned = unsafe { std::slice::from_raw_parts(interned.0, bytes.len()) };
                if interned == bytes {
                    return sym;
                }
                let (id, interned) = self.interner.intern_bytes(bytes);
                let sym = super::interner::sym(id);
                entry.insert((sym, Interned(interned.as_ptr())));
                sym
            }
            Entry::Vacant(entry) => {
                let (id, interned) = self.interner.intern_bytes(bytes);
                let sym = super::interner::sym(id);
                entry.insert((sym, Interned(interned.as_ptr())));
                sym
            }
        }
    }

    /// The symbol of the given optional string. See [`intern`](Self::intern).
    #[inline]
    pub fn intern_opt(&mut self, s: Option<&JavaStr>) -> Option<Sym> {
        s.map(|s| self.intern(s))
    }

    /// Forgets the cached addresses (between classes: the strings of a class reader live as long
    /// as its class).
    pub fn clear(&mut self) {
        self.memo = FxHashMap::default();
    }
}

impl Clone for InternMemo {
    /// A memo in front of the same interner, empty.
    fn clone(&self) -> Self {
        InternMemo::new(self.interner.clone())
    }
}

impl fmt::Debug for InternMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InternMemo")
            .field("len", &self.memo.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jstr::JavaString;

    #[test]
    fn a_reused_address_is_checked() {
        let interner = Arc::new(Interner::new());
        let mut memo = InternMemo::new(interner.clone());
        let mut buffer = JavaString::from("java/util/List");
        let a = memo.intern(&buffer);
        assert_eq!(memo.intern(&buffer), a);
        // The same address and length, another string.
        buffer.clear();
        buffer.push_str("java/util/Lisp");
        let b = memo.intern(&buffer);
        assert_ne!(a, b);
        assert_eq!(interner.resolve(b), "java/util/Lisp");
        assert_eq!(memo.intern(JavaStr::from_str("java/util/List")), a);
        memo.clear();
        assert_eq!(memo.intern(&buffer), b);
    }
}
