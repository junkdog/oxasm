//! Port of ASM's `SourceValueTest`: unit tests for `SourceValue`.
//!
//! - `testEquals`: `equals(null)` is the comparison of an `Option` with `None`.
//! - `testHashcode`: Java's numeric `hashCode` is not exposed, so the test compares the hashes of
//!   values: like ASM's, a hash depends on the instructions only.

use std::hash::{Hash, Hasher};

use oxasm::opcodes::NOP;
use oxasm_analysis::{SourceValue, Value};
use oxasm_tree::{Insn, InsnId, InsnList};

/// A `new InsnNode(Opcodes.NOP)`: an instruction of a list, whose handle a `SourceValue` keeps.
fn new_nop() -> InsnId {
    InsnList::new().add(Insn::Simple { opcode: NOP })
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
    assert_ne!(
        hash(&SourceValue::<InsnId>::new(1)),
        hash(&SourceValue::of(1, new_nop()))
    );
}
