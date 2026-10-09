//! [`SmallSet`]: ASM's `SmallSet`, the instruction set of a [`SourceValue`](crate::SourceValue).

use std::borrow::Cow;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::FxBuildHasher;

/// An immutable set, optimized for sets of at most two elements (ASM's `SmallSet`, which is
/// limited to two elements, and the `HashSet`s its `union` returns for more).
///
/// Sets are equal if they have the same elements. Elements iterate in insertion order up to two
/// elements, in an unspecified order beyond.
///
/// A set of three or more elements is a slice sorted by the elements' hashes, which its clones
/// share, as Java's references do, and a union remembers the sets it was made of: an analysis
/// copies its frames, and merges mostly compare a value with a copy of itself or with one of the
/// sets its union was made of, which then costs no walk over the elements, and otherwise with a
/// subset, one merge walk over both slices.
#[derive(Clone, Debug)]
pub struct SmallSet<T>(Repr<T>);

#[derive(Clone, Debug)]
enum Repr<T> {
    Empty,
    One(T),
    /// Two distinct elements.
    Two(T, T),
    /// At least three elements, shared by the clones of the set.
    Many(Arc<Many<T>>),
}

/// A set of at least three elements.
#[derive(Debug)]
struct Many<T> {
    /// The identity of this set, unique for the whole process (unlike its address, which a later
    /// set can reuse).
    id: u64,
    /// The ids of sets of at least three elements known to be subsets of this one (the operands of
    /// the union that made it, and theirs), 0 for none.
    subsets: [u64; SUBSETS],
    /// The elements with their hashes, sorted by hash (elements of equal hashes in any order).
    elements: Box<[Entry<T>]>,
}

/// An element of a [`Many`] set and its hash.
type Entry<T> = (u64, T);

fn hash_of<T: Hash>(element: &T) -> u64 {
    FxBuildHasher.hash_one(element)
}

/// How many subsets a [`Many`] remembers.
const SUBSETS: usize = 6;

/// The id of the next [`Many`] set; 0 is no set.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl<T: Copy + Eq> Many<T> {
    /// Whether `other` is known to be a subset of this set without looking at their elements.
    fn has_known_subset(&self, other: &Many<T>) -> bool {
        self.id == other.id || self.subsets.contains(&other.id)
    }

    /// Whether this set contains `element`, of hash `hash`.
    fn contains(&self, hash: u64, element: &T) -> bool {
        let start = self.elements.partition_point(|&(h, _)| h < hash);
        self.elements[start..]
            .iter()
            .take_while(|&&(h, _)| h == hash)
            .any(|(_, e)| e == element)
    }

    /// Whether every element of `other` is in this set.
    fn is_superset(&self, other: &Many<T>) -> bool {
        let (a, b) = (&self.elements, &other.elements);
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

impl<T: Copy + Eq + Hash> SmallSet<T> {
    /// The empty set.
    pub fn new() -> SmallSet<T> {
        SmallSet(Repr::Empty)
    }

    /// The set with exactly one element.
    pub fn singleton(element: T) -> SmallSet<T> {
        SmallSet(Repr::One(element))
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
            Repr::Two(a, b) => a == element || b == element,
            Repr::Many(set) => set.contains(hash_of(element), element),
        }
    }

    /// Whether every element of `other` is in this set.
    pub fn is_superset(&self, other: &SmallSet<T>) -> bool {
        match (&self.0, &other.0) {
            (_, Repr::Empty) => true,
            (_, Repr::One(b)) => self.contains(b),
            (Repr::Empty | Repr::One(_), _) => false,
            (_, Repr::Two(c, d)) => self.contains(c) && self.contains(d),
            (Repr::Two(..), Repr::Many(_)) => false,
            (Repr::Many(a), Repr::Many(b)) => a.is_superset(b),
        }
    }

    /// An iterator over the elements.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        let (inline, many) = match &self.0 {
            Repr::Empty => ([None, None], None),
            Repr::One(a) => ([Some(*a), None], None),
            Repr::Two(a, b) => ([Some(*a), Some(*b)], None),
            Repr::Many(set) => ([None, None], Some(set.elements.iter().map(|&(_, e)| e))),
        };
        inline
            .into_iter()
            .flatten()
            .chain(many.into_iter().flatten())
    }

    /// The union of this set and `other` (ASM's `union`).
    pub fn union(&self, other: &SmallSet<T>) -> SmallSet<T> {
        match (&self.0, &other.0) {
            (_, Repr::Empty) => self.clone(),
            (Repr::Empty, _) => other.clone(),
            (Repr::One(a), Repr::One(b)) if a == b => self.clone(),
            (Repr::One(a), Repr::One(b)) => SmallSet(Repr::Two(*a, *b)),
            _ if self.is_superset(other) => self.clone(),
            _ if other.is_superset(self) => other.clone(),
            _ => {
                let elements = merge(&self.entries(), &other.entries());
                // The operands first, then the subsets they remember, nearest first.
                let mut subsets = [0; SUBSETS];
                let operands = [&self.0, &other.0].map(|operand| match operand {
                    Repr::Many(set) => Some(set.as_ref()),
                    _ => None,
                });
                let ids = operands.iter().flatten().map(|set| set.id).chain(
                    operands
                        .iter()
                        .flatten()
                        .flat_map(|set| set.subsets.iter().copied().filter(|&id| id != 0)),
                );
                for (slot, id) in subsets.iter_mut().zip(ids) {
                    *slot = id;
                }
                SmallSet(Repr::Many(Arc::new(Many {
                    id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                    subsets,
                    elements,
                })))
            }
        }
    }

    /// The elements with their hashes, sorted by hash.
    fn entries(&self) -> Cow<'_, [Entry<T>]> {
        if let Repr::Many(set) = &self.0 {
            return Cow::Borrowed(&set.elements);
        }
        let mut entries: Vec<Entry<T>> = self.iter().map(|e| (hash_of(&e), e)).collect();
        entries.sort_unstable_by_key(|&(h, _)| h);
        Cow::Owned(entries)
    }
}

/// The union of two sets of entries sorted by hash, sorted by hash.
fn merge<T: Copy + Eq>(a: &[Entry<T>], b: &[Entry<T>]) -> Box<[Entry<T>]> {
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
    out.into_boxed_slice()
}

impl<T: Copy + Eq + Hash> Default for SmallSet<T> {
    fn default() -> SmallSet<T> {
        SmallSet::new()
    }
}

impl<T: Copy + Eq + Hash> PartialEq for SmallSet<T> {
    fn eq(&self, other: &SmallSet<T>) -> bool {
        self.len() == other.len() && self.is_superset(other)
    }
}

impl<T: Copy + Eq + Hash> Eq for SmallSet<T> {}

impl<T: Copy + Eq + Hash> Hash for SmallSet<T> {
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

impl<T: Copy + Eq + Hash> FromIterator<T> for SmallSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> SmallSet<T> {
        iter.into_iter().fold(SmallSet::new(), |set, element| {
            set.union(&SmallSet::singleton(element))
        })
    }
}
