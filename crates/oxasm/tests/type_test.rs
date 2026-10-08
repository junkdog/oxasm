//! Port of ASM's `TypeTest` (asm/src/test/java/org/objectweb/asm/TypeTest.java).
//!
//! Dropped (reflection entry points, not ported; see `ty.rs`): `testGetTypeFromClass`,
//! `testGetTypeFromConstructor`, `testGetTypeFromMethod`, `testGetArgumentTypesFromMethod`,
//! `testGetReturnTypeFromMethod`, `testGetInternalNameFromClass`, `testGetDescriptorFromClass`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use oxasm::opcodes::*;
use oxasm::{Type, TypeSort};

const METHOD_DESCRIPTORS: &[&str] = &[
    "(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V",
    "()I",
    "()LI;",
    "()Ljava/lang/Object;",
    "()[I",
    "()[LI;",
    "()[[Ljava/lang/Object;",
];

const PRIMITIVES: [(Type, TypeSort, usize, &str, &str); 9] = [
    (Type::VOID, TypeSort::Void, 0, "V", "void"),
    (Type::BOOLEAN, TypeSort::Boolean, 1, "Z", "boolean"),
    (Type::CHAR, TypeSort::Char, 1, "C", "char"),
    (Type::BYTE, TypeSort::Byte, 1, "B", "byte"),
    (Type::SHORT, TypeSort::Short, 1, "S", "short"),
    (Type::INT, TypeSort::Int, 1, "I", "int"),
    (Type::FLOAT, TypeSort::Float, 1, "F", "float"),
    (Type::LONG, TypeSort::Long, 2, "J", "long"),
    (Type::DOUBLE, TypeSort::Double, 2, "D", "double"),
];

fn hash(t: &Type) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn test_get_sort_type_constants() {
    for (t, sort, ..) in PRIMITIVES {
        assert_eq!(sort, t.get_sort());
    }
}

#[test]
fn test_get_size_type_constants() {
    for (t, _, size, ..) in PRIMITIVES {
        assert_eq!(size, t.get_size());
    }
}

#[test]
fn test_get_descriptor_type_constants() {
    for (t, _, _, descriptor, _) in PRIMITIVES {
        assert_eq!(descriptor, &*t.get_descriptor());
    }
}

#[test]
fn test_get_type_from_descriptor() {
    let descriptors = [
        "I",
        "V",
        "Z",
        "B",
        "C",
        "S",
        "D",
        "F",
        "J",
        "LI;",
        "LV;",
        "Ljava/lang/Object;",
        "[I",
        "[LI;",
        "[[Ljava/lang/Object;",
    ];
    for descriptor in descriptors.iter().chain(METHOD_DESCRIPTORS) {
        let t = Type::get_type(descriptor);
        assert_eq!(*descriptor, &*t.get_descriptor());
        assert_eq!(*descriptor, t.to_string());
    }
}

#[test]
fn test_get_type_from_descriptor_invalid() {
    let error = Type::try_get_type("-").unwrap_err();
    assert_eq!("Invalid descriptor: -", error.to_string());
}

#[test]
#[should_panic(expected = "Invalid descriptor: -")]
fn test_get_type_from_descriptor_invalid_panics() {
    let _ = Type::get_type("-");
}

#[test]
fn test_get_object_type() {
    for internal_name in [
        "I",
        "V",
        "java/lang/Object",
        "[I",
        "[LI;",
        "[[Ljava/lang/Object;",
    ] {
        let t = Type::get_object_type(internal_name);
        assert_eq!(internal_name, t.get_internal_name());
        let expected = if internal_name.starts_with('[') {
            TypeSort::Array
        } else {
            TypeSort::Object
        };
        assert_eq!(expected, t.get_sort());
    }
}

#[test]
fn test_get_method_type_from_descriptor() {
    for method_descriptor in METHOD_DESCRIPTORS {
        let t = Type::get_method_type(method_descriptor);
        assert_eq!(TypeSort::Method, t.get_sort());
        assert_eq!(*method_descriptor, &*t.get_descriptor());
        assert_eq!(*method_descriptor, t.to_string());
    }
}

#[test]
fn test_get_argument_types_get_return_type_and_get_method_type() {
    for method_descriptor in METHOD_DESCRIPTORS {
        let argument_types = Type::get_argument_types_of(method_descriptor);
        let return_type = Type::get_return_type_of(method_descriptor);
        let method_type = Type::get_method_type_of(&return_type, &argument_types);
        assert_eq!(TypeSort::Method, method_type.get_sort());
        assert_eq!(*method_descriptor, &*method_type.get_descriptor());
        assert_eq!(argument_types, method_type.get_argument_types());
        assert_eq!(return_type, method_type.get_return_type());
    }
}

#[test]
fn test_get_argument_types_invalid_method_descriptor() {
    // ASM throws a RuntimeException (and must not loop forever).
    assert!(Type::try_get_argument_types_of("(Ljava/lang/String").is_err());
    let panicked = std::panic::catch_unwind(|| Type::get_argument_types_of("(Ljava/lang/String"));
    assert!(panicked.is_err());
}

#[test]
fn test_get_argument_types_invalid_argument_descriptor() {
    let error = Type::try_get_argument_types_of("(Ljava/lang/String;-)V").unwrap_err();
    assert_eq!(
        "Invalid descriptor: (Ljava/lang/String;-)V",
        error.to_string()
    );
}

#[test]
fn test_get_return_type_from_descriptor() {
    assert_eq!(Type::INT, Type::get_return_type_of("()I"));
    assert_eq!(
        Type::INT,
        Type::get_return_type_of("(Lpkg/classMethod();)I")
    );
}

#[test]
fn test_get_argument_count_from_type() {
    assert_eq!(
        14,
        Type::get_method_type("(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V")
            .get_argument_count()
    );
}

#[test]
fn test_get_argument_count_from_descriptor() {
    assert_eq!(
        14,
        Type::get_argument_count_of(
            "(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V"
        )
    );
    assert_eq!(0, Type::get_argument_count_of("()I"));
}

#[test]
fn test_get_arguments_and_return_size_from_type() {
    let sizes = |d: &str| Type::get_method_type(d).get_arguments_and_return_sizes();
    assert_eq!(
        17 << 2,
        sizes("(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V")
    );
    assert_eq!((1 << 2) | 1, sizes("()I"));
    assert_eq!((1 << 2) | 1, sizes("()F"));
    assert_eq!((1 << 2) | 2, sizes("()J"));
    assert_eq!((1 << 2) | 2, sizes("()D"));
    assert_eq!((1 << 2) | 1, sizes("()LD;"));
}

#[test]
fn test_get_arguments_and_return_size_from_descriptor() {
    let sizes = |d: &str| Type::get_arguments_and_return_sizes_of(d);
    assert_eq!(
        17 << 2,
        sizes("(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V")
    );
    assert_eq!((1 << 2) | 1, sizes("()I"));
    assert_eq!((1 << 2) | 1, sizes("()F"));
    assert_eq!((1 << 2) | 2, sizes("()J"));
    assert_eq!((1 << 2) | 2, sizes("()D"));
    assert_eq!((1 << 2) | 1, sizes("()LD;"));
}

#[test]
fn test_get_sort() {
    assert_eq!(TypeSort::Array, Type::get_type("[LI;").get_sort());
    assert_eq!(TypeSort::Array, Type::get_object_type("[LI;").get_sort());
    assert_eq!(TypeSort::Object, Type::get_type("LI;").get_sort());
    assert_eq!(TypeSort::Object, Type::get_object_type("I").get_sort());
}

#[test]
fn test_get_dimensions() {
    assert_eq!(1, Type::get_type("[I").get_dimensions());
    assert_eq!(3, Type::get_type("[[[LI;").get_dimensions());
}

#[test]
fn test_get_element_type() {
    assert_eq!(Type::INT, Type::get_type("[I").get_element_type());
    assert_eq!(
        Type::get_object_type("I"),
        Type::get_type("[[[LI;").get_element_type()
    );
}

#[test]
fn test_get_class_name() {
    for (t, _, _, _, class_name) in PRIMITIVES {
        assert_eq!(class_name, t.get_class_name());
    }
    assert_eq!("I[]", Type::get_object_type("[LI;").get_class_name());
    assert_eq!(
        "java.lang.Object",
        Type::get_object_type("java/lang/Object").get_class_name()
    );
    assert_eq!(
        "java.lang.Object",
        Type::get_type("Ljava/lang/Object;").get_class_name()
    );
}

#[test]
fn test_get_internal_name() {
    assert_eq!("[LI;", Type::get_object_type("[LI;").get_internal_name());
    assert_eq!(
        "java/lang/Object",
        Type::get_object_type("java/lang/Object").get_internal_name()
    );
    assert_eq!(
        "java/lang/Object",
        Type::get_type("Ljava/lang/Object;").get_internal_name()
    );
}

#[test]
fn test_get_arguments_and_return_size() {
    let t = Type::get_type("(IZBCSDFJLI;LV;Ljava/lang/Object;[I[LI;[[Ljava/lang/Object;)V");
    assert_eq!(17 << 2, t.get_arguments_and_return_sizes());
}

#[test]
fn test_get_descriptor() {
    assert_eq!("[LI;", &*Type::get_object_type("[LI;").get_descriptor());
    assert_eq!(
        "Ljava/lang/Object;",
        &*Type::get_object_type("java/lang/Object").get_descriptor()
    );
    assert_eq!(
        "Ljava/lang/Object;",
        &*Type::get_type("Ljava/lang/Object;").get_descriptor()
    );
}

#[test]
fn test_get_method_descriptor() {
    assert_eq!(
        "(IJ)V",
        Type::get_method_descriptor(&Type::VOID, &[Type::INT, Type::LONG])
    );
    assert_eq!(
        "(Ljava/lang/Object;)V",
        Type::get_method_descriptor(&Type::VOID, &[Type::get_type("Ljava/lang/Object;")])
    );
    assert_eq!(
        "(Ljava/lang/Object;)V",
        Type::get_method_descriptor(&Type::VOID, &[Type::get_object_type("java/lang/Object")])
    );
}

#[test]
fn test_get_opcode_iaload_or_store() {
    assert_eq!(BALOAD, Type::BOOLEAN.get_opcode(IALOAD));
    assert_eq!(BALOAD, Type::BYTE.get_opcode(IALOAD));
    assert_eq!(CALOAD, Type::CHAR.get_opcode(IALOAD));
    assert_eq!(SALOAD, Type::SHORT.get_opcode(IALOAD));
    assert_eq!(IALOAD, Type::INT.get_opcode(IALOAD));
    assert_eq!(FALOAD, Type::FLOAT.get_opcode(IALOAD));
    assert_eq!(LALOAD, Type::LONG.get_opcode(IALOAD));
    assert_eq!(DALOAD, Type::DOUBLE.get_opcode(IALOAD));
    assert_eq!(
        AALOAD,
        Type::get_type("Ljava/lang/Object;").get_opcode(IALOAD)
    );
    assert_eq!(
        AALOAD,
        Type::get_object_type("java/lang/Object").get_opcode(IALOAD)
    );
    assert_eq!(
        AASTORE,
        Type::get_type("Ljava/lang/Object;").get_opcode(IASTORE)
    );
    assert_eq!(
        AASTORE,
        Type::get_object_type("java/lang/Object").get_opcode(IASTORE)
    );
    assert_eq!(AASTORE, Type::get_type("[I").get_opcode(IASTORE));
}

#[test]
fn test_get_opcode_return() {
    assert_eq!(RETURN, Type::VOID.get_opcode(IRETURN));
    assert_eq!(IRETURN, Type::BOOLEAN.get_opcode(IRETURN));
    assert_eq!(IRETURN, Type::BYTE.get_opcode(IRETURN));
    assert_eq!(IRETURN, Type::CHAR.get_opcode(IRETURN));
    assert_eq!(IRETURN, Type::SHORT.get_opcode(IRETURN));
    assert_eq!(IRETURN, Type::INT.get_opcode(IRETURN));
    assert_eq!(FRETURN, Type::FLOAT.get_opcode(IRETURN));
    assert_eq!(LRETURN, Type::LONG.get_opcode(IRETURN));
    assert_eq!(DRETURN, Type::DOUBLE.get_opcode(IRETURN));
    assert_eq!(
        ARETURN,
        Type::get_type("Ljava/lang/Object;").get_opcode(IRETURN)
    );
    assert_eq!(
        ARETURN,
        Type::get_object_type("java/lang/Object").get_opcode(IRETURN)
    );
    assert_eq!(ARETURN, Type::get_type("[I").get_opcode(IRETURN));
}

#[test]
fn test_get_opcode_iadd() {
    assert_eq!(IADD, Type::BOOLEAN.get_opcode(IADD));
    assert_eq!(IADD, Type::BYTE.get_opcode(IADD));
    assert_eq!(IADD, Type::CHAR.get_opcode(IADD));
    assert_eq!(IADD, Type::SHORT.get_opcode(IADD));
    assert_eq!(IADD, Type::INT.get_opcode(IADD));
    assert_eq!(FADD, Type::FLOAT.get_opcode(IADD));
    assert_eq!(LADD, Type::LONG.get_opcode(IADD));
    assert_eq!(DADD, Type::DOUBLE.get_opcode(IADD));
}

#[test]
fn test_get_opcode_unsupported() {
    // ASM's UnsupportedOperationException: `checked_opcode` is None, `get_opcode` panics.
    let unsupported = [
        (Type::VOID, IADD),
        (Type::VOID, ILOAD),
        (Type::VOID, IALOAD),
        (Type::get_type("LI;"), IADD),
        (Type::get_type("[I"), IADD),
        (Type::get_object_type("I"), IADD),
        (Type::get_method_type("()V"), IADD),
        (Type::get_method_type("()V"), IALOAD),
    ];
    for (t, opcode) in unsupported {
        assert_eq!(None, t.checked_opcode(opcode), "{t} {opcode}");
        assert!(std::panic::catch_unwind(|| t.get_opcode(opcode)).is_err());
    }
}

#[test]
fn test_equals() {
    // `equalsNull` has no Rust analogue: a `Type` is never null.
    assert_ne!(Type::get_object_type("I"), Type::INT);
    assert_ne!(Type::get_object_type("I"), Type::get_object_type("HI"));
    assert_ne!(Type::get_object_type("I"), Type::get_object_type("J"));
    assert_eq!(Type::get_object_type("I"), Type::get_type("LI;"));
    assert_eq!(Type::get_type("LI;"), Type::get_object_type("I"));
    assert_eq!(Type::INT, Type::get_type("I"));
}

#[test]
fn test_hashcode() {
    assert_eq!(
        hash(&Type::get_type("Ljava/lang/Object;")),
        hash(&Type::get_object_type("java/lang/Object"))
    );
    assert_ne!(hash(&Type::INT), hash(&Type::get_object_type("I")));
}

// -------------------------------------------------------------------------------------------------
// oxasm: descriptor properties
// -------------------------------------------------------------------------------------------------

mod properties {
    use oxasm::{Type, TypeSort};
    use proptest::prelude::*;

    fn field_descriptor() -> impl Strategy<Value = String> {
        (
            0..4usize,
            prop_oneof![
                "[ZCBSIFJD]".prop_map(String::from),
                "[a-z][a-z0-9/$]{0,8}".prop_map(|n| format!("L{n};")),
                // Non-ASCII and lone-surrogate-free Unicode names survive too.
                "[a-z\u{e9}\u{4e2d}\u{1f600}]{1,4}".prop_map(|n| format!("L{n};")),
            ],
        )
            .prop_map(|(dims, element)| format!("{}{element}", "[".repeat(dims)))
    }

    proptest! {
        #[test]
        fn field_descriptors_round_trip(d in field_descriptor()) {
            let t = Type::get_type(&*d);
            prop_assert_eq!(d.as_str(), &*t.get_descriptor());
            if d.starts_with('L') {
                prop_assert_eq!(t.get_sort(), TypeSort::Object);
                prop_assert_eq!(Type::get_object_type(t.get_internal_name()), t.clone());
            } else if d.starts_with('[') {
                prop_assert_eq!(t.get_dimensions(), d.bytes().take_while(|&b| b == b'[').count());
                prop_assert_eq!(Type::get_object_type(&*d), t.clone());
            }
            // An owned copy is the same type.
            let owned = t.clone().into_owned();
            prop_assert_eq!(&owned, &t);
            prop_assert_eq!(&*owned.get_descriptor(), &*t.get_descriptor());
            prop_assert_eq!(owned.get_internal_name(), t.get_internal_name());
        }

        #[test]
        fn method_descriptors_round_trip(
            args in prop::collection::vec(field_descriptor(), 0..6),
            ret in prop_oneof![Just("V".to_owned()), field_descriptor()],
        ) {
            let d = format!("({}){ret}", args.concat());
            let t = Type::get_method_type(&*d);
            let arguments = t.get_argument_types();
            prop_assert_eq!(arguments.len(), args.len());
            prop_assert_eq!(t.get_argument_count(), args.len());
            prop_assert_eq!(Type::get_method_descriptor(&t.get_return_type(), &arguments), d.as_str());
            let sizes: usize = arguments.iter().map(Type::get_size).sum::<usize>() + 1;
            let ret_size = t.get_return_type().get_size();
            prop_assert_eq!(t.get_arguments_and_return_sizes() as usize, (sizes << 2) | ret_size);
            let owned = t.clone().into_owned();
            prop_assert_eq!(owned.get_argument_types(), arguments);
            prop_assert_eq!(owned.get_return_type(), t.get_return_type());
        }

        #[test]
        fn arbitrary_descriptors_never_panic_through_try(d in "[()\\[LVIJZ;/a\u{e9}]{0,12}") {
            if let Ok(t) = Type::try_get_type(&*d) {
                let _ = t.get_descriptor();
                let _ = t.get_internal_name();
                if t.get_sort() == TypeSort::Array {
                    let _ = t.try_get_element_type();
                }
            }
            let _ = Type::try_get_argument_types_of(&*d);
            let _ = Type::try_get_return_type_of(&*d);
            let _ = Type::try_get_argument_count_of(&*d);
            let _ = Type::try_get_arguments_and_return_sizes_of(&*d);
            let o = Type::get_object_type(&*d);
            let _ = (o.get_descriptor(), o.get_internal_name(), o.get_dimensions());
        }
    }
}

// oxasm-specific: ASM parses descriptors by `char`, so a malformed descriptor with a non-ASCII
// character where a type character belongs counts that character once (found by the analysis
// parity sweep over fuzzed classes).
#[test]
fn test_malformed_method_descriptor_non_ascii() {
    // The first character, which is skipped as the '(', is U+0604.
    assert_eq!(0, Type::get_argument_count_of("\u{604})Z"));
    assert_eq!(Type::BOOLEAN, Type::get_return_type_of("\u{604})Z"));
    assert_eq!(
        1 << 2 | 1,
        Type::get_arguments_and_return_sizes_of("\u{604})Z")
    );
    // An argument "type" é counts as one argument (whose type is invalid).
    assert_eq!(1, Type::get_argument_count_of("(é)V"));
    assert_eq!(2 << 2, Type::get_arguments_and_return_sizes_of("(é)V"));
    assert!(Type::try_get_argument_types_of("(é)V").is_err());
}

// oxasm-specific: a `Type` borrows its descriptor (ADR-15's P9 amendment); `into_owned` copies
// the part of it the type uses, and the owned type, and the types derived from it, outlive it.
#[test]
fn test_into_owned() {
    let method = {
        let descriptor = String::from("(I[Ljava/lang/String;)Ljava/util/List;");
        Type::get_method_type(&*descriptor).into_owned()
    };
    let arguments = method.get_argument_types();
    assert_eq!(Type::INT, arguments[0]);
    assert_eq!(
        "java/lang/String",
        arguments[1].get_element_type().get_internal_name()
    );
    assert_eq!(
        "Ljava/util/List;",
        &*method.get_return_type().get_descriptor()
    );

    // An object type keeps the characters around its internal name.
    let element = {
        let descriptor = String::from("[[Ljava/lang/Object;");
        Type::get_type(&*descriptor).get_element_type().into_owned()
    };
    assert_eq!("java/lang/Object", element.get_internal_name());
    assert_eq!("Ljava/lang/Object;", &*element.get_descriptor());

    // Even when the last of them is not ASCII (a malformed descriptor).
    let malformed = {
        let descriptor = String::from("Lfo\u{e9}");
        Type::get_type(&*descriptor).into_owned()
    };
    assert_eq!("fo", malformed.get_internal_name());
    assert_eq!("Lfo\u{e9}", &*malformed.get_descriptor());

    let internal = {
        let name = String::from("a/B");
        Type::get_object_type(&*name).into_owned()
    };
    assert_eq!("La/B;", &*internal.get_descriptor());
    assert_eq!(Type::get_type("La/B;"), internal);

    for (t, ..) in PRIMITIVES {
        assert_eq!(t, t.clone().into_owned());
    }
}
