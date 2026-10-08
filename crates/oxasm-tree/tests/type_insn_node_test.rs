//! Port of ASM's `TypeInsnNodeTest`.

use oxasm::opcodes::{CHECKCAST, NEW};
use oxasm::sym;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let type_insn_node = Insn::type_insn(NEW, sym::JAVA_LANG_OBJECT);

    assert_eq!(type_insn_node.opcode(), Some(NEW));
    assert_eq!(type_insn_node.kind(), InsnKind::TypeInsn);
    let Insn::Type { desc, .. } = &type_insn_node else {
        unreachable!()
    };
    assert_eq!(*desc, sym::JAVA_LANG_OBJECT);
}

#[test]
fn test_set_opcode() {
    let mut type_insn_node = Insn::type_insn(NEW, sym::JAVA_LANG_OBJECT);

    type_insn_node.set_opcode(CHECKCAST);

    assert_eq!(type_insn_node.opcode(), Some(CHECKCAST));
}
