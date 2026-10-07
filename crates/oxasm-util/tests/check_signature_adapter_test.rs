//! Port of ASM's `CheckSignatureAdapterTest`, plus tests of the nested adapters with delegates
//! that return distinct nested visitors (the adapter holds pointers to them; ADR-18), which also
//! run under Miri.

use oxasm::signature::{SignatureReader, SignatureVisitor, SignatureWriter};
use oxasm::{JavaStr, jstr, opcodes};
use oxasm_testkit::{panic_message, panics};
use oxasm_util::TraceSignatureVisitor;
use oxasm_util::check::{CheckSignatureAdapter, SignatureKind};

/// The message of ASM's `IllegalStateException`s, which have none.
const ILLEGAL_STATE: &str = "java.lang.IllegalStateException";

fn adapter(kind: SignatureKind) -> CheckSignatureAdapter<'static> {
    CheckSignatureAdapter::new(kind, None)
}

#[test]
fn test_visit_formal_type_parameter_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);
    check_signature_adapter.visit_superclass();

    let message = panic_message(|| check_signature_adapter.visit_formal_type_parameter(jstr!("T")));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_formal_type_parameter_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_formal_type_parameter(jstr!("T")));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_class_bound_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_class_bound();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_class_bound_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| {
        check_signature_adapter.visit_class_bound();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_interface_bound_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_interface_bound();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_interface_bound_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| {
        check_signature_adapter.visit_interface_bound();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_super_class_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);
    check_signature_adapter.visit_superclass();

    let message = panic_message(|| {
        check_signature_adapter.visit_superclass();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_super_class_method_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Method);

    let message = panic_message(|| {
        check_signature_adapter.visit_superclass();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_interface_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_interface();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_interface_method_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Method);

    let message = panic_message(|| {
        check_signature_adapter.visit_interface();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_parameter_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_parameter_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_parameter_type_method_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Method);
    check_signature_adapter.visit_return_type();

    let message = panic_message(|| {
        check_signature_adapter.visit_parameter_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_return_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_return_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_return_type_method_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Method);
    check_signature_adapter.visit_return_type();

    let message = panic_message(|| {
        check_signature_adapter.visit_return_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_exception_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_exception_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_exception_type_method_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Method);

    let message = panic_message(|| {
        check_signature_adapter.visit_exception_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_base_type_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);
    check_signature_adapter.visit_base_type('I');

    let message = panic_message(|| check_signature_adapter.visit_base_type('I'));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_base_type_type_signature_illegal_void_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_base_type('V'));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Base type descriptor can't be V"
    );
}

#[test]
fn test_visit_base_type_type_signature_illegal_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_base_type('A'));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Base type descriptor must be one of ZCBSIFJD"
    );
}

#[test]
fn test_visit_base_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| check_signature_adapter.visit_base_type('I'));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_type_variable_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| check_signature_adapter.visit_type_variable(jstr!("T")));

    assert_eq!(message, ILLEGAL_STATE);
}

// testVisitTypeVariable_typeSignature_nullArgument dropped: a `null` name is unrepresentable
// (the empty name of the next test gets the same message).

#[test]
fn test_visit_type_variable_type_signature_empty_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_type_variable(jstr!("")));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type variable (must not be null or empty)"
    );
}

#[test]
fn test_visit_type_variable_type_signature_illegal_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_type_variable(jstr!("LT;")));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type variable (must not contain . ; [ / < > \
         or :): LT;"
    );
}

#[test]
fn test_visit_type_variable_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);
    check_signature_adapter.visit_type_variable(jstr!("T"));

    let message = panic_message(|| check_signature_adapter.visit_type_variable(jstr!("T")));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_array_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| {
        check_signature_adapter.visit_array_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_array_type_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);
    check_signature_adapter.visit_array_type();

    let message = panic_message(|| {
        check_signature_adapter.visit_array_type();
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_class_type_class_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);

    let message = panic_message(|| check_signature_adapter.visit_class_type(jstr!("A")));

    assert_eq!(message, ILLEGAL_STATE);
}

// testVisitClassType_typeSignature_nullArgument dropped: a `null` name is unrepresentable (the
// empty name of the next test gets the same message).

#[test]
fn test_visit_class_type_type_signature_empty_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_class_type(jstr!("")));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid class name (must not be null or empty)"
    );
}

#[test]
fn test_visit_class_type_type_signature_illegal_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_class_type(jstr!("<A>")));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid class name (must not contain . ; [ < > or \
         :): <A>"
    );
}

#[test]
fn test_visit_class_type_type_signature_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);
    check_signature_adapter.visit_class_type(jstr!("A"));

    let message = panic_message(|| check_signature_adapter.visit_class_type(jstr!("A")));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_class_type_non_java_identifier() {
    let mut check_signature_adapter = adapter(SignatureKind::Class);
    let signature_visitor = check_signature_adapter.visit_superclass();

    assert!(!panics(
        || signature_visitor.visit_class_type(jstr!("Foo Bar"))
    ));
}

#[test]
fn test_visit_inner_class_type_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_inner_class_type(jstr!("A")));

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_type_argument_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_type_argument());

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_type_argument_wildcard_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| {
        check_signature_adapter.visit_type_argument_wildcard('+');
    });

    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn test_visit_type_argument_wildcard_illegal_argument() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);
    check_signature_adapter.visit_class_type(jstr!("A"));

    let message = panic_message(|| {
        check_signature_adapter.visit_type_argument_wildcard('*');
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Wildcard must be one of +-="
    );
}

#[test]
fn test_visit_end_illegal_state() {
    let mut check_signature_adapter = adapter(SignatureKind::Type);

    let message = panic_message(|| check_signature_adapter.visit_end());

    assert_eq!(message, ILLEGAL_STATE);
}

// -------------------------------------------------------------------------------------------------
// The parameterized tests, over the signatures of the precompiled classes (`SignaturesProviders`).
// They read the corpus from files, which Miri's isolation forbids: the Miri-friendly versions
// are the `small_*` tests below.
// -------------------------------------------------------------------------------------------------

fn corpus() -> &'static oxasm_testkit::signatures::Signatures {
    oxasm_testkit::signatures::signatures()
}

/// Reads `signature` (as a type if `kind` is `Type`) into `visitor`.
fn read(signature: &str, kind: SignatureKind, visitor: &mut dyn SignatureVisitor) {
    let reader = SignatureReader::new(signature);
    match kind {
        SignatureKind::Type => reader.accept_type(visitor),
        SignatureKind::Class | SignatureKind::Method => reader.accept(visitor),
    }
    .unwrap_or_else(|e| panic!("cannot read {signature}: {e}"));
}

/// Reads `signature` through a check adapter into a `SignatureWriter`, which must write it back.
fn assert_checked_round_trip(signature: &str, kind: SignatureKind) {
    let mut signature_writer = SignatureWriter::new();
    read(
        signature,
        kind,
        &mut CheckSignatureAdapter::new(kind, Some(&mut signature_writer)),
    );
    assert_eq!(signature_writer.to_string(), signature);
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_class_signature() {
    assert!(!corpus().class.is_empty());
    for signature in &corpus().class {
        assert_checked_round_trip(signature, SignatureKind::Class);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_class_signature_no_delegate() {
    for signature in &corpus().class {
        read(
            signature,
            SignatureKind::Class,
            &mut adapter(SignatureKind::Class),
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_method_signature() {
    assert!(!corpus().method.is_empty());
    for signature in &corpus().method {
        assert_checked_round_trip(signature, SignatureKind::Method);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_method_signature_no_delegate() {
    for signature in &corpus().method {
        read(
            signature,
            SignatureKind::Method,
            &mut adapter(SignatureKind::Method),
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_type_signature() {
    assert!(!corpus().field.is_empty());
    for signature in &corpus().field {
        assert_checked_round_trip(signature, SignatureKind::Type);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn test_visit_methods_type_signature_no_delegate() {
    for signature in &corpus().field {
        read(
            signature,
            SignatureKind::Type,
            &mut adapter(SignatureKind::Type),
        );
    }
}

// -------------------------------------------------------------------------------------------------
// oxasm-specific: delegates whose nested visitors are distinct objects (TraceSignatureVisitor),
// or the delegate itself (SignatureWriter), and re-entering a parent after its children. These
// exercise the pointers the adapter keeps to its delegate's nested visitors, under Miri too.
// -------------------------------------------------------------------------------------------------

const SMALL_CLASS_SIGNATURES: &[&str] = &[
    "<E:Ljava/lang/Enum<TE;>;>Ljava/lang/Object;Ljava/lang/Comparable<TE;>;Ljava/io/Serializable;",
    "<D::Ljava/lang/reflect/GenericDeclaration;>Ljava/lang/Object;Ljava/lang/reflect/Type;",
    "<K:Ljava/lang/Object;V:Ljava/lang/Object;>Ljava/util/AbstractMap<TK;TV;>;Ljava/util/concurrent/ConcurrentMap<TK;TV;>;Ljava/io/Serializable;",
    "<T:Ljava/lang/Object;R:TT;>Ljava/lang/Object;",
    "Ljava/lang/Object;Ljava/util/List<+Ljava/util/Map$Entry<-[[TT;*>;>.Inner<[I>;",
];

const SMALL_METHOD_SIGNATURES: &[&str] = &[
    "<T:Ljava/lang/Object;>(TT;[J[[TT;)V",
    "(Ljava/util/Map<TK;+Ljava/util/List<*>;>;IZ)Ljava/util/Set<Ljava/util/Map$Entry<TK;TV;>;>;^TE;^Ljava/io/IOException;",
    "<A::Ljava/lang/Comparable<-TA;>;:Ljava/io/Serializable;>([TA;)TA;",
];

const SMALL_TYPE_SIGNATURES: &[&str] = &[
    "[TT;",
    "LAA<[[B>;",
    "Ljava/lang/Class<*>;",
    "Lpkg/Outer<TT;>.Inner<-Ljava/lang/Number;+[Ljava/lang/String;TU;>.Deep;",
];

/// The access flags of the TraceSignatureVisitor for each kind (an interface's class signature
/// is printed with "extends").
fn trace(kind: SignatureKind) -> TraceSignatureVisitor {
    TraceSignatureVisitor::new(match kind {
        SignatureKind::Class => opcodes::ACC_PUBLIC,
        SignatureKind::Method | SignatureKind::Type => 0,
    })
}

/// What a TraceSignatureVisitor prints for `signature`.
fn traced(trace: &TraceSignatureVisitor) -> (String, Option<String>, Option<String>) {
    (
        trace.get_declaration().to_string(),
        trace.get_return_type().map(JavaStr::to_string),
        trace.get_exceptions().map(|e| e.to_string()),
    )
}

/// Reads `signature` into a TraceSignatureVisitor, directly and through a check adapter, which
/// must not change what it prints.
fn assert_checked_trace(signature: &str, kind: SignatureKind) {
    let mut direct = trace(kind);
    read(signature, kind, &mut direct);
    let mut checked = trace(kind);
    read(
        signature,
        kind,
        &mut CheckSignatureAdapter::new(kind, Some(&mut checked)),
    );
    assert_eq!(traced(&checked), traced(&direct), "{signature}");
}

#[test]
fn small_signatures_through_a_trace_signature_visitor() {
    for signature in SMALL_CLASS_SIGNATURES {
        assert_checked_trace(signature, SignatureKind::Class);
    }
    for signature in SMALL_METHOD_SIGNATURES {
        assert_checked_trace(signature, SignatureKind::Method);
    }
    for signature in SMALL_TYPE_SIGNATURES {
        assert_checked_trace(signature, SignatureKind::Type);
    }
}

#[test]
fn small_signatures_through_a_signature_writer() {
    for signature in SMALL_CLASS_SIGNATURES {
        assert_checked_round_trip(signature, SignatureKind::Class);
    }
    for signature in SMALL_METHOD_SIGNATURES {
        assert_checked_round_trip(signature, SignatureKind::Method);
    }
    for signature in SMALL_TYPE_SIGNATURES {
        assert_checked_round_trip(signature, SignatureKind::Type);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reads the corpus from files")]
fn corpus_signatures_through_a_trace_signature_visitor() {
    for signature in &corpus().class {
        assert_checked_trace(signature, SignatureKind::Class);
    }
    for signature in &corpus().method {
        assert_checked_trace(signature, SignatureKind::Method);
    }
    for signature in &corpus().field {
        assert_checked_trace(signature, SignatureKind::Type);
    }
}

/// Drives the events of `<T:Ljava/util/List<TT;>;>(TT;[I)Ljava/util/Map<TT;*>;^Ljava/io/IOException;`
/// by hand, re-entering the method adapter after each nested visitor (and the class type
/// adapters after their type arguments).
fn drive_method_signature(visitor: &mut dyn SignatureVisitor) {
    visitor.visit_formal_type_parameter(jstr!("T"));
    {
        let bound = visitor.visit_class_bound();
        bound.visit_class_type(jstr!("java/util/List"));
        {
            let argument = bound.visit_type_argument_wildcard('=');
            argument.visit_type_variable(jstr!("T"));
        }
        bound.visit_end();
    }
    visitor
        .visit_parameter_type()
        .visit_type_variable(jstr!("T"));
    {
        let parameter = visitor.visit_parameter_type();
        let element = parameter.visit_array_type();
        element.visit_base_type('I');
    }
    {
        let return_type = visitor.visit_return_type();
        return_type.visit_class_type(jstr!("java/util/Map"));
        return_type
            .visit_type_argument_wildcard('=')
            .visit_type_variable(jstr!("T"));
        return_type.visit_type_argument();
        return_type.visit_end();
    }
    {
        let exception = visitor.visit_exception_type();
        exception.visit_class_type(jstr!("java/io/IOException"));
        exception.visit_end();
    }
}

const DRIVEN_METHOD_SIGNATURE: &str =
    "<T:Ljava/util/List<TT;>;>(TT;[I)Ljava/util/Map<TT;*>;^Ljava/io/IOException;";

#[test]
fn hand_driven_events_with_reentered_parents() {
    let mut signature_writer = SignatureWriter::new();
    drive_method_signature(&mut CheckSignatureAdapter::new(
        SignatureKind::Method,
        Some(&mut signature_writer),
    ));
    assert_eq!(signature_writer.to_string(), DRIVEN_METHOD_SIGNATURE);

    let mut direct = trace(SignatureKind::Method);
    drive_method_signature(&mut direct);
    let mut checked = trace(SignatureKind::Method);
    drive_method_signature(&mut CheckSignatureAdapter::new(
        SignatureKind::Method,
        Some(&mut checked),
    ));
    assert_eq!(traced(&checked), traced(&direct));

    // And the events are the reader's.
    let mut from_reader = trace(SignatureKind::Method);
    read(
        DRIVEN_METHOD_SIGNATURE,
        SignatureKind::Method,
        &mut from_reader,
    );
    assert_eq!(traced(&checked), traced(&from_reader));
}

#[test]
fn nested_adapters_check_their_own_state() {
    let mut trace = trace(SignatureKind::Method);
    let mut check_signature_adapter =
        CheckSignatureAdapter::new(SignatureKind::Method, Some(&mut trace));
    let parameter = check_signature_adapter.visit_parameter_type();
    parameter.visit_base_type('I');
    // The nested adapter checks a type signature: a second type is illegal.
    let message = panic_message(|| parameter.visit_class_type(jstr!("A")));
    assert_eq!(message, ILLEGAL_STATE);
    // A parameter type cannot be void, a return type can.
    let message = panic_message(|| {
        check_signature_adapter
            .visit_parameter_type()
            .visit_base_type('V')
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Base type descriptor can't be V"
    );
    check_signature_adapter
        .visit_return_type()
        .visit_base_type('V');
    // Exceptions only come after the return type.
    check_signature_adapter
        .visit_exception_type()
        .visit_type_variable(jstr!("E"));
    let message = panic_message(|| {
        check_signature_adapter.visit_parameter_type();
    });
    assert_eq!(message, ILLEGAL_STATE);
}

#[test]
fn malformed_events_from_a_reader_are_rejected() {
    // The reader passes a `.` separated inner class name check with a `/` in it.
    let message = panic_message(|| {
        let mut check_signature_adapter = adapter(SignatureKind::Type);
        let _ = SignatureReader::new("LA<TT;>.B/C;").accept_type(&mut check_signature_adapter);
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid inner class name (must not contain . ; [ / \
         < > or :): B/C"
    );
}
