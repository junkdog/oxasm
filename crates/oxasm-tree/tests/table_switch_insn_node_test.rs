//! Port of ASM's `TableSwitchInsnNodeTest`.

use oxasm::Label;
use oxasm::opcodes::TABLESWITCH;
use oxasm_tree::{Insn, InsnKind, TableSwitchInsn};

#[test]
fn test_constructor() {
    let dflt = Label::new();
    let labels = vec![Label::new()];

    let table_switch_insn_node = Insn::table_switch_insn(0, 1, dflt, labels.clone());

    assert_eq!(table_switch_insn_node.kind(), InsnKind::TableSwitchInsn);
    assert_eq!(table_switch_insn_node.opcode(), Some(TABLESWITCH));
    assert_eq!(
        table_switch_insn_node,
        Insn::TableSwitch(Box::new(TableSwitchInsn {
            min: 0,
            max: 1,
            dflt,
            labels
        }))
    );
}
