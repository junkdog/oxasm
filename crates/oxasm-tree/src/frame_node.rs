//! [`FrameNode`]: ASM's `FrameNode`.

use std::alloc::{self, Layout};
use std::fmt;
use std::ptr::NonNull;

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
/// values of both are in one allocation, with their lengths, and an empty frame has none. A
/// frame node is 12 bytes, aligned to 4, which keeps it in a 16-byte [`Insn`](crate::Insn)
/// without a box (T7 and T13 in oxasm-optimization.md). Their values can be changed in place
/// ([`local_mut`](Self::local_mut), [`stack_mut`](Self::stack_mut)); a frame with other lengths
/// is a new node ([`from_parts`](Self::from_parts)).
#[repr(C, packed(4))]
pub struct FrameNode {
    /// The type of this frame. A chop frame carries the number of removed locals, which ASM
    /// stores as the length of a `local` list of `null`s.
    pub kind: FrameKind,
    /// The frame's [`Header`], followed by the types of the local variables, then those of the
    /// operand stack elements; `None` when there are none.
    values: Option<NonNull<Header>>,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<FrameNode>() == 12 && align_of::<FrameNode>() == 4);

// SAFETY: a frame node owns its allocation, like a `Box<[SymFrameValue]>`, and the values are
// `Send`.
unsafe impl Send for FrameNode {}

// SAFETY: as above: `&FrameNode` only gives out shared slices of the values, which are `Sync`.
unsafe impl Sync for FrameNode {}

/// The lengths at the start of a frame's allocation, aligned so that the values follow it
/// directly.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Header {
    /// The number of values.
    len: u32,
    /// The number of local variable values, at the start of the values.
    local_len: u32,
}

const _: () = assert!(size_of::<Header>().is_multiple_of(align_of::<SymFrameValue>()));

/// The layout of a frame's allocation with `len` values.
fn layout(len: usize) -> Layout {
    let (layout, offset) = Layout::new::<Header>()
        .extend(Layout::array::<SymFrameValue>(len).expect("a frame fits in memory"))
        .expect("a frame fits in memory");
    debug_assert_eq!(offset, size_of::<Header>());
    layout
}

/// The values that follow the given header.
fn values_of(header: NonNull<Header>) -> NonNull<SymFrameValue> {
    // The header's size is a multiple of the values' alignment (asserted above), and the
    // allocation starts with it, so the values start right after it.
    header
        .cast::<u8>()
        .map_addr(|addr| addr.saturating_add(size_of::<Header>()))
        .cast()
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
    /// A frame node with the given values: `len` of them, the first `local_len` of which are
    /// local variables.
    fn alloc(
        kind: FrameKind,
        local_len: usize,
        len: usize,
        values: impl Iterator<Item = SymFrameValue>,
    ) -> FrameNode {
        if len == 0 {
            return FrameNode { kind, values: None };
        }
        assert!(
            local_len <= len,
            "a frame's local values are some of its values"
        );
        let len32 = u32::try_from(len).expect("a frame has fewer than 2^32 values");
        #[allow(clippy::cast_possible_truncation)] // at most `len`
        let header = Header {
            len: len32,
            local_len: local_len as u32,
        };
        let layout = layout(len);
        // SAFETY: the layout's size is not zero: it has a header.
        let raw = unsafe { alloc::alloc(layout) };
        let Some(ptr) = NonNull::new(raw.cast::<Header>()) else {
            alloc::handle_alloc_error(layout)
        };
        // SAFETY: the allocation starts with room for a header, aligned for it.
        unsafe { ptr.write(header) };
        // SAFETY: `len` values fit after the header, aligned for them; the slice is of
        // `MaybeUninit`s, which need no initialization.
        let slots = unsafe {
            std::slice::from_raw_parts_mut(
                values_of(ptr).cast::<std::mem::MaybeUninit<_>>().as_ptr(),
                len,
            )
        };
        let mut written = 0;
        for (slot, value) in slots.iter_mut().zip(values) {
            slot.write(value);
            written += 1;
        }
        // Leaks the allocation on failure, but reads no uninitialized value.
        assert_eq!(written, len, "a frame's values are as many as it counts");
        FrameNode {
            kind,
            values: Some(ptr),
        }
    }

    /// The values (the local variables, then the stack), and the number of local variables.
    fn parts(&self) -> (&[SymFrameValue], usize) {
        let Some(ptr) = self.values else {
            return (&[], 0);
        };
        // SAFETY: `alloc` initialized the header, which lives as long as `self`.
        let Header { len, local_len } = unsafe { ptr.read() };
        // SAFETY: `alloc` initialized `len` values after the header, which live as long as
        // `self`; nothing changes them while `self` is borrowed.
        let values = unsafe { std::slice::from_raw_parts(values_of(ptr).as_ptr(), len as usize) };
        (values, local_len as usize)
    }

    /// [`parts`](Self::parts), to change the values in place.
    fn parts_mut(&mut self) -> (&mut [SymFrameValue], usize) {
        let Some(ptr) = self.values else {
            return (&mut [], 0);
        };
        // SAFETY: as in `parts`.
        let Header { len, local_len } = unsafe { ptr.read() };
        // SAFETY: as in `parts`, and `&mut self` makes the access unique.
        let values =
            unsafe { std::slice::from_raw_parts_mut(values_of(ptr).as_ptr(), len as usize) };
        (values, local_len as usize)
    }

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
        FrameNode::alloc(
            kind,
            local.len(),
            local.len() + stack.len(),
            local.iter().chain(stack).copied(),
        )
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
        FrameNode::alloc(kind, local.len(), local.len() + stack.len(), values)
    }

    /// The types of the local variables of this stack map frame: for full and new frames, all
    /// of them; for append frames, the appended ones; empty otherwise. Long and double values
    /// are represented by a single element.
    pub fn local(&self) -> &[SymFrameValue] {
        let (values, local_len) = self.parts();
        &values[..local_len]
    }

    /// The types of the operand stack elements of this stack map frame: for full and new
    /// frames, all of them; for same locals 1 stack item frames, that item; empty otherwise.
    pub fn stack(&self) -> &[SymFrameValue] {
        let (values, local_len) = self.parts();
        &values[local_len..]
    }

    /// The types of the local variables of this stack map frame, to change in place.
    pub fn local_mut(&mut self) -> &mut [SymFrameValue] {
        let (values, local_len) = self.parts_mut();
        &mut values[..local_len]
    }

    /// The types of the operand stack elements of this stack map frame, to change in place.
    pub fn stack_mut(&mut self) -> &mut [SymFrameValue] {
        let (values, local_len) = self.parts_mut();
        &mut values[local_len..]
    }

    /// The types of the local variables, then those of the operand stack elements.
    pub(crate) fn values_mut(&mut self) -> &mut [SymFrameValue] {
        self.parts_mut().0
    }

    /// Whether this frame owns heap memory: an empty frame has none.
    pub(crate) fn owns_memory(&self) -> bool {
        let values = self.values;
        values.is_some()
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

impl Drop for FrameNode {
    fn drop(&mut self) {
        if let Some(ptr) = self.values {
            // SAFETY: `alloc` initialized the header.
            let len = unsafe { ptr.read() }.len as usize;
            // SAFETY: `alloc` allocated the pointer with this layout, and this is its only free.
            unsafe { alloc::dealloc(ptr.as_ptr().cast(), layout(len)) };
        }
    }
}

impl Clone for FrameNode {
    fn clone(&self) -> FrameNode {
        let (values, local_len) = self.parts();
        FrameNode::alloc(self.kind, local_len, values.len(), values.iter().copied())
    }
}

impl PartialEq for FrameNode {
    fn eq(&self, other: &FrameNode) -> bool {
        let (kind, other_kind) = (self.kind, other.kind);
        kind == other_kind && self.parts() == other.parts()
    }
}

impl Eq for FrameNode {}

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

#[cfg(test)]
mod tests {
    use oxasm::{Interner, Label};

    use super::*;

    fn values(interner: &Interner, n: usize) -> Vec<SymFrameValue> {
        (0..n)
            .map(|i| match i % 4 {
                0 => SymFrameValue::Integer,
                1 => SymFrameValue::Object(interner.intern(&format!("C{i}"))),
                2 => SymFrameValue::Uninitialized(Label::new()),
                _ => SymFrameValue::Long,
            })
            .collect()
    }

    #[test]
    fn empty_frames_allocate_nothing() {
        let frame = FrameNode::new(FrameKind::Same, &[], &[]);
        assert!(!frame.owns_memory());
        assert!(frame.local().is_empty() && frame.stack().is_empty());
        let chop = FrameNode::from_parts(FrameKind::Chop(2), &[], &[]);
        assert_eq!(chop.clone(), chop);
        assert_ne!(chop, frame);
    }

    #[test]
    fn values_round_trip_and_change_in_place() {
        let interner = Interner::new();
        for (locals, stack) in [(0, 1), (1, 0), (3, 2), (17, 9)] {
            let local = values(&interner, locals);
            let stack_values = values(&interner, stack);
            let mut frame = FrameNode::from_parts(FrameKind::Full, &local, &stack_values);
            assert_eq!(frame.owns_memory(), locals + stack > 0);
            assert_eq!(frame.local(), local);
            assert_eq!(frame.stack(), stack_values);
            let clone = frame.clone();
            assert_eq!(clone, frame);
            for value in frame.values_mut() {
                *value = SymFrameValue::Top;
            }
            assert!(
                frame
                    .local()
                    .iter()
                    .chain(frame.stack())
                    .all(|v| *v == SymFrameValue::Top)
            );
            assert_eq!(clone.local(), local);
            if locals + stack > 0 {
                assert_ne!(clone, frame);
            }
            drop(clone);
            assert_eq!(frame.local().len(), locals);
        }
    }

    #[test]
    fn frames_unaligned_in_a_vector() {
        // A `FrameNode` is 12 bytes: every other frame of a vector is at an address that isn't a
        // multiple of 8.
        let interner = Interner::new();
        let local = values(&interner, 5);
        let frames: Vec<FrameNode> = (0..7)
            .map(|i| FrameNode::from_parts(FrameKind::New, &local[..i % 6], &local[..1]))
            .collect();
        let copies = frames.clone();
        drop(frames);
        for (i, frame) in copies.iter().enumerate() {
            assert_eq!(frame.local(), &local[..i % 6]);
            assert_eq!(frame.stack(), &local[..1]);
        }
    }

    #[test]
    fn frames_move_to_another_thread() {
        let interner = Interner::new();
        let local = values(&interner, 3);
        let frame = FrameNode::from_parts(FrameKind::Append, &local, &[]);
        let back = std::thread::spawn(move || frame).join().unwrap();
        assert_eq!(back.local(), local);
    }
}
