//! Port of ASM's `BasicValueTest`: unit tests for `BasicValue`.
//!
//! - `testEquals`: `equals(new Object())` cannot be written (values of other types do not
//!   compare); that assertion is dropped.
//! - `testHashCode`: Rust hashes are not numbers to compare with 0. ASM's `hashCode()` is that of
//!   the type (0 for uninitialized values): the port checks that equal values hash the same, and
//!   that the uninitialized and int values hash differently.

use std::hash::{BuildHasher, RandomState};

use oxasm::Type;
use oxasm_analysis::{BasicValue, Value};

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

// -------------------------------------------------------------------------------------------------
// oxasm's own tests: the values of `BasicValue::new` are interned for the whole process.
// -------------------------------------------------------------------------------------------------

#[test]
fn new_values_of_equal_types_are_equal_however_the_type_was_made() {
    let descriptor = oxasm::JavaString::from("(Ljava/lang/String;[J)LFoo;");
    let from_method = Type::get_return_type_of(&descriptor);
    let value = BasicValue::new(Some(from_method));
    drop(descriptor);
    let state = RandomState::new();

    for other in [
        BasicValue::new(Some(Type::get_object_type("Foo"))),
        BasicValue::new(Some(Type::get_type("LFoo;"))),
    ] {
        assert_eq!(value, other);
        assert_eq!(state.hash_one(value), state.hash_one(other));
        assert_eq!(value.get_type(), other.get_type());
        assert_eq!("LFoo;", other.to_string());
    }
    // The value keeps its type when the descriptor it was made from is gone.
    assert_eq!("LFoo;", value.to_string());
    assert_eq!(1, value.get_size());
}

#[test]
fn new_values_of_the_types_of_the_constants_equal_them_but_print_as_types() {
    let object = Type::get_object_type("java/lang/Object");
    let cases = [
        (BasicValue::INT_VALUE, Type::INT, "I", 1),
        (BasicValue::FLOAT_VALUE, Type::FLOAT, "F", 1),
        (BasicValue::LONG_VALUE, Type::LONG, "J", 2),
        (BasicValue::DOUBLE_VALUE, Type::DOUBLE, "D", 2),
        (BasicValue::REFERENCE_VALUE, object, "Ljava/lang/Object;", 1),
        (BasicValue::RETURNADDRESS_VALUE, Type::VOID, "V", 1),
    ];
    for (constant, ty, descriptor, size) in cases {
        let value = BasicValue::new(Some(ty.clone()));
        assert_eq!(constant, value, "{descriptor}");
        assert_eq!(Some(&ty), value.get_type());
        assert_eq!(descriptor, value.to_string());
        assert_eq!(size, value.get_size());
        assert_eq!(size, constant.get_size());
        assert_eq!(
            format!("BasicValue(Of(Type({descriptor:?})))"),
            format!("{value:?}")
        );
    }
    assert_eq!("BasicValue(Int)", format!("{:?}", BasicValue::INT_VALUE));
    assert_eq!(
        "BasicValue(Uninitialized)",
        format!("{:?}", BasicValue::new(None))
    );
    // Values of other types are different, the boolean and int types included.
    assert_ne!(BasicValue::INT_VALUE, BasicValue::new(Some(Type::BOOLEAN)));
    assert_ne!(BasicValue::INT_VALUE, BasicValue::UNINITIALIZED_VALUE);
    assert_ne!(
        BasicValue::REFERENCE_VALUE,
        BasicValue::new(Some(Type::get_object_type("java/lang/String")))
    );
    // An array of longs is a reference.
    assert_eq!(1, BasicValue::new(Some(Type::get_type("[J"))).get_size());
}

#[test]
fn new_values_made_by_several_threads_are_equal() {
    let names: Vec<String> = (0..200).map(|i| format!("p/C{i}")).collect();
    let values: Vec<Vec<BasicValue>> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let names = &names;
                scope.spawn(move || {
                    // Each thread starts at another type, so that they insert concurrently.
                    let mut values: Vec<(usize, BasicValue)> = (0..names.len())
                        .map(|i| (i + t * 25) % names.len())
                        .map(|i| (i, BasicValue::new(Some(Type::get_object_type(&*names[i])))))
                        .collect();
                    values.sort_by_key(|&(i, _)| i);
                    values.into_iter().map(|(_, value)| value).collect()
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    for (i, name) in names.iter().enumerate() {
        let expected = BasicValue::new(Some(Type::get_object_type(&**name)));
        for thread_values in &values {
            assert_eq!(expected, thread_values[i]);
        }
        assert_ne!(expected, values[0][(i + 1) % names.len()]);
    }
}

#[test]
fn new_values_of_equal_types_with_other_descriptors_print_their_own() {
    // A malformed method descriptor: the return type's internal name is followed by a `3`, which
    // ASM's `getDescriptor()` keeps, where the type equals that of `Ljava/util/List;`.
    let malformed = Type::get_return_type_of("()Ljava/util/List3");
    let list = Type::get_object_type("java/util/List");
    assert_eq!(list, malformed);
    for _ in 0..2 {
        let (a, b) = (
            BasicValue::new(Some(list.clone())),
            BasicValue::new(Some(malformed.clone())),
        );
        assert_eq!(a, b);
        assert_eq!("Ljava/util/List;", a.to_string());
        assert_eq!("Ljava/util/List3", b.to_string());
    }
    let object = Type::get_return_type_of("()Ljava/lang/Object3");
    let value = BasicValue::new(Some(object));
    assert_eq!(BasicValue::REFERENCE_VALUE, value);
    assert_eq!(
        BasicValue::new(Some(Type::get_type("Ljava/lang/Object;"))),
        value
    );
    assert_eq!("Ljava/lang/Object3", value.to_string());
    assert_eq!(
        "Ljava/lang/Object;",
        BasicValue::new(Some(Type::get_object_type("java/lang/Object"))).to_string()
    );
}
