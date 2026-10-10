//! Port of ASM's `FrameNodeTest`. `testConstructor_illegalArgument` (an unknown frame type) is
//! dropped: `FrameKind` cannot represent one (ADR-09).

use oxasm::constant::FrameValue;
use oxasm::{FrameKind, Interner, Label, MethodVisitor};
use oxasm_tree::{FrameNode, Insn, InsnKind, InsnList, InsnNode, SymFrameValue};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let object = |s: &str| SymFrameValue::Object(interner.intern(s));
    let locals = [object("l")];
    let stack = [object("s"), object("t")];

    let frame_node = FrameNode::new(FrameKind::Full, &locals, &stack);

    assert_eq!(InsnNode::from(frame_node.clone()).kind(), InsnKind::Frame);
    assert_eq!(frame_node.kind, FrameKind::Full);
    assert_eq!(frame_node.local(), locals);
    assert_eq!(frame_node.stack(), stack);
}

/// oxasm-specific: a frame node keeps what its kind uses, as ASM's constructor does, and
/// replays it.
#[test]
fn constructor_keeps_what_the_kind_uses() {
    let values = [SymFrameValue::Integer, SymFrameValue::Float];
    let frame = |kind| FrameNode::new(kind, &values, &values);
    assert_eq!(frame(FrameKind::Append).local(), values);
    assert!(frame(FrameKind::Append).stack().is_empty());
    assert!(frame(FrameKind::Chop(2)).local().is_empty());
    assert!(frame(FrameKind::Same).local().is_empty() && frame(FrameKind::Same).stack().is_empty());
    assert_eq!(frame(FrameKind::Same1).stack(), [SymFrameValue::Integer]);
    assert!(frame(FrameKind::Same1).local().is_empty());
    assert_eq!(Insn::frame(frame(FrameKind::Same)).opcode(), None);
}

/// oxasm-specific: `from_parts` keeps the values as they are (ASM's field assignments), which
/// `accept` trims to what the kind uses; the values can change in place.
#[test]
fn from_parts_keeps_the_values_and_they_change_in_place() {
    let interner = Interner::new();
    let values = [SymFrameValue::Integer, SymFrameValue::Float];
    let mut frame = FrameNode::from_parts(FrameKind::Same1, &values, &values);
    assert_eq!(frame.local(), values);
    assert_eq!(frame.stack(), values);
    assert_ne!(frame, FrameNode::new(FrameKind::Same1, &values, &values));

    frame.local_mut()[1] = SymFrameValue::Long;
    frame.stack_mut()[0] = SymFrameValue::Object(interner.intern("A"));
    assert_eq!(frame.local(), [SymFrameValue::Integer, SymFrameValue::Long]);
    assert_eq!(
        frame.stack(),
        [
            SymFrameValue::Object(interner.intern("A")),
            SymFrameValue::Float
        ]
    );

    let mut visitor = FrameRecorder::default();
    frame.accept(&interner, &mut visitor);
    assert_eq!(
        visitor.frames,
        [(FrameKind::Same1, vec![], vec![FrameValue::from("A")])]
    );
}

/// oxasm-specific: the frames of a list are visited through one buffer (T7 in
/// oxasm-optimization.md), whatever their sizes and order.
#[test]
fn a_list_visits_frames_of_every_size() {
    let interner = Interner::new();
    let object = |s: &str| SymFrameValue::Object(interner.intern(s));
    let label = Label::new();
    let mut insn_list = InsnList::new();
    let frames = [
        FrameNode::new(
            FrameKind::Full,
            &[object("A"), SymFrameValue::Long, object("B")],
            &[SymFrameValue::Uninitialized(label)],
        ),
        FrameNode::new(FrameKind::Same, &[], &[]),
        FrameNode::new(FrameKind::Same1, &[], &[SymFrameValue::Integer]),
        FrameNode::from_parts(FrameKind::Chop(2), &[object("C")], &[object("D")]),
        FrameNode::new(FrameKind::Append, &[object("E")], &[]),
    ];
    for frame in &frames {
        insn_list.add(Insn::frame(frame.clone()));
        insn_list.add(Insn::Simple { opcode: 0 });
    }

    let mut visitor = FrameRecorder::default();
    insn_list.accept(&interner, &mut visitor);
    let object = FrameValue::from;
    assert_eq!(
        visitor.frames,
        [
            (
                FrameKind::Full,
                vec![object("A"), FrameValue::Long, object("B")],
                vec![FrameValue::Uninitialized(label)]
            ),
            (FrameKind::Same, vec![], vec![]),
            (FrameKind::Same1, vec![], vec![FrameValue::Integer]),
            (FrameKind::Chop(2), vec![], vec![]),
            (FrameKind::Append, vec![object("E")], vec![]),
        ]
    );
    assert_eq!(visitor.insns, frames.len());
}

/// Records the visited frames, and counts the other instructions.
#[derive(Default)]
struct FrameRecorder {
    frames: Vec<(
        FrameKind,
        Vec<FrameValue<'static>>,
        Vec<FrameValue<'static>>,
    )>,
    insns: usize,
}

impl MethodVisitor for FrameRecorder {
    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        let owned = |values: &[FrameValue<'_>]| values.iter().map(FrameValue::to_owned).collect();
        self.frames.push((kind, owned(local), owned(stack)));
    }

    fn visit_insn(&mut self, _opcode: u8) {
        self.insns += 1;
    }
}
