//! Port of ASM's `CheckAnnotationAdapterTest`.

use oxasm::constant::AnnotationValue;
use oxasm::{AnnotationVisitor, Type, jstr};
use oxasm_testkit::panic_message;
use oxasm_util::check::CheckAnnotationAdapter;

#[test]
fn test_visit_illegal_annotation_name() {
    let mut check_annotation_adapter = CheckAnnotationAdapter::new(None);

    let message = panic_message(|| check_annotation_adapter.visit(None, &AnnotationValue::Int(0)));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Annotation value name must not be null"
    );
}

// testVisit_illegalAnnotationValue1 dropped: it passes a `new Object()`, which an
// `AnnotationValue` cannot hold (ADR-05).

#[test]
fn test_visit_illegal_annotation_value2() {
    let mut check_annotation_adapter = CheckAnnotationAdapter::new(None);

    let message = panic_message(|| {
        check_annotation_adapter.visit(
            Some(jstr!("name")),
            &AnnotationValue::Type(Type::get_method_type(jstr!("()V"))),
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid annotation value"
    );
}

#[test]
fn test_visit_after_end() {
    let mut check_annotation_adapter = CheckAnnotationAdapter::new(None);
    check_annotation_adapter.visit_end();

    let message = panic_message(|| {
        check_annotation_adapter.visit(Some(jstr!("name")), &AnnotationValue::Int(0));
    });

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot call a visit method after visitEnd has been called"
    );
}

// testVisitEnum_illegalAnnotationEnumValue dropped: a `null` enum value is unrepresentable (the
// value is a `&JavaStr`).

// -------------------------------------------------------------------------------------------------
// oxasm-specific
// -------------------------------------------------------------------------------------------------

#[test]
fn nested_visitors_are_checked() {
    let mut check_annotation_adapter = CheckAnnotationAdapter::new(None);

    // An array has unnamed values.
    let mut array = check_annotation_adapter
        .visit_array(Some(jstr!("array")))
        .expect("a check adapter");
    array.visit(None, &AnnotationValue::Int(0));
    array.visit_end();
    drop(array);

    // A nested annotation has named values, and a checked descriptor.
    let mut annotation = check_annotation_adapter
        .visit_annotation(Some(jstr!("annotation")), jstr!("LA;"))
        .expect("a check adapter");
    let message = panic_message(|| annotation.visit(None, &AnnotationValue::Int(0)));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Annotation value name must not be null"
    );
    drop(annotation);

    let message = panic_message(|| {
        check_annotation_adapter.visit_annotation(Some(jstr!("annotation")), jstr!("I;"));
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid descriptor: I;"
    );
    let message = panic_message(|| {
        check_annotation_adapter.visit_enum(Some(jstr!("e")), jstr!("LE"), jstr!("V"));
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid descriptor: LE"
    );
}

#[test]
fn events_are_forwarded() {
    #[derive(Default)]
    struct Recorder(Vec<String>);
    impl AnnotationVisitor for Recorder {
        fn visit(&mut self, name: Option<&oxasm::JavaStr>, _value: &AnnotationValue<'_>) {
            self.0.push(format!("visit {name:?}"));
        }
        fn visit_end(&mut self) {
            self.0.push("end".to_owned());
        }
    }

    let mut recorder = Recorder::default();
    let mut check_annotation_adapter = CheckAnnotationAdapter::new(Some(Box::new(&mut recorder)));
    check_annotation_adapter.visit(Some(jstr!("a")), &AnnotationValue::Int(1));
    check_annotation_adapter.visit_end();
    drop(check_annotation_adapter);

    assert_eq!(recorder.0, ["visit Some(\"a\")", "end"]);
}
