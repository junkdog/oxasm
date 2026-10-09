//! The labels of a method being written (ADR-03): what ASM's `Label` objects hold for the
//! `MethodWriter`, in a side table keyed by [`Label`], and the control flow graph built from them
//! (ASM's `Label`, `Edge` and `Frame` objects, in arenas).
//!
//! ASM's label state lives in the label objects, so it survives from one method to another; here
//! each method writer has its own table. A label that a method writer never saw has the state of
//! a new ASM label: unresolved, at bytecode offset 0.
//!
//! ASM's object graph (a label's frame, shared by the labels at the same bytecode offset; the
//! linked lists of basic blocks, of outgoing edges and of blocks to process) is kept as is, with
//! indices instead of references: [`BlockId`]s into the blocks, [`FrameId`]s into the frames, and
//! edge indices.

use rustc_hash::FxHashMap;

use crate::label::Label;
use crate::writer::byte_vector::ByteVector;
use crate::writer::frame::{Frame, FrameMut, FrameValues};

/// The index of a label's state (a basic block, if the label starts one) in a [`LabelTable`].
pub(crate) type BlockId = u32;
/// The index of a frame (ASM's `Label.frame` object) in a [`LabelTable`].
pub(crate) type FrameId = u32;
/// The index of an edge of the control flow graph in a [`LabelTable`].
pub(crate) type EdgeId = u32;

/// A flag indicating that a label is only used for debug attributes. Such a label is not the
/// start of a basic block, the target of a jump instruction, or an exception handler. (The flag is
/// the label's own, [`Label::is_debug_only`]; this constant documents ASM's value.)
#[allow(dead_code)] // as in ASM
pub(crate) const FLAG_DEBUG_ONLY: u16 = 1;
/// A flag indicating that a label is the target of a jump instruction, or the start of an
/// exception handler.
pub(crate) const FLAG_JUMP_TARGET: u16 = 2;
/// A flag indicating that the bytecode offset of a label is known.
pub(crate) const FLAG_RESOLVED: u16 = 4;
/// A flag indicating that a label corresponds to a reachable basic block.
pub(crate) const FLAG_REACHABLE: u16 = 8;
/// A flag indicating that the basic block corresponding to a label ends with a subroutine call.
/// By construction in the method writer's `visit_jump_insn`, labels with this flag set have at
/// least two outgoing edges:
///
/// - the first one corresponds to the instruction that follows the jsr instruction in the
///   bytecode, i.e. where execution continues when it returns from the jsr call. This is a
///   virtual control flow edge, since execution never goes directly from the jsr to the next
///   instruction. Instead, it goes to the subroutine and eventually returns to the instruction
///   following the jsr. This virtual edge is used to compute the real outgoing edges of the basic
///   blocks ending with a ret instruction, in [`LabelTable::add_subroutine_ret_successors`].
/// - the second one corresponds to the target of the jsr instruction.
pub(crate) const FLAG_SUBROUTINE_CALLER: u16 = 16;
/// A flag indicating that the basic block corresponding to a label is the start of a subroutine.
pub(crate) const FLAG_SUBROUTINE_START: u16 = 32;
/// A flag indicating that the basic block corresponding to a label is the end of a subroutine.
pub(crate) const FLAG_SUBROUTINE_END: u16 = 64;

/// A sentinel used to indicate the end of a list of labels (ASM's `Label.EMPTY_LIST`).
pub(crate) const EMPTY_LIST: BlockId = BlockId::MAX;

/// A control flow graph edge corresponding to a jump or ret instruction (ASM's `Edge.JUMP`). Only
/// used when computing all the frames.
pub(crate) const EDGE_JUMP: i32 = 0;
/// A control flow graph edge corresponding to an exception handler (ASM's `Edge.EXCEPTION`).
/// Only used when computing the maximum stack size.
pub(crate) const EDGE_EXCEPTION: i32 = 0x7FFF_FFFF;

/// An edge in the control flow graph of a method (ASM's `Edge`). Each node of this graph is a
/// basic block, represented with the label corresponding to its first instruction. Each edge goes
/// from one node to another, i.e. from one basic block to another (called the predecessor and
/// successor blocks, respectively). An edge corresponds either to a jump or ret instruction or to
/// an exception handler.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Edge {
    /// Information about this control flow graph edge:
    ///
    /// - when computing the maximum stack size, either a stack size delta (for an edge
    ///   corresponding to a jump instruction), or [`EDGE_EXCEPTION`] (for an edge corresponding
    ///   to an exception handler). The stack size delta is the stack size just after the jump
    ///   instruction, minus the stack size at the beginning of the predecessor basic block, i.e.
    ///   the one containing the jump instruction.
    /// - when computing all the frames, either [`EDGE_JUMP`] (for an edge corresponding to a jump
    ///   instruction), or the index, in the type table, of the exception type that is handled
    ///   (for an edge corresponding to an exception handler).
    pub(crate) info: i32,
    /// The successor block of this control flow graph edge.
    pub(crate) successor: BlockId,
    /// The next edge in the list of outgoing edges of a basic block.
    pub(crate) next_edge: Option<EdgeId>,
}

/// The type of forward references stored with two bytes in the bytecode. This is the case, for
/// instance, of a forward reference from an ifnull instruction.
const FORWARD_REFERENCE_TYPE_SHORT: i32 = 0x1000_0000;
/// The type of forward references stored in four bytes in the bytecode. This is the case, for
/// instance, of a forward reference from a lookupswitch instruction.
const FORWARD_REFERENCE_TYPE_WIDE: i32 = 0x2000_0000;
/// The type of forward references stored in two bytes in the `stack_map_frame` of a
/// StackMapTable. This is the case, for instance, of a forward reference from an
/// `Uninitialized_variable_info` verification type.
const FORWARD_REFERENCE_TYPE_STACK_MAP: i32 = 0x3000_0000;
/// The bit mask to extract the type of a forward reference.
const FORWARD_REFERENCE_TYPE_MASK: i32 = 0xF000_0000_u32 as i32;
/// The bit mask to extract the 'handle' of a forward reference to a label.
const FORWARD_REFERENCE_HANDLE_MASK: i32 = 0x0FFF_FFFF;

/// What ASM's `Label` holds for the method writer.
#[derive(Clone, Debug)]
pub(crate) struct Block {
    /// The label of this state.
    pub(crate) label: Label,
    /// The type and status of this label or basic block (the `FLAG_*` values; the debug only
    /// flag is the label's own).
    pub(crate) flags: u16,
    /// The offset of this label in the bytecode of its method, in bytes. This value is set if
    /// and only if the `FLAG_RESOLVED` flag is set.
    pub(crate) bytecode_offset: i32,
    /// The last forward reference to this label in [`LabelTable::forward_references`], linked
    /// to the previous ones, or [`NO_REFERENCE`].
    last_forward_reference: u32,

    // Fields for the control flow and data flow graph analysis algorithms (used to compute the
    // maximum stack size or the stack map frames). A control flow graph contains one node per
    // "basic block", and one edge per "jump" from one basic block to another. Each node (i.e.,
    // each basic block) is represented with the label that corresponds to its first
    // instruction. Each node also stores the list of its successors in the graph, as a linked
    // list of edges.
    //
    // The control flow analysis algorithms used to compute the maximum stack size or the stack
    // map frames are similar and use two steps. The first step, during the visit of each
    // instruction, builds information about the state of the local variables and the operand
    // stack at the end of each basic block, called the "output frame", relatively to the frame
    // state at the beginning of the basic block, which is called the "input frame", and which is
    // unknown during this step. The second step, in the method writer's `compute_all_frames` and
    // `compute_max_stack_and_local`, is a fix point algorithm that computes information about
    // the input frame of each basic block, from the input state of the first basic block (known
    // from the method signature), and by the using the previously computed relative output
    // frames.
    //
    // The algorithm used to compute the maximum stack size only computes the relative output and
    // absolute input stack heights, while the algorithm used to compute stack map frames
    // computes relative output frames and absolute input frames.
    /// The number of elements in the input stack of the basic block corresponding to this label.
    /// This field is computed in `compute_max_stack_and_local`.
    pub(crate) input_stack_size: i16,
    /// The number of elements in the output stack, at the end of the basic block corresponding
    /// to this label. This field is only computed for basic blocks that end with a RET
    /// instruction.
    pub(crate) output_stack_size: i16,
    /// The maximum height reached by the output stack, relatively to the top of the input stack,
    /// in the basic block corresponding to this label (when computing the maximum stack size;
    /// when computing frames, the frame holds it). This maximum is always positive or null.
    pub(crate) output_stack_max: i16,
    /// The id of the subroutine to which this basic block belongs, or 0. If the basic block
    /// belongs to several subroutines, this is the id of the "oldest" subroutine that contains
    /// it (with the convention that a subroutine calling another one is "older" than the
    /// callee). This field is computed in `compute_max_stack_and_local`, if the method contains
    /// JSR instructions.
    pub(crate) subroutine_id: i16,
    /// The input and output stack map frames of the basic block corresponding to this label
    /// (ASM's `Label.frame`), shared by the labels at the same bytecode offset when computing all
    /// the frames. Only used when computing all the frames or the inserted frames.
    pub(crate) frame: Option<FrameId>,
    /// The successor of this label, in the order they are visited in `visit_label`. This linked
    /// list does not include labels used for debug info only. When computing all the frames or
    /// the inserted frames, it does not contain either successive labels that denote the same
    /// bytecode offset (in this case only the first label appears in this list).
    pub(crate) next_basic_block: Option<BlockId>,
    /// The outgoing edges of the basic block corresponding to this label, in the control flow
    /// graph of its method. These edges are stored in a linked list, linked to each other by
    /// their `next_edge` field.
    pub(crate) outgoing_edges: Option<EdgeId>,
    /// The next element in the list of labels to which this label belongs, or `None` if it does
    /// not belong to any list. All lists of labels must end with the [`EMPTY_LIST`] sentinel, in
    /// order to ensure that this field is `None` if and only if this label does not belong to a
    /// list of labels. Note that there can be several lists of labels at the same time, but that
    /// a label can belong to at most one list at a time (unless some lists share a common tail,
    /// but this is not used in practice).
    ///
    /// Lists of labels are used in `compute_all_frames` and `compute_max_stack_and_local` to
    /// compute stack map frames and the maximum stack size, respectively, as well as in
    /// [`LabelTable::mark_subroutine`] and [`LabelTable::add_subroutine_ret_successors`] to
    /// compute the basic blocks belonging to subroutines and their outgoing edges. Outside of
    /// these methods, this field should be `None` (this property is a precondition and a
    /// postcondition of these methods).
    pub(crate) next_list_element: Option<BlockId>,
}

impl Block {
    /// The state of a new label.
    fn new(label: Label) -> Block {
        Block {
            label,
            flags: 0,
            bytecode_offset: 0,
            last_forward_reference: NO_REFERENCE,
            input_stack_size: 0,
            output_stack_size: 0,
            output_stack_max: 0,
            subroutine_id: 0,
            frame: None,
            next_basic_block: None,
            outgoing_edges: None,
            next_list_element: None,
        }
    }
}

/// A forward reference to a label (an element of ASM's `Label.forwardReferences`).
#[derive(Clone, Copy, Debug)]
struct ForwardReference {
    /// The bytecode offset of the instruction that refers to the label.
    source_insn_bytecode_offset: i32,
    /// The reference type and handle (the `FORWARD_REFERENCE_*` values).
    reference: i32,
    /// The previous forward reference to the same label, or [`NO_REFERENCE`].
    previous: u32,
}

/// The end of a list of [`ForwardReference`]s.
const NO_REFERENCE: u32 = u32::MAX;

/// A free slot of [`LabelTable::slots`].
const NO_BLOCK: BlockId = BlockId::MAX;
/// The number of slots of a [`LabelTable`] that has not reserved any.
const MIN_SLOTS: usize = 16;

/// The labels of a method writer, and the control flow graph of its basic blocks.
#[derive(Clone, Debug, Default)]
pub(crate) struct LabelTable {
    /// The index in `blocks` of the labels seen, direct-mapped on the low bits of their id: a
    /// power of two number of slots, at most half full, [`NO_BLOCK`] where free. The labels of a
    /// method mostly have consecutive ids (a class reader creates them one after the other), so
    /// they rarely share a slot. The block of a slot tells whether it holds the label looked up.
    slots: Vec<BlockId>,
    /// The index in `blocks` of the labels whose slot another label took first.
    overflow: FxHashMap<Label, BlockId>,
    /// The state of each label seen.
    blocks: Vec<Block>,
    /// The forward references of all the labels, each label's linked from its last one (one
    /// list for the method, where ASM has an array per label: a block owns no memory).
    forward_references: Vec<ForwardReference>,
    /// The frames of the basic blocks (ASM's `Frame` objects that labels refer to).
    frames: Vec<Frame>,
    /// The arrays of the frames.
    frame_values: FrameValues,
    /// The edges of the control flow graph.
    edges: Vec<Edge>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Java's int arithmetic
impl LabelTable {
    /// Reserves room for `additional` more labels.
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.blocks.reserve(additional);
        // About one reference per label (most labels are jump targets).
        self.forward_references.reserve(additional);
        self.reserve_slots(self.blocks.len().saturating_add(additional));
    }

    /// Whether this table holds memory (a table that saw no label holds none).
    pub(crate) fn has_memory(&self) -> bool {
        self.slots.capacity() > 0 || self.blocks.capacity() > 0
    }

    /// Removes every label, keeping the memory, for another method: a table as new, but for
    /// its capacity.
    pub(crate) fn clear(&mut self) {
        // Free only the slots the labels took: the table may be much larger than the method.
        let mask = self.slots.len().wrapping_sub(1);
        for block in &self.blocks {
            self.slots[block.label.id() as usize & mask] = NO_BLOCK;
        }
        self.overflow.clear();
        self.blocks.clear();
        self.forward_references.clear();
        self.frames.clear();
        self.frame_values.clear();
        self.edges.clear();
    }

    /// Makes `slots` at least twice as large as the given number of labels, and maps the labels
    /// seen again if it grows.
    fn reserve_slots(&mut self, labels: usize) {
        let slots = labels
            .saturating_mul(2)
            .max(MIN_SLOTS)
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX / 2 + 1);
        if slots <= self.slots.len() {
            return;
        }
        self.slots.clear();
        self.slots.resize(slots, NO_BLOCK);
        self.overflow.clear();
        let mask = slots - 1;
        for (id, block) in self.blocks.iter().enumerate() {
            let slot = &mut self.slots[block.label.id() as usize & mask];
            if *slot == NO_BLOCK {
                *slot = id as BlockId;
            } else {
                self.overflow.insert(block.label, id as BlockId);
            }
        }
    }

    /// The slot of the given label (`slots` must not be empty).
    #[inline]
    fn slot(&self, label: Label) -> usize {
        label.id() as usize & (self.slots.len() - 1)
    }

    /// The index of the state of the given label, created if needed.
    pub(crate) fn id(&mut self, label: Label) -> BlockId {
        if let Some(id) = self.find(label) {
            return id;
        }
        let id = self.blocks.len() as BlockId;
        if self.blocks.len() >= self.slots.len() / 2 {
            self.reserve_slots(self.blocks.len() + 1);
        }
        let slot = self.slot(label);
        if self.slots[slot] == NO_BLOCK {
            self.slots[slot] = id;
        } else {
            self.overflow.insert(label, id);
        }
        self.blocks.push(Block::new(label));
        id
    }

    /// The state of the given label, created if needed.
    pub(crate) fn block_mut(&mut self, label: Label) -> &mut Block {
        let id = self.id(label);
        &mut self.blocks[id as usize]
    }

    /// The index of the state of the given label, if seen.
    #[inline]
    pub(crate) fn find(&self, label: Label) -> Option<BlockId> {
        if self.slots.is_empty() {
            return None;
        }
        match self.slots[self.slot(label)] {
            // A label goes to `overflow` only when its slot is taken, and slots stay taken.
            NO_BLOCK => None,
            id if self.blocks[id as usize].label == label => Some(id),
            _ if self.overflow.is_empty() => None,
            _ => self.overflow.get(&label).copied(),
        }
    }

    /// The state of the given label, if seen.
    pub(crate) fn block(&self, label: Label) -> Option<&Block> {
        self.find(label).map(|i| &self.blocks[i as usize])
    }

    /// The state of the label of the given index.
    pub(crate) fn get(&self, id: BlockId) -> &Block {
        &self.blocks[id as usize]
    }

    /// The state of the label of the given index.
    pub(crate) fn get_mut(&mut self, id: BlockId) -> &mut Block {
        &mut self.blocks[id as usize]
    }

    /// Returns the "canonical" block corresponding to the given block's bytecode offset, if
    /// known, otherwise the block itself (ASM's `Label.getCanonicalInstance`). The canonical
    /// instance is the first label (in the order of their visit by `visit_label`) corresponding
    /// to this bytecode offset. It cannot be known for labels which have not been visited yet.
    ///
    /// This method should only be used when computing all the frames: it returns the owner of
    /// the block's frame, if any.
    pub(crate) fn canonical(&self, id: BlockId) -> BlockId {
        match self.get(id).frame {
            Some(frame) => self.frames[frame as usize].owner.unwrap_or(id),
            None => id,
        }
    }

    /// Adds the given frame, and returns its index.
    pub(crate) fn add_frame(&mut self, frame: Frame) -> FrameId {
        self.frames.push(frame);
        (self.frames.len() - 1) as FrameId
    }

    /// The frame of the given index.
    pub(crate) fn frame(&self, id: FrameId) -> &Frame {
        &self.frames[id as usize]
    }

    /// The frame of the given index, with the arrays of the frames.
    pub(crate) fn frame_mut(&mut self, id: FrameId) -> FrameMut<'_> {
        FrameMut {
            frame: &mut self.frames[id as usize],
            values: &mut self.frame_values,
        }
    }

    /// Reserves room for `additional` more values in the arrays of the frames.
    pub(crate) fn reserve_frame_values(&mut self, additional: usize) {
        self.frame_values.reserve(additional);
    }

    /// The arrays of the frames.
    pub(crate) fn frame_values(&self) -> &FrameValues {
        &self.frame_values
    }

    /// Removes the arrays of the frames from the table (until [`Self::put_frame_values`] puts
    /// them back), so that a frame can be updated while the label table is read.
    pub(crate) fn take_frame_values(&mut self) -> FrameValues {
        std::mem::take(&mut self.frame_values)
    }

    /// Puts back the arrays taken with [`Self::take_frame_values`].
    pub(crate) fn put_frame_values(&mut self, values: FrameValues) {
        self.frame_values = values;
    }

    /// The edge of the given index.
    pub(crate) fn edge(&self, id: EdgeId) -> &Edge {
        &self.edges[id as usize]
    }

    /// Adds an edge with the given info and successor at the head of the outgoing edges of the
    /// given block (ASM's `basicBlock.outgoingEdges = new Edge(info, successor,
    /// basicBlock.outgoingEdges)`).
    pub(crate) fn add_outgoing_edge(&mut self, block: BlockId, info: i32, successor: BlockId) {
        let next_edge = self.get(block).outgoing_edges;
        let edge = self.new_edge(info, successor, next_edge);
        self.get_mut(block).outgoing_edges = Some(edge);
    }

    /// Adds a new edge, and returns its index.
    pub(crate) fn new_edge(
        &mut self,
        info: i32,
        successor: BlockId,
        next_edge: Option<EdgeId>,
    ) -> EdgeId {
        self.edges.push(Edge {
            info,
            successor,
            next_edge,
        });
        (self.edges.len() - 1) as EdgeId
    }

    /// Sets the next edge of the given edge.
    pub(crate) fn set_next_edge(&mut self, edge: EdgeId, next_edge: Option<EdgeId>) {
        self.edges[edge as usize].next_edge = next_edge;
    }

    /// The bytecode offset of the given label (ASM's `bytecodeOffset` field: 0 until resolved).
    pub(crate) fn offset(&self, label: Label) -> i32 {
        self.block(label).map_or(0, |b| b.bytecode_offset)
    }

    /// The bytecode offset of the given label, if it is resolved.
    pub(crate) fn resolved_offset(&self, label: Label) -> Option<i32> {
        self.block(label)
            .filter(|b| b.flags & FLAG_RESOLVED != 0)
            .map(|b| b.bytecode_offset)
    }

    /// Adds a forward reference to the label of the block `id`.
    fn add_forward_reference(
        &mut self,
        id: BlockId,
        source_insn_bytecode_offset: i32,
        reference: i32,
    ) {
        let block = &mut self.blocks[id as usize];
        let index = self.forward_references.len() as u32;
        self.forward_references.push(ForwardReference {
            source_insn_bytecode_offset,
            reference,
            previous: block.last_forward_reference,
        });
        block.last_forward_reference = index;
    }

    /// Puts a reference to the given label in the bytecode of a method (ASM's `Label.put(code,
    /// sourceInsnBytecodeOffset, wideReference)`). If the bytecode offset of the label is known,
    /// the relative bytecode offset between the label and the instruction referencing it is
    /// computed and written directly. Otherwise, a null relative offset is written and a new
    /// forward reference is declared for this label.
    pub(crate) fn put(
        &mut self,
        label: Label,
        code: &mut ByteVector,
        source_insn_bytecode_offset: i32,
        wide_reference: bool,
    ) {
        let id = self.id(label);
        let block = &self.blocks[id as usize];
        if block.flags & FLAG_RESOLVED == 0 {
            if wide_reference {
                self.add_forward_reference(
                    id,
                    source_insn_bytecode_offset,
                    FORWARD_REFERENCE_TYPE_WIDE | code.size() as i32,
                );
                code.put_int(-1);
            } else {
                self.add_forward_reference(
                    id,
                    source_insn_bytecode_offset,
                    FORWARD_REFERENCE_TYPE_SHORT | code.size() as i32,
                );
                code.put_short(-1);
            }
        } else if wide_reference {
            code.put_int(
                block
                    .bytecode_offset
                    .wrapping_sub(source_insn_bytecode_offset),
            );
        } else {
            code.put_short(
                block
                    .bytecode_offset
                    .wrapping_sub(source_insn_bytecode_offset),
            );
        }
    }

    /// Puts a reference to the given label in the `stack_map_frame` entries of a method (ASM's
    /// `Label.put(stackMapTableEntries)`). If the bytecode offset of the label is known, it is
    /// written directly. Otherwise, a null relative offset is written and a new forward
    /// reference is declared for this label.
    pub(crate) fn put_stack_map(&mut self, label: Label, stack_map_table_entries: &mut ByteVector) {
        let id = self.id(label);
        if self.blocks[id as usize].flags & FLAG_RESOLVED == 0 {
            self.add_forward_reference(
                id,
                0,
                FORWARD_REFERENCE_TYPE_STACK_MAP | stack_map_table_entries.size() as i32,
            );
        }
        stack_map_table_entries.put_short(self.blocks[id as usize].bytecode_offset);
    }

    /// Sets the bytecode offset of the given label to the given value and resolves the forward
    /// references to this label, if any (ASM's `Label.resolve`). Returns whether a forward
    /// reference needed an ASM specific instruction (a jump too far for a 2 bytes offset).
    pub(crate) fn resolve(
        &mut self,
        label: Label,
        code: &mut ByteVector,
        mut stack_map_table_entries: Option<&mut ByteVector>,
        bytecode_offset: i32,
    ) -> bool {
        let block = self.block_mut(label);
        block.flags |= FLAG_RESOLVED;
        block.bytecode_offset = bytecode_offset;
        let mut has_asm_instructions = false;
        // The references from the last one to the first, as ASM walks its array.
        let mut next = block.last_forward_reference;
        while next != NO_REFERENCE {
            let ForwardReference {
                source_insn_bytecode_offset,
                reference,
                previous,
            } = self.forward_references[next as usize];
            next = previous;
            let relative_offset = bytecode_offset.wrapping_sub(source_insn_bytecode_offset);
            let mut handle = (reference & FORWARD_REFERENCE_HANDLE_MASK) as usize;
            let reference_type = reference & FORWARD_REFERENCE_TYPE_MASK;
            if reference_type == FORWARD_REFERENCE_TYPE_SHORT {
                let code = code.data_mut();
                if relative_offset < i32::from(i16::MIN) || relative_offset > i32::from(i16::MAX) {
                    // Change the opcode of the jump instruction, in order to be able to find it
                    // later in ClassReader.accept (see the EXPAND_ASM_INSNS option). This also
                    // makes the rest of the code invalid, until it is replaced with valid
                    // instructions using a ClassReader -> ClassWriter round trip.
                    let source = source_insn_bytecode_offset as usize;
                    let opcode = code[source];
                    if opcode < crate::opcodes::IFNULL {
                        // Change IFEQ ... JSR to ASM_IFEQ ... ASM_JSR.
                        code[source] = opcode.wrapping_add(crate::constants::ASM_OPCODE_DELTA);
                    } else {
                        // Change IFNULL and IFNONNULL to ASM_IFNULL and ASM_IFNONNULL.
                        code[source] =
                            opcode.wrapping_add(crate::constants::ASM_IFNULL_OPCODE_DELTA);
                    }
                    has_asm_instructions = true;
                }
                code[handle] = (relative_offset >> 8) as u8;
                handle += 1;
                code[handle] = relative_offset as u8;
            } else if reference_type == FORWARD_REFERENCE_TYPE_WIDE {
                let code = code.data_mut();
                code[handle..handle + 4].copy_from_slice(&relative_offset.to_be_bytes());
            } else if let Some(entries) = stack_map_table_entries.as_deref_mut() {
                let data = entries.data_mut();
                data[handle] = (bytecode_offset >> 8) as u8;
                handle += 1;
                data[handle] = bytecode_offset as u8;
            }
        }
        has_asm_instructions
    }

    // ---------------------------------------------------------------------------------------------
    // Methods related to subroutines
    // ---------------------------------------------------------------------------------------------

    /// Finds the basic blocks that belong to the subroutine starting with the given basic block,
    /// and marks these blocks as belonging to this subroutine (ASM's `markSubroutine`). This
    /// method follows the control flow graph to find all the blocks that are reachable from the
    /// given basic block WITHOUT following any jsr target.
    ///
    /// Note: a precondition and postcondition of this method is that all labels must have a
    /// `None` `next_list_element`.
    pub(crate) fn mark_subroutine(&mut self, start: BlockId, subroutine_id: i16) {
        // Data flow algorithm: put this basic block in a list of blocks to process (which are
        // blocks belonging to subroutine subroutine_id) and, while there are blocks to process,
        // remove one from the list, mark it as belonging to the subroutine, and add its successor
        // basic blocks in the control flow graph to the list of blocks to process (if not already
        // done).
        let mut list_of_blocks_to_process = start;
        self.get_mut(start).next_list_element = Some(EMPTY_LIST);
        while list_of_blocks_to_process != EMPTY_LIST {
            // Remove a basic block from the list of blocks to process.
            let basic_block = list_of_blocks_to_process;
            list_of_blocks_to_process = self.take_next_list_element(basic_block);
            // If it is not already marked as belonging to a subroutine, mark it as belonging to
            // subroutine_id and add its successors to the list of blocks to process (unless
            // already done).
            if self.get(basic_block).subroutine_id == 0 {
                self.get_mut(basic_block).subroutine_id = subroutine_id;
                list_of_blocks_to_process =
                    self.push_successors(basic_block, list_of_blocks_to_process);
            }
        }
    }

    /// Finds the basic blocks that end a subroutine starting with the basic block `start` and,
    /// for each one of them, adds an outgoing edge to the basic block following the given
    /// subroutine call (ASM's `addSubroutineRetSuccessors`). In other words, completes the
    /// control flow graph by adding the edges corresponding to the return from this subroutine,
    /// when called from the given caller basic block.
    ///
    /// Note: a precondition and postcondition of this method is that all labels must have a
    /// `None` `next_list_element`.
    ///
    /// `subroutine_caller` is a basic block that ends with a jsr to `start`, which is supposed to
    /// correspond to the start of a subroutine.
    pub(crate) fn add_subroutine_ret_successors(
        &mut self,
        start: BlockId,
        subroutine_caller: BlockId,
    ) {
        // Data flow algorithm: put this basic block in a list blocks to process (which are blocks
        // belonging to a subroutine starting with this label) and, while there are blocks to
        // process, remove one from the list, put it in a list of blocks that have been
        // processed, add a return edge to the successor of subroutine_caller if applicable, and
        // add its successor basic blocks in the control flow graph to the list of blocks to
        // process (if not already done).
        let mut list_of_processed_blocks = EMPTY_LIST;
        let mut list_of_blocks_to_process = start;
        self.get_mut(start).next_list_element = Some(EMPTY_LIST);
        while list_of_blocks_to_process != EMPTY_LIST {
            // Move a basic block from the list of blocks to process to the list of processed
            // blocks.
            let basic_block = list_of_blocks_to_process;
            list_of_blocks_to_process = self.next_list_element(basic_block);
            self.get_mut(basic_block).next_list_element = Some(list_of_processed_blocks);
            list_of_processed_blocks = basic_block;

            // Add an edge from this block to the successor of the caller basic block, if this
            // block is the end of a subroutine and if this block and subroutine_caller do not
            // belong to the same subroutine.
            let block = self.get(basic_block);
            if block.flags & FLAG_SUBROUTINE_END != 0
                && block.subroutine_id != self.get(subroutine_caller).subroutine_id
            {
                // By construction, the first outgoing edge of a basic block that ends with a jsr
                // instruction leads to the jsr continuation block, i.e. where execution continues
                // when ret is called (see FLAG_SUBROUTINE_CALLER).
                if let Some(caller_edge) = self.get(subroutine_caller).outgoing_edges {
                    let info = i32::from(block.output_stack_size);
                    let successor = self.edge(caller_edge).successor;
                    self.add_outgoing_edge(basic_block, info, successor);
                }
            }
            // Add its successors to the list of blocks to process. Note that push_successors
            // does not push basic blocks which are already in a list. Here this means either in
            // the list of blocks to process, or in the list of already processed blocks. This
            // second list is important to make sure we don't reprocess an already processed
            // block.
            list_of_blocks_to_process =
                self.push_successors(basic_block, list_of_blocks_to_process);
        }
        // Reset the next_list_element of all the basic blocks that have been processed to None,
        // so that this method can be called again with a different subroutine or subroutine
        // caller.
        while list_of_processed_blocks != EMPTY_LIST {
            list_of_processed_blocks = self.take_next_list_element(list_of_processed_blocks);
        }
    }

    /// Adds the successors of the given block in the method's control flow graph (except those
    /// corresponding to a jsr target, and those already in a list of labels) to the given list
    /// of blocks to process, and returns the new list (ASM's `pushSuccessors`).
    fn push_successors(&mut self, block: BlockId, list_of_labels_to_process: BlockId) -> BlockId {
        let mut new_list_of_labels_to_process = list_of_labels_to_process;
        let block = self.get(block);
        let is_subroutine_caller = block.flags & FLAG_SUBROUTINE_CALLER != 0;
        let first_edge = block.outgoing_edges;
        // By construction, the second outgoing edge of a basic block that ends with a jsr
        // instruction leads to the jsr target (see FLAG_SUBROUTINE_CALLER).
        let jsr_target_edge = first_edge.and_then(|edge| self.edge(edge).next_edge);
        let mut outgoing_edge = first_edge;
        while let Some(edge) = outgoing_edge {
            let Edge {
                successor,
                next_edge,
                ..
            } = *self.edge(edge);
            let is_jsr_target = is_subroutine_caller && Some(edge) == jsr_target_edge;
            if !is_jsr_target && self.get(successor).next_list_element.is_none() {
                // Add this successor to the list of blocks to process, if it does not already
                // belong to a list of labels.
                self.get_mut(successor).next_list_element = Some(new_list_of_labels_to_process);
                new_list_of_labels_to_process = successor;
            }
            outgoing_edge = next_edge;
        }
        new_list_of_labels_to_process
    }

    /// The next element of the list to which the given block belongs (which it must belong to).
    pub(crate) fn next_list_element(&self, block: BlockId) -> BlockId {
        self.get(block).next_list_element.unwrap_or(EMPTY_LIST)
    }

    /// Removes the given block from the list to which it belongs, and returns the next element
    /// of this list.
    pub(crate) fn take_next_list_element(&mut self, block: BlockId) -> BlockId {
        self.get_mut(block)
            .next_list_element
            .take()
            .unwrap_or(EMPTY_LIST)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Labels with consecutive ids, and labels whose ids share their low bits (they collide at
    /// every table size up to 1024 slots), in and out of order, with or without the debug only
    /// flag, map to the same block as in a hash map, before and after the slots grow.
    #[test]
    fn labels_map_to_their_blocks() {
        let collides = |label: &Label| label.id().is_multiple_of(1024);
        let mut labels = Vec::new();
        while labels.iter().filter(|l| collides(l)).count() < 8 {
            labels.push(Label::new());
        }
        let colliding = labels.iter().copied().filter(collides);
        let consecutive = labels[100..400].iter().rev().copied();
        let mut order: Vec<Label> = colliding.chain(consecutive).collect();
        order.extend(order.clone().iter().map(|l| l.with_debug_only(true)));
        for reserved in [None, Some(4), Some(1000)] {
            let mut table = LabelTable::default();
            if let Some(reserved) = reserved {
                table.reserve(reserved);
            }
            let mut model = HashMap::new();
            for &label in &order {
                let next = BlockId::try_from(model.len()).unwrap();
                let expected = *model.entry(label).or_insert(next);
                assert_eq!(table.id(label), expected, "{label:?}, {reserved:?}");
                assert_eq!(table.get(expected).label, label);
            }
            for &label in &order {
                assert_eq!(table.find(label), model.get(&label).copied());
            }
            assert!(!table.overflow.is_empty(), "some labels collide");
            for &label in labels[400..500].iter().filter(|l| !collides(l)) {
                assert_eq!(table.find(label), None);
                assert!(table.block(label).is_none());
            }
        }
    }
}
