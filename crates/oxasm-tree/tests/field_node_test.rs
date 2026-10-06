//! Port of ASM's `FieldNodeTest`. `testConstructor_illegalState` is dropped: there is no
//! subclassing, nor `api` (ADR-09).

use oxasm_tree::FieldNode;

#[test]
fn test_constructor() {
    let field_node = FieldNode::new(123, "field", "I", None, None);

    assert_eq!(field_node.access, 123);
    assert_eq!(field_node.name, "field");
    assert_eq!(field_node.desc, "I");
}
