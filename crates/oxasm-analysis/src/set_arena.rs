//! [`SetArena`]: where the large sets of [`SmallSet`](crate::SmallSet)s are stored (optimization
//! lead A6).

use std::alloc::{self, Layout};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr::NonNull;

/// The storage of the [`SmallSet`](crate::SmallSet)s of two or more elements, and so of the
/// [`SourceValue`](crate::SourceValue)s that hold them: a set borrows its elements from an arena,
/// which frees them all at once, when it is dropped or [`reset`](Self::reset).
///
/// This makes sets and values `Copy`: copying one copies a reference, where a reference-counted
/// set would update the count, and dropping one does nothing, so the frames of an analysis copy
/// and drop as plain memory. Java's garbage collector gives ASM the same.
///
/// A [`SourceInterpreter`](crate::SourceInterpreter) allocates in the arena it was made with: a
/// union of sets that are not subsets of each other is a new set. The arena keeps every set until
/// it is dropped or reset, so a long-lived arena grows with the analyses that use it. One arena per
/// class (or per method) bounds it:
///
/// ```
/// # use oxasm_analysis::{Analyzer, SetArena, SourceInterpreter};
/// # fn analyze_class(class_node: &oxasm_tree::ClassNode) {
/// let mut sets = SetArena::new();
/// for method in &class_node.methods {
///     let mut analyzer = Analyzer::new(SourceInterpreter::new(&sets));
///     // ... analyze the method and use its frames ...
///     drop(analyzer);
///     sets.reset(); // the borrow checker rejects this while any value still borrows `sets`
/// }
/// # }
/// ```
pub struct SetArena {
    /// Every chunk, the current one last.
    chunks: RefCell<Vec<Chunk>>,
    /// The free part of the current chunk: its first byte, and how many bytes it has.
    free: Cell<Option<NonNull<u8>>>,
    free_len: Cell<usize>,
}

/// A block of memory of the arena, freed when the arena drops it.
struct Chunk {
    ptr: NonNull<u8>,
    layout: Layout,
}

/// The size of the first chunk; each next one is twice as large as the previous one, up to
/// [`MAX_CHUNK`] (or larger, for a larger allocation).
const MIN_CHUNK: usize = 4 << 10;
const MAX_CHUNK: usize = 1 << 20;
/// The alignment of the chunks, enough for any element type of a `SmallSet` but an over-aligned
/// one, which gets a chunk of its own alignment.
const CHUNK_ALIGN: usize = 16;

impl SetArena {
    /// An empty arena. It allocates nothing until its first set.
    pub fn new() -> SetArena {
        SetArena {
            chunks: RefCell::new(Vec::new()),
            free: Cell::new(None),
            free_len: Cell::new(0),
        }
    }

    /// Frees every set of this arena, keeping its largest block of memory for the next ones.
    pub fn reset(&mut self) {
        let chunks = self.chunks.get_mut();
        let largest = chunks
            .iter()
            .enumerate()
            .max_by_key(|(_, chunk)| chunk.layout.size())
            .map(|(index, _)| index);
        let Some(largest) = largest else {
            return;
        };
        let kept = chunks.swap_remove(largest);
        chunks.clear();
        self.free.set(Some(kept.ptr));
        self.free_len.set(kept.layout.size());
        chunks.push(kept);
    }

    /// The number of bytes this arena has taken from the allocator.
    pub fn allocated_bytes(&self) -> usize {
        self.chunks
            .borrow()
            .iter()
            .map(|chunk| chunk.layout.size())
            .sum()
    }

    /// A copy of `values` in this arena.
    pub(crate) fn alloc_slice<T: Copy>(&self, values: &[T]) -> &[T] {
        if values.is_empty() {
            return &[];
        }
        let size = size_of_val(values);
        let ptr = self.alloc_bytes(size, align_of::<T>()).cast::<T>();
        // SAFETY: `alloc_bytes` returned `size` bytes for this copy alone, aligned for `T`, in a
        // chunk that lives as long as `&self` (chunks are only freed by `reset`, which takes
        // `&mut self`, or by the drop). `values` is borrowed, so it does not overlap them.
        unsafe {
            ptr.as_ptr()
                .copy_from_nonoverlapping(values.as_ptr(), values.len())
        };
        // SAFETY: the `values.len()` elements at `ptr` were just initialized, as `T` (which is
        // `Copy`, so no drop is ever owed), and nothing writes them again while `&self` lives.
        unsafe { std::slice::from_raw_parts(ptr.as_ptr(), values.len()) }
    }

    /// A copy of `value` in this arena.
    pub(crate) fn alloc<T: Copy>(&self, value: T) -> &T {
        &self.alloc_slice(std::slice::from_ref(&value))[0]
    }

    /// `size` (non-zero) bytes of this arena, aligned to `align`, for one allocation.
    fn alloc_bytes(&self, size: usize, align: usize) -> NonNull<u8> {
        if let Some(free) = self.free.get() {
            let padding = free.as_ptr().align_offset(align);
            let free_len = self.free_len.get();
            if padding <= free_len && size <= free_len - padding {
                // SAFETY: `padding + size <= free_len`, so both offsets stay within the free part
                // of the current chunk.
                let start = unsafe { free.add(padding) };
                // SAFETY: as above.
                self.free.set(Some(unsafe { start.add(size) }));
                self.free_len.set(free_len - padding - size);
                return start;
            }
        }
        self.alloc_chunk(size, align)
    }

    /// A new current chunk with room for `size` bytes aligned to `align`, and those bytes.
    #[cold]
    fn alloc_chunk(&self, size: usize, align: usize) -> NonNull<u8> {
        let mut chunks = self.chunks.borrow_mut();
        let previous = chunks.last().map_or(0, |chunk| chunk.layout.size());
        let chunk_size = (previous * 2).clamp(MIN_CHUNK, MAX_CHUNK).max(size);
        let layout = Layout::from_size_align(chunk_size, align.max(CHUNK_ALIGN))
            .expect("a set arena chunk of a valid size");
        // SAFETY: `layout` has a non-zero size (at least `MIN_CHUNK`).
        let ptr = unsafe { alloc::alloc(layout) };
        let Some(ptr) = NonNull::new(ptr) else {
            alloc::handle_alloc_error(layout);
        };
        chunks.push(Chunk { ptr, layout });
        // SAFETY: `size <= chunk_size`, the chunk's size; its start is aligned to `align`.
        self.free.set(Some(unsafe { ptr.add(size) }));
        self.free_len.set(chunk_size - size);
        ptr
    }
}

impl Default for SetArena {
    fn default() -> SetArena {
        SetArena::new()
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: `ptr` was allocated by `alloc::alloc` with `layout`, and is freed only here.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

// SAFETY: the arena owns its chunks, as a `Vec<Box<[u8]>>` would, and its cells point into them;
// moving it to another thread moves all of that, and no set can borrow it across the move. It is
// not `Sync` (its `Cell`s), so no two threads allocate in it at once.
unsafe impl Send for SetArena {}

impl fmt::Debug for SetArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetArena")
            .field("chunks", &self.chunks.borrow().len())
            .field("allocated_bytes", &self.allocated_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_stay_valid_as_chunks_are_added() {
        let arena = SetArena::new();
        let slices: Vec<&[(u64, u32)]> = (0..2000u32)
            .map(|i| {
                let values: Vec<(u64, u32)> = (0..i % 37).map(|j| (u64::from(i), j)).collect();
                arena.alloc_slice(&values)
            })
            .collect();
        for (i, slice) in (0..2000u32).zip(&slices) {
            assert_eq!(slice.len(), (i % 37) as usize);
            assert!(
                slice
                    .iter()
                    .zip(0..)
                    .all(|(&(a, b), j)| a == u64::from(i) && b == j)
            );
        }
        assert!(arena.chunks.borrow().len() > 1);
    }

    #[test]
    fn allocations_are_aligned() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        #[repr(align(64))]
        struct Wide(u8);
        let arena = SetArena::new();
        let a = arena.alloc(1u8);
        let b = arena.alloc(2u64);
        let c = arena.alloc(Wide(3));
        let d = arena.alloc_slice(&[4u16, 5, 6]);
        assert_eq!((*a, *b, *c, d), (1, 2, Wide(3), &[4u16, 5, 6][..]));
        assert_eq!(std::ptr::from_ref(b).addr() % align_of::<u64>(), 0);
        assert_eq!(std::ptr::from_ref(c).addr() % 64, 0);
        assert!(arena.alloc_slice::<u32>(&[]).is_empty());
    }

    #[test]
    fn a_large_allocation_gets_a_chunk_of_its_size() {
        let arena = SetArena::new();
        let values = vec![7u64; MAX_CHUNK / 4];
        let slice = arena.alloc_slice(&values);
        assert_eq!(slice, &values[..]);
        let small = arena.alloc(8u64);
        assert_eq!((*small, slice[0]), (8, 7));
    }

    #[test]
    fn reset_keeps_the_largest_chunk() {
        let mut arena = SetArena::new();
        for i in 0..5000u64 {
            arena.alloc_slice(&[i, i + 1, i + 2]);
        }
        let largest = arena.chunks.borrow().iter().map(|c| c.layout.size()).max();
        arena.reset();
        assert_eq!(arena.chunks.borrow().len(), 1);
        assert_eq!(Some(arena.allocated_bytes()), largest);
        let after = arena.alloc_slice(&[1u64, 2, 3]);
        assert_eq!(after, &[1, 2, 3]);
        assert_eq!(arena.chunks.borrow().len(), 1);
        let mut empty = SetArena::new();
        empty.reset();
        assert_eq!(empty.allocated_bytes(), 0);
    }

    #[test]
    fn an_arena_moves_to_another_thread() {
        let arena = SetArena::new();
        arena.alloc_slice(&[1u32, 2, 3]);
        let bytes = std::thread::spawn(move || {
            let slice = arena.alloc_slice(&[4u32, 5]);
            assert_eq!(slice, &[4, 5]);
            arena.allocated_bytes()
        })
        .join()
        .unwrap();
        assert_eq!(bytes, MIN_CHUNK);
    }
}
