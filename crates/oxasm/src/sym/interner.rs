//! [`Interner`]: the "v4" layout of ADR-04's P9 amendment.
//!
//! The lookup structure and the string storage are apart:
//!
//! - **64 hash shards.** Each has an open-addressing table of `AtomicU64` slots (`tag << 32 |
//!   sym`), probed with acquire loads only, so a lookup that finds its string takes no lock.
//!   Inserts and table growth take the shard's mutex. A grown table replaces the old one, which
//!   stays allocated until the interner drops, so a reader still probing it stays valid (at worst
//!   it misses, and retries under the lock). A shard's first own table (its first insert copies
//!   the shared one) is in the shard itself, so that a new interner's first inserts allocate no
//!   table, nor does its drop free one.
//! - **64 byte arenas,** one per inserting thread. Strings are stored length-prefixed (a
//!   little-endian `u16`, then the bytes, at an even offset) in chunks that never move, and a
//!   string never straddles two of them. Each chunk spans 128 KiB of offsets; an arena's first
//!   five are smaller (4 KiB, doubling), so that a new interner costs a few pages. A `Sym` is `(offset / 2) << 6 | arena`, so a lookup
//!   goes from the slot straight to the bytes. Each chunk ends with a bitmap of the offsets where
//!   a published string starts (1 bit per 2-byte unit), which is how [`Interner::resolve`] stays
//!   sound for a `Sym` it never returned.
//! - **An insert stages the bytes in its thread's arena first.** A thread owns its arena's index
//!   (in every interner) until it exits, so it needs no lock for it, except in the last arena,
//!   which the threads past the first 63 share under its lock. The insert then takes the shard's
//!   mutex, looks again, and publishes the string's start bit and then the slot, both with release
//!   stores. The lock order is fixed: arena (if shared), then shard. If another thread got
//!   there first, the staged bytes stay unpublished, and the next insert into the arena
//!   overwrites them.
//! - **Cache lines:** what lookups read (a shard's table pointer, an arena's chunk pointers) is
//!   apart from what inserts write (the mutexes). So is the interner itself (the pointers to its
//!   shards and arenas, which every lookup reads) from the reference counts of the `Arc` that
//!   holds it, which every member node of the tree writes when it is made and dropped.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::cell::UnsafeCell;
use std::fmt;
use std::mem::MaybeUninit;
use std::hash::Hasher;
use std::num::NonZeroU32;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::{Mutex, MutexGuard, PoisonError};

use rustc_hash::FxHasher;

use super::{Sym, WELL_KNOWN};
use crate::jstr::JavaStr;

const SHARD_BITS: u32 = 6;
const SHARDS: usize = 1 << SHARD_BITS;
pub(super) const ARENA_BITS: u32 = 6;
const ARENAS: usize = 1 << ARENA_BITS;
/// The span of offsets of a chunk: it holds the longest string plus its prefix. (Miri runs with
/// small chunks, so that its tests roll over to new chunks.)
const CHUNK_BITS: u32 = if cfg!(miri) { 6 } else { 17 };
const CHUNK: usize = 1 << CHUNK_BITS;
/// The first chunks of an arena are smaller than their span (see [`capacity`]): the first
/// `CAPACITY_STEPS` double, up to `CHUNK`.
const CAPACITY_STEPS: usize = if cfg!(miri) { 1 } else { 5 };
/// Offsets are in 2-byte units, in the `32 - ARENA_BITS` high bits of a `Sym`.
const ARENA_BYTES: usize = if cfg!(miri) {
    CHUNK * 64
} else {
    2 << (32 - ARENA_BITS)
};
const CHUNKS: usize = ARENA_BYTES >> CHUNK_BITS;
/// The initial number of slots of a shard's table.
const TABLE0: usize = if cfg!(miri) { 2 } else { 16 };

/// The longest string an interner holds, in bytes: 65535, the longest `CONSTANT_Utf8` (whose
/// modified UTF-8 is never shorter than its semi-UTF-8).
pub const MAX_LEN: usize = if CHUNK - 2 < u16::MAX as usize {
    CHUNK - 2
} else {
    u16::MAX as usize
};

/// Where the given well-known strings are stored when an interner is seeded with them, in order,
/// from arena 0's first offset: [`Arena::stage`]'s placement, as a `const fn`.
pub(super) const fn well_known_offsets<const N: usize>(strings: &[&str]) -> [u32; N] {
    let mut offsets = [0; N];
    let mut end = 2;
    let mut i = 0;
    while i < N {
        let (offset, new_end) = place(end, strings[i].len());
        #[allow(clippy::cast_possible_truncation)] // the first chunk holds them all
        {
            offsets[i] = offset as u32;
        }
        end = new_end;
        i += 1;
    }
    offsets
}

/// The size of the string bytes of the chunk of the given index: 4 KiB for an arena's first,
/// doubling up to `CHUNK` (128 KiB) from the sixth on. A new interner's arenas then cost a few
/// pages, not a full chunk each; the offsets past a chunk's capacity are skipped.
#[inline]
const fn capacity(chunk_index: usize) -> usize {
    if chunk_index >= CAPACITY_STEPS {
        CHUNK
    } else {
        CHUNK >> (CAPACITY_STEPS - chunk_index)
    }
}

/// The words of a chunk's bitmap of string starts: one bit per 2-byte unit of its capacity.
const fn bitmap_words(chunk_index: usize) -> usize {
    (capacity(chunk_index) / 2).div_ceil(64)
}

/// The offset at which a string of `len` bytes goes, given the arena's `end`, and the arena's new
/// end after it: the string and its prefix never straddle two chunks, nor a chunk's capacity,
/// and offsets are even.
const fn place(end: usize, len: usize) -> (usize, usize) {
    let need = 2 + len;
    let mut offset = end;
    while (offset & (CHUNK - 1)) + need > capacity(offset >> CHUNK_BITS) {
        offset = (offset + CHUNK) & !(CHUNK - 1);
    }
    (offset, (offset + need + 1) & !1)
}

#[inline]
fn arena_of(sym: u32) -> usize {
    sym as usize & (ARENAS - 1)
}

#[inline]
fn offset_of(sym: u32) -> usize {
    (sym >> ARENA_BITS) as usize * 2
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic under these locks leaves no broken invariant: what it had staged stays unpublished.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// -------------------------------------------------------------------------------------------------
// Arenas
// -------------------------------------------------------------------------------------------------

/// An arena: the chunk pointers, which every lookup reads, apart from the lock that inserts take.
struct Arena {
    r: ArenaRead,
    w: ArenaWrite,
}

#[repr(align(128))]
struct ArenaRead {
    /// The `CHUNKS` chunk pointers, each null until its chunk is allocated: null until the
    /// arena's first insert, or (arena 0) the [shared](Shared) chunk pointers until then. A chunk
    /// is `capacity` bytes of strings, then `bitmap_words` words.
    chunks: AtomicPtr<AtomicPtr<u8>>,
}

#[repr(align(128))]
struct ArenaWrite {
    /// What inserts change: only the thread that owns the arena's index touches it, or a thread
    /// that holds `lock` (in the [shared arena](SHARED_ARENA)); see [`thread_arena`].
    state: UnsafeCell<ArenaState>,
    lock: Mutex<()>,
    /// The arena's end, for [`Interner::string_bytes`].
    published_end: AtomicUsize,
    /// The number of first chunks that are the shared well-known chunks (only in arena 0).
    shared_chunks: usize,
}

// SAFETY: `state` is only touched by the thread that owns the arena's index (which no other live
// thread owns, in any interner), by a thread that holds `lock`, or through `&mut`.
unsafe impl Sync for ArenaWrite {}

struct ArenaState {
    /// The offset just past the last published string (even; it starts at 2, so that no `Sym` is
    /// 0).
    end: usize,
    /// Whether `chunks` is this arena's own (a leaked `Box<[AtomicPtr<u8>]>`), rather than null or
    /// the shared one.
    owns_chunks: bool,
    /// One past the index of the last chunk allocated (chunks are allocated in order).
    chunks_end: usize,
}

fn chunk_layout(chunk_index: usize) -> Layout {
    let size = capacity(chunk_index) + bitmap_words(chunk_index) * 8;
    Layout::from_size_align(size, 64).expect("a valid layout")
}

impl Arena {
    fn new() -> Arena {
        Arena {
            r: ArenaRead {
                chunks: AtomicPtr::new(ptr::null_mut()),
            },
            w: ArenaWrite {
                state: UnsafeCell::new(ArenaState {
                    end: 2,
                    owns_chunks: false,
                    chunks_end: 0,
                }),
                lock: Mutex::new(()),
                published_end: AtomicUsize::new(2),
                shared_chunks: 0,
            },
        }
    }

    /// Arena 0 of an interner: its first chunks are the shared well-known chunks, and its strings
    /// start after them.
    fn with_well_known_chunks(shared: &'static Shared) -> Arena {
        Arena {
            r: ArenaRead {
                chunks: AtomicPtr::new(shared.chunk_pointers.as_ptr().cast_mut()),
            },
            w: ArenaWrite {
                state: UnsafeCell::new(ArenaState {
                    end: shared.chunks * CHUNK,
                    owns_chunks: false,
                    chunks_end: shared.chunks,
                }),
                lock: Mutex::new(()),
                published_end: AtomicUsize::new(shared.chunks * CHUNK),
                shared_chunks: shared.chunks,
            },
        }
    }

    /// The chunk pointers, or `None` before the arena's first insert.
    #[inline]
    fn chunk_pointers(&self) -> Option<&[AtomicPtr<u8>]> {
        let chunks = self.r.chunks.load(Acquire);
        if chunks.is_null() {
            return None;
        }
        // SAFETY: a non-null pointer is a leaked `Box<[AtomicPtr<u8>]>` of `CHUNKS` elements,
        // freed only when the arena drops.
        Some(unsafe { std::slice::from_raw_parts(chunks, CHUNKS) })
    }

    /// The bytes of the string published at `offset` (from a table slot, or just published by
    /// this thread).
    #[inline]
    fn bytes_published(&self, offset: usize) -> &[u8] {
        // A published string's chunk pointers and chunk are allocated, with release stores before
        // the slot's.
        let chunks = self.chunk_pointers().expect("a published string");
        let base = chunks[offset >> CHUNK_BITS].load(Acquire);
        // SAFETY: the prefix of a published string, in its allocated chunk.
        let prefix = unsafe { base.add(offset & (CHUNK - 1)) };
        string_at(prefix)
    }

    /// The bytes of the string that starts at `offset`, if one is published there.
    #[inline]
    fn bytes(&self, offset: usize) -> Option<&[u8]> {
        let chunk_index = offset >> CHUNK_BITS;
        let base = self.chunk_pointers()?.get(chunk_index)?.load(Acquire);
        let in_chunk = offset & (CHUNK - 1);
        if base.is_null() || in_chunk >= capacity(chunk_index) {
            return None;
        }
        let unit = in_chunk / 2;
        let bit = 1 << (unit % 64);
        if bitmap(base, chunk_index)[unit / 64].load(Acquire) & bit == 0 {
            return None;
        }
        // SAFETY: a string starts there: its start bit was set, with a release store, after its
        // prefix and bytes were written, within this chunk, and they never change.
        let prefix = unsafe { base.add(in_chunk) };
        Some(string_at(prefix))
    }

    /// Writes `bytes`, prefixed by their length, past the arena's end (unpublished), and returns
    /// their offset and the arena's end after them. Under the arena's lock.
    fn stage(&self, state: &mut ArenaState, bytes: &[u8]) -> (usize, usize) {
        let (offset, new_end) = place(state.end, bytes.len());
        assert!(
            new_end <= ARENA_BYTES,
            "an interner arena is full ({ARENA_BYTES} bytes)"
        );
        if !state.owns_chunks {
            // The arena's own chunk pointers, which start as the shared ones, if any. Readers of
            // the shared ones still find the strings they can have symbols of.
            let chunks: Box<[AtomicPtr<u8>]> = match self.chunk_pointers() {
                Some(shared) => shared
                    .iter()
                    .map(|chunk| AtomicPtr::new(chunk.load(Relaxed)))
                    .collect(),
                None => (0..CHUNKS)
                    .map(|_| AtomicPtr::new(ptr::null_mut()))
                    .collect(),
            };
            self.r
                .chunks
                .store(Box::into_raw(chunks).cast::<AtomicPtr<u8>>(), Release);
            state.owns_chunks = true;
        }
        let chunks = self.chunk_pointers().expect("just stored");
        let chunk_index = offset >> CHUNK_BITS;
        let chunk = &chunks[chunk_index];
        let mut base = chunk.load(Relaxed);
        if base.is_null() {
            base = new_chunk(chunk_index);
            chunk.store(base, Release);
            state.chunks_end = chunk_index + 1;
        }
        let len = u16::try_from(bytes.len()).expect("at most MAX_LEN bytes");
        // SAFETY: `2 + bytes.len()` bytes from `offset` are within the chunk (see `place`), and
        // past `end`, so no thread reads them: under the arena's lock, no other writes them.
        let prefix = unsafe { base.add(offset & (CHUNK - 1)) };
        // SAFETY: as above.
        unsafe { ptr::copy_nonoverlapping(len.to_le_bytes().as_ptr(), prefix, 2) };
        // SAFETY: as above.
        let string = unsafe { prefix.add(2) };
        // SAFETY: as above.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), string, bytes.len()) };
        (offset, new_end)
    }

    /// Marks the string staged at `offset` as published. Under the arena's lock.
    fn publish(&self, offset: usize) {
        let chunks = self.chunk_pointers().expect("a staged string");
        let chunk_index = offset >> CHUNK_BITS;
        let base = chunks[chunk_index].load(Relaxed);
        let unit = (offset & (CHUNK - 1)) / 2;
        let word = &bitmap(base, chunk_index)[unit / 64];
        // Only the holder of the arena's lock writes its bitmaps: no read-modify-write is needed.
        word.store(word.load(Relaxed) | 1 << (unit % 64), Release);
    }
}

/// A new chunk: its string bytes uninitialized (only published strings are read), its bitmap
/// zeroed.
fn new_chunk(chunk_index: usize) -> *mut u8 {
    let layout = chunk_layout(chunk_index);
    // SAFETY: the layout has a non-zero size.
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        handle_alloc_error(layout);
    }
    // SAFETY: the bitmap's `bitmap_words * 8` bytes, past the `capacity` bytes, are in the
    // allocation.
    let words = unsafe { base.add(capacity(chunk_index)) };
    // SAFETY: as above.
    unsafe { ptr::write_bytes(words, 0, bitmap_words(chunk_index) * 8) };
    base
}

/// The bitmap of string starts of the chunk of the given index at `base`.
#[inline]
fn bitmap<'a>(base: *const u8, chunk_index: usize) -> &'a [AtomicU64] {
    // SAFETY: a chunk's bitmap is `bitmap_words` words, zeroed, 8-aligned (the capacity is a
    // multiple of 64) past its `capacity` bytes, and accessed only atomically; the chunk lives
    // as long as its arena.
    let words = unsafe { base.add(capacity(chunk_index)) };
    // SAFETY: as above.
    unsafe { std::slice::from_raw_parts(words.cast::<AtomicU64>(), bitmap_words(chunk_index)) }
}

/// The bytes of the published string whose prefix is at `prefix`.
#[inline]
fn string_at<'a>(prefix: *const u8) -> &'a [u8] {
    // SAFETY: (the callers') a published string's prefix: two initialized bytes, never written
    // again.
    let len = u16::from_le_bytes(unsafe { prefix.cast::<[u8; 2]>().read() }) as usize;
    // SAFETY: as above.
    let string = unsafe { prefix.add(2) };
    // SAFETY: the `len` bytes after the prefix are the string's, initialized, never written
    // again, and they live as long as the arena.
    unsafe { std::slice::from_raw_parts(string, len) }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let state = self.w.state.get_mut();
        if !state.owns_chunks {
            // No chunk pointers, or the shared ones: no chunk of its own either.
            return;
        }
        let chunks = *self.r.chunks.get_mut();
        // SAFETY: `chunks` is the arena's own leaked `Box<[AtomicPtr<u8>]>` of `CHUNKS` elements;
        // `&mut self`: nothing borrows it any more.
        let chunks = unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(chunks, CHUNKS)) };
        // The well-known chunks are shared, and never freed.
        for (chunk_index, chunk) in chunks
            .iter()
            .enumerate()
            .take(state.chunks_end)
            .skip(self.w.shared_chunks)
        {
            let base = chunk.load(Relaxed);
            if !base.is_null() {
                // SAFETY: allocated by `stage` with this layout, and no longer borrowed.
                unsafe { dealloc(base, chunk_layout(chunk_index)) };
            }
        }
    }
}

/// What every interner shares, made once per process and never freed: the chunks that hold the
/// well-known strings (arena 0's first chunks, at the offsets of `well_known_offsets`), and a
/// table per shard with their symbols. A new interner starts with these, and copies them on its
/// first insert (into arena 0, or into a shard), so that it costs only its shards and arenas.
struct Shared {
    /// The number of well-known chunks.
    chunks: usize,
    /// `CHUNKS` chunk pointers: the well-known chunks, then nulls.
    chunk_pointers: Box<[AtomicPtr<u8>]>,
    /// The table of each shard (leaked), and its number of strings.
    tables: Box<[(SharedTable, usize)]>,
}

/// A shared table (leaked), never written after `Shared` is made.
struct SharedTable(*mut Table);

// SAFETY: the shared table and chunks are written while `SHARED` is initialized, and only read
// afterwards (an interner copies them before inserting).
unsafe impl Send for SharedTable {}
// SAFETY: as above.
unsafe impl Sync for SharedTable {}

fn shared() -> &'static Shared {
    static SHARED: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let arena = Arena::new();
        let mut state = ArenaState {
            end: 2,
            owns_chunks: false,
            chunks_end: 0,
        };
        let mut slots: Vec<Vec<u64>> = vec![Vec::new(); SHARDS];
        for (s, &expected) in WELL_KNOWN.iter().zip(&super::WELL_KNOWN_OFFSETS) {
            let (offset, new_end) = arena.stage(&mut state, s.as_bytes());
            assert_eq!(offset, expected as usize, "the well-known offsets");
            arena.publish(offset);
            state.end = new_end;
            let hash = hash(s.as_bytes());
            let sym = Sym::well_known(expected).as_u32();
            slots[shard_index(hash)].push((u64::from(tag(hash)) << 32) | u64::from(sym));
        }
        // Keep the scratch arena's chunk pointers (it is forgotten, not dropped).
        let chunks = state.end.div_ceil(CHUNK);
        let chunk_pointers = arena.chunk_pointers().expect("staged strings");
        // SAFETY: the arena's own chunk pointers, a leaked `Box<[AtomicPtr<u8>]>` of `CHUNKS`
        // elements, which nothing else frees: the arena is forgotten.
        let chunk_pointers = unsafe {
            Box::from_raw(ptr::slice_from_raw_parts_mut(
                chunk_pointers.as_ptr().cast_mut(),
                CHUNKS,
            ))
        };
        std::mem::forget(arena);
        let tables = slots
            .iter()
            .map(|slots| {
                let table = new_table((slots.len() * 2).next_power_of_two().max(2));
                // SAFETY: just allocated, and not shared yet.
                let table_ref = unsafe { &*table };
                for &slot in slots {
                    put(table_ref, slot);
                }
                (SharedTable(table), slots.len())
            })
            .collect();
        Shared {
            chunks,
            chunk_pointers,
            tables,
        }
    })
}

// -------------------------------------------------------------------------------------------------
// Shards
// -------------------------------------------------------------------------------------------------

/// A shard: its table pointer, which every lookup reads, apart from the lock that inserts take,
/// and the slots of its first own table, so that a new interner's first inserts allocate no
/// table.
struct Shard {
    r: ShardRead,
    w: ShardWrite,
    first: FirstSlots,
}

#[repr(align(128))]
struct ShardRead {
    /// The shared table, `first`, or a leaked `Box<Table>`; replaced (never freed) when the
    /// table grows.
    table: AtomicPtr<Table>,
    /// The header of the shard's first own table, whose slots are `FirstSlots`: written once,
    /// under the shard's lock, before `table` points to it.
    first: UnsafeCell<MaybeUninit<Table>>,
}

// SAFETY: `first` is written once, under the shard's lock, before a release store of `table`
// publishes it, and only read afterwards.
unsafe impl Sync for ShardRead {}
// SAFETY: `first` only ever points to the same shard's `FirstSlots`, and is written once the shard
// is in its interner's boxed slice, which never moves it.
unsafe impl Send for ShardRead {}

#[repr(align(128))]
struct ShardWrite {
    writer: Mutex<Writer>,
}

/// The slots of a shard's first own table: initialized under the shard's lock, when its first
/// insert copies the shared table.
#[repr(align(128))]
struct FirstSlots(UnsafeCell<MaybeUninit<[AtomicU64; TABLE0]>>);

// SAFETY: initialized once, under the shard's lock, before a release store of the shard's `table`
// publishes them; afterwards only accessed atomically.
unsafe impl Sync for FirstSlots {}

/// An open-addressing table of `tag << 32 | sym` slots (0 is empty), a power of two long. Its
/// slots are a shard's [`FirstSlots`], or a leaked `Box<[AtomicU64]>` (as are the shared tables',
/// which are never freed).
struct Table {
    slots: *const AtomicU64,
    len: usize,
}

impl Table {
    #[inline]
    fn slots(&self) -> &[AtomicU64] {
        // SAFETY: `len` initialized slots, which live as long as the table (see `Table`).
        unsafe { std::slice::from_raw_parts(self.slots, self.len) }
    }
}

/// Which table a shard's table is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Own {
    /// The shared one.
    Shared,
    /// The shard's first own table, in the shard.
    First,
    /// A leaked `Box<Table>` with leaked slots.
    Heap,
}

struct Writer {
    /// The number of strings in the table.
    len: usize,
    /// Which table the shard's table is.
    own: Own,
    /// The heap tables that this shard's table replaced, freed when the interner drops.
    retired: Vec<*mut Table>,
}

// SAFETY: the retired tables are owned allocations, only touched under the shard's mutex or with
// `&mut`.
unsafe impl Send for Writer {}

fn new_table(len: usize) -> *mut Table {
    let slots: Box<[AtomicU64]> = (0..len).map(|_| AtomicU64::new(0)).collect();
    Box::into_raw(Box::new(Table {
        slots: Box::into_raw(slots).cast::<AtomicU64>(),
        len,
    }))
}

/// Frees a table of [`new_table`].
///
/// # Safety
///
/// `table` is from `new_table`, and nothing borrows it any more.
unsafe fn free_table(table: *mut Table) {
    // SAFETY: (the caller's) leaked by `new_table`.
    let table = unsafe { Box::from_raw(table) };
    // SAFETY: as above.
    drop(unsafe {
        Box::from_raw(ptr::slice_from_raw_parts_mut(
            table.slots.cast_mut(),
            table.len,
        ))
    });
}

/// Stores a slot in a table that has room. Under the shard's lock.
fn put(table: &Table, slot: u64) {
    let slots = table.slots();
    let mask = slots.len() - 1;
    let mut i = (slot >> 32) as usize & mask;
    while slots[i].load(Relaxed) != 0 {
        i = (i + 1) & mask;
    }
    slots[i].store(slot, Release);
}

impl Shard {
    /// A shard that starts with the given shared table.
    fn new(shared: &(SharedTable, usize)) -> Shard {
        Shard {
            r: ShardRead {
                table: AtomicPtr::new(shared.0.0),
                first: UnsafeCell::new(MaybeUninit::uninit()),
            },
            w: ShardWrite {
                writer: Mutex::new(Writer {
                    len: shared.1,
                    own: Own::Shared,
                    retired: Vec::new(),
                }),
            },
            first: FirstSlots(UnsafeCell::new(MaybeUninit::uninit())),
        }
    }

    #[inline]
    fn table(&self) -> &Table {
        // SAFETY: a table is freed only when the interner drops (a replaced one is retired).
        unsafe { &*self.r.table.load(Acquire) }
    }

    /// The shard's first own table, its slots zeroed: not yet published. Under the shard's lock,
    /// once.
    fn first_table(&self) -> *mut Table {
        let slots = self.first.0.get().cast::<AtomicU64>();
        // SAFETY: under the shard's lock, before `table` points to it: no other thread accesses
        // the first slots yet; an all-zero `AtomicU64` is valid.
        unsafe { ptr::write_bytes(slots, 0, TABLE0) };
        let header = self.r.first.get().cast::<Table>();
        // SAFETY: as above, for the header.
        unsafe { header.write(Table { slots, len: TABLE0 }) };
        header
    }

    /// The `Sym` of `bytes` in this shard, and its interned bytes.
    #[inline]
    fn find<'a>(&self, arenas: &'a [Arena], tag: u32, bytes: &[u8]) -> Option<(u32, &'a [u8])> {
        let slots = self.table().slots();
        let mask = slots.len() - 1;
        let mut i = tag as usize & mask;
        loop {
            let slot = slots[i].load(Acquire);
            if slot == 0 {
                return None;
            }
            if (slot >> 32) as u32 == tag {
                #[allow(clippy::cast_possible_truncation)] // the low half
                let sym = slot as u32;
                let interned = arenas[arena_of(sym)].bytes_published(offset_of(sym));
                if interned == bytes {
                    return Some((sym, interned));
                }
            }
            i = (i + 1) & mask;
        }
    }

    /// Publishes `sym` in this shard's table. Under the shard's lock.
    fn insert(&self, writer: &mut Writer, tag: u32, sym: u32) {
        let table = self.table();
        let full = (writer.len + 1) * 2 > table.len;
        let table = if full || writer.own == Own::Shared {
            // A larger table, or the shard's own copy of the shared one.
            let len = if full { table.len * 2 } else { table.len }.max(TABLE0);
            let (grown, own) = if len == TABLE0 && writer.own == Own::Shared {
                (self.first_table(), Own::First)
            } else {
                (new_table(len), Own::Heap)
            };
            // SAFETY: just made, and not shared yet.
            let grown_ref = unsafe { &*grown };
            for slot in table.slots() {
                let slot = slot.load(Relaxed);
                if slot != 0 {
                    put(grown_ref, slot);
                }
            }
            // Under the shard's lock: a plain load and store, rather than a locked swap.
            let old = self.r.table.load(Relaxed);
            self.r.table.store(grown, Release);
            if writer.own == Own::Heap {
                writer.retired.push(old);
            }
            writer.own = own;
            grown_ref
        } else {
            table
        };
        put(table, (u64::from(tag) << 32) | u64::from(sym));
        writer.len += 1;
    }
}

impl Drop for Shard {
    fn drop(&mut self) {
        let writer = self
            .w
            .writer
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let own = (writer.own == Own::Heap).then_some(*self.r.table.get_mut());
        let tables = own.into_iter().chain(writer.retired.drain(..));
        for table in tables {
            // SAFETY: each is from `new_table`; `&mut self`: nothing borrows them.
            unsafe { free_table(table) };
        }
    }
}

// -------------------------------------------------------------------------------------------------
// The arena of a thread
// -------------------------------------------------------------------------------------------------

/// The arena that threads share, under its lock: those that find no free index, and those whose
/// thread locals are being destroyed.
const SHARED_ARENA: usize = ARENAS - 1;

/// Which arena the threads insert into, the same in every interner: a thread takes a free index
/// the first time it inserts, and returns it when it exits. While it owns the index, no other
/// thread inserts into the arena, so it needs no lock. Once the 63 others are taken, threads
/// share the [last arena](SHARED_ARENA), under its lock.
struct ArenaIndices {
    free: Vec<usize>,
    next: usize,
}

static ARENA_INDICES: Mutex<ArenaIndices> = Mutex::new(ArenaIndices {
    free: Vec::new(),
    next: 0,
});

/// A thread's arena index; `owned` if it returns to the free list when the thread exits.
struct ThreadArena {
    index: usize,
    owned: bool,
}

impl ThreadArena {
    fn take() -> ThreadArena {
        let mut indices = lock(&ARENA_INDICES);
        if let Some(index) = indices.free.pop() {
            return ThreadArena { index, owned: true };
        }
        if indices.next < SHARED_ARENA {
            indices.next += 1;
            ThreadArena {
                index: indices.next - 1,
                owned: true,
            }
        } else {
            ThreadArena {
                index: SHARED_ARENA,
                owned: false,
            }
        }
    }
}

impl Drop for ThreadArena {
    fn drop(&mut self) {
        if self.owned {
            lock(&ARENA_INDICES).free.push(self.index);
        }
    }
}

/// The arena the current thread inserts into, and whether it owns it (or else must lock it).
fn thread_arena() -> (usize, bool) {
    thread_local!(static ARENA: ThreadArena = ThreadArena::take());
    ARENA
        .try_with(|arena| (arena.index, arena.owned))
        .unwrap_or((SHARED_ARENA, false))
}

// -------------------------------------------------------------------------------------------------
// The interner
// -------------------------------------------------------------------------------------------------

/// A set of interned strings, each designated by a [`Sym`] (ADR-04's P9 amendment).
///
/// It is `Send + Sync`: threads intern and resolve concurrently, and a lookup that finds its
/// string takes no lock. The strings live until the interner drops; each is stored once, and
/// never moves. A new interner holds the [well-known symbols](super#constants).
// Aligned so that in an `Arc`, its fields are not on the cache line of the reference counts.
#[repr(align(128))]
pub struct Interner {
    shards: Box<[Shard]>,
    arenas: Box<[Arena]>,
}

impl Interner {
    /// Constructs an interner that holds the well-known symbols.
    #[must_use]
    pub fn new() -> Interner {
        // It starts with the shared well-known strings and tables.
        let shared = shared();
        Interner {
            shards: shared.tables.iter().map(Shard::new).collect(),
            arenas: std::iter::once(Arena::with_well_known_chunks(shared))
                .chain((1..ARENAS).map(|_| Arena::new()))
                .collect(),
        }
    }

    /// The symbol of the given string, which is interned if it isn't yet.
    ///
    /// # Panics
    ///
    /// If the string is longer than [`MAX_LEN`] bytes, or if the arena of the current thread is
    /// full (128 MiB).
    #[inline]
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        sym(self.intern_bytes(s.as_ref().as_bytes()).0)
    }

    /// The symbol of the given string, and its interned bytes. See [`intern`](Self::intern).
    #[inline]
    pub(super) fn intern_bytes(&self, bytes: &[u8]) -> (u32, &[u8]) {
        assert!(
            bytes.len() <= MAX_LEN,
            "a string of {} bytes, longer than MAX_LEN",
            bytes.len()
        );
        let hash = hash(bytes);
        let shard = &self.shards[shard_index(hash)];
        match shard.find(&self.arenas, tag(hash), bytes) {
            Some(found) => found,
            None => self.insert(bytes, hash),
        }
    }

    /// Interns `bytes`, staged in the arena of this thread, unless another thread did meanwhile.
    #[cold]
    fn insert(&self, bytes: &[u8], hash: u64) -> (u32, &[u8]) {
        let shard = &self.shards[shard_index(hash)];
        let tag = tag(hash);
        let (arena_index, owned) = thread_arena();
        let arena = &self.arenas[arena_index];
        // The lock order: the arena (unless this thread owns it), then the shard.
        let _arena_lock = (!owned).then(|| lock(&arena.w.lock));
        // SAFETY: this thread owns the arena's index, or holds its lock.
        let state = unsafe { &mut *arena.w.state.get() };
        let (offset, new_end) = arena.stage(state, bytes);
        let mut writer = lock(&shard.w.writer);
        if let Some(found) = shard.find(&self.arenas, tag, bytes) {
            // The staged bytes stay unpublished.
            return found;
        }
        arena.publish(offset);
        state.end = new_end;
        arena.w.published_end.store(new_end, Relaxed);
        let sym =
            u32::try_from(((offset / 2) << ARENA_BITS) | arena_index).expect("within an arena");
        shard.insert(&mut writer, tag, sym);
        (sym, arena.bytes_published(offset))
    }

    /// The symbol of the given string, if it is interned.
    #[inline]
    pub fn get(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Option<Sym> {
        let bytes = s.as_ref().as_bytes();
        let hash = hash(bytes);
        let shard = &self.shards[shard_index(hash)];
        shard
            .find(&self.arenas, tag(hash), bytes)
            .map(|(id, _)| sym(id))
    }

    /// The string of the given symbol.
    ///
    /// # Panics
    ///
    /// If the symbol is not one of this interner's (see [`try_resolve`](Self::try_resolve)).
    #[inline]
    #[must_use]
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        match self.try_resolve(sym) {
            Some(s) => s,
            None => panic!("{sym:?} is not a symbol of this interner"),
        }
    }

    /// The string of the given symbol, or `None` if it is not one of this interner's. A symbol of
    /// another interner either resolves to some string of this one or gives `None`.
    #[inline]
    #[must_use]
    pub fn try_resolve(&self, sym: Sym) -> Option<&JavaStr> {
        let id = sym.as_u32();
        let bytes = self.arenas[arena_of(id)].bytes(offset_of(id))?;
        // SAFETY: interned bytes were copied from a `JavaStr`.
        Some(unsafe { JavaStr::from_semi_utf8_unchecked(bytes) })
    }

    /// The number of interned strings, including the well-known ones.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| lock(&shard.w.writer).len)
            .sum()
    }

    /// Always false: an interner holds the well-known symbols.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The size of the arenas' strings and their prefixes, in bytes (not counting the shared
    /// well-known strings).
    #[must_use]
    pub fn string_bytes(&self) -> usize {
        self.arenas
            .iter()
            .map(|arena| {
                arena.w.published_end.load(Relaxed) - (arena.w.shared_chunks * CHUNK).max(2)
            })
            .sum()
    }
}

impl Default for Interner {
    fn default() -> Self {
        Interner::new()
    }
}

impl fmt::Debug for Interner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Interner")
            .field("len", &self.len())
            .field("string_bytes", &self.string_bytes())
            .finish_non_exhaustive()
    }
}

#[inline]
fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.finish()
}

#[inline]
#[allow(clippy::cast_possible_truncation)] // the low bits
fn shard_index(hash: u64) -> usize {
    hash as usize & (SHARDS - 1)
}

#[inline]
fn tag(hash: u64) -> u32 {
    (hash >> 32) as u32
}

#[inline]
pub(super) fn sym(id: u32) -> Sym {
    Sym(NonZeroU32::new(id).expect("offsets start at 2"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jstr::JavaString;
    use crate::sym;

    #[test]
    fn well_known_symbols_are_seeded() {
        let interner = Interner::new();
        assert_eq!(interner.len(), WELL_KNOWN.len());
        assert_eq!(interner.intern("<init>"), sym::INIT);
        assert_eq!(interner.intern(""), sym::EMPTY);
        assert_eq!(interner.resolve(sym::VALUES), "values");
        assert_eq!(
            interner.resolve(sym::JAVA_LANG_INVOKE_LAMBDA_METAFACTORY),
            "java/lang/invoke/LambdaMetafactory"
        );
        let other = Interner::new();
        for (i, s) in WELL_KNOWN.iter().enumerate() {
            let expected = Sym::well_known(crate::sym::WELL_KNOWN_OFFSETS[i]);
            assert_eq!(interner.get(*s), Some(expected), "{s}");
            assert_eq!(interner.get(*s), other.get(*s));
            assert_eq!(interner.resolve(interner.intern(*s)), *s);
        }
    }

    #[test]
    fn interning_is_idempotent() {
        let interner = Interner::new();
        let a = interner.intern("java/util/List");
        let b = interner.intern(&JavaString::from("java/util/List"));
        assert_eq!(a, b);
        assert_ne!(a, interner.intern("java/util/Map"));
        assert_eq!(interner.get("java/util/List"), Some(a));
        assert_eq!(interner.get("java/util/Set"), None);
        assert_eq!(interner.resolve(a), "java/util/List");
    }

    #[test]
    fn lone_surrogates_round_trip() {
        let interner = Interner::new();
        let s = JavaString::from_utf16(&[0x61, 0xD800, 0x62]);
        let sym = interner.intern(&s);
        assert_eq!(interner.resolve(sym), &*s);
    }

    #[test]
    fn foreign_symbols_do_not_resolve_inside_strings() {
        let a = Interner::new();
        let b = Interner::new();
        let long = a.intern("abcdefghijklmnopqrstuvwxyz");
        // Every id in and around the string, in both interners: only real starts resolve.
        for id in long.as_u32().saturating_sub(64 * 4)..long.as_u32() + 64 * 20 {
            let Some(sym) = Sym::from_u32(id) else {
                continue;
            };
            for interner in [&a, &b] {
                if let Some(s) = interner.try_resolve(sym) {
                    assert_eq!(interner.get(s), Some(sym));
                }
            }
        }
        // The 2-byte units of the string's prefix and 26 bytes, past its start.
        for unit in 1..14 {
            let inside = Sym::from_u32(long.as_u32() + (unit << ARENA_BITS)).unwrap();
            assert!(
                a.try_resolve(inside).is_none(),
                "{inside:?} is inside {long:?}"
            );
        }
        assert!(
            Interner::new()
                .try_resolve(Sym::from_u32(u32::MAX).unwrap())
                .is_none()
        );
    }

    #[test]
    #[should_panic(expected = "not a symbol of this interner")]
    fn resolving_a_foreign_symbol_panics() {
        let a = Interner::new();
        let sym = a.intern("java/util/List");
        let _ = Interner::new().resolve(sym);
    }

    #[test]
    fn empty_and_long_strings() {
        let interner = Interner::new();
        let long = "x".repeat(MAX_LEN);
        let b = interner.intern(&*long);
        let c = interner.intern(&long[1..]);
        assert_eq!(interner.resolve(b), &*long);
        assert_eq!(interner.resolve(c), &long[1..]);
        assert_eq!(interner.intern(&*long), b);
        assert_eq!(interner.intern(""), sym::EMPTY);
    }

    #[test]
    #[should_panic(expected = "longer than MAX_LEN")]
    fn too_long_strings_panic() {
        let _ = Interner::new().intern(&*"x".repeat(MAX_LEN + 1));
    }

    #[test]
    fn concurrent_interning_agrees() {
        let n = if cfg!(miri) { 200 } else { 20_000 };
        let strings: Vec<JavaString> = (0..n)
            .map(|i| JavaString::from(format!("p/S{}", i % (n / 2)).as_str()))
            .collect();
        let interner = Interner::new();
        let results: Vec<Vec<Sym>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|t| {
                    let (interner, strings) = (&interner, &strings);
                    s.spawn(move || {
                        (0..strings.len())
                            .map(|k| {
                                let s = &strings[(k * 7 + t * 13) % strings.len()];
                                let sym = interner.intern(s);
                                assert_eq!(interner.resolve(sym), &**s);
                                sym
                            })
                            .collect()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(interner.len(), n / 2 + WELL_KNOWN.len());
        for (t, syms) in results.iter().enumerate() {
            for (k, &sym) in syms.iter().enumerate() {
                assert_eq!(
                    interner.intern(&strings[(k * 7 + t * 13) % strings.len()]),
                    sym
                );
            }
        }
        if cfg!(miri) {
            // Table growth and chunk rollover were exercised.
            let retired: usize = interner
                .shards
                .iter()
                .map(|s| lock(&s.w.writer).retired.len())
                .sum();
            let rolled = interner
                .arenas
                .iter()
                .filter(|a| {
                    a.chunk_pointers()
                        .is_some_and(|c| !c[1].load(Relaxed).is_null())
                })
                .count();
            assert!(
                retired > 0 && rolled > 0,
                "retired tables {retired}, arenas with a second chunk {rolled}"
            );
        }
    }

    #[test]
    fn threads_past_the_owned_arenas_share_one() {
        // More live threads than arenas: some of them insert into the shared arena, concurrently.
        let interner = Interner::new();
        let threads = ARENAS + 6;
        let barrier = std::sync::Barrier::new(threads);
        let syms: Vec<Vec<Sym>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let (interner, barrier) = (&interner, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        (0..3)
                            .map(|k| interner.intern(&*format!("s{}", (t + k) % 8)))
                            .collect()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(interner.len(), WELL_KNOWN.len() + 8);
        for (t, syms) in syms.iter().enumerate() {
            for (k, &sym) in syms.iter().enumerate() {
                assert_eq!(interner.resolve(sym), &*format!("s{}", (t + k) % 8));
            }
        }
    }

    #[test]
    fn thread_arenas_are_reused() {
        // `join` waits for the thread's locals to drop, which returns its arena index.
        let interner = std::sync::Arc::new(Interner::new());
        for i in 0..if cfg!(miri) { 4 } else { 100 } {
            let interner = interner.clone();
            std::thread::spawn(move || interner.intern(&*format!("t{i}")))
                .join()
                .unwrap();
        }
        // Other tests' threads hold indices meanwhile, but far fewer than 100.
        let used = interner
            .arenas
            .iter()
            .filter(|a| a.chunk_pointers().is_some())
            .count();
        assert!(used < 32, "{used} arenas used by sequential threads");
    }
}
