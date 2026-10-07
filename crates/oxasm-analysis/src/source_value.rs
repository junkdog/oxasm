//! [`SourceValue`]: ASM's `SourceValue`.

use std::hash::Hash;

use oxasm_tree::InsnId;

use crate::small_set::SmallSet;
use crate::value::Value;

/// What a [`SourceValue`] records as the producer of a value: an instruction ([`InsnId`]), or a type
/// that instructions convert into, for the interpreters whose values can also come from elsewhere.
///
/// ASM's `SourceValue.insns` is a set of `AbstractInsnNode`s, and an interpreter that needs a
/// producer that is no instruction (sift's `ParameterOrigin`, the caller of a method, for a
/// parameter's entry value) subclasses `AbstractInsnNode`. Here, it uses its own producer type:
///
/// ```
/// # use oxasm_tree::InsnId;
/// #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// enum Origin {
///     Insn(InsnId),
///     Parameter(usize),
/// }
///
/// impl From<InsnId> for Origin {
///     fn from(insn: InsnId) -> Origin {
///         Origin::Insn(insn)
///     }
/// }
/// // `SourceValue<Origin>` is now a value of the `SourceInterpreter` operations.
/// ```
///
/// It is implemented for every type with these bounds.
pub trait Producer: Copy + Eq + Hash + From<InsnId> {}

impl<P: Copy + Eq + Hash + From<InsnId>> Producer for P {}

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
/// The producers are [`InsnId`]s by default, or any other [`Producer`].
///
/// Like ASM's `hashCode()`, its hash depends on its producers only.
#[derive(Clone, Debug)]
pub struct SourceValue<P = InsnId> {
    /// The size of this value, in 32 bits words: 1 for `byte`, `boolean`, `char`, `short`,
    /// `int`, `float`, object and array types, and 2 for `long` and `double`.
    pub size: usize,
    /// The instructions (or other producers) that can produce this value.
    pub insns: SmallSet<P>,
}

impl<P: Producer> SourceValue<P> {
    /// A value of the given size, produced by nothing.
    pub fn new(size: usize) -> SourceValue<P> {
        SourceValue {
            size,
            insns: SmallSet::new(),
        }
    }

    /// A value of the given size, produced by `insn`.
    pub fn of(size: usize, insn: P) -> SourceValue<P> {
        SourceValue {
            size,
            insns: SmallSet::singleton(insn),
        }
    }

    /// A value of the given size, produced by the producers of `insns`.
    pub fn with_insns(size: usize, insns: SmallSet<P>) -> SourceValue<P> {
        SourceValue { size, insns }
    }
}

impl<P: Producer> PartialEq for SourceValue<P> {
    fn eq(&self, other: &SourceValue<P>) -> bool {
        self.size == other.size && self.insns == other.insns
    }
}

impl<P: Producer> Eq for SourceValue<P> {}

impl<P: Producer> Hash for SourceValue<P> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.insns.hash(state);
    }
}

impl<P: Producer> Value for SourceValue<P> {
    fn get_size(&self) -> usize {
        self.size
    }
}
