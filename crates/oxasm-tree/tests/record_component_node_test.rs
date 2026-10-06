//! Port of ASM's `RecordComponentNodeTest`. `testConstructor_illegalState` is dropped: there is
//! no subclassing, nor `api` (ADR-09).

use oxasm_tree::RecordComponentNode;

#[test]
fn test_constructor() {
    let component_node = RecordComponentNode::new("component", "I", None);

    assert_eq!(component_node.name, "component");
    assert_eq!(component_node.descriptor, "I");
}
