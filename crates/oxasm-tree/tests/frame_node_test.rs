//! Port of ASM's `FrameNodeTest`. `testConstructor_illegalArgument` (an unknown frame type) is
//! dropped: `FrameKind` cannot represent one (ADR-09).

use oxasm::{FrameKind, Interner};
use oxasm_tree::{FrameNode, Insn, InsnKind, InsnNode, SymFrameValue};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let object = |s: &str| SymFrameValue::Object(interner.intern(s));
    let locals = [object("l")];
    let stack = [object("s"), object("t")];

    let frame_node = FrameNode::new(FrameKind::Full, &locals, &stack);

    assert_eq!(InsnNode::from(frame_node.clone()).kind(), InsnKind::Frame);
    assert_eq!(frame_node.kind, FrameKind::Full);
    assert_eq!(frame_node.local, locals);
    assert_eq!(frame_node.stack, stack);
}

/// oxasm-specific: a frame node keeps what its kind uses, as ASM's constructor does, and
/// replays it.
#[test]
fn constructor_keeps_what_the_kind_uses() {
    let values = [SymFrameValue::Integer, SymFrameValue::Float];
    let frame = |kind| FrameNode::new(kind, &values, &values);
    assert_eq!(frame(FrameKind::Append).local, values);
    assert!(frame(FrameKind::Append).stack.is_empty());
    assert!(frame(FrameKind::Chop(2)).local.is_empty());
    assert!(frame(FrameKind::Same).local.is_empty() && frame(FrameKind::Same).stack.is_empty());
    assert_eq!(frame(FrameKind::Same1).stack, [SymFrameValue::Integer]);
    assert!(frame(FrameKind::Same1).local.is_empty());
    assert_eq!(Insn::frame(frame(FrameKind::Same)).opcode(), None);
}
