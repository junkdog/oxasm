//! Port of ASM's `SmallSetTest`: unit tests for `SmallSet`, plus proptests.
//!
//! ASM's elements are distinct `Object`s; here they are distinct integers. ASM's `union` returns
//! one of its operands when it can, and its `equals` is `Set` equality: here sets are compared
//! by content, and with `Set.of(...)` through a `HashSet` of their elements.
//!
//! - `testIterator_next_noSuchElement`: the iterator returns `None` where ASM's throws
//!   `NoSuchElementException`.
//! - `testIterator_remove` is dropped: `SmallSet` is immutable, so the
//!   `UnsupportedOperationException` of `iterator().remove()` cannot be written.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hash, Hasher};

use oxasm_analysis::SmallSet;
use proptest::prelude::*;

const ELEMENT1: u32 = 1;
const ELEMENT2: u32 = 2;
const ELEMENT3: u32 = 3;
const ELEMENT4: u32 = 4;

/// ASM's `Set.of(...)` equality: the same elements.
#[track_caller]
fn assert_set_of(set: &SmallSet<u32>, elements: &[u32]) {
    let expected: HashSet<u32> = elements.iter().copied().collect();
    assert_eq!(set.iter().collect::<HashSet<u32>>(), expected);
    assert_eq!(set.len(), expected.len());
}

#[test]
fn test_union_one_element_empty_set() {
    let set1 = SmallSet::singleton(ELEMENT1);
    let set2 = SmallSet::new();

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(set1, union1);
    assert_eq!(set1, union2);
}

#[test]
fn test_union_one_element_one_element() {
    let set1 = SmallSet::singleton(ELEMENT1);
    let set2 = SmallSet::singleton(ELEMENT1);

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(union1, union2);
    assert_set_of(&union1, &[ELEMENT1]);
}

#[test]
fn test_union_one_element_two_elements_super_set() {
    let set1 = new_small_set(ELEMENT1, ELEMENT2);
    let set2 = SmallSet::singleton(ELEMENT1);

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(union1, union2);
    assert_set_of(&union1, &[ELEMENT1, ELEMENT2]);
}

#[test]
fn test_union_two_elements_two_elements_equal_sets() {
    let set1 = new_small_set(ELEMENT1, ELEMENT2);
    let set2 = new_small_set(ELEMENT2, ELEMENT1);

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(union1, union2);
    assert_set_of(&union1, &[ELEMENT1, ELEMENT2]);
}

#[test]
fn test_union_two_elements_one_element_distinct_sets() {
    let set1 = new_small_set(ELEMENT1, ELEMENT2);
    let set2 = SmallSet::singleton(ELEMENT3);

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(union1, union2);
    assert_set_of(&union1, &[ELEMENT1, ELEMENT2, ELEMENT3]);
}

#[test]
fn test_union_two_elements_two_elements_distinc_sets() {
    let set1 = new_small_set(ELEMENT1, ELEMENT2);
    let set2 = new_small_set(ELEMENT3, ELEMENT4);

    let union1 = set1.union(&set2);
    let union2 = set2.union(&set1);

    assert_eq!(union1, union2);
    assert_set_of(&union1, &[ELEMENT1, ELEMENT2, ELEMENT3, ELEMENT4]);
}

#[test]
fn test_iterator_next_first_element() {
    let set = new_small_set(ELEMENT1, ELEMENT2);
    let mut iterator = set.iter().peekable();

    let element = iterator.next();

    assert_eq!(Some(ELEMENT1), element);
    assert!(iterator.peek().is_some());
}

#[test]
fn test_iterator_next_second_element() {
    let set = new_small_set(ELEMENT1, ELEMENT2);
    let mut iterator = set.iter().peekable();
    iterator.next();

    let element = iterator.next();

    assert_eq!(Some(ELEMENT2), element);
    assert!(iterator.peek().is_none());
}

#[test]
fn test_iterator_next_no_such_element() {
    let set = new_small_set(ELEMENT1, ELEMENT2);
    let mut iterator = set.iter();
    iterator.next();
    iterator.next();

    let next = iterator.next();

    // ASM: assertThrows(NoSuchElementException.class, next).
    assert_eq!(None, next);
}

// testIterator_remove: dropped, SmallSet is immutable (ASM's iterator().remove() throws
// UnsupportedOperationException).

fn new_small_set(element1: u32, element2: u32) -> SmallSet<u32> {
    SmallSet::singleton(element1).union(&SmallSet::singleton(element2))
}

// -------------------------------------------------------------------------------------------------
// Properties: sets of three or more elements against a `HashSet`, with elements whose hashes
// collide (sets of three or more are sorted by hash).
// -------------------------------------------------------------------------------------------------

/// An element of only three hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Colliding(u8);

impl Hash for Colliding {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0 % 3).hash(state);
    }
}

/// A set made by unions of the given groups (each itself made by unions of singletons), and the
/// same set as a `HashSet`.
fn build(groups: &[Vec<u8>]) -> (SmallSet<Colliding>, HashSet<u8>) {
    let set = groups.iter().fold(SmallSet::new(), |set, group| {
        set.union(&group.iter().map(|&e| Colliding(e)).collect())
    });
    (set, groups.iter().flatten().copied().collect())
}

fn groups() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(prop::collection::vec(0u8..12, 0..8), 1..4)
}

proptest! {
    #[test]
    fn prop_small_set_is_a_set(a in groups(), b in groups()) {
        let (set_a, model_a) = build(&a);
        let (set_b, model_b) = build(&b);
        for (set, model) in [(&set_a, &model_a), (&set_b, &model_b)] {
            prop_assert_eq!(set.len(), model.len());
            prop_assert_eq!(&set.iter().map(|e| e.0).collect::<HashSet<u8>>(), model);
            for e in 0..12 {
                prop_assert_eq!(set.contains(&Colliding(e)), model.contains(&e));
            }
        }
        prop_assert_eq!(set_a.is_superset(&set_b), model_a.is_superset(&model_b));
        prop_assert_eq!(set_a == set_b, model_a == model_b);
        if set_a == set_b {
            let hash = |set: &SmallSet<Colliding>| rustc_hash::FxBuildHasher.hash_one(set);
            prop_assert_eq!(hash(&set_a), hash(&set_b));
        }
        let union = set_a.union(&set_b);
        let model_union: HashSet<u8> = model_a.union(&model_b).copied().collect();
        prop_assert_eq!(union.len(), model_union.len());
        prop_assert_eq!(union.iter().map(|e| e.0).collect::<HashSet<u8>>(), model_union);
        prop_assert!(union.is_superset(&set_a) && union.is_superset(&set_b));
    }
}
