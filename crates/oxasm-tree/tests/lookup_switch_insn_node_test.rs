//! Port of ASM's `LookupSwitchInsnNodeTest`.

use oxasm::Label;
use oxasm::opcodes::LOOKUPSWITCH;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let dflt = Label::new();
    let keys = vec![1];
    let labels = vec![Label::new()];

    let lookup_switch_insn_node = Insn::lookup_switch_insn(dflt, keys.clone(), labels.clone());

    assert_eq!(lookup_switch_insn_node.kind(), InsnKind::LookupSwitchInsn);
    assert_eq!(lookup_switch_insn_node.opcode(), Some(LOOKUPSWITCH));
    let Insn::LookupSwitch(actual) = &lookup_switch_insn_node else {
        unreachable!()
    };
    assert_eq!(actual.dflt, dflt);
    assert_eq!(actual.keys, [1]);
    assert_eq!(actual.labels, labels);
}
