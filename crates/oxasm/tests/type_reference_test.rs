//! Port of ASM's `TypeReferenceTest` (asm/src/test/java/org/objectweb/asm/TypeReferenceTest.java).

use oxasm::TypeReference;

#[test]
fn test_new_type_reference() {
    let type_reference = TypeReference::new_type_reference(TypeReference::FIELD);
    assert_eq!(TypeReference::FIELD, type_reference.get_sort());
    assert_eq!(
        u32::from(TypeReference::FIELD) << 24,
        type_reference.get_value()
    );
}

#[test]
fn test_new_type_parameter_reference() {
    let type_reference =
        TypeReference::new_type_parameter_reference(TypeReference::CLASS_TYPE_PARAMETER, 3);
    assert_eq!(
        TypeReference::CLASS_TYPE_PARAMETER,
        type_reference.get_sort()
    );
    assert_eq!(3, type_reference.get_type_parameter_index());
}

#[test]
fn test_new_type_parameter_bound_reference() {
    let type_reference = TypeReference::new_type_parameter_bound_reference(
        TypeReference::CLASS_TYPE_PARAMETER,
        3,
        7,
    );
    assert_eq!(
        TypeReference::CLASS_TYPE_PARAMETER,
        type_reference.get_sort()
    );
    assert_eq!(3, type_reference.get_type_parameter_index());
    assert_eq!(7, type_reference.get_type_parameter_bound_index());
}

#[test]
fn test_new_super_type_reference() {
    let type_reference = TypeReference::new_super_type_reference(-1);
    assert_eq!(TypeReference::CLASS_EXTENDS, type_reference.get_sort());
    assert_eq!(-1, type_reference.get_super_type_index());
}

#[test]
fn test_new_formal_parameter_reference() {
    let type_reference = TypeReference::new_formal_parameter_reference(3);
    assert_eq!(
        TypeReference::METHOD_FORMAL_PARAMETER,
        type_reference.get_sort()
    );
    assert_eq!(3, type_reference.get_formal_parameter_index());
}

#[test]
fn test_new_exception_reference() {
    let type_reference = TypeReference::new_exception_reference(3);
    assert_eq!(TypeReference::THROWS, type_reference.get_sort());
    assert_eq!(3, type_reference.get_exception_index());
}

#[test]
fn test_new_try_catch_reference() {
    let type_reference = TypeReference::new_try_catch_reference(3);
    assert_eq!(
        TypeReference::EXCEPTION_PARAMETER,
        type_reference.get_sort()
    );
    assert_eq!(3, type_reference.get_try_catch_block_index());
}

#[test]
fn test_new_type_argument_reference() {
    let type_reference = TypeReference::new_type_argument_reference(TypeReference::CAST, 3);
    assert_eq!(TypeReference::CAST, type_reference.get_sort());
    assert_eq!(3, type_reference.get_type_argument_index());
}
