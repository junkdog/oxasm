//! Port of ASM's `TypeAnnotationNodeTest`. `testConstructor_illegalState` is dropped: there is
//! no subclassing, nor `api` (ADR-09).

use oxasm::{Interner, TypePath, TypeReference};
use oxasm_tree::TypeAnnotationNode;

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let type_path = TypePath::from_string("[").unwrap();
    let type_annotation_node = TypeAnnotationNode::new(
        TypeReference::new(123),
        type_path.clone(),
        interner.intern("LI;"),
    );

    assert_eq!(type_annotation_node.type_ref.get_value(), 123);
    assert_eq!(type_annotation_node.type_path, type_path);
    assert_eq!(interner.resolve(type_annotation_node.desc()), "LI;");
}
