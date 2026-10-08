//! Port of ASM's `FieldNodeTest`. `testConstructor_illegalState` is dropped: there is no
//! subclassing, nor `api` (ADR-09).

use std::sync::Arc;

use oxasm::Interner;
use oxasm_tree::FieldNode;

#[test]
fn test_constructor() {
    let interner = Arc::new(Interner::new());
    let field_node = FieldNode::new(
        interner.clone(),
        123,
        interner.intern("field"),
        interner.intern("I"),
        None,
        None,
    );

    assert_eq!(field_node.access, 123);
    assert_eq!(field_node.resolve(field_node.name), "field");
    assert_eq!(field_node.resolve(field_node.desc), "I");
}
