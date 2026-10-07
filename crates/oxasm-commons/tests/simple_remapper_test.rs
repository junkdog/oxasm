//! Port of ASM's `SimpleRemapperTest`.

use oxasm::{JavaStr, jstr};
use oxasm_commons::{Remapper, SimpleRemapper};

fn map_signature(remapper: &SimpleRemapper, signature: &str) -> String {
    remapper
        .map_signature(Some(jstr!(signature)), false)
        .unwrap()
        .to_string()
}

#[test]
fn test_map_signature_remap_parent_only_nested_class_extends() {
    let input_signature = "LOuter<Ljava/lang/Object;>.Inner;";
    let remapper = SimpleRemapper::from_iter([("Outer", "RenamedOuter")]);

    let remapped_signature = map_signature(&remapper, input_signature);

    assert_eq!(
        remapped_signature,
        "LRenamedOuter<Ljava/lang/Object;>.Inner;"
    );
}

#[test]
fn test_map_signature_remap_child_only_nested_class_extends() {
    let input_signature = "LOuter<Ljava/lang/Object;>.Inner;";
    let remapper = SimpleRemapper::from_iter([("Outer$Inner", "Outer$RenamedInner")]);

    let remapped_signature = map_signature(&remapper, input_signature);

    assert_eq!(
        remapped_signature,
        "LOuter<Ljava/lang/Object;>.RenamedInner;"
    );
}

#[test]
fn test_map_signature_remap_child_only_nested_class_extends_identifiers_with_dollar_sign() {
    let input_signature = "LOuter<Ljava/lang/Object;>.Inner$1;";
    let remapper = SimpleRemapper::from_iter([("Outer$Inner$1", "Outer$RenamedInner$1")]);

    let remapped_signature = map_signature(&remapper, input_signature);

    assert_eq!(
        remapped_signature,
        "LOuter<Ljava/lang/Object;>.RenamedInner$1;"
    );
}

#[test]
fn test_map_signature_remap_both_parent_and_child_nested_class_extends() {
    let input_signature = "LOuter<Ljava/lang/Object;>.Inner;";
    let remapper = SimpleRemapper::from_iter([
        ("Outer", "RenamedOuter"),
        ("Outer$Inner", "RenamedOuter$RenamedInner"),
    ]);

    let remapped_signature = map_signature(&remapper, input_signature);

    assert_eq!(
        remapped_signature,
        "LRenamedOuter<Ljava/lang/Object;>.RenamedInner;"
    );
}

/// oxasm-specific: the key conventions of `SimpleRemapper` (ASM documents them in its javadoc).
#[test]
fn test_key_conventions() {
    use oxasm::opcodes::HandleKind;
    use oxasm::{Constant, Handle, Type};

    let remapper = SimpleRemapper::from_iter([
        ("pkg/C", "pkg/D"),
        ("pkg/C.m(I)V", "n"),
        ("pkg/C.f", "g"),
        (".run()Ljava/lang/Runnable;", "call"),
        ("Lpkg/A;.value", "other"),
    ]);
    let name = |s: std::borrow::Cow<'_, JavaStr>| s.to_string();
    assert_eq!(name(remapper.map_type(jstr!("pkg/C"))), "pkg/D");
    assert_eq!(name(remapper.map_type(jstr!("[[Lpkg/C;"))), "[[Lpkg/D;");
    assert_eq!(name(remapper.map_type(jstr!("pkg/E"))), "pkg/E");
    assert_eq!(name(remapper.map_desc(jstr!("[Lpkg/C;"))), "[Lpkg/D;");
    assert_eq!(name(remapper.map_desc(jstr!("I"))), "I");
    assert_eq!(
        name(remapper.map_method_desc(jstr!("(ILpkg/C;[Lpkg/C;)Lpkg/C;"))),
        "(ILpkg/D;[Lpkg/D;)Lpkg/D;"
    );
    assert_eq!(
        name(remapper.map_method_name(jstr!("pkg/C"), jstr!("m"), jstr!("(I)V"))),
        "n"
    );
    assert_eq!(
        name(remapper.map_method_name(jstr!("pkg/C"), jstr!("m"), jstr!("(J)V"))),
        "m"
    );
    assert_eq!(
        name(remapper.map_field_name(jstr!("pkg/C"), jstr!("f"), jstr!("I"))),
        "g"
    );
    assert_eq!(
        name(remapper.map_annotation_attribute_name(jstr!("Lpkg/A;"), jstr!("value"))),
        "other"
    );
    let bsm = Handle::new(HandleKind::InvokeStatic, "pkg/B", "bsm", "()V", false);
    assert_eq!(
        name(remapper.map_invoke_dynamic_method_name(
            jstr!("run"),
            jstr!("()Ljava/lang/Runnable;"),
            &bsm,
            &[]
        )),
        "call"
    );
    assert_eq!(
        remapper.map_value(&Constant::Type(Type::get_method_type("(Lpkg/C;)V"))),
        Constant::Type(Type::get_method_type("(Lpkg/D;)V"))
    );
    assert_eq!(
        remapper.map_value(&Constant::Handle(Handle::new(
            HandleKind::InvokeStatic,
            "pkg/C",
            "m",
            "(I)V",
            false
        ))),
        Constant::Handle(Handle::new(
            HandleKind::InvokeStatic,
            "pkg/D",
            "n",
            "(I)V",
            false
        ))
    );
    assert_eq!(
        remapper.map_value(&Constant::Handle(Handle::new(
            HandleKind::GetStatic,
            "pkg/C",
            "f",
            "Lpkg/C;",
            false
        ))),
        Constant::Handle(Handle::new(
            HandleKind::GetStatic,
            "pkg/D",
            "g",
            "Lpkg/D;",
            false
        ))
    );
    assert_eq!(remapper.map_signature(None, false), None);
}
