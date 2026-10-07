//! [`SmallSet`]: ASM's `SmallSet`, the instruction set of a [`SourceValue`](crate::SourceValue).

use std::hash::{BuildHasher, Hash, Hasher};

use rustc_hash::{FxBuildHasher, FxHashSet};

/// An immutable set, optimized for sets of at most two elements (ASM's `SmallSet`, which is
/// limited to two elements, and the `HashSet`s its `union` returns for more).
///
/// Sets are equal if they have the same elements. Elements iterate in insertion order up to two
/// elements, in an unspecified order beyond.
#[derive(Clone, Debug)]
pub struct SmallSet<T>(Repr<T>);

#[derive(Clone, Debug)]
enum Repr<T> {
    Empty,
    One(T),
    /// Two distinct elements.
    Two(T, T),
    /// At least three elements.
    Many(Box<FxHashSet<T>>),
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
            Repr::Many(set) => set.len(),
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
            Repr::Many(set) => set.contains(element),
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
            (Repr::Many(a), Repr::Many(b)) => a.len() >= b.len() && b.is_subset(a),
        }
    }

    /// An iterator over the elements.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        let (inline, many) = match &self.0 {
            Repr::Empty => ([None, None], None),
            Repr::One(a) => ([Some(*a), None], None),
            Repr::Two(a, b) => ([Some(*a), Some(*b)], None),
            Repr::Many(set) => ([None, None], Some(set.iter().copied())),
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
                let mut result = FxHashSet::default();
                result.extend(self.iter());
                result.extend(other.iter());
                SmallSet(Repr::Many(Box::new(result)))
            }
        }
    }
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
        let sum = self
            .iter()
            .map(|element| FxBuildHasher.hash_one(element))
            .fold(0u64, u64::wrapping_add);
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
