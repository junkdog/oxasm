//! [`FrameNode`]: ASM's `FrameNode`.

use oxasm::constant::FrameValue;
use oxasm::{FrameKind, MethodVisitor};

/// A node that represents a stack map frame. These nodes are pseudo instructions that can be
/// inserted in an instruction list. In fact, these nodes must (\*) be inserted *just before* any
/// instruction node *i* that follows an unconditional branch instruction such as `GOTO` or
/// `THROW`, that is the target of a jump instruction, or that starts an exception handler block.
/// The stack map frame types must describe the values of the local variables and of the operand
/// stack elements *just before* *i* is executed.
///
/// (\*) this is mandatory only for classes whose version is greater than or equal to `V1_6`.
///
/// Uninitialized values are [`FrameValue::Uninitialized`] with the label of the `NEW`
/// instruction, which ASM stores as a `LabelNode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameNode {
    /// The type of this frame. A chop frame carries the number of removed locals, which ASM
    /// stores as the length of a `local` list of `null`s.
    pub kind: FrameKind,
    /// The types of the local variables of this stack map frame: for full and new frames, all
    /// of them; for append frames, the appended ones; empty otherwise. Long and double values
    /// are represented by a single element.
    pub local: Vec<FrameValue<'static>>,
    /// The types of the operand stack elements of this stack map frame: for full and new
    /// frames, all of them; for same locals 1 stack item frames, that item; empty otherwise.
    pub stack: Vec<FrameValue<'static>>,
}

impl FrameNode {
    /// Constructs a new frame node, keeping of `local` and `stack` what the frame kind uses, as
    /// ASM does (one stack item for `F_SAME1`, nothing for `F_SAME` and `F_CHOP`, ...).
    ///
    /// ASM throws on its internal `F_INSERT` frame type, which a class reader can only produce
    /// from ASM's pseudo instructions in malformed input; here it is kept, without content.
    pub fn new(kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) -> FrameNode {
        let owned = |values: &[FrameValue<'_>]| values.iter().map(FrameValue::to_owned).collect();
        let (local, stack) = match kind {
            FrameKind::New | FrameKind::Full => (owned(local), owned(stack)),
            FrameKind::Append => (owned(local), Vec::new()),
            FrameKind::Chop(_) | FrameKind::Same | FrameKind::Insert => (Vec::new(), Vec::new()),
            FrameKind::Same1 => (Vec::new(), owned(&stack[..stack.len().min(1)])),
        };
        FrameNode { kind, local, stack }
    }

    /// Makes the given visitor visit this frame, with the content its kind uses.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        match self.kind {
            FrameKind::New | FrameKind::Full => {
                method_visitor.visit_frame(self.kind, &self.local, &self.stack);
            }
            FrameKind::Append => method_visitor.visit_frame(self.kind, &self.local, &[]),
            FrameKind::Chop(_) | FrameKind::Same | FrameKind::Insert => {
                method_visitor.visit_frame(self.kind, &[], &[]);
            }
            FrameKind::Same1 => method_visitor.visit_frame(self.kind, &[], &self.stack),
        }
    }
}
