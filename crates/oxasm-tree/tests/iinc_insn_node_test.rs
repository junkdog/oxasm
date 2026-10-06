//! Port of ASM's `IincInsnNodeTest`.

use oxasm::opcodes::IINC;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let iinc_insn_node = Insn::Iinc { var: 1, incr: 2 };

    assert_eq!(iinc_insn_node.kind(), InsnKind::IincInsn);
    assert_eq!(iinc_insn_node.opcode(), Some(IINC));
    assert!(matches!(iinc_insn_node, Insn::Iinc { var: 1, incr: 2 }));
}
