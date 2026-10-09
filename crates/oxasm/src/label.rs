//! [`Label`]: a position in the bytecode of a method (ADR-03).

use std::cell::Cell;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// A position in the bytecode of a method. Labels are used for jump, goto, and switch
/// instructions, and for try catch blocks. A label designates the *instruction* that is just
/// after. Note however that there can be other elements between a label and the instruction it
/// designates (such as other labels, stack map frames, line numbers, etc.).
///
/// Unlike ASM's `Label`, this is only an identity: a process-wide unique, `Copy` id. Everything
/// ASM stores *in* a label (its bytecode offset, forward references, basic block data, the
/// `info` field) lives in side tables of the consumer that needs it (the method writer, the tree,
/// a printer), so the same label can be used by several consumers at once, from several threads.
/// A label's bytecode offset, once known, is asked of the method writer that resolved it.
///
/// The one exception is ASM's `FLAG_DEBUG_ONLY`, which a class reader sets on the labels it
/// creates only for debug information (line numbers, local variables), and which tells a method
/// writer that such a label does not start a basic block. Like in ASM, it travels with the label
/// through any adapter, so it is a bit of the label value; it is not part of the label's identity
/// (equality, ordering and hashing ignore it).
#[derive(Clone, Copy)]
pub struct Label(NonZeroU64);

/// The bit of a label value that holds ASM's `FLAG_DEBUG_ONLY`.
const DEBUG_ONLY: u64 = 1 << 63;

/// The number of ids a thread takes from [`NEXT_LABEL_ID`] at once. The blocks are
/// `k * ID_BLOCK + 1 ..= (k + 1) * ID_BLOCK`, so a thread's ids stay consecutive modulo
/// `ID_BLOCK` from one block to the next (a direct-mapped table of up to `ID_BLOCK` slots, like
/// the method writer's, sees no collisions between the labels a thread creates in a row).
const ID_BLOCK: u64 = 1 << 12;

/// The start of the next block of ids.
static NEXT_LABEL_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The next id of this thread's block and the end of the block (`0..0` before its first
    /// label): one global `fetch_add` per [`ID_BLOCK`] labels instead of one per label.
    static LABEL_IDS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// Takes a new block of ids for this thread and returns its first id.
#[cold]
fn next_id_block(ids: &Cell<(u64, u64)>) -> u64 {
    let start = NEXT_LABEL_ID.fetch_add(ID_BLOCK, Ordering::Relaxed);
    assert!(start < DEBUG_ONLY - ID_BLOCK, "2^63 labels created");
    ids.set((start + 1, start + ID_BLOCK));
    start
}

impl Label {
    /// Creates a new label, distinct from every label created before (by any thread).
    #[allow(clippy::new_without_default)] // a default would be a fresh label, which is surprising
    #[inline]
    #[must_use]
    pub fn new() -> Label {
        let id = LABEL_IDS.with(|ids| {
            let (next, end) = ids.get();
            if next < end {
                ids.set((next + 1, end));
                next
            } else {
                next_id_block(ids)
            }
        });
        Label(NonZeroU64::new(id).expect("label ids start at 1"))
    }

    /// The id of this label: unique in the process, increasing in creation order within a thread.
    #[inline]
    #[must_use]
    pub fn id(self) -> u64 {
        self.0.get() & !DEBUG_ONLY
    }

    /// Whether this label was created by a class reader only for debug information (ASM's
    /// `FLAG_DEBUG_ONLY`): it designates the start or end of a local variable range, or a line
    /// number, but no instruction refers to it.
    #[doc(hidden)]
    #[inline]
    #[must_use]
    pub fn is_debug_only(self) -> bool {
        self.0.get() & DEBUG_ONLY != 0
    }

    /// This label, with ASM's `FLAG_DEBUG_ONLY` set or cleared (the same label for every other
    /// purpose). The tree clears it, as ASM's `LabelNode` hands out fresh labels.
    #[doc(hidden)]
    #[inline]
    #[must_use]
    pub fn with_debug_only(self, debug_only: bool) -> Label {
        let value = if debug_only {
            self.0.get() | DEBUG_ONLY
        } else {
            self.id()
        };
        // The id part is never zero.
        Label(NonZeroU64::new(value).expect("labels have a non zero id"))
    }
}

impl PartialEq for Label {
    fn eq(&self, other: &Label) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Label {}

impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Label) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Label {
    fn cmp(&self, other: &Label) -> std::cmp::Ordering {
        self.id().cmp(&other.id())
    }
}

impl std::hash::Hash for Label {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L#{}", self.id())
    }
}

/// `L#<id>`, like ASM's `"L" + System.identityHashCode(this)`.
impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique_and_ordered() {
        let (a, b) = (Label::new(), Label::new());
        assert_ne!(a, b);
        assert!(a < b);
        let c = a;
        assert_eq!(a, c);
    }

    #[test]
    fn debug_only_flag_is_not_part_of_the_identity() {
        let a = Label::new();
        let b = a.with_debug_only(true);
        assert!(b.is_debug_only() && !a.is_debug_only());
        assert_eq!(a, b);
        assert_eq!(a.id(), b.id());
        let hash = |l: Label| {
            use std::hash::{BuildHasher, BuildHasherDefault};
            BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().hash_one(l)
        };
        assert_eq!(hash(a), hash(b));
        assert!(!b.with_debug_only(false).is_debug_only());
        assert_eq!(format!("{a}"), format!("{b}"));
    }

    #[test]
    fn a_threads_ids_are_consecutive_modulo_the_block_size() {
        // Another thread takes blocks in between, so the blocks of this one need not be adjacent.
        let ids: Vec<u64> = std::thread::scope(|s| {
            s.spawn(|| (0..ID_BLOCK).for_each(|_| _ = Label::new()));
            s.spawn(|| (0..3 * ID_BLOCK).map(|_| Label::new().id()).collect())
                .join()
                .unwrap()
        });
        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1]);
            assert_eq!((pair[0] + 1) % ID_BLOCK, pair[1] % ID_BLOCK);
        }
    }

    #[test]
    fn labels_are_unique_across_threads() {
        let ids: Vec<u64> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        (0..3 * ID_BLOCK)
                            .map(|_| Label::new().id())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }
}
