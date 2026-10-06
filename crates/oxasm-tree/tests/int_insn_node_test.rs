//! Port of ASM's `IntInsnNodeTest`.

use oxasm::opcodes::{BIPUSH, SIPUSH};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let int_insn_node = Insn::Int {
        opcode: BIPUSH,
        operand: 0,
    };

    assert_eq!(int_insn_node.opcode(), Some(BIPUSH));
    assert_eq!(int_insn_node.kind(), InsnKind::IntInsn);
}

#[test]
fn test_set_opcode() {
    let mut int_insn_node = Insn::Int {
        opcode: BIPUSH,
        operand: 0,
    };

    int_insn_node.set_opcode(SIPUSH);

    assert_eq!(int_insn_node.opcode(), Some(SIPUSH));
}
