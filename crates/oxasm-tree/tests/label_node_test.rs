//! Port of ASM's `LabelNodeTest`. A `LabelNode` is an `Insn::Label`, whose label is given
//! (ASM's `LabelNode()` creates one lazily, `LabelNode(Label)` takes one).

use oxasm::Label;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let label = Label::new();
    let label_node1 = Insn::Label(Label::new());
    let label_node2 = Insn::Label(label);

    assert_eq!(label_node1.kind(), InsnKind::Label);
    assert!(label_node1.as_label().is_some());
    assert_eq!(label_node2.as_label(), Some(label));
    assert_eq!(label_node2.opcode(), None);
}
