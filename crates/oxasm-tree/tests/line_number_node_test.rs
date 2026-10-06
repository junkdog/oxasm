//! Port of ASM's `LineNumberNodeTest`.

use oxasm::Label;
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let label_node = Label::new();
    let line_number_node = Insn::Line {
        line: 42,
        start: label_node,
    };

    assert!(matches!(line_number_node, Insn::Line { line: 42, start } if start == label_node));
    assert_eq!(line_number_node.kind(), InsnKind::Line);
    assert_eq!(line_number_node.opcode(), None);
}
