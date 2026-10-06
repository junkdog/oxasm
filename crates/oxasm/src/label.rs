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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label(NonZeroU64);

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
        self.0.get()
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L#{}", self.0)
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
