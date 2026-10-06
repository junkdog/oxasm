//! Port of ASM's `LdcInsnNodeTest`.

use oxasm::constant::Constant;
use oxasm::opcodes::LDC;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let ldc_insn_node = Insn::Ldc(Constant::from("s").into_owned());

    assert_eq!(ldc_insn_node.kind(), InsnKind::LdcInsn);
    assert_eq!(ldc_insn_node.opcode(), Some(LDC));
    assert!(matches!(&ldc_insn_node, Insn::Ldc(cst) if *cst == Constant::from("s")));
}
