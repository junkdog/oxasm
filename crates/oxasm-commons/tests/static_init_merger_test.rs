//! Port of ASM's `StaticInitMergerTest`: unit tests for `StaticInitMerger`.

mod common;

use oxasm::opcodes::*;
use oxasm::{ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, ParsingOptions, jstr};
use oxasm_commons::StaticInitMerger;
use oxasm_tree::ClassNode;

fn class_writer_output(num_static_init_blocks: usize) -> Vec<u8> {
    let class_node = new_class_with_static_init_blocks(num_static_init_blocks);
    let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
    let mut static_init_merger = StaticInitMerger::new("$clinit$", &mut class_writer);

    class_node.accept(&mut static_init_merger);

    class_writer.to_bytes().unwrap()
}

/// ASM's test loads the class, and checks that its `counter` field is 5 after its static
/// initialization. The JVM oracle only verifies classes (it does not initialize them), so the
/// initialization is checked here through the merged method's code (which ASM's test does not
/// look at), and the class through the JVM in [`test_all_methods_multiple_static_init_blocks_new_instance`].
#[test]
fn test_all_methods_multiple_static_init_blocks() {
    let class_file = class_writer_output(5);

    let mut class_node = ClassNode::default();
    ClassReader::new(&class_file)
        .unwrap()
        .accept(&mut class_node, ParsingOptions::empty())
        .unwrap();
    let method_names: Vec<String> = class_node
        .methods
        .iter()
        .map(|m| format!("{} {:#x}", m.name, m.access))
        .collect();
    // The merged <clinit> is the last method here, and the second one in ASM (see the
    // StaticInitMerger docs).
    assert_eq!(
        method_names,
        [
            "$clinit$0 0xa",
            "$clinit$1 0xa",
            "$clinit$2 0xa",
            "$clinit$3 0xa",
            "$clinit$4 0xa",
            "<init> 0x1",
            "<clinit> 0xa",
        ]
    );
    let merged_clinit = class_node.methods.last().unwrap();
    assert_eq!(
        common::to_text(merged_clinit),
        "    INVOKESTATIC A.$clinit$0 ()V\n    INVOKESTATIC A.$clinit$1 ()V\n    \
         INVOKESTATIC A.$clinit$2 ()V\n    INVOKESTATIC A.$clinit$3 ()V\n    \
         INVOKESTATIC A.$clinit$4 ()V\n    RETURN\n    MAXSTACK = 0\n    MAXLOCALS = 0\n"
    );
    for renamed in &class_node.methods[..5] {
        assert_eq!(
            common::to_text(renamed),
            "    GETSTATIC A.counter : I\n    ICONST_1\n    IADD\n    PUTSTATIC A.counter : I\n    \
             RETURN\n    MAXSTACK = 2\n    MAXLOCALS = 0\n"
        );
    }
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_multiple_static_init_blocks_new_instance() {
    common::assert_new_instance(&class_writer_output(5));
}

fn new_class_with_static_init_blocks(num_static_init_blocks: usize) -> ClassNode {
    let mut class_node = ClassNode::default();
    class_node.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("A"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    let _ = class_node.visit_field(
        ACC_PUBLIC + ACC_STATIC,
        jstr!("counter"),
        jstr!("I"),
        None,
        None,
    );
    for _ in 0..num_static_init_blocks {
        let mut method_visitor = class_node
            .visit_method(ACC_PUBLIC, jstr!("<clinit>"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_field_insn(GETSTATIC, jstr!("A"), jstr!("counter"), jstr!("I"));
        method_visitor.visit_insn(ICONST_1);
        method_visitor.visit_insn(IADD);
        method_visitor.visit_field_insn(PUTSTATIC, jstr!("A"), jstr!("counter"), jstr!("I"));
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
    }
    let mut method_visitor = class_node
        .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
        .unwrap();
    method_visitor.visit_var_insn(ALOAD, 0);
    method_visitor.visit_method_insn(
        INVOKESPECIAL,
        jstr!("java/lang/Object"),
        jstr!("<init>"),
        jstr!("()V"),
        false,
    );
    method_visitor.visit_insn(RETURN);
    method_visitor.visit_maxs(0, 0);
    drop(method_visitor);
    class_node.visit_end();
    class_node
}
