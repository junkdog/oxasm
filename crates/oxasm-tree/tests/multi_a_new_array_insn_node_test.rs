//! Port of ASM's `MultiANewArrayInsnNodeTest`.

use oxasm::opcodes::MULTIANEWARRAY;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let multi_a_new_array_insn_node = Insn::multi_a_new_array_insn("[[I", 2);

    assert_eq!(
        multi_a_new_array_insn_node.kind(),
        InsnKind::MultiANewArrayInsn
    );
    assert_eq!(multi_a_new_array_insn_node.opcode(), Some(MULTIANEWARRAY));
    let Insn::MultiANewArray { desc, dims } = &multi_a_new_array_insn_node else {
        unreachable!()
    };
    assert_eq!(desc, "[[I");
    assert_eq!(*dims, 2);
}
