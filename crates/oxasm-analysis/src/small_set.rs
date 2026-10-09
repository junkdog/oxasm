//! [`SmallSet`]: ASM's `SmallSet`, the instruction set of a [`SourceValue`](crate::SourceValue).

use std::borrow::Cow;
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};

use rustc_hash::FxBuildHasher;

use crate::set_arena::SetArena;

/// An immutable set, optimized for sets of at most two elements (ASM's `SmallSet`, which is
/// limited to two elements, and the `HashSet`s its `union` returns for more).
///
/// Sets are equal if they have the same elements. Elements iterate in insertion order up to two
/// elements, in an unspecified order beyond.
///
/// A set of two elements is a pair, and a set of three or more elements a slice sorted by the
/// elements' hashes, in a [`SetArena`] that the set borrows for `'s`: its copies share it, as
/// Java's references do, and a set is `Copy`, with nothing to drop, and as small as one element
/// and a tag (16 bytes for an [`InsnId`](oxasm_tree::InsnId)). A union remembers the sets it was
/// made of: an analysis copies its frames, and merges mostly compare a value with a copy of itself
/// or with one of the sets its union was made of, which then costs no walk over the elements, and
/// otherwise with a subset, one merge walk over both slices.
pub struct SmallSet<'s, T>(Repr<'s, T>);

enum Repr<'s, T> {
    Empty,
    One(T),
    /// Two distinct elements, in a [`SetArena`], in insertion order.
    Two(&'s [T; 2]),
    /// At least three elements, in a [`SetArena`].
    Many(&'s Many<'s, T>),
}

impl<T: Copy> Clone for SmallSet<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Copy> Copy for SmallSet<'_, T> {}

impl<T: Copy> Clone for Repr<'_, T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Copy> Copy for Repr<'_, T> {}

/// A set of at least three elements. Its address identifies it: sets are only freed with their
/// arena, so no two sets that can be compared share an address.
#[derive(Clone, Copy)]
struct Many<'s, T> {
    /// The addresses of sets of at least three elements known to be subsets of this one (the
    /// operands of the union that made it, and theirs), 0 for none.
    subsets: [usize; SUBSETS],
    /// The elements with their hashes, sorted by hash (elements of equal hashes in any order).
    elements: &'s [Entry<T>],
}

/// An element of a [`Many`] set and its hash.
type Entry<T> = (u64, T);

fn hash_of<T: Hash>(element: &T) -> u64 {
    FxBuildHasher.hash_one(element)
}

/// How many subsets a [`Many`] remembers.
const SUBSETS: usize = 6;

impl<T: Copy + Eq> Many<'_, T> {
    /// The identity of this set.
    fn id(&self) -> usize {
        std::ptr::from_ref(self).addr()
    }

    /// Whether `other` is known to be a subset of this set without looking at their elements.
    fn has_known_subset(&self, other: &Many<'_, T>) -> bool {
        let id = other.id();
        self.id() == id || self.subsets.contains(&id)
    }

    /// Whether this set contains `element`, of hash `hash`.
    fn contains(&self, hash: u64, element: &T) -> bool {
        let start = self.elements.partition_point(|&(h, _)| h < hash);
        self.elements[start..]
            .iter()
            .take_while(|&&(h, _)| h == hash)
            .any(|(_, e)| e == element)
    }

    /// Whether this set contains every element of `elements`.
    #[inline]
    fn contains_all(&self, elements: &[T]) -> bool
    where
        T: Hash,
    {
        elements.iter().all(|e| self.contains(hash_of(e), e))
    }

    /// Whether every element of `other` is in this set.
    fn is_superset(&self, other: &Many<'_, T>) -> bool {
        let (a, b) = (self.elements, other.elements);
        if self.has_known_subset(other) {
            return true;
        }
        if a.len() < b.len() {
            return false;
        }
        if b.len() * (a.len().ilog2() as usize + 1) < a.len() {
            // A few elements in a large set: a binary search each.
            return b
                .iter()
                .all(|(hash, element)| self.contains(*hash, element));
        }
        let mut i = 0;
        b.iter().all(|&(hash, element)| {
            while i < a.len() && a[i].0 < hash {
                i += 1;
            }
            // `i` stays at the start of the run of `hash`, which the next element may share.
            a[i..]
                .iter()
                .take_while(|&&(h, _)| h == hash)
                .any(|&(_, e)| e == element)
        })
    }
}

impl<'s, T: Copy + Eq + Hash> SmallSet<'s, T> {
    /// The empty set.
    pub fn new() -> SmallSet<'s, T> {
        SmallSet(Repr::Empty)
    }

    /// The set with exactly one element.
    pub fn singleton(element: T) -> SmallSet<'s, T> {
        SmallSet(Repr::One(element))
    }

    /// The set of the given elements: the union of their singletons, its sets of two or more
    /// elements in `sets`.
    pub fn from_elements<I: IntoIterator<Item = T>>(
        elements: I,
        sets: &'s SetArena,
    ) -> SmallSet<'s, T> {
        elements.into_iter().fold(SmallSet::new(), |set, element| {
            set.union(&SmallSet::singleton(element), sets)
        })
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Empty => 0,
            Repr::One(_) => 1,
            Repr::Two(..) => 2,
            Repr::Many(set) => set.elements.len(),
        }
    }

    /// Whether this set is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self.0, Repr::Empty)
    }

    /// Whether this set contains `element`.
    pub fn contains(&self, element: &T) -> bool {
        match &self.0 {
            Repr::Empty => false,
            Repr::One(a) => a == element,
            Repr::Two([a, b]) => a == element || b == element,
            Repr::Many(set) => set.contains(hash_of(element), element),
        }
    }

    /// Whether this set and `other` are equal in the same form: the same elements in the same
    /// order up to two elements, the same set in an arena beyond. Equal sets can be in different
    /// forms (two elements in the other order, or equal elements in two sets of an arena), so
    /// `false` does not mean they differ: it is a cheap first check, which most of an analysis's
    /// merges pass.
    #[inline]
    pub(crate) fn is_identical(&self, other: &SmallSet<'_, T>) -> bool {
        match (&self.0, &other.0) {
            (Repr::Empty, Repr::Empty) => true,
            (Repr::One(a), Repr::One(b)) => a == b,
            (Repr::Two(a), Repr::Two(b)) => std::ptr::eq(*a, *b) || a == b,
            (Repr::Many(a), Repr::Many(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }

    /// Whether every element of `other` is in this set.
    pub fn is_superset(&self, other: &SmallSet<'_, T>) -> bool {
        match (&self.0, &other.0) {
            (_, Repr::Empty) => true,
            (_, Repr::One(b)) => self.contains(b),
            (Repr::Empty | Repr::One(_), _) => false,
            (_, Repr::Two([c, d])) => self.contains(c) && self.contains(d),
            (Repr::Two(..), Repr::Many(_)) => false,
            (Repr::Many(a), Repr::Many(b)) => a.is_superset(b),
        }
    }

    /// An iterator over the elements.
    pub fn iter(&self) -> impl Iterator<Item = T> + 's {
        let (inline, many) = match self.0 {
            Repr::Empty => ([None, None], None),
            Repr::One(a) => ([Some(a), None], None),
            Repr::Two(&[a, b]) => ([Some(a), Some(b)], None),
            Repr::Many(set) => ([None, None], Some(set.elements.iter().map(|&(_, e)| e))),
        };
        inline
            .into_iter()
            .flatten()
            .chain(many.into_iter().flatten())
    }

    /// The union of this set and `other` (ASM's `union`). A set of two or more elements that is
    /// neither operand is allocated in `sets`.
    pub fn union(&self, other: &SmallSet<'s, T>, sets: &'s SetArena) -> SmallSet<'s, T> {
        self.union_if_grown(other, sets).unwrap_or(*self)
    }

    /// The [`union`](Self::union) of this set and `other`, or `None` if it is this set (`other`
    /// is a subset of it). Each case of the two sets' forms is decided by one test of their
    /// elements, where a superset test and then a union would test them up to three times.
    #[inline]
    pub(crate) fn union_if_grown(
        &self,
        other: &SmallSet<'s, T>,
        sets: &'s SetArena,
    ) -> Option<SmallSet<'s, T>> {
        match (&self.0, &other.0) {
            (_, Repr::Empty) => None,
            (Repr::Empty, _) => Some(*other),
            (Repr::One(a), Repr::One(b)) if a == b => None,
            (Repr::One(a), Repr::One(b)) => Some(SmallSet(Repr::Two(sets.alloc([*a, *b])))),
            (Repr::One(a), Repr::Two([c, d])) if a == c || a == d => Some(*other),
            (Repr::Two([a, b]), Repr::One(c)) if c == a || c == b => None,
            (Repr::Two([a, b]), Repr::Two([c, d])) if (a == c && b == d) || (a == d && b == c) => {
                None
            }
            (Repr::One(_) | Repr::Two(_), Repr::One(_) | Repr::Two(_)) => {
                Some(self.new_union(other, sets))
            }
            (Repr::Many(set), Repr::One(_) | Repr::Two(_)) => {
                if set.contains_all(other.inline_elements()) {
                    None
                } else {
                    Some(self.new_union(other, sets))
                }
            }
            (Repr::One(_) | Repr::Two(_), Repr::Many(set)) => {
                if set.contains_all(self.inline_elements()) {
                    Some(*other)
                } else {
                    Some(self.new_union(other, sets))
                }
            }
            (Repr::Many(a), Repr::Many(b)) => {
                if a.is_superset(b) {
                    None
                } else if b.is_superset(a) {
                    Some(*other)
                } else {
                    Some(self.new_union(other, sets))
                }
            }
        }
    }

    /// The union of this set and `other`, neither a subset of the other, as a new set in `sets`.
    #[inline(never)]
    fn new_union(&self, other: &SmallSet<'s, T>, sets: &'s SetArena) -> SmallSet<'s, T> {
        let elements = sets.alloc_slice(&merge(&self.entries(), &other.entries()));
        // The operands first, then the subsets they remember, nearest first.
        let mut subsets = [0; SUBSETS];
        let operands = [&self.0, &other.0].map(|operand| match operand {
            Repr::Many(set) => Some(*set),
            _ => None,
        });
        let ids = operands.iter().flatten().map(|set| set.id()).chain(
            operands
                .iter()
                .flatten()
                .flat_map(|set| set.subsets.iter().copied().filter(|&id| id != 0)),
        );
        for (slot, id) in subsets.iter_mut().zip(ids) {
            *slot = id;
        }
        SmallSet(Repr::Many(sets.alloc(Many { subsets, elements })))
    }

    /// The elements of a set of at most two elements (none for a larger set).
    #[inline]
    fn inline_elements(&self) -> &[T] {
        match &self.0 {
            Repr::One(a) => std::slice::from_ref(a),
            Repr::Two(pair) => &pair[..],
            Repr::Empty | Repr::Many(_) => &[],
        }
    }

    /// The elements with their hashes, sorted by hash.
    fn entries(&self) -> Cow<'s, [Entry<T>]> {
        if let Repr::Many(set) = self.0 {
            return Cow::Borrowed(set.elements);
        }
        let mut entries: Vec<Entry<T>> = self.iter().map(|e| (hash_of(&e), e)).collect();
        entries.sort_unstable_by_key(|&(h, _)| h);
        Cow::Owned(entries)
    }
}

/// The union of two sets of entries sorted by hash, sorted by hash.
fn merge<T: Copy + Eq>(a: &[Entry<T>], b: &[Entry<T>]) -> Vec<Entry<T>> {
    let mut out: Vec<Entry<T>> = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    // The start of the run of `out`'s last hash.
    let mut run = 0;
    while i < a.len() || j < b.len() {
        let entry = if j == b.len() || (i < a.len() && a[i].0 <= b[j].0) {
            i += 1;
            a[i - 1]
        } else {
            j += 1;
            b[j - 1]
        };
        if out.last().is_some_and(|&(h, _)| h == entry.0) {
            if out[run..].iter().any(|&(_, e)| e == entry.1) {
                continue;
            }
        } else {
            run = out.len();
        }
        out.push(entry);
    }
    out
}

impl<T: Copy + Eq + Hash> Default for SmallSet<'_, T> {
    fn default() -> Self {
        SmallSet::new()
    }
}

impl<T: Copy + Eq + Hash> PartialEq for SmallSet<'_, T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // An analysis compares mostly identical copies (a frame's return value with the last one).
        self.is_identical(other) || (self.len() == other.len() && self.is_superset(other))
    }
}

impl<T: Copy + Eq + Hash> Eq for SmallSet<'_, T> {}

impl<T: Copy + Eq + Hash> Hash for SmallSet<'_, T> {
    /// A hash that does not depend on the order of the elements: the sum of their hashes, as
    /// Java's `AbstractSet.hashCode()` (0 for the empty set).
    fn hash<H: Hasher>(&self, state: &mut H) {
        let sum = match &self.0 {
            Repr::Many(set) => set
                .elements
                .iter()
                .map(|&(h, _)| h)
                .fold(0, u64::wrapping_add),
            _ => self.iter().map(|e| hash_of(&e)).fold(0, u64::wrapping_add),
        };
        state.write_u64(sum);
    }
}

impl<T: Copy + Eq + Hash + fmt::Debug> fmt::Debug for SmallSet<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}
