//! [`InternMemo`]: a cache in front of an interner for the strings of one class.

use std::fmt;
use std::sync::Arc;

use super::{Interner, Sym};
use crate::jstr::JavaStr;

/// The number of entries of a memo's cache: a power of two, several times the distinct strings
/// of most classes, so that few of them collide.
const ENTRIES: usize = 512;

/// An [`Interner`] with a cache keyed by the address and length of the strings it interns.
///
/// A class reader passes the same `&JavaStr` for every reference to a constant pool entry, so
/// while one class is visited, most strings are repeats of an earlier one at the same address:
/// the memo finds those without hashing their content. The cache is direct-mapped: each
/// `(address, length)` has one entry, which a colliding string replaces (the next lookup of the
/// first one then goes to the interner, which is always correct). A hit is checked against the
/// content (the address of a temporary string can be reused within a class), using the address
/// of the interned bytes, which the memo keeps, so a hit doesn't touch the interner at all.
/// [`clear`](Self::clear) the memo between classes, to keep it small: it frees the cache, which
/// the next class's first string allocates again.
pub struct InternMemo {
    interner: Arc<Interner>,
    /// The cache, empty until the first string.
    entries: Box<[Entry]>,
}

/// A cached string: its address and length, its symbol, and its interned bytes. An address of 0
/// marks an empty entry (no string is at address 0).
#[derive(Clone, Copy)]
struct Entry {
    address: usize,
    interned: Interned,
    len: u32,
    sym: Sym,
}

const VACANT: Entry = Entry {
    address: 0,
    interned: Interned(std::ptr::null()),
    len: 0,
    sym: super::EMPTY,
};

/// The address of interned bytes, which live as long as their interner.
#[derive(Clone, Copy)]
struct Interned(*const u8);

// SAFETY: the address of immutable bytes, owned by an `Interner`, which is `Send + Sync`.
unsafe impl Send for Interned {}
// SAFETY: as above.
unsafe impl Sync for Interned {}

/// The entry of the string at `address` with `len` bytes (Fibonacci hashing of both).
#[inline]
#[allow(clippy::cast_possible_truncation)] // the top bits: less than `ENTRIES`
fn index(address: usize, len: usize) -> usize {
    let key = (address ^ len.rotate_left(48)) as u64;
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - ENTRIES.trailing_zeros())) as usize
}

impl InternMemo {
    /// Constructs an empty memo in front of the given interner.
    #[must_use]
    pub fn new(interner: Arc<Interner>) -> InternMemo {
        InternMemo {
            interner,
            entries: Box::default(),
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
        if self.entries.is_empty() {
            self.entries = vec![VACANT; ENTRIES].into_boxed_slice();
        }
        let address = bytes.as_ptr() as usize;
        let entry = &mut self.entries[index(address, bytes.len())];
        if entry.address == address && entry.len as usize == bytes.len() {
            // SAFETY: `entry.interned` is the address of `entry.len` (here `bytes.len()`) bytes
            // interned by `self.interner`, which never move nor change while it lives, and
            // `self` keeps it alive.
            let interned = unsafe { std::slice::from_raw_parts(entry.interned.0, bytes.len()) };
            if interned == bytes {
                return entry.sym;
            }
        }
        let (id, interned) = self.interner.intern_bytes(bytes);
        let sym = super::interner::sym(id);
        *entry = Entry {
            address,
            interned: Interned(interned.as_ptr()),
            len: u32::try_from(bytes.len()).expect("at most MAX_LEN bytes"),
            sym,
        };
        sym
    }

    /// The symbol of the given optional string. See [`intern`](Self::intern).
    #[inline]
    pub fn intern_opt(&mut self, s: Option<&JavaStr>) -> Option<Sym> {
        s.map(|s| self.intern(s))
    }

    /// Forgets the cached addresses (between classes: the strings of a class reader live as long
    /// as its class).
    pub fn clear(&mut self) {
        self.entries = Box::default();
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
        let len = self.entries.iter().filter(|e| e.address != 0).count();
        f.debug_struct("InternMemo")
            .field("len", &len)
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

    #[test]
    fn colliding_strings_stay_correct() {
        let interner = Arc::new(Interner::new());
        let mut memo = InternMemo::new(interner.clone());
        // More strings than entries: some share an entry, and replace each other. (Under Miri,
        // whose arenas are small, 100 strings in 512 entries still collide.)
        let count = if cfg!(miri) { 100 } else { 2 * ENTRIES };
        let strings: Vec<JavaString> = (0..count)
            .map(|i| JavaString::from(format!("s{i}").as_str()))
            .collect();
        let syms: Vec<Sym> = strings.iter().map(|s| memo.intern(s)).collect();
        for (s, sym) in strings.iter().zip(&syms) {
            assert_eq!(memo.intern(s), *sym);
            assert_eq!(interner.resolve(*sym), s.as_java_str());
        }
    }
}
