//! [`InternMemo`]: a cache in front of an interner for the strings of one class.

use std::cell::Cell;
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
/// [`clear`](Self::clear) the memo between classes, to keep it small: it gives up the cache, which
/// the next class's first string takes again. A thread keeps the last cache given up (12 KiB), so
/// that the memo of its next class doesn't allocate one.
pub struct InternMemo {
    interner: Arc<Interner>,
    /// The cache, empty until the first string.
    cache: Cache,
}

/// A memo's entries, and the generation they are valid in: an entry of another generation is
/// empty. A cache that a memo gives up is valid in no generation when the next memo takes it.
#[derive(Default)]
struct Cache {
    entries: Box<[Entry]>,
    /// The generation, from 1 to `u16::MAX`, in the high half of a key.
    generation: u32,
}

/// A cached string: its address, its key (its length in the low half, the generation of the
/// cache in the high half), its symbol, and its interned bytes. A key of 0 marks an entry that
/// no generation filled.
#[derive(Clone, Copy)]
struct Entry {
    address: usize,
    interned: Interned,
    key: u32,
    sym: Sym,
}

const VACANT: Entry = Entry {
    address: 0,
    interned: Interned(std::ptr::null()),
    key: 0,
    sym: super::EMPTY,
};

/// The address of interned bytes, which live as long as their interner.
#[derive(Clone, Copy)]
struct Interned(*const u8);

// SAFETY: the address of immutable bytes, owned by an `Interner`, which is `Send + Sync`.
unsafe impl Send for Interned {}
// SAFETY: as above.
unsafe impl Sync for Interned {}

thread_local! {
    /// The cache a memo of this thread gave up last, for the next one.
    static SPARE: Cell<Option<Cache>> = const { Cell::new(None) };
}

impl Cache {
    /// The key of a string of `len` bytes in this cache's generation, or 0 (no entry's) for a
    /// string longer than an interner holds.
    #[inline]
    #[allow(clippy::cast_possible_truncation)] // at most `u16::MAX`
    fn key(&self, len: usize) -> u32 {
        if len <= usize::from(u16::MAX) {
            self.generation | len as u32
        } else {
            0
        }
    }

    /// This thread's spare cache, in a new generation, or a new cache.
    fn take() -> Cache {
        match SPARE.try_with(Cell::take).ok().flatten() {
            Some(mut cache) if cache.generation < u32::from(u16::MAX) << 16 => {
                cache.generation += 1 << 16;
                cache
            }
            spare => {
                // A new cache, or the spare's generations used up: every entry empty again.
                let mut entries = spare.map(|cache| cache.entries).unwrap_or_default();
                if entries.is_empty() {
                    entries = vec![VACANT; ENTRIES].into_boxed_slice();
                } else {
                    entries.fill(VACANT);
                }
                Cache {
                    entries,
                    generation: 1 << 16,
                }
            }
        }
    }

    /// Makes the cache this thread's spare (replacing the previous one), unless it is empty.
    fn give_up(self) {
        if !self.entries.is_empty() {
            // During the thread's exit, the cache is dropped instead.
            let _ = SPARE.try_with(|spare| spare.set(Some(self)));
        }
    }
}

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
            cache: Cache::default(),
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
        let address = bytes.as_ptr() as usize;
        let index = index(address, bytes.len());
        // No entry before the first string (the cache is empty).
        if let Some(entry) = self.cache.entries.get(index)
            && entry.address == address
            && entry.key == self.cache.key(bytes.len())
        {
            // SAFETY: an entry of the cache's generation, so `entry.interned` is the address of
            // `bytes.len()` (its key's length) bytes interned by `self.interner`, which never
            // move nor change while it lives, and `self` keeps it alive.
            let interned = unsafe { std::slice::from_raw_parts(entry.interned.0, bytes.len()) };
            if interned == bytes {
                return entry.sym;
            }
        }
        self.intern_missed(bytes, index)
    }

    /// The symbol of `bytes`, which the cache doesn't hold, from the interner; cached in the
    /// entry of the given index.
    #[inline(never)]
    fn intern_missed(&mut self, bytes: &[u8], index: usize) -> Sym {
        if self.cache.entries.is_empty() {
            self.cache = Cache::take();
        }
        let (id, interned) = self.interner.intern_bytes(bytes);
        let sym = super::interner::sym(id);
        self.cache.entries[index] = Entry {
            address: bytes.as_ptr() as usize,
            interned: Interned(interned.as_ptr()),
            // Interned, so at most `MAX_LEN` (`u16::MAX`) bytes.
            key: self.cache.key(bytes.len()),
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
        std::mem::take(&mut self.cache).give_up();
    }
}

impl Drop for InternMemo {
    fn drop(&mut self) {
        self.clear();
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
        let generation = self.cache.generation;
        let len = (self.cache.entries.iter())
            .filter(|e| e.key & !0xffff == generation)
            .count();
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
    fn a_reused_cache_forgets_its_strings() {
        let buffer = JavaString::from("java/util/List");
        let first = Arc::new(Interner::new());
        let mut memo = InternMemo::new(first.clone());
        let a = memo.intern(&buffer);
        // The cache becomes this thread's spare, which the next memo takes.
        drop(memo);
        let second = Arc::new(Interner::new());
        // Another symbol than in the first interner.
        second.intern("java/util/Map");
        let mut memo = InternMemo::new(second.clone());
        // Its first string takes the cache, the next one would find the first memo's entry.
        memo.intern(JavaStr::from_str("java/util/Set"));
        let b = memo.intern(&buffer);
        assert_ne!(a, b);
        assert_eq!(second.resolve(b), "java/util/List");
    }

    #[test]
    #[cfg_attr(miri, ignore = "slow: 2^16 caches taken")]
    fn generations_wrap_around() {
        let set = JavaStr::from_str("java/util/Set");
        let buffer = JavaString::from("java/util/List");
        let interners = [Interner::new(), Interner::new()].map(Arc::new);
        interners[1].intern("java/util/Map");
        let syms = interners.each_ref().map(|i| i.intern(&buffer));
        assert_ne!(syms[0], syms[1]);
        // The first memo of the thread makes a new cache, in the first generation.
        SPARE.with(Cell::take);
        let mut memo = InternMemo::new(interners[0].clone());
        memo.intern(set);
        assert_eq!(memo.intern(&buffer), syms[0]);
        drop(memo);
        // The next ones take it, in the other generations, without the buffer.
        for _ in 1..u16::MAX {
            InternMemo::new(interners[0].clone()).intern(set);
        }
        // Then the first generation again: the first memo's entry is gone.
        let mut memo = InternMemo::new(interners[1].clone());
        memo.intern(set);
        assert_eq!(memo.cache.generation, 1 << 16);
        assert_eq!(memo.intern(&buffer), syms[1]);
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
