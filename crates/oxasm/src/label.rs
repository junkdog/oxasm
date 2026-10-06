//! [`Label`]: a position in the bytecode of a method (ADR-03).

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

static NEXT_LABEL_ID: AtomicU64 = AtomicU64::new(1);

impl Label {
    /// Creates a new label, distinct from every label created before.
    #[allow(clippy::new_without_default)] // a default would be a fresh label, which is surprising
    #[must_use]
    pub fn new() -> Label {
        let id = NEXT_LABEL_ID.fetch_add(1, Ordering::Relaxed);
        Label(NonZeroU64::new(id).expect("2^64 labels created"))
    }

    /// The id of this label: unique in the process, increasing in creation order.
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
    /// purpose).
    #[inline]
    #[must_use]
    pub(crate) fn with_debug_only(self, debug_only: bool) -> Label {
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
    fn labels_are_unique_across_threads() {
        let ids: Vec<u64> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| s.spawn(|| (0..1000).map(|_| Label::new().id()).collect::<Vec<_>>()))
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
