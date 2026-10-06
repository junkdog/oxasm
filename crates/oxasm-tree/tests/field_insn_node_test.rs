//! Port of ASM's `FieldInsnNodeTest`.

use oxasm::opcodes::{GETSTATIC, PUTSTATIC};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let field_insn_node = Insn::field_insn(GETSTATIC, "owner", "name", "I");

    assert_eq!(field_insn_node.kind(), InsnKind::FieldInsn);
    assert_eq!(field_insn_node.opcode(), Some(GETSTATIC));
    let Insn::Field {
        owner, name, desc, ..
    } = &field_insn_node
    else {
        unreachable!()
    };
    assert_eq!(owner, "owner");
    assert_eq!(name, "name");
    assert_eq!(desc, "I");
}

#[test]
fn test_set_opcode() {
    let mut field_insn_node = Insn::field_insn(GETSTATIC, "owner", "name", "I");

    field_insn_node.set_opcode(PUTSTATIC);

    assert_eq!(field_insn_node.opcode(), Some(PUTSTATIC));
}
