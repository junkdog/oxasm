//! Port of ASM's `SignatureReaderTest`
//! (asm/src/test/java/org/objectweb/asm/signature/SignatureReaderTest.java), plus oxasm's
//! signature depth-bound tests (ADR-06).

use oxasm::signature::{SignatureError, SignatureReader, SignatureVisitor, SignatureWriter};
use oxasm_testkit::signatures::signatures;

/// ASM's anonymous `new SignatureVisitor(api) {}`: every method keeps its default.
struct Noop;

impl SignatureVisitor for Noop {}

#[test]
fn test_accept_valid_class_or_method_signature() {
    let corpus = signatures();
    for signature in corpus.class.iter().chain(&corpus.method) {
        let result = SignatureReader::new(signature.as_str()).accept(&mut Noop);
        assert_eq!(Ok(()), result, "{signature}");
    }
}

#[test]
fn test_accept_valid_field_signature() {
    for signature in &signatures().field {
        let result = SignatureReader::new(signature.as_str()).accept_type(&mut Noop);
        assert_eq!(Ok(()), result, "{signature}");
    }
}

#[test]
fn test_accept_invalid_signature() {
    let result = SignatureReader::new("-").accept(&mut Noop);
    assert_eq!(Err(SignatureError::Malformed(0)), result);
}

// -------------------------------------------------------------------------------------------------
// oxasm: malformed input never panics, and nesting is bounded (ASM 9.11's limit and message)
// -------------------------------------------------------------------------------------------------

fn nested(depth: usize) -> String {
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str("LA<");
    }
    s.push('I');
    for _ in 0..depth {
        s.push_str(">;");
    }
    s
}

#[test]
fn type_argument_nesting_is_bounded() {
    // 255 levels of type arguments (256 parse levels) are fine...
    let ok = nested(255);
    let mut writer = SignatureWriter::new();
    assert_eq!(
        Ok(()),
        SignatureReader::new(ok.as_str()).accept_type(&mut writer)
    );
    assert_eq!(ok, writer.to_string());
    // ...one more is not.
    let too_deep = nested(256);
    let error = SignatureReader::new(too_deep.as_str())
        .accept_type(&mut Noop)
        .unwrap_err();
    assert_eq!(SignatureError::TooManyNestedTypeArguments, error);
    assert_eq!("Too many nested type arguments", error.to_string());
}

#[test]
fn array_nesting_is_iterative() {
    // Arrays do not recurse (as in ASM 9.11), so any number of dimensions is fine.
    let deep_array = format!("{}I", "[".repeat(100_000));
    let mut writer = SignatureWriter::new();
    assert_eq!(
        Ok(()),
        SignatureReader::new(deep_array.as_str()).accept_type(&mut writer)
    );
    assert_eq!(deep_array, writer.to_string());
}

#[test]
fn malformed_signatures_are_errors() {
    for malformed in [
        "",
        "<",
        "<T",
        "<T:",
        "<T:Ljava/lang/Object;",
        "(",
        "(I",
        "()",
        "LA",
        "LA<",
        "LA<I",
        "TA",
        "[",
        "LA.",
        "Q",
        "LA<->;",
        "()I^",
    ] {
        let reader = SignatureReader::new(malformed);
        assert!(reader.accept(&mut Noop).is_err(), "accept {malformed:?}");
        if !malformed.starts_with(['<', '(']) {
            assert!(
                reader.accept_type(&mut Noop).is_err(),
                "accept_type {malformed:?}"
            );
        }
    }
}
