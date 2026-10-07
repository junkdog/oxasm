//! Port of ASM's `SimpleVerifierTest`: unit tests for `SimpleVerifier`.
//!
//! ASM's `SimpleVerifier` loads the classes it needs; this one asks a `ClassHierarchy` (ADR-07).
//!
//! - `testConstructor` keeps its non-throwing half: there is no subclassing, so the
//!   `IllegalStateException` of `new SimpleVerifier() {}` cannot be written (ADR-09).
//! - `testMerge_objectTypes` uses a `MapHierarchy` of the JDK classes it needs, with the parity
//!   oracle's facts. The model has no implemented interfaces, which this test does not need (no
//!   class of it implements another of its types).
//! - The `testIsAssignableFrom_*` tests override `getClass` and `getSuperClass`, to make sure
//!   that the result does not depend on class loading: here, the verifier asks an
//!   `ObjectHierarchy`, which knows every class as a direct subclass of `java/lang/Object`
//!   (`getSuperClass` returning `java/lang/Object` for any type, and dummy classes that are not
//!   assignable to each other).

use oxasm::{MapHierarchy, ObjectHierarchy, Type};
use oxasm_analysis::{BasicValue, SimpleVerifier};

/// The JDK classes of `testMerge_objectTypes`, as the parity oracle's `hierarchy.tsv` gives
/// them: `name \t super class \t is interface`.
const JDK_HIERARCHY: &str = "\
java/lang/Object\t\t0
java/lang/Float\tjava/lang/Number\t0
java/lang/Integer\tjava/lang/Number\t0
java/lang/Long\tjava/lang/Number\t0
java/lang/Number\tjava/lang/Object\t0
java/lang/String\tjava/lang/Object\t0
java/util/List\tjava/lang/Object\t1
java/util/Map\tjava/lang/Object\t1
";

#[test]
fn test_constructor() {
    let _ = SimpleVerifier::new(ObjectHierarchy);
    // ASM: assertThrows(IllegalStateException.class, () -> new SimpleVerifier() {}) - dropped,
    // there is no subclassing (ADR-09).
}

#[test]
fn test_merge_object_types() {
    for [internal_name1, internal_name2, expected_internal_name] in [
        ["java/lang/String", "java/lang/Number", "java/lang/Object"],
        ["java/lang/Integer", "java/lang/Number", "java/lang/Number"],
        ["java/lang/Float", "java/lang/Integer", "java/lang/Number"],
        ["java/lang/Long", "java/util/List", "java/lang/Object"],
        ["java/util/Map", "java/util/List", "java/lang/Object"],
    ] {
        let value1 = BasicValue::new(Some(Type::get_object_type(internal_name1)));
        let value2 = BasicValue::new(Some(Type::get_object_type(internal_name2)));
        let verifier =
            SimpleVerifier::new(MapHierarchy::from_tsv(JDK_HIERARCHY.as_bytes()).unwrap());

        // A `None` merge is value1.
        let merge1 = verifier
            .merge(&value1, &value2)
            .unwrap()
            .unwrap_or_else(|| value1.clone());
        let merge2 = verifier
            .merge(&value2, &value1)
            .unwrap()
            .unwrap_or_else(|| value2.clone());

        let expected_value = BasicValue::new(Some(Type::get_object_type(expected_internal_name)));
        assert_eq!(expected_value, merge1, "{internal_name1}, {internal_name2}");
        assert_eq!(expected_value, merge2, "{internal_name2}, {internal_name1}");
    }
}

#[test]
fn test_is_assignable_from_subclass_with_interfaces() {
    let base_type = Type::get_object_type("C");
    let super_type = Type::get_object_type("D");
    let interface_type = Type::get_object_type("I");
    // ASM returns dummy classes (int.class, float.class and double.class) for these types, to
    // make sure isAssignable does not rely on them: ObjectHierarchy knows them as unrelated
    // classes.
    let simple_verifier = SimpleVerifier::for_class(
        base_type.clone(),
        Some(super_type.clone()),
        vec![interface_type.clone()],
        false,
        ObjectHierarchy,
    );

    assert!(
        simple_verifier
            .is_assignable_from(&base_type, &base_type)
            .unwrap()
    );
    assert!(
        simple_verifier
            .is_assignable_from(&super_type, &base_type)
            .unwrap()
    );
    assert!(
        simple_verifier
            .is_assignable_from(&interface_type, &base_type)
            .unwrap()
    );
}

#[test]
fn test_is_assignable_from_interface() {
    let base_type = Type::get_object_type("C");
    let interface_type = Type::get_object_type("I");
    // ASM overrides getSuperClass to return java/lang/Object for any type: so does
    // ObjectHierarchy.
    let simple_verifier = SimpleVerifier::for_class(
        interface_type.clone(),
        None,
        Vec::new(),
        true,
        ObjectHierarchy,
    );

    assert!(
        simple_verifier
            .is_assignable_from(&interface_type, &base_type)
            .unwrap()
    );
    assert!(
        simple_verifier
            .is_assignable_from(&interface_type, &Type::get_object_type("[I"))
            .unwrap()
    );
    assert!(
        !simple_verifier
            .is_assignable_from(&interface_type, &Type::INT)
            .unwrap()
    );
}
