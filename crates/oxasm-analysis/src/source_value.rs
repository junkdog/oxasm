//! [`SourceValue`]: ASM's `SourceValue`.

use oxasm_tree::InsnId;

use crate::small_set::SmallSet;
use crate::value::Value;

/// A [`Value`] that keeps track of the instructions that can produce it (ASM's `SourceValue`).
///
/// For instance, for the Java code below, the instructions that can produce the value of `i` at
/// line 5 are the two `ISTORE` instructions at lines 1 and 3:
///
/// ```text
/// 1: i = 0;
/// 2: if (...) {
/// 3:   i = 1;
/// 4: }
/// 5: return i;
/// ```
///
/// Like ASM's `hashCode()`, its hash depends on its instructions only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceValue {
    /// The size of this value, in 32 bits words: 1 for `byte`, `boolean`, `char`, `short`,
    /// `int`, `float`, object and array types, and 2 for `long` and `double`.
    pub size: usize,
    /// The instructions that can produce this value.
    pub insns: SmallSet<InsnId>,
}

impl SourceValue {
    /// A value of the given size, produced by no instruction.
    pub fn new(size: usize) -> SourceValue {
        SourceValue {
            size,
            insns: SmallSet::new(),
        }
    }

    /// A value of the given size, produced by `insn`.
    pub fn of(size: usize, insn: InsnId) -> SourceValue {
        SourceValue {
            size,
            insns: SmallSet::singleton(insn),
        }
    }

    /// A value of the given size, produced by the instructions of `insns`.
    pub fn with_insns(size: usize, insns: SmallSet<InsnId>) -> SourceValue {
        SourceValue { size, insns }
    }
}

impl std::hash::Hash for SourceValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.insns.hash(state);
    }
}

impl Value for SourceValue {
    fn get_size(&self) -> usize {
        self.size
    }
}
