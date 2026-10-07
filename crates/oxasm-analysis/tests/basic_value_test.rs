//! Port of ASM's `BasicValueTest`: unit tests for `BasicValue`.
//!
//! - `testEquals`: `equals(new Object())` cannot be written (values of other types do not
//!   compare); that assertion is dropped.
//! - `testHashCode`: Rust hashes are not numbers to compare with 0. ASM's `hashCode()` is that of
//!   the type (0 for uninitialized values): the port checks that equal values hash the same, and
//!   that the uninitialized and int values hash differently.

use std::hash::{BuildHasher, RandomState};

use oxasm::Type;
use oxasm_analysis::BasicValue;

#[test]
fn test_is_reference() {
    assert!(BasicValue::REFERENCE_VALUE.is_reference());
    assert!(BasicValue::new(Some(Type::get_object_type("[I"))).is_reference());
    assert!(!BasicValue::UNINITIALIZED_VALUE.is_reference());
    assert!(!BasicValue::INT_VALUE.is_reference());
}

#[test]
fn test_equals() {
    let equals_same_uninitialized_value = BasicValue::new(None) == BasicValue::UNINITIALIZED_VALUE;
    let equals_same_value = BasicValue::new(Some(Type::INT)) == BasicValue::INT_VALUE;
    let equals_this = BasicValue::INT_VALUE == BasicValue::INT_VALUE;
    // ASM: BasicValue.INT_VALUE.equals(new Object()) is false - dropped, values of other types
    // do not compare.

    assert!(equals_same_uninitialized_value);
    assert!(equals_same_value);
    assert!(equals_this);
}

#[test]
fn test_hash_code() {
    // ASM: assertEquals(0, BasicValue.UNINITIALIZED_VALUE.hashCode()) and
    // assertNotEquals(0, BasicValue.INT_VALUE.hashCode()).
    let state = RandomState::new();
    let hash = |value: &BasicValue| state.hash_one(value);

    assert_eq!(
        hash(&BasicValue::UNINITIALIZED_VALUE),
        hash(&BasicValue::new(None))
    );
    assert_eq!(
        hash(&BasicValue::INT_VALUE),
        hash(&BasicValue::new(Some(Type::INT)))
    );
    assert_ne!(
        hash(&BasicValue::UNINITIALIZED_VALUE),
        hash(&BasicValue::INT_VALUE)
    );
}

#[test]
fn test_to_string() {
    assert_eq!(".", BasicValue::UNINITIALIZED_VALUE.to_string());
    assert_eq!("A", BasicValue::RETURNADDRESS_VALUE.to_string());
    assert_eq!("R", BasicValue::REFERENCE_VALUE.to_string());
    assert_eq!(
        "LI;",
        BasicValue::new(Some(Type::get_object_type("I"))).to_string()
    );
}
