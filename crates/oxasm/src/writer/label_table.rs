//! The labels of a method being written (ADR-03): what ASM's `Label` objects hold for the
//! `MethodWriter`, in a side table keyed by [`Label`].
//!
//! ASM's label state lives in the label objects, so it survives from one method to another; here
//! each method writer has its own table. A label that a method writer never saw has the state of
//! a new ASM label: unresolved, at bytecode offset 0.

use rustc_hash::FxHashMap;

use crate::label::Label;
use crate::writer::byte_vector::ByteVector;
use crate::writer::frame::Frame;

/// A flag indicating that a label is only used for debug attributes.
#[allow(dead_code)] // used by the frame computation (P6)
pub(crate) const FLAG_DEBUG_ONLY: u16 = 1;
/// A flag indicating that the bytecode offset of a label is known.
pub(crate) const FLAG_RESOLVED: u16 = 4;

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
#[derive(Clone, Debug, Default)]
pub(crate) struct Block {
    /// The type and status of this label or basic block (the `FLAG_*` values; the debug only
    /// flag is the label's own).
    pub(crate) flags: u16,
    /// The offset of this label in the bytecode of its method, in bytes. This value is set if
    /// and only if the `FLAG_RESOLVED` flag is set.
    pub(crate) bytecode_offset: i32,
    /// The forward references to this label: `(source instruction bytecode offset, reference
    /// type | handle)` pairs, in the order they were added.
    forward_references: Vec<(i32, i32)>,
    /// The input and output stack map frames of this basic block, if computed (ASM's
    /// `Label.frame`).
    pub(crate) frame: Option<Box<Frame>>,
}

/// The labels of a method writer.
#[derive(Clone, Debug, Default)]
pub(crate) struct LabelTable {
    /// The index in `blocks` of each label seen.
    index: FxHashMap<Label, u32>,
    /// The state of each label seen.
    blocks: Vec<Block>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Java's int arithmetic
impl LabelTable {
    /// The state of the given label, created if needed.
    pub(crate) fn block_mut(&mut self, label: Label) -> &mut Block {
        let next = self.blocks.len() as u32;
        let index = *self.index.entry(label).or_insert(next);
        if index == next {
            self.blocks.push(Block::default());
        }
        &mut self.blocks[index as usize]
    }

    /// The state of the given label, if seen.
    pub(crate) fn block(&self, label: Label) -> Option<&Block> {
        self.index.get(&label).map(|&i| &self.blocks[i as usize])
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
        let block = self.block_mut(label);
        if block.flags & FLAG_RESOLVED == 0 {
            if wide_reference {
                block.forward_references.push((
                    source_insn_bytecode_offset,
                    FORWARD_REFERENCE_TYPE_WIDE | code.size() as i32,
                ));
                code.put_int(-1);
            } else {
                block.forward_references.push((
                    source_insn_bytecode_offset,
                    FORWARD_REFERENCE_TYPE_SHORT | code.size() as i32,
                ));
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
        let block = self.block_mut(label);
        if block.flags & FLAG_RESOLVED == 0 {
            block.forward_references.push((
                0,
                FORWARD_REFERENCE_TYPE_STACK_MAP | stack_map_table_entries.size() as i32,
            ));
        }
        stack_map_table_entries.put_short(block.bytecode_offset);
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
        for &(source_insn_bytecode_offset, reference) in block.forward_references.iter().rev() {
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
}
