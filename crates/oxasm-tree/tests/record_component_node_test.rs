//! Port of ASM's `RecordComponentNodeTest`. `testConstructor_illegalState` is dropped: there is
//! no subclassing, nor `api` (ADR-09).

use std::sync::Arc;

use oxasm::Interner;
use oxasm_tree::RecordComponentNode;

#[test]
fn test_constructor() {
    let interner = Arc::new(Interner::new());
    let component_node = RecordComponentNode::new(
        interner.clone(),
        interner.intern("component"),
        interner.intern("I"),
        None,
    );

    assert_eq!(component_node.resolve(component_node.name), "component");
    assert_eq!(component_node.resolve(component_node.descriptor), "I");
}
