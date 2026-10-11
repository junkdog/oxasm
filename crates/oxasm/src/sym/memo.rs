//! [`InternMemo`]: a cache in front of an interner for the strings of one class.

use std::cell::Cell;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use super::{Interner, Sym};
use crate::jstr::JavaStr;
use crate::reader::ClassFileBytes;

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
/// of the interned bytes, which the memo keeps, so a hit doesn't touch the interner at all;
/// except for a string within the bytes of the class file being visited (see
/// [`set_class_file`](Self::set_class_file)), which can't change, and was checked when cached.
/// [`clear`](Self::clear) the memo between classes, to keep it small: it gives up the cache, which
/// the next class's first string takes again. A thread keeps the last cache given up (12 KiB), so
/// that the memo of its next class doesn't allocate one.
pub struct InternMemo {
    interner: Arc<Interner>,
    /// The cache, empty until the first string.
    cache: Cache,
    /// The addresses of the class file being visited (see
    /// [`set_class_file`](Self::set_class_file)), or `0..0`.
    class_file: Range<usize>,
}

/// A memo's entries, and the generation they are valid in: an entry of another generation is
/// empty. A cache that a memo gives up is valid in no generation when the next memo takes it.
#[derive(Default)]
struct Cache {
    entries: Box<[Entry]>,
    /// The generation, from 1 to [`MAX_GENERATION`], in bits 16 to 30 of a key.
    generation: u32,
}

/// The last generation of a cache, after which its entries are emptied.
const MAX_GENERATION: u32 = (1 << 15) - 1;

/// The bit of a key whose entry's hits are checked against the content: a string that was not
/// within the bytes of a class file being visited.
const CHECKED: u32 = 1 << 31;

/// A cached string: its address, its key (its length in the low half, the generation of the
/// cache in bits 16 to 30, and [`CHECKED`]), its symbol, and its interned bytes. A key of 0 marks
/// an entry that no generation filled.
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
    /// The key of a string of `len` bytes in this cache's generation, without [`CHECKED`], or 0
    /// (no entry's) for a string longer than an interner holds.
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
            Some(mut cache) if cache.generation < MAX_GENERATION << 16 => {
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
            class_file: 0..0,
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
            // An entry without `CHECKED`: a string within the class file, which hasn't changed
            // since it was cached (and checked).
            debug_assert!(self.interned(entry, bytes.len()) == bytes);
            return entry.sym;
        }
        self.intern_missed(bytes, index)
    }

    /// The interned bytes of an entry of the cache's generation, whose key's length is `len`.
    #[inline]
    fn interned(&self, entry: &Entry, len: usize) -> &[u8] {
        // SAFETY: an entry of the cache's generation, so `entry.interned` is the address of `len`
        // (its key's length) bytes interned by `self.interner`, which never move nor change
        // while it lives, and `self` keeps it alive.
        unsafe { std::slice::from_raw_parts(entry.interned.0, len) }
    }

    /// Whether `bytes` lie within the class file being visited.
    #[inline]
    fn in_class_file(&self, bytes: &[u8]) -> bool {
        let range = bytes.as_ptr_range();
        self.class_file.start <= range.start as usize && range.end as usize <= self.class_file.end
    }

    /// The symbol of `bytes`, which the cache holds as a string to check, or doesn't hold; from
    /// the interner in the second case, and then cached in the entry of the given index.
    #[inline(never)]
    fn intern_missed(&mut self, bytes: &[u8], index: usize) -> Sym {
        if self.cache.entries.is_empty() {
            self.cache = Cache::take();
        }
        let key = self.cache.key(bytes.len());
        let entry = &self.cache.entries[index];
        if entry.address == bytes.as_ptr() as usize
            && entry.key == key | CHECKED
            && self.interned(entry, bytes.len()) == bytes
        {
            let sym = entry.sym;
            // A string cached before the class file was set may be within it.
            if self.in_class_file(bytes) {
                self.cache.entries[index].key = key;
            }
            return sym;
        }
        let (id, interned) = self.interner.intern_bytes_inserting(bytes);
        let sym = super::interner::sym(id);
        self.cache.entries[index] = Entry {
            address: bytes.as_ptr() as usize,
            interned: Interned(interned.as_ptr()),
            // Interned, so at most `MAX_LEN` (`u16::MAX`) bytes.
            key: if self.in_class_file(bytes) {
                key
            } else {
                key | CHECKED
            },
            sym,
        };
        sym
    }

    /// The symbol of the given optional string. See [`intern`](Self::intern).
    #[inline]
    pub fn intern_opt(&mut self, s: Option<&JavaStr>) -> Option<Sym> {
        s.map(|s| self.intern(s))
    }

    /// Sets the class file being visited (see
    /// [`ClassVisitor::visit_class_file`](crate::ClassVisitor::visit_class_file)), whose strings
    /// are not checked against the content again when found in the cache, until
    /// [`clear`](Self::clear). `None` forgets the cached addresses, as `clear` does.
    pub fn set_class_file(&mut self, class_file: Option<ClassFileBytes<'_>>) {
        // The strings found unchecked are those of one class file.
        if class_file.is_none() || !self.class_file.is_empty() {
            self.clear();
        }
        if let Some(class_file) = class_file {
            let range = class_file.as_ptr_range();
            self.class_file = range.start as usize..range.end as usize;
        }
    }

    /// Forgets the cached addresses (between classes: the strings of a class reader live as long
    /// as its class), and the class file being visited.
    pub fn clear(&mut self) {
        std::mem::take(&mut self.cache).give_up();
        self.class_file = 0..0;
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
            .filter(|e| e.key & !(0xffff | CHECKED) == generation)
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
    #[cfg_attr(miri, ignore = "slow: 2^15 caches taken")]
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
        for _ in 1..MAX_GENERATION {
            InternMemo::new(interners[0].clone()).intern(set);
        }
        // Then the first generation again: the first memo's entry is gone.
        let mut memo = InternMemo::new(interners[1].clone());
        memo.intern(set);
        assert_eq!(memo.cache.generation, 1 << 16);
        assert_eq!(memo.intern(&buffer), syms[1]);
    }

    /// The key of the entry of the given string.
    fn key_of(memo: &InternMemo, s: &JavaStr) -> u32 {
        let bytes = s.as_bytes();
        memo.cache.entries[index(bytes.as_ptr() as usize, bytes.len())].key
    }

    #[test]
    fn strings_within_the_class_file_are_not_checked_again() {
        let class_file = b"java/util/Listjava/util/Map".to_vec();
        let list = JavaStr::from_str(std::str::from_utf8(&class_file[..14]).unwrap());
        let map = JavaStr::from_str(std::str::from_utf8(&class_file[14..]).unwrap());
        let set = JavaString::from("java/util/Set");
        let interner = Arc::new(Interner::new());
        let mut memo = InternMemo::new(interner.clone());
        // Cached before the class file is set: checked.
        let a = memo.intern(list);
        assert_ne!(key_of(&memo, list) & CHECKED, 0);
        memo.set_class_file(Some(ClassFileBytes::new(&class_file)));
        let b = memo.intern(map);
        let c = memo.intern(&set);
        assert_eq!(key_of(&memo, map) & CHECKED, 0);
        assert_ne!(key_of(&memo, &set) & CHECKED, 0);
        // Found again (and the first one, now within the class file, no longer checked).
        assert_eq!(memo.intern(list), a);
        assert_eq!(key_of(&memo, list) & CHECKED, 0);
        assert_eq!(memo.intern(map), b);
        assert_eq!(memo.intern(&set), c);
        assert_eq!(interner.resolve(a), "java/util/List");
        assert_eq!(interner.resolve(b), "java/util/Map");
        assert_eq!(interner.resolve(c), "java/util/Set");
    }

    #[test]
    fn the_class_file_is_forgotten() {
        let class_file = b"java/util/List".to_vec();
        let list = JavaStr::from_str(std::str::from_utf8(&class_file).unwrap());
        let mut memo = InternMemo::new(Arc::new(Interner::new()));
        // At `clear`, with the cached strings.
        memo.set_class_file(Some(ClassFileBytes::new(&class_file)));
        memo.intern(list);
        memo.clear();
        assert!(memo.class_file.is_empty() && memo.cache.entries.is_empty());
        // When the reader takes it back.
        memo.set_class_file(Some(ClassFileBytes::new(&class_file)));
        memo.intern(list);
        memo.set_class_file(None);
        assert!(memo.class_file.is_empty() && memo.cache.entries.is_empty());
        // When another class file is set: the strings of the first one are forgotten.
        memo.set_class_file(Some(ClassFileBytes::new(&class_file)));
        memo.intern(list);
        let other = b"java/util/Map".to_vec();
        memo.set_class_file(Some(ClassFileBytes::new(&other)));
        assert!(memo.cache.entries.is_empty());
        assert_eq!(memo.class_file, {
            let range = other.as_ptr_range();
            range.start as usize..range.end as usize
        });
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
