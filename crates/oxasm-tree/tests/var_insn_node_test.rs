//! Port of ASM's `VarInsnNodeTest`.

use oxasm::opcodes::{ALOAD, ASTORE};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let var_insn_node = Insn::Var {
        opcode: ALOAD,
        var: 123,
    };

    assert_eq!(var_insn_node.opcode(), Some(ALOAD));
    assert_eq!(var_insn_node.kind(), InsnKind::VarInsn);
    assert!(matches!(var_insn_node, Insn::Var { var: 123, .. }));
}

#[test]
fn test_set_opcode() {
    let mut var_insn_node = Insn::Var {
        opcode: ALOAD,
        var: 123,
    };

    var_insn_node.set_opcode(ASTORE);

    assert_eq!(var_insn_node.opcode(), Some(ASTORE));
}
