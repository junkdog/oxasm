//! Port of ASM's `SourceValueTest`: unit tests for `SourceValue`.
//!
//! - `testEquals`: `equals(null)` is the comparison of an `Option` with `None`.
//! - `testHashcode`: Java's numeric `hashCode` is not exposed, so the test compares the hashes of
//!   values: like ASM's, a hash depends on the instructions only.
//!
//! Not a port: a proptest of `SourceInterpreter::merge_values`, the loop of `Frame.merge` that
//! the analysis uses for `SourceValue`s, and the size of a value (optimization lead A12).

use std::hash::{Hash, Hasher};

use oxasm::opcodes::NOP;
use oxasm_analysis::{SetArena, SmallSet, SourceInterpreter, SourceValue, Value};
use oxasm_tree::{Insn, InsnId, InsnList};
use proptest::prelude::*;

/// A `new InsnNode(Opcodes.NOP)`: an instruction of a list, whose handle a `SourceValue` keeps.
fn new_nop() -> InsnId {
    InsnList::new().add(Insn::Simple { opcode: NOP })
}

/// An analysis writes every value of every frame it stores: a set is one element and a tag, its
/// sets of two or more elements in a `SetArena`.
#[test]
fn source_values_are_three_words() {
    assert_eq!(size_of::<SmallSet<'_, InsnId>>(), 16);
    assert_eq!(size_of::<SourceValue<'_, InsnId>>(), 24);
}

#[test]
fn test_get_size() {
    assert_eq!(2, SourceValue::<InsnId>::new(2).get_size());
}

#[test]
fn test_equals() {
    let null_value: Option<&SourceValue> = None;

    let equals_same = SourceValue::<InsnId>::new(1) == SourceValue::<InsnId>::new(1);
    let equals_value_with_different_source =
        SourceValue::<InsnId>::new(1) == SourceValue::of(1, new_nop());
    let equals_value_with_different_value =
        SourceValue::<InsnId>::new(1) == SourceValue::<InsnId>::new(2);
    let equals_null = Some(&SourceValue::<InsnId>::new(1)) == null_value;

    assert!(equals_same);
    assert!(!equals_value_with_different_source);
    assert!(!equals_value_with_different_value);
    assert!(!equals_null);
}

fn hash(value: &SourceValue) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn test_hashcode() {
    // ASM: assertEquals(0, new SourceValue(1).hashCode()), the hash of an empty set (which does
    // not depend on the size).
    assert_eq!(
        hash(&SourceValue::<InsnId>::new(1)),
        hash(&SourceValue::<InsnId>::new(2))
    );
    // Not the first instruction of a list: in the process's first list, that id is all zeros,
    // which Fx hashes to 0, the empty set's hash.
    let mut list = InsnList::new();
    list.add(Insn::Simple { opcode: NOP });
    let nop = list.add(Insn::Simple { opcode: NOP });
    assert_ne!(
        hash(&SourceValue::<InsnId>::new(1)),
        hash(&SourceValue::of(1, nop))
    );
}

// -------------------------------------------------------------------------------------------------
// Properties: `SourceInterpreter::merge_values` against ASM's `Frame.merge` loop.
// -------------------------------------------------------------------------------------------------

/// A producer of a few values, whose sets of two or more are in an arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Producer(u8);

impl From<InsnId> for Producer {
    fn from(_: InsnId) -> Producer {
        Producer(u8::MAX)
    }
}

/// A value of the given size and producers.
fn value<'s>(size: usize, producers: &[u8], sets: &'s SetArena) -> SourceValue<'s, Producer> {
    SourceValue::with_insns(
        size,
        SmallSet::from_elements(producers.iter().map(|&p| Producer(p)), sets),
    )
}

/// The size and producers of the value at an index of the merged frame, and how the value of the
/// other frame is made (0: a copy of it, 1: from the same producers, an equal value but not a
/// copy beyond two producers, 2: from the size and producers that follow).
type MergedValue = (usize, Vec<u8>, u8, usize, Vec<u8>);

fn merged_values() -> impl Strategy<Value = Vec<MergedValue>> {
    let producers = || prop::collection::vec(0u8..6, 0..5);
    prop::collection::vec(
        (1usize..=2, producers(), 0u8..3, 1usize..=2, producers()),
        0..12,
    )
}

proptest! {
    #[test]
    fn prop_merge_values_is_the_frame_merge_loop(spec in merged_values()) {
        let sets = SetArena::new();
        let mut values = Vec::new();
        let mut other = Vec::new();
        for (size, producers, kind, other_size, other_producers) in &spec {
            let value = value(*size, producers, &sets);
            values.push(value);
            other.push(match kind {
                0 => value,
                1 => self::value(*size, producers, &sets),
                _ => self::value(*other_size, other_producers, &sets),
            });
        }

        let mut expected = values.clone();
        let mut expected_changed = false;
        for (value, other) in expected.iter_mut().zip(&other) {
            if let Some(merged) = SourceInterpreter::merge(&sets, value, other) {
                // What lets `merge_values` skip ASM's `equals`: a merged value is a new one.
                prop_assert_ne!(merged, *value);
                *value = merged;
                expected_changed = true;
            }
        }
        let changed = SourceInterpreter::merge_values(&sets, &mut values, &other);
        prop_assert_eq!(changed, expected_changed);
        prop_assert_eq!(values, expected);
    }
}
