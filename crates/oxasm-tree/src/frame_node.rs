//! [`FrameNode`]: ASM's `FrameNode`.

use oxasm::constant::FrameValue;
use oxasm::{FrameKind, Interner, MethodVisitor};

use crate::strings::Strings;
use crate::value::SymFrameValue;

/// A node that represents a stack map frame. These nodes are pseudo instructions that can be
/// inserted in an instruction list. In fact, these nodes must (\*) be inserted *just before* any
/// instruction node *i* that follows an unconditional branch instruction such as `GOTO` or
/// `THROW`, that is the target of a jump instruction, or that starts an exception handler block.
/// The stack map frame types must describe the values of the local variables and of the operand
/// stack elements *just before* *i* is executed.
///
/// (\*) this is mandatory only for classes whose version is greater than or equal to `V1_6`.
///
/// Uninitialized values are [`SymFrameValue::Uninitialized`] with the label of the `NEW`
/// instruction, which ASM stores as a `LabelNode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameNode {
    /// The type of this frame. A chop frame carries the number of removed locals, which ASM
    /// stores as the length of a `local` list of `null`s.
    pub kind: FrameKind,
    /// The types of the local variables of this stack map frame: for full and new frames, all
    /// of them; for append frames, the appended ones; empty otherwise. Long and double values
    /// are represented by a single element.
    pub local: Vec<SymFrameValue>,
    /// The types of the operand stack elements of this stack map frame: for full and new
    /// frames, all of them; for same locals 1 stack item frames, that item; empty otherwise.
    pub stack: Vec<SymFrameValue>,
}

/// The parts of `local` and `stack` that a frame of the given kind uses, as ASM keeps them (one
/// stack item for `F_SAME1`, nothing for `F_SAME` and `F_CHOP`, ...).
///
/// ASM throws on its internal `F_INSERT` frame type, which a class reader can only produce from
/// ASM's pseudo instructions in malformed input; here it is kept, without content.
fn used<'a, T>(kind: FrameKind, local: &'a [T], stack: &'a [T]) -> (&'a [T], &'a [T]) {
    match kind {
        FrameKind::New | FrameKind::Full => (local, stack),
        FrameKind::Append => (local, &[]),
        FrameKind::Chop(_) | FrameKind::Same | FrameKind::Insert => (&[], &[]),
        FrameKind::Same1 => (&[], &stack[..stack.len().min(1)]),
    }
}

impl FrameNode {
    /// Constructs a new frame node, keeping of `local` and `stack` what the frame kind uses, as
    /// ASM does (one stack item for `F_SAME1`, nothing for `F_SAME` and `F_CHOP`, ...).
    pub fn new(kind: FrameKind, local: &[SymFrameValue], stack: &[SymFrameValue]) -> FrameNode {
        let (local, stack) = used(kind, local, stack);
        FrameNode {
            kind,
            local: local.to_vec(),
            stack: stack.to_vec(),
        }
    }

    /// A frame node for a visited frame.
    pub(crate) fn from_visited(
        kind: FrameKind,
        local: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
        strings: &mut Strings<'_>,
    ) -> FrameNode {
        let (local, stack) = used(kind, local, stack);
        let mut intern = |values: &[FrameValue<'_>]| {
            values
                .iter()
                .map(|value| SymFrameValue::from_visited(value, strings))
                .collect()
        };
        FrameNode {
            kind,
            local: intern(local),
            stack: intern(stack),
        }
    }

    /// Makes the given visitor visit this frame, with the content its kind uses.
    pub fn accept(&self, interner: &Interner, method_visitor: &mut dyn MethodVisitor) {
        fn resolve<'i>(interner: &'i Interner, values: &[SymFrameValue]) -> Vec<FrameValue<'i>> {
            values.iter().map(|value| value.resolve(interner)).collect()
        }
        let (local, stack) = used(self.kind, &self.local, &self.stack);
        method_visitor.visit_frame(
            self.kind,
            &resolve(interner, local),
            &resolve(interner, stack),
        );
    }
}
