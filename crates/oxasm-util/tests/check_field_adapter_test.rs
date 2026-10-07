//! Port of ASM's `CheckFieldAdapterTest`.

use oxasm::{Attribute, FieldVisitor, JavaString, TypeReference, UnknownAttribute, jstr};
use oxasm_testkit::{panic_message, panics};
use oxasm_util::check::CheckFieldAdapter;

/// ASM's test `Comment` attribute (its content does not matter here).
fn comment() -> Attribute {
    Attribute::Unknown(UnknownAttribute {
        name: JavaString::from("Comment"),
        content: Vec::new(),
    })
}

#[test]
fn test_constructor() {
    assert!(!panics(|| CheckFieldAdapter::new(None)));
    // The subclass half (an anonymous subclass must use the api constructor) is dropped: there
    // is no subclassing (ADR-13).
}

#[test]
fn test_visit_type_annotation_illegal_type_annotation() {
    let mut adapter = CheckFieldAdapter::new(None);

    let message = panic_message(|| {
        adapter.visit_type_annotation(
            TypeReference::new_formal_parameter_reference(0),
            None,
            jstr!("LA;"),
            true,
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type reference sort 0x16"
    );
}

// testVisitAttribute_illegalAttribute dropped: a `null` attribute is unrepresentable.

#[test]
fn test_visit_attribute_after_end() {
    let mut adapter = CheckFieldAdapter::new(None);
    adapter.visit_end();

    let message = panic_message(|| adapter.visit_attribute(&comment()));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot call a visit method after visitEnd has been called"
    );
}

// -------------------------------------------------------------------------------------------------
// oxasm-specific
// -------------------------------------------------------------------------------------------------

#[test]
fn annotations_are_checked() {
    let mut adapter = CheckFieldAdapter::new(None);

    let message = panic_message(|| adapter.visit_annotation(jstr!("LA"), true));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid descriptor: LA"
    );

    let message = panic_message(|| {
        adapter.visit_type_annotation(
            TypeReference::new_type_reference(TypeReference::FIELD),
            None,
            jstr!("V"),
            true,
        );
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid descriptor: V"
    );

    // A type reference of the right sort, with stray bits.
    let message = panic_message(|| {
        adapter.visit_type_annotation(
            TypeReference::new(u32::from(TypeReference::FIELD) << 24 | 1),
            None,
            jstr!("LA;"),
            true,
        );
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type reference 0x13000001"
    );

    // The returned annotation visitor is a check adapter.
    let mut annotation = adapter
        .visit_annotation(jstr!("LA;"), true)
        .expect("a check adapter");
    let message =
        panic_message(|| annotation.visit(None, &oxasm::constant::AnnotationValue::Int(0)));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Annotation value name must not be null"
    );
}

#[test]
fn visit_end_twice() {
    let mut adapter = CheckFieldAdapter::new(None);
    adapter.visit_end();

    let message = panic_message(|| adapter.visit_end());

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot call a visit method after visitEnd has been called"
    );
}
