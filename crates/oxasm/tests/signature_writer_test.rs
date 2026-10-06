//! Port of ASM's `SignatureWriterTest`
//! (asm/src/test/java/org/objectweb/asm/signature/SignatureWriterTest.java), plus proptests.

use oxasm::signature::{SignatureReader, SignatureWriter};
use oxasm_testkit::signatures::signatures;
use proptest::prelude::*;

#[test]
fn test_read_and_write_class_or_method_signature() {
    let corpus = signatures();
    for signature in corpus.class.iter().chain(&corpus.method) {
        let mut signature_writer = SignatureWriter::new();
        SignatureReader::new(signature.as_str())
            .accept(&mut signature_writer)
            .unwrap();
        assert_eq!(*signature, signature_writer.to_string());
    }
}

#[test]
fn test_read_and_write_field_signature() {
    for signature in &signatures().field {
        let mut signature_writer = SignatureWriter::new();
        SignatureReader::new(signature.as_str())
            .accept_type(&mut signature_writer)
            .unwrap();
        assert_eq!(*signature, signature_writer.to_string());
    }
}

// -------------------------------------------------------------------------------------------------
// Properties: generated well-formed signatures round trip; arbitrary input never panics.
// -------------------------------------------------------------------------------------------------

fn identifier() -> impl Strategy<Value = String> {
    "[A-Za-z_$][A-Za-z0-9_$/]{0,6}".prop_map(|s| s.trim_end_matches('/').replace("//", "/"))
}

fn reference_type() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        identifier().prop_map(|n| format!("L{n};")),
        identifier().prop_map(|n| format!("T{n};")),
        "[ZCBSIFJD]".prop_map(|b| format!("[{b}")),
    ];
    leaf.prop_recursive(4, 24, 4, |inner| {
        prop_oneof![
            inner.clone().prop_map(|t| format!("[{t}")),
            (
                identifier(),
                prop::collection::vec(
                    prop_oneof![
                        Just("*".to_owned()),
                        inner.clone(),
                        inner.clone().prop_map(|t| format!("+{t}")),
                        inner.clone().prop_map(|t| format!("-{t}")),
                    ],
                    1..3
                ),
                prop::option::of(identifier()),
            )
                .prop_map(|(name, args, inner_class)| {
                    let inner_class = inner_class.map(|i| format!(".{}", i.replace('/', "")));
                    format!(
                        "L{name}<{}>{};",
                        args.concat(),
                        inner_class.unwrap_or_default()
                    )
                }),
        ]
    })
}

fn java_type() -> impl Strategy<Value = String> {
    prop_oneof![1 => "[ZCBSIFJD]".prop_map(String::from), 4 => reference_type()]
}

fn formals() -> impl Strategy<Value = String> {
    // An empty class bound needs an interface bound, as javac emits (`T::Ljava/lang/Runnable;`):
    // otherwise a following formal named `T...` or `L...` would read as the class bound.
    let param = (
        identifier(),
        prop::option::of(reference_type()),
        prop::collection::vec(reference_type(), 0..2),
    )
        .prop_filter("empty class bound without interface bound", |(_, c, i)| {
            c.is_some() || !i.is_empty()
        });
    prop::collection::vec(param, 0..3).prop_map(|params| {
        if params.is_empty() {
            return String::new();
        }
        let body: String = params
            .into_iter()
            .map(|(name, class_bound, itf_bounds)| {
                let name = name.replace('/', "");
                let itfs: String = itf_bounds.iter().map(|b| format!(":{b}")).collect();
                format!("{name}:{}{itfs}", class_bound.unwrap_or_default())
            })
            .collect();
        format!("<{body}>")
    })
}

proptest! {
    #[test]
    fn type_signatures_round_trip(signature in java_type()) {
        let mut writer = SignatureWriter::new();
        SignatureReader::new(signature.as_str()).accept_type(&mut writer).unwrap();
        prop_assert_eq!(signature, writer.to_string());
    }

    #[test]
    fn method_signatures_round_trip(
        formals in formals(),
        params in prop::collection::vec(java_type(), 0..4),
        ret in prop_oneof![Just("V".to_owned()), java_type()],
        throws in prop::collection::vec(reference_type(), 0..2),
    ) {
        let throws: String = throws.iter().map(|t| format!("^{t}")).collect();
        let signature = format!("{formals}({}){ret}{throws}", params.concat());
        let mut writer = SignatureWriter::new();
        SignatureReader::new(signature.as_str()).accept(&mut writer).unwrap();
        prop_assert_eq!(signature, writer.to_string());
    }

    #[test]
    fn class_signatures_round_trip(
        formals in formals(),
        supertypes in prop::collection::vec(identifier().prop_map(|n| format!("L{n};")), 1..4),
    ) {
        let signature = format!("{formals}{}", supertypes.concat());
        let mut writer = SignatureWriter::new();
        SignatureReader::new(signature.as_str()).accept(&mut writer).unwrap();
        prop_assert_eq!(signature, writer.to_string());
    }

    #[test]
    fn arbitrary_input_never_panics(input in "[<>():;.^*+\\-\\[LTVIJZ/a-c\\u{e9}]{0,24}") {
        let reader = SignatureReader::new(input.as_str());
        let _ = reader.accept(&mut SignatureWriter::new());
        let _ = reader.accept_type(&mut SignatureWriter::new());
    }
}
