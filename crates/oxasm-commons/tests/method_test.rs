//! Port of ASM's `MethodTest`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use oxasm::Type;
use oxasm_commons::Method;
use oxasm_testkit::{panic_message, panics};

#[test]
fn test_constructor_from_descriptor() {
    let method = Method::new("name", "(I)J");

    assert_eq!(method.get_name(), "name");
    assert_eq!(method.get_descriptor(), "(I)J");
    assert_eq!(method.get_return_type(), Type::LONG);
    assert_eq!(method.get_argument_types(), [Type::INT]);
    assert_eq!(method.to_string(), "name(I)J");
}

#[test]
fn test_constructor_from_types() {
    let method = Method::from_types("name", &Type::LONG, &[Type::INT]);

    assert_eq!(method.get_name(), "name");
    assert_eq!(method.get_descriptor(), "(I)J");
    assert_eq!(method.get_return_type(), Type::LONG);
    assert_eq!(method.get_argument_types(), [Type::INT]);
    assert_eq!(method.to_string(), "name(I)J");
}

// testGetMethod_fromMethodObject and testGetMethod_fromConstructorObject: dropped, Java reflection
// (a non-goal, see the port plan).

#[test]
fn test_get_method_from_descriptor() {
    let method = Method::get_method(
        "boolean name(byte, char, short, int, float, long, double, pkg.Class, pkg.Class[])",
    );

    assert_eq!(method.get_name(), "name");
    assert_eq!(method.get_descriptor(), "(BCSIFJDLpkg/Class;[Lpkg/Class;)Z");
}

#[test]
fn test_get_method_from_invalid_descriptor() {
    assert_eq!(
        panic_message(|| Method::get_method("name()")),
        "java.lang.IllegalArgumentException"
    );
    assert_eq!(
        panic_message(|| Method::get_method("void name")),
        "java.lang.IllegalArgumentException"
    );
    assert_eq!(
        panic_message(|| Method::get_method("void name(]")),
        "java.lang.IllegalArgumentException"
    );
}

#[test]
fn test_get_method_with_default_package() {
    let without_default_package = Method::get_method_with_default_package(
        "void name(Object)",
        /* default_package= */ false,
    );
    let with_default_package = Method::get_method_with_default_package(
        "void name(Object)",
        /* default_package= */ true,
    );

    assert_eq!(
        without_default_package.get_descriptor(),
        "(Ljava/lang/Object;)V"
    );
    assert_eq!(with_default_package.get_descriptor(), "(LObject;)V");
}

#[test]
fn test_equals() {
    // Java's `equals(null)` has no Rust counterpart (an `Option<Method>` is never equal to a
    // `Method`).
    let equals_method_with_different_name =
        Method::new("name", "()V") == Method::new("other", "()V");
    let equals_method_with_different_descriptor =
        Method::new("name", "()V") == Method::new("name", "(I)J");
    let equals_same = Method::new("name", "()V") == Method::get_method("void name()");

    assert!(!equals_method_with_different_name);
    assert!(!equals_method_with_different_descriptor);
    assert!(equals_same);
}

#[test]
fn test_hash_code() {
    let mut hasher = DefaultHasher::new();
    Method::new("name", "()V").hash(&mut hasher);
    assert_ne!(hasher.finish(), 0);
}

/// oxasm-specific: ASM's parsing quirks are kept (Java's `trim`, `substring` exceptions, and the
/// `[]` count cutting the element type).
#[test]
fn test_get_method_parsing_details() {
    assert_eq!(
        Method::get_method("void  m ( int ,\tlong[][] )").get_descriptor(),
        "(I[[J)V"
    );
    assert_eq!(Method::get_method("void m()").get_descriptor(), "()V");
    assert_eq!(Method::get_method("void m(,)").get_descriptor(), "()V");
    assert_eq!(
        Method::get_method("a[]b m()").get_descriptor(),
        "()[Ljava/lang/a[;"
    );
    assert!(panics(|| Method::get_method("void m(a), b)")));
    assert_eq!(
        panic_message(|| Method::get_method("void m(a), b)")),
        "java.lang.StringIndexOutOfBoundsException: Range [10, 8) out of bounds for length 13"
    );
}
