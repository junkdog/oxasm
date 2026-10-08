//! Port of ASM's `FieldInsnNodeTest`.

use oxasm::Interner;
use oxasm::opcodes::{GETSTATIC, PUTSTATIC};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let field_insn_node = Insn::field_insn(
        GETSTATIC,
        interner.intern("owner"),
        interner.intern("name"),
        interner.intern("I"),
    );

    assert_eq!(field_insn_node.kind(), InsnKind::FieldInsn);
    assert_eq!(field_insn_node.opcode(), Some(GETSTATIC));
    let Insn::Field {
        owner, name, desc, ..
    } = &field_insn_node
    else {
        unreachable!()
    };
    assert_eq!(interner.resolve(*owner), "owner");
    assert_eq!(interner.resolve(*name), "name");
    assert_eq!(interner.resolve(*desc), "I");
}

#[test]
fn test_set_opcode() {
    let interner = Interner::new();
    let mut field_insn_node = Insn::field_insn(
        GETSTATIC,
        interner.intern("owner"),
        interner.intern("name"),
        interner.intern("I"),
    );

    field_insn_node.set_opcode(PUTSTATIC);

    assert_eq!(field_insn_node.opcode(), Some(PUTSTATIC));
}
