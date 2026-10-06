//! Port of ASM's `JumpInsnNodeTest`. A `LabelNode` is its `Label` (ADR-11).

use oxasm::Label;
use oxasm::opcodes::{GOTO, IFEQ};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let label_node = Label::new();
    let jump_insn_node = Insn::Jump {
        opcode: GOTO,
        label: label_node,
    };

    assert_eq!(jump_insn_node.opcode(), Some(GOTO));
    assert_eq!(jump_insn_node.kind(), InsnKind::JumpInsn);
    assert!(matches!(jump_insn_node, Insn::Jump { label, .. } if label == label_node));
}

#[test]
fn test_set_opcode() {
    let mut jump_insn_node = Insn::Jump {
        opcode: GOTO,
        label: Label::new(),
    };

    jump_insn_node.set_opcode(IFEQ);

    assert_eq!(jump_insn_node.opcode(), Some(IFEQ));
}
