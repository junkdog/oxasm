//! [`InsnList`]: ASM's `InsnList`, a doubly linked list of instructions, here backed by an arena
//! with generational handles ([`InsnId`]) instead of node identity (ADR-11).

use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::ManuallyDrop;
use std::ops::{Index, IndexMut};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use oxasm::constant::FrameValue;
use oxasm::{Interner, Label, MethodVisitor};
use rustc_hash::FxHashMap;

use crate::insn::{InsnNode, InsnTypeAnnotations};

/// The "no slot" value of the slot links.
const NIL: u32 = u32::MAX;

/// The [`Slot::next_owner`] of a slot that is not in the chain of owners.
const UNCHAINED: u32 = u32::MAX - 1;

/// The source of list ids: each list (and each [`InsnList::clear`]) takes the next one, so that
/// the handles of a list never designate a node of another list. It is the start of the next
/// block of [`LIST_ID_BLOCK`] ids, and wraps after 2^32 ids, which only matters for a handle kept
/// across that many lists.
static NEXT_LIST_ID: AtomicU32 = AtomicU32::new(0);

/// The number of list ids a thread takes from [`NEXT_LIST_ID`] at once, so that threads making
/// trees in parallel don't all write its cache line (like label ids).
const LIST_ID_BLOCK: u32 = 1 << 8;

thread_local! {
    /// The next list id of this thread's block, and the number of ids left in it.
    static LIST_IDS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

/// The id of a new list (or of a cleared one).
fn next_list_id() -> u32 {
    LIST_IDS.with(|ids| {
        let (next, left) = ids.get();
        if left > 0 {
            ids.set((next.wrapping_add(1), left - 1));
            next
        } else {
            let start = NEXT_LIST_ID.fetch_add(LIST_ID_BLOCK, Ordering::Relaxed);
            ids.set((start.wrapping_add(1), LIST_ID_BLOCK - 1));
            start
        }
    })
}

/// A handle to an instruction of an [`InsnList`] (what ASM does with the identity of an
/// `AbstractInsnNode`).
///
/// A handle stays valid while its instruction is in the list, whatever happens to the other
/// instructions. Once the instruction is removed (or replaced, or moved to another list), the
/// handle is *stale*: the list answers `None` for it, or panics for operations that need an
/// instruction of the list. It never designates another instruction, of this list or another.
#[derive(Clone, Copy, Eq, PartialOrd, Ord)]
pub struct InsnId {
    list: u32,
    slot: u32,
    generation: u32,
}

impl PartialEq for InsnId {
    /// One test of the three fields, without a branch each: an analysis compares its values'
    /// producers by the million.
    #[inline]
    fn eq(&self, other: &InsnId) -> bool {
        ((self.list ^ other.list) | (self.slot ^ other.slot) | (self.generation ^ other.generation))
            == 0
    }
}

impl Hash for InsnId {
    /// The derived hash: the three fields, in order (consistent with `eq`, which compares them).
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.list.hash(state);
        self.slot.hash(state);
        self.generation.hash(state);
    }
}

impl fmt::Debug for InsnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InsnId({}:{}#{})", self.list, self.slot, self.generation)
    }
}

/// An arena slot: a node and its links, or a free slot.
#[derive(Clone, Debug)]
struct Slot {
    /// Incremented each time the slot is freed, so that handles to its former node are stale.
    generation: u32,
    previous: u32,
    next: u32,
    /// The next slot in the chain of the slots that may hold a node owning memory (`NIL` at
    /// its end), or `UNCHAINED`. It fits in the slot's padding.
    next_owner: u32,
    node: Option<InsnNode>,
}

// The chain of owners costs no memory: `next_owner` fills what was the slot's padding.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Slot>() == 40);

/// The positions of the instructions, built on demand (ASM's `cache`, and the `index` field of
/// the nodes).
#[derive(Clone, Debug)]
struct Positions {
    /// The slot of each instruction, in list order.
    slots: Vec<u32>,
    /// The position of the instruction of each slot (garbage for free slots).
    of_slot: Vec<u32>,
}

/// A doubly linked list of instructions (ASM's `InsnList`).
///
/// Instructions are added by value and designated by [`InsnId`] handles, which every operation
/// takes in place of a node, in O(1). Like ASM, [`get`](Self::get) and
/// [`index_of`](Self::index_of) build a cache of the instruction positions on first use, which
/// every structural change invalidates; [`label_position`](Self::label_position) does the same
/// for the labels.
///
/// ASM's precondition that an instruction belongs to at most one list holds by construction: a
/// list owns its nodes. Its other preconditions are checked: a handle that is not an instruction
/// of this list makes accessors return `None`, and mutators panic (where ASM corrupts the list,
/// or its `CheckedInsnList` throws `NoSuchElementException`).
pub struct InsnList {
    /// The id in the handles of this list.
    id: u32,
    /// The arena. Its slots have no drop glue: dropping the list drops only the nodes that own
    /// memory (see the `Drop` impl).
    slots: Vec<ManuallyDrop<Slot>>,
    /// The first slot in the chain of the slots that may hold a node owning memory, or `NIL`:
    /// every slot that was given such a node, or lent out mutably. A slot stays in the chain
    /// once in it, whatever it holds later.
    owners: u32,
    /// The free slots.
    free: Vec<u32>,
    first: u32,
    last: u32,
    len: usize,
    positions: OnceLock<Positions>,
    labels: OnceLock<FxHashMap<Label, InsnId>>,
    /// The type annotations of the instructions that have some, by slot. Few instructions have
    /// any, so they are not in the nodes.
    annotations: FxHashMap<u32, InsnTypeAnnotations>,
}

impl InsnList {
    /// Constructs an empty list.
    pub fn new() -> InsnList {
        InsnList {
            id: next_list_id(),
            slots: Vec::new(),
            owners: NIL,
            free: Vec::new(),
            first: NIL,
            last: NIL,
            len: 0,
            positions: OnceLock::new(),
            labels: OnceLock::new(),
            annotations: FxHashMap::default(),
        }
    }

    /// Reserves capacity for at least `additional` more instructions (an oxasm extension, as
    /// for a `Vec`).
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
    }

    /// The number of instructions in this list (ASM's `size()`).
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether this list is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The first instruction in this list, or `None` if the list is empty.
    pub fn first(&self) -> Option<InsnId> {
        self.id_of(self.first)
    }

    /// The last instruction in this list, or `None` if the list is empty.
    pub fn last(&self) -> Option<InsnId> {
        self.id_of(self.last)
    }

    /// The instruction at the given index, or `None` if `index >= len()` (where ASM throws
    /// `IndexOutOfBoundsException`). Builds the position cache, if needed: once it is built,
    /// this runs in constant time.
    pub fn get(&self, index: usize) -> Option<&InsnNode> {
        self.id_at(index).map(|id| &self[id])
    }

    /// The handle of the instruction at the given index; see [`get`](Self::get).
    pub fn id_at(&self, index: usize) -> Option<InsnId> {
        if index >= self.len {
            return None;
        }
        self.id_of(self.positions().slots[index])
    }

    /// Whether the given handle designates an instruction of this list. Unlike ASM's, this
    /// does not scan the list.
    pub fn contains(&self, id: InsnId) -> bool {
        self.slot_of(id).is_some()
    }

    /// The index of the given instruction in this list, or `None` if it is not an instruction of
    /// this list (for which ASM's result is undefined). Builds the position cache, if needed:
    /// once it is built, this runs in constant time.
    pub fn index_of(&self, id: InsnId) -> Option<usize> {
        let slot = self.slot_of(id)?;
        Some(self.positions().of_slot[slot as usize] as usize)
    }

    /// The instruction designated by the given handle, or `None` if it is not an instruction of
    /// this list.
    pub fn node(&self, id: InsnId) -> Option<&InsnNode> {
        let slot = self.slot_of(id)?;
        self.slots[slot as usize].node.as_ref()
    }

    /// The instruction designated by the given handle, or `None` if it is not an instruction of
    /// this list. This invalidates the label cache, since the instruction may change.
    pub fn node_mut(&mut self, id: InsnId) -> Option<&mut InsnNode> {
        let slot = self.slot_of(id)?;
        self.labels.take();
        // The caller may store a node that owns memory.
        self.chain_owner(slot);
        self.slots[slot as usize].node.as_mut()
    }

    /// The instruction after the given one, or `None` if it is the last one or not an
    /// instruction of this list (ASM's `getNext()`).
    pub fn next(&self, id: InsnId) -> Option<InsnId> {
        let slot = self.slot_of(id)?;
        self.id_of(self.slots[slot as usize].next)
    }

    /// The instruction before the given one, or `None` if it is the first one or not an
    /// instruction of this list (ASM's `getPrevious()`).
    pub fn previous(&self, id: InsnId) -> Option<InsnId> {
        let slot = self.slot_of(id)?;
        self.id_of(self.slots[slot as usize].previous)
    }

    /// The label node of the given label, i.e. the first [`Insn::Label`](crate::Insn::Label)
    /// instruction with this label, if any. Builds a cache of the label nodes, if needed: once
    /// it is built, this runs in constant time.
    pub fn label_position(&self, label: Label) -> Option<InsnId> {
        self.labels
            .get_or_init(|| {
                let mut labels = FxHashMap::default();
                for (id, node) in self.iter_with_ids() {
                    if let Some(label) = node.insn.as_label() {
                        labels.entry(label).or_insert(id);
                    }
                }
                labels
            })
            .get(&label)
            .copied()
    }

    /// The index of the label node of the given label; see
    /// [`label_position`](Self::label_position).
    pub fn label_index(&self, label: Label) -> Option<usize> {
        self.index_of(self.label_position(label)?)
    }

    /// An iterator over the instructions of this list, in order.
    pub fn iter(&self) -> Iter<'_> {
        Iter { ids: self.ids() }
    }

    /// An iterator over the handles of the instructions of this list, in order (ASM's
    /// `toArray()`, once collected).
    pub fn ids(&self) -> Ids<'_> {
        Ids {
            list: self,
            front: self.first,
            back: self.last,
            remaining: self.len,
        }
    }

    /// An iterator over the handles and instructions of this list, in order.
    pub fn iter_with_ids(
        &self,
    ) -> impl DoubleEndedIterator<Item = (InsnId, &InsnNode)> + ExactSizeIterator {
        IterWithIds { ids: self.ids() }
    }

    /// Makes the given visitor visit all the instructions in this list, with the strings of the
    /// given interner (that of the method that holds the list).
    pub fn accept(&self, interner: &Interner, method_visitor: &mut dyn MethodVisitor) {
        // The values of the frames, resolved, for one allocation per list instead of one per
        // frame.
        let mut frame_values = Vec::new();
        let mut current = self.first;
        while current != NIL {
            self.accept_slot(current, interner, method_visitor, &mut frame_values);
            current = self.slots[current as usize].next;
        }
    }

    /// Makes the given visitor visit the given instruction, then its type annotations if it is
    /// a real instruction (ASM's `AbstractInsnNode.accept`), with the strings of the given
    /// interner. [`Insn::accept`](crate::Insn::accept) visits the instruction alone.
    ///
    /// # Panics
    ///
    /// If `id` is not an instruction of this list.
    pub fn accept_insn(
        &self,
        id: InsnId,
        interner: &Interner,
        method_visitor: &mut dyn MethodVisitor,
    ) {
        self.accept_slot(
            self.expect_slot(id),
            interner,
            method_visitor,
            &mut Vec::new(),
        );
    }

    /// [`accept_insn`](Self::accept_insn) for the instruction in the given (linked) slot.
    fn accept_slot<'i>(
        &self,
        slot: u32,
        interner: &'i Interner,
        method_visitor: &mut dyn MethodVisitor,
        frame_values: &mut Vec<FrameValue<'i>>,
    ) {
        let node = self.slots[slot as usize]
            .node
            .as_ref()
            .expect("a linked slot holds a node");
        node.insn
            .accept_with(interner, method_visitor, frame_values);
        if !self.annotations.is_empty()
            && node.opcode().is_some()
            && let Some(annotations) = self.annotations.get(&slot)
        {
            annotations.accept(interner, method_visitor);
        }
    }

    /// The type annotations of the given instruction, if it has some (ASM's
    /// `visibleTypeAnnotations` and `invisibleTypeAnnotations` of the node).
    pub fn type_annotations(&self, id: InsnId) -> Option<&InsnTypeAnnotations> {
        let slot = self.slot_of(id)?;
        self.annotations.get(&slot)
    }

    /// The type annotations of the given instruction, which are created (empty) if it has none.
    /// They are dropped with the instruction, when it is removed or replaced; moving a list into
    /// another keeps them.
    ///
    /// # Panics
    ///
    /// If `id` is not an instruction of this list.
    pub fn type_annotations_mut(&mut self, id: InsnId) -> &mut InsnTypeAnnotations {
        let slot = self.expect_slot(id);
        self.annotations.entry(slot).or_default()
    }

    /// Replaces an instruction of this list with another instruction. Returns the handle of the
    /// new instruction, and the replaced one (whose handle is now stale).
    ///
    /// # Panics
    ///
    /// If `old` is not an instruction of this list.
    pub fn set(&mut self, old: InsnId, node: impl Into<InsnNode>) -> (InsnId, InsnNode) {
        let old_slot = self.expect_slot(old);
        let (previous, next) = {
            let slot = &self.slots[old_slot as usize];
            (slot.previous, slot.next)
        };
        self.unlink(old_slot);
        let old_node = self.free(old_slot);
        let new_slot = self.alloc(node.into());
        self.link(new_slot, previous, next);
        // Like ASM, keep the position cache, if any: only one instruction changed.
        if let Some(positions) = self.positions.get_mut() {
            let index = positions.of_slot[old_slot as usize];
            positions.slots[index as usize] = new_slot;
            if positions.of_slot.len() <= new_slot as usize {
                positions.of_slot.resize(new_slot as usize + 1, 0);
            }
            positions.of_slot[new_slot as usize] = index;
        }
        self.labels.take();
        (self.id_of(new_slot).expect("just linked"), old_node)
    }

    /// Adds the given instruction to the end of this list, and returns its handle.
    ///
    /// A list being built (room reserved, no free slot, no cache built) takes a fast path: a new
    /// slot pushed and linked after the last one (T11 in oxasm-optimization.md). Anything else
    /// takes the general path, out of line.
    #[inline(always)]
    pub fn add(&mut self, node: impl Into<InsnNode>) -> InsnId {
        let node = node.into();
        // The caches' tests come first: they are atomic loads, which the slots' length and
        // capacity would otherwise be loaded again after, keeping `push`'s growth path.
        if self.positions.get().is_none()
            && self.labels.get().is_none()
            && self.free.is_empty()
            && self.slots.len() < self.slots.capacity()
            && self.slots.len() < UNCHAINED as usize
        {
            self.push(node)
        } else {
            self.add_slow(node)
        }
    }

    /// Appends the given node in a new slot, which the caller checked is within the capacity and
    /// below `UNCHAINED`, when no cache is built: there is nothing to invalidate, and the new
    /// slot's generation is 0. The slot joins the chain of owners if the node owns memory: a test
    /// that the inlined call sites of [`add`](Self::add) resolve at compile time, since they know
    /// the node's variant.
    #[inline(always)]
    fn push(&mut self, node: InsnNode) -> InsnId {
        #[allow(clippy::cast_possible_truncation)] // below `UNCHAINED`
        let slot = self.slots.len() as u32;
        let last = self.last;
        let next_owner = if node.insn.owns_memory() {
            std::mem::replace(&mut self.owners, slot)
        } else {
            UNCHAINED
        };
        self.slots.push(ManuallyDrop::new(Slot {
            generation: 0,
            previous: last,
            next: NIL,
            next_owner,
            node: Some(node),
        }));
        // `NIL` is out of bounds, so the bounds check is the test for an empty list.
        match self.slots.get_mut(last as usize) {
            Some(previous) => previous.next = slot,
            None => self.first = slot,
        }
        self.last = slot;
        self.len += 1;
        InsnId {
            list: self.id,
            slot,
            generation: 0,
        }
    }

    /// [`add`](Self::add)'s general path.
    #[inline(never)]
    fn add_slow(&mut self, node: InsnNode) -> InsnId {
        let slot = self.alloc(node);
        self.link(slot, self.last, NIL);
        self.invalidate();
        self.id_of(slot).expect("just linked")
    }

    /// Moves the instructions of the given list to the end of this list, leaving it empty.
    /// Returns the handles of the first and last moved instructions in this list, or `None` if
    /// `other` was empty. The handles of `other` are stale afterwards.
    pub fn add_list(&mut self, other: &mut InsnList) -> Option<(InsnId, InsnId)> {
        self.move_between(other, self.last, NIL)
    }

    /// Inserts the given instruction at the beginning of this list, and returns its handle.
    pub fn insert(&mut self, node: impl Into<InsnNode>) -> InsnId {
        let slot = self.alloc(node.into());
        self.link(slot, NIL, self.first);
        self.invalidate();
        self.id_of(slot).expect("just linked")
    }

    /// Moves the instructions of the given list to the beginning of this list, leaving it
    /// empty; see [`add_list`](Self::add_list).
    pub fn insert_list(&mut self, other: &mut InsnList) -> Option<(InsnId, InsnId)> {
        self.move_between(other, NIL, self.first)
    }

    /// Inserts the given instruction after the specified instruction, and returns its handle.
    ///
    /// # Panics
    ///
    /// If `previous` is not an instruction of this list.
    pub fn insert_after(&mut self, previous: InsnId, node: impl Into<InsnNode>) -> InsnId {
        let previous = self.expect_slot(previous);
        let next = self.slots[previous as usize].next;
        let slot = self.alloc(node.into());
        self.link(slot, previous, next);
        self.invalidate();
        self.id_of(slot).expect("just linked")
    }

    /// Moves the instructions of the given list after the specified instruction, leaving it
    /// empty; see [`add_list`](Self::add_list).
    ///
    /// # Panics
    ///
    /// If `previous` is not an instruction of this list.
    pub fn insert_list_after(
        &mut self,
        previous: InsnId,
        other: &mut InsnList,
    ) -> Option<(InsnId, InsnId)> {
        let previous = self.expect_slot(previous);
        let next = self.slots[previous as usize].next;
        self.move_between(other, previous, next)
    }

    /// Inserts the given instruction before the specified instruction, and returns its handle.
    ///
    /// # Panics
    ///
    /// If `next` is not an instruction of this list.
    pub fn insert_before(&mut self, next: InsnId, node: impl Into<InsnNode>) -> InsnId {
        let next = self.expect_slot(next);
        let previous = self.slots[next as usize].previous;
        let slot = self.alloc(node.into());
        self.link(slot, previous, next);
        self.invalidate();
        self.id_of(slot).expect("just linked")
    }

    /// Moves the instructions of the given list before the specified instruction, leaving it
    /// empty; see [`add_list`](Self::add_list).
    ///
    /// # Panics
    ///
    /// If `next` is not an instruction of this list.
    pub fn insert_list_before(
        &mut self,
        next: InsnId,
        other: &mut InsnList,
    ) -> Option<(InsnId, InsnId)> {
        let next = self.expect_slot(next);
        let previous = self.slots[next as usize].previous;
        self.move_between(other, previous, next)
    }

    /// Removes the given instruction from this list, and returns it.
    ///
    /// # Panics
    ///
    /// If `id` is not an instruction of this list.
    pub fn remove(&mut self, id: InsnId) -> InsnNode {
        let slot = self.expect_slot(id);
        self.unlink(slot);
        self.invalidate();
        self.free(slot)
    }

    /// Removes all the instructions of this list. Their handles are stale afterwards.
    pub fn clear(&mut self) {
        // A new id makes the handles of the removed instructions stale, whatever their slots
        // hold later.
        *self = InsnList::new();
    }

    /// A cursor over this list (ASM's `ListIterator`), before the instruction at the given
    /// index (after the last one if `index == len()`).
    ///
    /// # Panics
    ///
    /// If `index > len()` (where ASM throws `IndexOutOfBoundsException`).
    pub fn cursor(&mut self, index: usize) -> InsnCursor<'_> {
        assert!(
            index <= self.len,
            "cursor index {index} out of bounds for length {}",
            self.len
        );
        let (next, previous) = if index == self.len {
            (NIL, self.last)
        } else {
            let mut current = self.first;
            for _ in 0..index {
                current = self.slots[current as usize].next;
            }
            (current, self.slots[current as usize].previous)
        };
        InsnCursor {
            list: self,
            next,
            previous,
            last_returned: NIL,
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Arena management
    // ---------------------------------------------------------------------------------------------

    /// The handle of the node in the given slot, or `None` for `NIL`.
    #[inline]
    fn id_of(&self, slot: u32) -> Option<InsnId> {
        (slot != NIL).then(|| InsnId {
            list: self.id,
            slot,
            generation: self.slots[slot as usize].generation,
        })
    }

    /// The slot of the given handle, if it designates an instruction of this list.
    fn slot_of(&self, id: InsnId) -> Option<u32> {
        let slot = self.slots.get(id.slot as usize)?;
        (id.list == self.id && slot.generation == id.generation && slot.node.is_some())
            .then_some(id.slot)
    }

    /// The slot of the given handle.
    #[track_caller]
    fn expect_slot(&self, id: InsnId) -> u32 {
        self.slot_of(id)
            .unwrap_or_else(|| panic!("{id:?} is not an instruction of this list"))
    }

    /// Stores the given node in a free slot, and returns the slot (not linked yet). The slot
    /// joins the chain of owners if the node owns memory.
    #[inline(always)]
    fn alloc(&mut self, node: InsnNode) -> u32 {
        let owns_memory = node.insn.owns_memory();
        if let Some(slot) = self.free.pop() {
            self.slots[slot as usize].node = Some(node);
            if owns_memory {
                self.chain_owner(slot);
            }
            slot
        } else {
            let slot = u32::try_from(self.slots.len())
                .ok()
                .filter(|&slot| slot < UNCHAINED)
                .expect("too many instructions in an InsnList");
            let next_owner = if owns_memory {
                std::mem::replace(&mut self.owners, slot)
            } else {
                UNCHAINED
            };
            self.slots.push(ManuallyDrop::new(Slot {
                generation: 0,
                previous: NIL,
                next: NIL,
                next_owner,
                node: Some(node),
            }));
            slot
        }
    }

    /// Adds the given slot to the chain of owners, if it is not in it yet.
    #[inline]
    fn chain_owner(&mut self, slot: u32) {
        let entry = &mut self.slots[slot as usize];
        if entry.next_owner == UNCHAINED {
            entry.next_owner = self.owners;
            self.owners = slot;
        }
    }

    /// Frees the given (unlinked) slot, and returns its node.
    fn free(&mut self, slot: u32) -> InsnNode {
        if !self.annotations.is_empty() {
            self.annotations.remove(&slot);
        }
        let entry = &mut self.slots[slot as usize];
        entry.generation = entry.generation.wrapping_add(1);
        self.free.push(slot);
        entry.node.take().expect("freeing an occupied slot")
    }

    /// Links the given slot between `previous` and `next` (either may be `NIL`), which must be
    /// adjacent.
    #[inline(always)]
    fn link(&mut self, slot: u32, previous: u32, next: u32) {
        let entry = &mut self.slots[slot as usize];
        entry.previous = previous;
        entry.next = next;
        if previous == NIL {
            self.first = slot;
        } else {
            self.slots[previous as usize].next = slot;
        }
        if next == NIL {
            self.last = slot;
        } else {
            self.slots[next as usize].previous = slot;
        }
        self.len += 1;
    }

    /// Unlinks the given slot.
    fn unlink(&mut self, slot: u32) {
        let Slot { previous, next, .. } = *self.slots[slot as usize];
        if previous == NIL {
            self.first = next;
        } else {
            self.slots[previous as usize].next = next;
        }
        if next == NIL {
            self.last = previous;
        } else {
            self.slots[next as usize].previous = previous;
        }
        let entry = &mut self.slots[slot as usize];
        entry.previous = NIL;
        entry.next = NIL;
        self.len -= 1;
    }

    /// Moves the nodes of `other`, in order, between the adjacent `previous` and `next` slots of
    /// this list, and clears `other`.
    fn move_between(
        &mut self,
        other: &mut InsnList,
        mut previous: u32,
        next: u32,
    ) -> Option<(InsnId, InsnId)> {
        if other.is_empty() {
            return None;
        }
        let mut first = NIL;
        let mut current = other.first;
        while current != NIL {
            let entry = &mut other.slots[current as usize];
            let other_slot = current;
            current = entry.next;
            let node = entry.node.take().expect("a linked slot holds a node");
            let slot = self.alloc(node);
            self.link(slot, previous, next);
            if let Some(annotations) = other.annotations.remove(&other_slot) {
                self.annotations.insert(slot, annotations);
            }
            if first == NIL {
                first = slot;
            }
            previous = slot;
        }
        other.clear();
        self.invalidate();
        Some((
            self.id_of(first).expect("just linked"),
            self.id_of(previous).expect("just linked"),
        ))
    }

    /// Invalidates the caches, after a structural change.
    #[inline]
    fn invalidate(&mut self) {
        self.positions.take();
        self.labels.take();
    }

    /// The position cache, built if needed.
    fn positions(&self) -> &Positions {
        self.positions.get_or_init(|| {
            let mut slots = Vec::with_capacity(self.len);
            let mut of_slot = vec![0; self.slots.len()];
            let mut current = self.first;
            while current != NIL {
                of_slot[current as usize] =
                    u32::try_from(slots.len()).expect("fewer than 2^32 instructions");
                slots.push(current);
                current = self.slots[current as usize].next;
            }
            Positions { slots, of_slot }
        })
    }

    /// The position of the instruction in the given (linked) slot.
    fn position_of_slot(&self, slot: u32) -> usize {
        self.positions().of_slot[slot as usize] as usize
    }
}

impl Default for InsnList {
    fn default() -> InsnList {
        InsnList::new()
    }
}

/// Drops the nodes that own memory. Most instructions own none, and the slots are
/// `ManuallyDrop`, so only the slots in the chain of owners are visited, instead of a call to the
/// drop glue of each slot (T3 in oxasm-optimization.md), or a test of each slot's variant (T9).
impl Drop for InsnList {
    fn drop(&mut self) {
        let mut slot = self.owners;
        while slot != NIL {
            let entry = &mut self.slots[slot as usize];
            slot = entry.next_owner;
            entry.node = None;
        }
    }
}

/// A copy of the list, with new handles (the handles of the original list do not designate the
/// instructions of the copy).
impl Clone for InsnList {
    fn clone(&self) -> InsnList {
        InsnList {
            id: next_list_id(),
            slots: self.slots.clone(),
            owners: self.owners,
            free: self.free.clone(),
            first: self.first,
            last: self.last,
            len: self.len,
            positions: self.positions.clone(),
            labels: OnceLock::new(),
            annotations: self.annotations.clone(),
        }
    }
}

/// Lists are equal if they hold equal instructions, with equal type annotations, in the same
/// order.
impl PartialEq for InsnList {
    fn eq(&self, other: &InsnList) -> bool {
        self.len == other.len
            && self.ids().zip(other.ids()).all(|(id, other_id)| {
                self[id] == other[other_id]
                    && self.type_annotations(id) == other.type_annotations(other_id)
            })
    }
}

impl fmt::Debug for InsnList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Index<InsnId> for InsnList {
    type Output = InsnNode;

    /// # Panics
    ///
    /// If `id` is not an instruction of this list.
    #[track_caller]
    fn index(&self, id: InsnId) -> &InsnNode {
        self.node(id)
            .unwrap_or_else(|| panic!("{id:?} is not an instruction of this list"))
    }
}

impl IndexMut<InsnId> for InsnList {
    /// # Panics
    ///
    /// If `id` is not an instruction of this list.
    #[track_caller]
    fn index_mut(&mut self, id: InsnId) -> &mut InsnNode {
        self.node_mut(id)
            .unwrap_or_else(|| panic!("{id:?} is not an instruction of this list"))
    }
}

impl<'a> IntoIterator for &'a InsnList {
    type Item = &'a InsnNode;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// An iterator over the handles of the instructions of an [`InsnList`], in order.
#[derive(Clone, Debug)]
pub struct Ids<'a> {
    list: &'a InsnList,
    front: u32,
    back: u32,
    remaining: usize,
}

impl Iterator for Ids<'_> {
    type Item = InsnId;

    fn next(&mut self) -> Option<InsnId> {
        if self.remaining == 0 {
            return None;
        }
        let id = self.list.id_of(self.front);
        self.front = self.list.slots[self.front as usize].next;
        self.remaining -= 1;
        id
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for Ids<'_> {
    fn next_back(&mut self) -> Option<InsnId> {
        if self.remaining == 0 {
            return None;
        }
        let id = self.list.id_of(self.back);
        self.back = self.list.slots[self.back as usize].previous;
        self.remaining -= 1;
        id
    }
}

impl ExactSizeIterator for Ids<'_> {}

/// An iterator over the instructions of an [`InsnList`], in order.
#[derive(Clone, Debug)]
pub struct Iter<'a> {
    ids: Ids<'a>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a InsnNode;

    fn next(&mut self) -> Option<&'a InsnNode> {
        let slot = self.ids.front;
        self.ids.next()?;
        self.ids.list.slots[slot as usize].node.as_ref()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ids.size_hint()
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let slot = self.ids.back;
        self.ids.next_back()?;
        self.ids.list.slots[slot as usize].node.as_ref()
    }
}

impl ExactSizeIterator for Iter<'_> {}

/// An iterator over the handles and instructions of an [`InsnList`], in order: an [`Iter`] that
/// also yields the handles (the node of a linked slot needs no lookup by handle).
#[derive(Clone, Debug)]
struct IterWithIds<'a> {
    ids: Ids<'a>,
}

impl<'a> Iterator for IterWithIds<'a> {
    type Item = (InsnId, &'a InsnNode);

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.ids.front;
        let id = self.ids.next()?;
        Some((id, self.ids.list.slots[slot as usize].node.as_ref()?))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ids.size_hint()
    }
}

impl DoubleEndedIterator for IterWithIds<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let slot = self.ids.back;
        let id = self.ids.next_back()?;
        Some((id, self.ids.list.slots[slot as usize].node.as_ref()?))
    }
}

impl ExactSizeIterator for IterWithIds<'_> {}

/// A cursor over an [`InsnList`] that can modify it as it moves: ASM's `ListIterator` over an
/// `InsnList`, i.e. a position between two instructions, and the last instruction returned by
/// [`next`](Self::next) or [`previous`](Self::previous), which [`remove`](Self::remove) and
/// [`set`](Self::set) act upon.
#[derive(Debug)]
pub struct InsnCursor<'a> {
    list: &'a mut InsnList,
    next: u32,
    previous: u32,
    /// The last instruction returned by `next` or `previous`, if not removed since (ASM's
    /// `remove` field).
    last_returned: u32,
}

impl InsnCursor<'_> {
    /// The list.
    pub fn list(&self) -> &InsnList {
        self.list
    }

    /// Whether there is an instruction after the cursor.
    pub fn has_next(&self) -> bool {
        self.next != NIL
    }

    /// Moves the cursor after the next instruction, and returns it; `None` at the end of the
    /// list (where ASM throws `NoSuchElementException`).
    #[allow(clippy::should_implement_trait)] // it moves both ways and edits, it is not an Iterator
    pub fn next(&mut self) -> Option<InsnId> {
        if self.next == NIL {
            return None;
        }
        let result = self.next;
        self.previous = result;
        self.next = self.list.slots[result as usize].next;
        self.last_returned = result;
        self.list.id_of(result)
    }

    /// Whether there is an instruction before the cursor.
    pub fn has_previous(&self) -> bool {
        self.previous != NIL
    }

    /// Moves the cursor before the previous instruction, and returns it; `None` at the start of
    /// the list (where ASM throws `NoSuchElementException`).
    pub fn previous(&mut self) -> Option<InsnId> {
        if self.previous == NIL {
            return None;
        }
        let result = self.previous;
        self.next = result;
        self.previous = self.list.slots[result as usize].previous;
        self.last_returned = result;
        self.list.id_of(result)
    }

    /// The index of the instruction that [`next`](Self::next) would return, or the list length
    /// at the end of the list.
    pub fn next_index(&self) -> usize {
        if self.next == NIL {
            self.list.len()
        } else {
            self.list.position_of_slot(self.next)
        }
    }

    /// The index of the instruction that [`previous`](Self::previous) would return, or `None` at
    /// the start of the list (where ASM returns -1).
    pub fn previous_index(&self) -> Option<usize> {
        (self.previous != NIL).then(|| self.list.position_of_slot(self.previous))
    }

    /// Removes the last instruction returned by [`next`](Self::next) or
    /// [`previous`](Self::previous) from the list, and returns it.
    ///
    /// # Panics
    ///
    /// If there is no such instruction, or it was removed or replaced since (where ASM throws
    /// `IllegalStateException`).
    pub fn remove(&mut self) -> InsnNode {
        let removed = self.last_returned;
        assert!(
            removed != NIL,
            "no instruction to remove: call next or previous first"
        );
        if removed == self.next {
            self.next = self.list.slots[removed as usize].next;
        } else {
            self.previous = self.list.slots[removed as usize].previous;
        }
        self.last_returned = NIL;
        let id = self.list.id_of(removed).expect("a linked slot");
        self.list.remove(id)
    }

    /// Inserts the given instruction at the cursor position, i.e. before the instruction
    /// [`next`](Self::next) would return, and after the one [`previous`](Self::previous) would
    /// return; a subsequent `previous` returns it. Returns its handle.
    pub fn add(&mut self, node: impl Into<InsnNode>) -> InsnId {
        let id = if let Some(next) = self.list.id_of(self.next) {
            self.list.insert_before(next, node)
        } else if let Some(previous) = self.list.id_of(self.previous) {
            self.list.insert_after(previous, node)
        } else {
            self.list.add(node)
        };
        self.previous = id.slot;
        self.last_returned = NIL;
        id
    }

    /// Replaces the last instruction returned by [`next`](Self::next) or
    /// [`previous`](Self::previous) with the given one. Returns the handle of the new
    /// instruction, and the replaced one; a subsequent `remove` or `set` acts on the new one.
    ///
    /// # Panics
    ///
    /// If there is no such instruction, or it was removed since (where ASM throws
    /// `IllegalStateException`).
    pub fn set(&mut self, node: impl Into<InsnNode>) -> (InsnId, InsnNode) {
        let replaced = self.last_returned;
        assert!(
            replaced != NIL,
            "no instruction to replace: call next or previous first"
        );
        let id = self.list.id_of(replaced).expect("a linked slot");
        let (new_id, old_node) = self.list.set(id, node);
        if replaced == self.previous {
            self.previous = new_id.slot;
        } else {
            self.next = new_id.slot;
        }
        self.last_returned = new_id.slot;
        (new_id, old_node)
    }
}
