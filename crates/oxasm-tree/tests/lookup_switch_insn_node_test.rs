//! Port of ASM's `LookupSwitchInsnNodeTest`.

use oxasm::Label;
use oxasm::opcodes::LOOKUPSWITCH;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let dflt = Label::new();
    let keys = vec![1];
    let labels = vec![Label::new()];

    let lookup_switch_insn_node = Insn::LookupSwitch {
        dflt,
        keys: keys.clone(),
        labels: labels.clone(),
    };

    assert_eq!(lookup_switch_insn_node.kind(), InsnKind::LookupSwitchInsn);
    assert_eq!(lookup_switch_insn_node.opcode(), Some(LOOKUPSWITCH));
    let Insn::LookupSwitch {
        dflt: actual_dflt,
        keys: actual_keys,
        labels: actual_labels,
    } = &lookup_switch_insn_node
    else {
        unreachable!()
    };
    assert_eq!(*actual_dflt, dflt);
    assert_eq!(*actual_keys, [1]);
    assert_eq!(*actual_labels, labels);
}
