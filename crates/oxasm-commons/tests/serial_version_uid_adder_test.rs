//! Port of ASM's `SerialVersionUidAdderTest`: unit tests for `SerialVersionUIDAdder`
//! (`SerialVersionUidAdder`).

mod common;

use std::sync::Arc;

use oxasm::opcodes::*;
use oxasm::{ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, ParsingOptions, jstr};
use oxasm_commons::SerialVersionUidAdder;
use oxasm_testkit::{ClassFile, PrecompiledClass, t0_resource};
use oxasm_tree::{ClassNode, SymConstant};

/// ASM's `null` class visitor.
struct NoDelegate;

impl ClassVisitor for NoDelegate {}

#[test]
fn test_constructor() {
    // ASM's second assertion (a subclass must use the constructor with an api) is dropped:
    // there is no subclassing, and no api (ADR-09, ADR-13).
    let _ = SerialVersionUidAdder::new(NoDelegate);
}

#[test]
fn test_all_methods() {
    for (class_name, expected_svuid) in [
        ("SerialVersionClass", 4_654_798_559_887_898_126_i64),
        (
            "SerialVersionAnonymousInnerClass$1",
            2_591_057_588_230_880_800,
        ),
        ("SerialVersionInterface", 682_190_902_657_822_970),
        ("SerialVersionEmptyInterface", -2_126_445_979_242_430_981),
    ] {
        let class_file = t0_resource("asm-commons", &format!("{class_name}.class"));
        let class_reader = ClassReader::new(&class_file).unwrap();
        let mut svuid_adder = SerialVersionUidAdder::new(NoDelegate);

        class_reader
            .accept(&mut svuid_adder, ParsingOptions::empty())
            .unwrap();
        let svuid = svuid_adder.compute_svuid().unwrap();

        assert_eq!(svuid, expected_svuid, "{class_name}");
    }
}

#[test]
fn test_all_methods_enum() {
    let class_file = t0_resource("asm-commons", "SerialVersionEnum.class");
    let class_reader = ClassReader::new(&class_file).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut svuid_adder = SerialVersionUidAdder::new(&mut class_writer);

    class_reader
        .accept(&mut svuid_adder, ParsingOptions::empty())
        .unwrap();

    let new_class_file = class_writer.to_bytes().unwrap();
    assert!(
        !ClassFile::new(&new_class_file)
            .to_string()
            .contains("serialVersionUID")
    );
}

/// Tests that SerialVersionUIDAdder succeeds on all precompiled classes, and that it actually
/// adds a serialVersionUID field.
#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_reader
            .accept(
                &mut SerialVersionUidAdder::new(&mut class_writer),
                ParsingOptions::empty(),
            )
            .unwrap();

        let new_class_file = class_writer.to_bytes().unwrap();
        if (class_reader.get_access().unwrap() & ACC_ENUM) == 0 {
            assert!(
                ClassFile::new(&new_class_file)
                    .to_string()
                    .contains("serialVersionUID"),
                "{class_parameter}"
            );
        }
    }
}

/// oxasm-specific: the added field holds the computed value, and a class that already has a
/// serialVersionUID is left as is.
#[test]
fn test_added_field_value() {
    let class_file = t0_resource("asm-commons", "SerialVersionClass.class");
    let class_reader = ClassReader::new(&class_file).unwrap();
    let mut class_node = ClassNode::with_interner(Arc::default());
    let mut svuid_adder = SerialVersionUidAdder::new(&mut class_node);
    class_reader
        .accept(&mut svuid_adder, ParsingOptions::empty())
        .unwrap();
    assert!(!svuid_adder.has_svuid());
    let field = class_node.fields.last().unwrap();
    assert_eq!(field.resolve(field.name), "serialVersionUID");
    assert_eq!(field.access, ACC_FINAL + ACC_STATIC);
    assert_eq!(
        field.value,
        Some(SymConstant::Long(4_654_798_559_887_898_126))
    );

    // Visiting the result again finds the field.
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_node.accept(&mut class_writer);
    let with_svuid = class_writer.to_bytes().unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut svuid_adder = SerialVersionUidAdder::new(&mut class_writer);
    ClassReader::new(&with_svuid)
        .unwrap()
        .accept(&mut svuid_adder, ParsingOptions::empty())
        .unwrap();
    assert!(svuid_adder.has_svuid());
    assert_eq!(class_writer.to_bytes().unwrap(), with_svuid);
}

/// oxasm-specific: names are sorted as Java strings (by UTF-16 code units, where a
/// supplementary character sorts before U+E000, unlike in UTF-8), and written in modified UTF-8
/// (NUL as two bytes). The expected value is ASM's, for the same events.
#[test]
fn test_java_string_order_and_encoding() {
    let mut svuid_adder = SerialVersionUidAdder::new(NoDelegate);
    svuid_adder.visit(
        V1_8,
        ACC_PUBLIC,
        jstr!("p/C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[jstr!("p/\u{E000}"), jstr!("p/\u{10000}"), jstr!("p/A")],
    );
    for (access, name) in [
        (ACC_PUBLIC, "a\0b"),
        (ACC_PUBLIC, "\u{E000}"),
        (ACC_PUBLIC, "\u{10000}"),
        (ACC_PRIVATE | ACC_STATIC, "hidden"),
    ] {
        let _ = svuid_adder.visit_field(access, jstr!(name), jstr!("I"), None, None);
    }
    for (access, name, descriptor) in [
        (ACC_PUBLIC, "<init>", "(Lp/A;)V"),
        (ACC_STATIC, "<clinit>", "()V"),
        (ACC_PUBLIC, "m", "(Lp/\u{E000};)V"),
        (ACC_PUBLIC, "m", "(Lp/\u{10000};)V"),
        (ACC_PRIVATE, "n", "()V"),
    ] {
        let _ = svuid_adder.visit_method(access, jstr!(name), jstr!(descriptor), None, &[]);
    }
    svuid_adder.visit_inner_class(
        jstr!("p/C"),
        Some(jstr!("p/O")),
        Some(jstr!("C")),
        ACC_PROTECTED | ACC_STATIC,
    );

    assert_eq!(svuid_adder.compute_svuid(), Ok(-4_191_165_079_078_747_584));
}
