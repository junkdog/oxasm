//! Port of ASM's `MethodInsnNodeTest`. ASM's deprecated 4-argument constructor is
//! `Insn::method_insn_of`.

use oxasm::opcodes::{INVOKEINTERFACE, INVOKESPECIAL, INVOKESTATIC};
use oxasm_tree::{Insn, InsnKind};

fn is_interface(insn: &Insn) -> bool {
    let Insn::Method { is_interface, .. } = insn else {
        unreachable!()
    };
    *is_interface
}

#[test]
fn test_deprecated_constructor() {
    let method_insn_node1 = Insn::method_insn_of(INVOKESTATIC, "owner", "name", "()I");
    let method_insn_node2 = Insn::method_insn_of(INVOKEINTERFACE, "owner", "name", "()I");

    assert_eq!(method_insn_node1.kind(), InsnKind::MethodInsn);
    assert_eq!(method_insn_node2.kind(), InsnKind::MethodInsn);
    assert_eq!(method_insn_node1.opcode(), Some(INVOKESTATIC));
    assert_eq!(method_insn_node2.opcode(), Some(INVOKEINTERFACE));
    assert!(!is_interface(&method_insn_node1));
    assert!(is_interface(&method_insn_node2));
}

#[test]
fn test_construtor() {
    let method_insn_node = Insn::method_insn(INVOKESTATIC, "owner", "name", "()I", false);

    assert_eq!(method_insn_node.kind(), InsnKind::MethodInsn);
    assert_eq!(method_insn_node.opcode(), Some(INVOKESTATIC));
    let Insn::Method {
        owner, name, desc, ..
    } = &method_insn_node
    else {
        unreachable!()
    };
    assert_eq!(owner, "owner");
    assert_eq!(name, "name");
    assert_eq!(desc, "()I");
    assert!(!is_interface(&method_insn_node));
}

#[test]
fn test_set_opcode() {
    let mut method_insn_node = Insn::method_insn(INVOKESTATIC, "owner", "name", "()I", false);

    method_insn_node.set_opcode(INVOKESPECIAL);

    assert_eq!(method_insn_node.opcode(), Some(INVOKESPECIAL));
}
