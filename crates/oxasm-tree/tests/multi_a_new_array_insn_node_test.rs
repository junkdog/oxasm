//! Port of ASM's `MultiANewArrayInsnNodeTest`.

use oxasm::Interner;
use oxasm::opcodes::MULTIANEWARRAY;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let multi_a_new_array_insn_node = Insn::multi_a_new_array_insn(interner.intern("[[I"), 2);

    assert_eq!(
        multi_a_new_array_insn_node.kind(),
        InsnKind::MultiANewArrayInsn
    );
    assert_eq!(multi_a_new_array_insn_node.opcode(), Some(MULTIANEWARRAY));
    let Insn::MultiANewArray { desc, dims } = &multi_a_new_array_insn_node else {
        unreachable!()
    };
    assert_eq!(interner.resolve(*desc), "[[I");
    assert_eq!(*dims, 2);
}
