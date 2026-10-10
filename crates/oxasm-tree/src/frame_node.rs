//! [`FrameNode`]: ASM's `FrameNode`.

use std::fmt;

use oxasm::constant::FrameValue;
use oxasm::{FrameKind, Interner, MethodVisitor};

use crate::method_node::plain;
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
///
/// ASM's `local` and `stack` lists are [`local`](Self::local) and [`stack`](Self::stack): the
/// values of both are in one allocation, and an empty frame has none. This keeps a frame node in
/// an [`Insn`](crate::Insn) without a box (T7 in oxasm-optimization.md). Their values can be
/// changed in place ([`local_mut`](Self::local_mut), [`stack_mut`](Self::stack_mut)); a frame
/// with other lengths is a new node ([`from_parts`](Self::from_parts)).
#[derive(Clone, PartialEq, Eq)]
pub struct FrameNode {
    /// The type of this frame. A chop frame carries the number of removed locals, which ASM
    /// stores as the length of a `local` list of `null`s.
    pub kind: FrameKind,
    /// The number of local variable values, at the start of `values`.
    local_len: u32,
    /// The types of the local variables, then those of the operand stack elements.
    values: Box<[SymFrameValue]>,
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

/// The number of local variable values of a frame, as stored.
fn local_len(local: &[impl Sized]) -> u32 {
    u32::try_from(local.len()).expect("a frame has fewer than 2^32 local variables")
}

impl FrameNode {
    /// Constructs a new frame node, keeping of `local` and `stack` what the frame kind uses, as
    /// ASM does (one stack item for `F_SAME1`, nothing for `F_SAME` and `F_CHOP`, ...).
    pub fn new(kind: FrameKind, local: &[SymFrameValue], stack: &[SymFrameValue]) -> FrameNode {
        let (local, stack) = used(kind, local, stack);
        FrameNode::from_parts(kind, local, stack)
    }

    /// Constructs a frame node with the given values as they are, whatever the frame kind uses
    /// (what setting the fields of ASM's `FrameNode` does). [`accept`](Self::accept) visits only
    /// what the kind uses.
    pub fn from_parts(
        kind: FrameKind,
        local: &[SymFrameValue],
        stack: &[SymFrameValue],
    ) -> FrameNode {
        FrameNode {
            kind,
            local_len: local_len(local),
            values: local.iter().chain(stack).copied().collect(),
        }
    }

    /// A frame node for a visited frame, whose uninitialized values have plain labels (see
    /// [`MethodNode`](crate::MethodNode)).
    pub(crate) fn from_visited(
        kind: FrameKind,
        local: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
        strings: &mut Strings<'_>,
    ) -> FrameNode {
        let (local, stack) = used(kind, local, stack);
        let values = local.iter().chain(stack).map(|value| {
            match SymFrameValue::from_visited(value, strings) {
                SymFrameValue::Uninitialized(label) => SymFrameValue::Uninitialized(plain(label)),
                value => value,
            }
        });
        FrameNode {
            kind,
            local_len: local_len(local),
            values: values.collect(),
        }
    }

    /// The types of the local variables of this stack map frame: for full and new frames, all
    /// of them; for append frames, the appended ones; empty otherwise. Long and double values
    /// are represented by a single element.
    pub fn local(&self) -> &[SymFrameValue] {
        &self.values[..self.local_len as usize]
    }

    /// The types of the operand stack elements of this stack map frame: for full and new
    /// frames, all of them; for same locals 1 stack item frames, that item; empty otherwise.
    pub fn stack(&self) -> &[SymFrameValue] {
        &self.values[self.local_len as usize..]
    }

    /// The types of the local variables of this stack map frame, to change in place.
    pub fn local_mut(&mut self) -> &mut [SymFrameValue] {
        &mut self.values[..self.local_len as usize]
    }

    /// The types of the operand stack elements of this stack map frame, to change in place.
    pub fn stack_mut(&mut self) -> &mut [SymFrameValue] {
        &mut self.values[self.local_len as usize..]
    }

    /// The types of the local variables, then those of the operand stack elements.
    pub(crate) fn values_mut(&mut self) -> &mut [SymFrameValue] {
        &mut self.values
    }

    /// Whether this frame owns heap memory: an empty frame has none.
    pub(crate) fn owns_memory(&self) -> bool {
        !self.values.is_empty()
    }

    /// Makes the given visitor visit this frame, with the content its kind uses.
    pub fn accept(&self, interner: &Interner, method_visitor: &mut dyn MethodVisitor) {
        self.accept_with(interner, method_visitor, &mut Vec::new());
    }

    /// [`accept`](Self::accept), with the values resolved into the given buffer, which a list
    /// reuses for all its frames.
    pub(crate) fn accept_with<'i>(
        &self,
        interner: &'i Interner,
        method_visitor: &mut dyn MethodVisitor,
        buffer: &mut Vec<FrameValue<'i>>,
    ) {
        let (local, stack) = used(self.kind, self.local(), self.stack());
        buffer.clear();
        buffer.extend(
            local
                .iter()
                .chain(stack)
                .map(|value| value.resolve(interner)),
        );
        let (local, stack) = buffer.split_at(local.len());
        method_visitor.visit_frame(self.kind, local, stack);
    }
}

/// ASM's fields: the kind, `local` and `stack`.
impl fmt::Debug for FrameNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameNode")
            .field("kind", &self.kind)
            .field("local", &self.local())
            .field("stack", &self.stack())
            .finish()
    }
}
