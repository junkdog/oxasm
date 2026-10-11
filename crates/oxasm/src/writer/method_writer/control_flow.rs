//! The control flow graph of a method writer, and the computations built on it: ASM's
//! `visitLabel`, `visitMaxs`, `computeAllFrames`, `computeMaxStackAndLocal`, and its "utility
//! methods: control flow analysis algorithm" (`addSuccessorToCurrentBasicBlock`,
//! `endCurrentBasicBlockWithNoSuccessor`).

use crate::error::WriteError;
use crate::hierarchy::ClassHierarchy;
use crate::label::Label;
use crate::opcodes;
use crate::writer::frame::{self, Frame};
use crate::writer::handler;
use crate::writer::label_table::{
    BlockId, EDGE_EXCEPTION, EDGE_JUMP, EMPTY_LIST, FLAG_JUMP_TARGET, FLAG_REACHABLE,
    FLAG_SUBROUTINE_CALLER, LabelTable,
};
use crate::writer::method_writer::{Compute, MethodWriterState, descriptor_copy};
use crate::writer::symbol_table::SymbolTable;

/// Where the constants below are not applicable.
const NA: i8 = 0;

/// The stack size variation corresponding to each JVM opcode. The stack size variation for
/// opcode 'o' is given by the array element at index 'o'.
///
/// See <https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-6.html>.
#[rustfmt::skip]
const STACK_SIZE_DELTA: [i8; 202] = [
    0, // nop = 0 (0x0)
    1, // aconst_null = 1 (0x1)
    1, // iconst_m1 = 2 (0x2)
    1, // iconst_0 = 3 (0x3)
    1, // iconst_1 = 4 (0x4)
    1, // iconst_2 = 5 (0x5)
    1, // iconst_3 = 6 (0x6)
    1, // iconst_4 = 7 (0x7)
    1, // iconst_5 = 8 (0x8)
    2, // lconst_0 = 9 (0x9)
    2, // lconst_1 = 10 (0xa)
    1, // fconst_0 = 11 (0xb)
    1, // fconst_1 = 12 (0xc)
    1, // fconst_2 = 13 (0xd)
    2, // dconst_0 = 14 (0xe)
    2, // dconst_1 = 15 (0xf)
    1, // bipush = 16 (0x10)
    1, // sipush = 17 (0x11)
    1, // ldc = 18 (0x12)
    NA, // ldc_w = 19 (0x13)
    NA, // ldc2_w = 20 (0x14)
    1, // iload = 21 (0x15)
    2, // lload = 22 (0x16)
    1, // fload = 23 (0x17)
    2, // dload = 24 (0x18)
    1, // aload = 25 (0x19)
    NA, NA, NA, NA, // iload_0 ... iload_3 = 26 ... 29 (0x1a ... 0x1d)
    NA, NA, NA, NA, // lload_0 ... lload_3 = 30 ... 33 (0x1e ... 0x21)
    NA, NA, NA, NA, // fload_0 ... fload_3 = 34 ... 37 (0x22 ... 0x25)
    NA, NA, NA, NA, // dload_0 ... dload_3 = 38 ... 41 (0x26 ... 0x29)
    NA, NA, NA, NA, // aload_0 ... aload_3 = 42 ... 45 (0x2a ... 0x2d)
    -1, // iaload = 46 (0x2e)
    0, // laload = 47 (0x2f)
    -1, // faload = 48 (0x30)
    0, // daload = 49 (0x31)
    -1, // aaload = 50 (0x32)
    -1, // baload = 51 (0x33)
    -1, // caload = 52 (0x34)
    -1, // saload = 53 (0x35)
    -1, // istore = 54 (0x36)
    -2, // lstore = 55 (0x37)
    -1, // fstore = 56 (0x38)
    -2, // dstore = 57 (0x39)
    -1, // astore = 58 (0x3a)
    NA, NA, NA, NA, // istore_0 ... istore_3 = 59 ... 62 (0x3b ... 0x3e)
    NA, NA, NA, NA, // lstore_0 ... lstore_3 = 63 ... 66 (0x3f ... 0x42)
    NA, NA, NA, NA, // fstore_0 ... fstore_3 = 67 ... 70 (0x43 ... 0x46)
    NA, NA, NA, NA, // dstore_0 ... dstore_3 = 71 ... 74 (0x47 ... 0x4a)
    NA, NA, NA, NA, // astore_0 ... astore_3 = 75 ... 78 (0x4b ... 0x4e)
    -3, // iastore = 79 (0x4f)
    -4, // lastore = 80 (0x50)
    -3, // fastore = 81 (0x51)
    -4, // dastore = 82 (0x52)
    -3, // aastore = 83 (0x53)
    -3, // bastore = 84 (0x54)
    -3, // castore = 85 (0x55)
    -3, // sastore = 86 (0x56)
    -1, // pop = 87 (0x57)
    -2, // pop2 = 88 (0x58)
    1, // dup = 89 (0x59)
    1, // dup_x1 = 90 (0x5a)
    1, // dup_x2 = 91 (0x5b)
    2, // dup2 = 92 (0x5c)
    2, // dup2_x1 = 93 (0x5d)
    2, // dup2_x2 = 94 (0x5e)
    0, // swap = 95 (0x5f)
    -1, // iadd = 96 (0x60)
    -2, // ladd = 97 (0x61)
    -1, // fadd = 98 (0x62)
    -2, // dadd = 99 (0x63)
    -1, // isub = 100 (0x64)
    -2, // lsub = 101 (0x65)
    -1, // fsub = 102 (0x66)
    -2, // dsub = 103 (0x67)
    -1, // imul = 104 (0x68)
    -2, // lmul = 105 (0x69)
    -1, // fmul = 106 (0x6a)
    -2, // dmul = 107 (0x6b)
    -1, // idiv = 108 (0x6c)
    -2, // ldiv = 109 (0x6d)
    -1, // fdiv = 110 (0x6e)
    -2, // ddiv = 111 (0x6f)
    -1, // irem = 112 (0x70)
    -2, // lrem = 113 (0x71)
    -1, // frem = 114 (0x72)
    -2, // drem = 115 (0x73)
    0, // ineg = 116 (0x74)
    0, // lneg = 117 (0x75)
    0, // fneg = 118 (0x76)
    0, // dneg = 119 (0x77)
    -1, // ishl = 120 (0x78)
    -1, // lshl = 121 (0x79)
    -1, // ishr = 122 (0x7a)
    -1, // lshr = 123 (0x7b)
    -1, // iushr = 124 (0x7c)
    -1, // lushr = 125 (0x7d)
    -1, // iand = 126 (0x7e)
    -2, // land = 127 (0x7f)
    -1, // ior = 128 (0x80)
    -2, // lor = 129 (0x81)
    -1, // ixor = 130 (0x82)
    -2, // lxor = 131 (0x83)
    0, // iinc = 132 (0x84)
    1, // i2l = 133 (0x85)
    0, // i2f = 134 (0x86)
    1, // i2d = 135 (0x87)
    -1, // l2i = 136 (0x88)
    -1, // l2f = 137 (0x89)
    0, // l2d = 138 (0x8a)
    0, // f2i = 139 (0x8b)
    1, // f2l = 140 (0x8c)
    1, // f2d = 141 (0x8d)
    -1, // d2i = 142 (0x8e)
    0, // d2l = 143 (0x8f)
    -1, // d2f = 144 (0x90)
    0, // i2b = 145 (0x91)
    0, // i2c = 146 (0x92)
    0, // i2s = 147 (0x93)
    -3, // lcmp = 148 (0x94)
    -1, // fcmpl = 149 (0x95)
    -1, // fcmpg = 150 (0x96)
    -3, // dcmpl = 151 (0x97)
    -3, // dcmpg = 152 (0x98)
    -1, // ifeq = 153 (0x99)
    -1, // ifne = 154 (0x9a)
    -1, // iflt = 155 (0x9b)
    -1, // ifge = 156 (0x9c)
    -1, // ifgt = 157 (0x9d)
    -1, // ifle = 158 (0x9e)
    -2, // if_icmpeq = 159 (0x9f)
    -2, // if_icmpne = 160 (0xa0)
    -2, // if_icmplt = 161 (0xa1)
    -2, // if_icmpge = 162 (0xa2)
    -2, // if_icmpgt = 163 (0xa3)
    -2, // if_icmple = 164 (0xa4)
    -2, // if_acmpeq = 165 (0xa5)
    -2, // if_acmpne = 166 (0xa6)
    0, // goto = 167 (0xa7)
    1, // jsr = 168 (0xa8)
    0, // ret = 169 (0xa9)
    -1, // tableswitch = 170 (0xaa)
    -1, // lookupswitch = 171 (0xab)
    -1, // ireturn = 172 (0xac)
    -2, // lreturn = 173 (0xad)
    -1, // freturn = 174 (0xae)
    -2, // dreturn = 175 (0xaf)
    -1, // areturn = 176 (0xb0)
    0, // return = 177 (0xb1)
    NA, // getstatic = 178 (0xb2)
    NA, // putstatic = 179 (0xb3)
    NA, // getfield = 180 (0xb4)
    NA, // putfield = 181 (0xb5)
    NA, // invokevirtual = 182 (0xb6)
    NA, // invokespecial = 183 (0xb7)
    NA, // invokestatic = 184 (0xb8)
    NA, // invokeinterface = 185 (0xb9)
    NA, // invokedynamic = 186 (0xba)
    1, // new = 187 (0xbb)
    0, // newarray = 188 (0xbc)
    0, // anewarray = 189 (0xbd)
    0, // arraylength = 190 (0xbe)
    NA, // athrow = 191 (0xbf)
    0, // checkcast = 192 (0xc0)
    0, // instanceof = 193 (0xc1)
    -1, // monitorenter = 194 (0xc2)
    -1, // monitorexit = 195 (0xc3)
    NA, // wide = 196 (0xc4)
    NA, // multianewarray = 197 (0xc5)
    -1, // ifnull = 198 (0xc6)
    -1, // ifnonnull = 199 (0xc7)
    NA, // goto_w = 200 (0xc8)
    NA, // jsr_w = 201 (0xc9)
];

/// The stack size variation of the given opcode (`STACK_SIZE_DELTA[opcode]`).
///
/// # Errors
///
/// Java's `ArrayIndexOutOfBoundsException` for an opcode past `jsr_w`.
pub(super) fn stack_size_delta(opcode: u8) -> Result<i32, WriteError> {
    STACK_SIZE_DELTA
        .get(usize::from(opcode))
        .map(|&delta| i32::from(delta))
        .ok_or_else(|| stack_size_delta_out_of_bounds(opcode))
}

/// The error of [`stack_size_delta`], out of line: its formatting would make every instruction
/// visit save registers and reserve stack for it.
#[cold]
#[inline(never)]
fn stack_size_delta_out_of_bounds(opcode: u8) -> WriteError {
    WriteError::IndexOutOfBounds(format!(
        "Index {opcode} out of bounds for length {}",
        STACK_SIZE_DELTA.len()
    ))
}

/// ASM's `NullPointerException` where the control flow graph of a method is inconsistent: a jump
/// or exception handler to a label that was never visited, an exception handler range that does
/// not end, ...
fn null_pointer(what: &str) -> WriteError {
    WriteError::IllegalState(format!("java.lang.NullPointerException: {what}"))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodWriterState {
    /// ASM's `visitLabel`. Inline up to its basic block bookkeeping, which a writer that
    /// computes nothing skips.
    #[inline(always)]
    pub(super) fn visit_label(&mut self, label: Label) {
        // Resolve the forward references to this label, if any.
        let code_length = self.code_length();
        self.has_asm_instructions |= self.labels.resolve(
            label,
            &mut self.code,
            self.stack_map_table_entries.as_mut(),
            code_length,
        );
        // visitLabel starts a new basic block (except for debug only labels), so we need to
        // update the previous and current block references and list of successors.
        if label.is_debug_only() || self.compute == Compute::Nothing {
            return;
        }
        self.visit_label_block(label);
    }

    /// The basic block bookkeeping of [`Self::visit_label`], for a label that is not debug only
    /// and a writer that computes something.
    #[inline(never)]
    fn visit_label_block(&mut self, label: Label) {
        let block = self.labels.id(label);
        match self.compute {
            Compute::AllFrames => {
                let labels = &mut self.labels;
                if let Some(current_basic_block) = self.current_basic_block {
                    if labels.get(block).bytecode_offset
                        == labels.get(current_basic_block).bytecode_offset
                    {
                        // We use canonical() to store the state of a basic block in only one
                        // place, but this does not work for labels which have not been visited
                        // yet. Therefore, when we detect here two labels having the same bytecode
                        // offset, we need to
                        // - consolidate the state scattered in these two instances into the
                        //   canonical instance:
                        let jump_target = labels.get(block).flags & FLAG_JUMP_TARGET;
                        labels.get_mut(current_basic_block).flags |= jump_target;
                        // - make sure the two instances share the same frame (the implementation
                        //   of canonical() relies on this property; here the label's frame should
                        //   be None):
                        labels.get_mut(block).frame = labels.get(current_basic_block).frame;
                        // - and make sure to NOT assign 'label' into 'current_basic_block' or
                        //   'last_basic_block', so that they still refer to the canonical
                        //   instance for this bytecode offset.
                        return;
                    }
                    // End the current basic block (with one new successor).
                    labels.add_outgoing_edge(current_basic_block, EDGE_JUMP, block);
                }
                // Append 'label' at the end of the basic block list.
                if let Some(last_basic_block) = self.last_basic_block {
                    if labels.get(block).bytecode_offset
                        == labels.get(last_basic_block).bytecode_offset
                    {
                        // Same comment as above.
                        let jump_target = labels.get(block).flags & FLAG_JUMP_TARGET;
                        labels.get_mut(last_basic_block).flags |= jump_target;
                        // Here the label's frame should be None.
                        labels.get_mut(block).frame = labels.get(last_basic_block).frame;
                        self.current_basic_block = Some(last_basic_block);
                        return;
                    }
                    labels.get_mut(last_basic_block).next_basic_block = Some(block);
                }
                self.last_basic_block = Some(block);
                // Make it the new current basic block.
                self.current_basic_block = Some(block);
                // Here the label's frame should be None.
                let frame = labels.add_frame(Frame::new(Some(block)));
                labels.get_mut(block).frame = Some(frame);
            }
            Compute::InsertedFrames => match self.current_basic_block {
                // This case should happen only once, for the visitLabel call in the constructor.
                // Indeed, if compute is equal to COMPUTE_INSERTED_FRAMES, currentBasicBlock stays
                // unchanged.
                None => self.current_basic_block = Some(block),
                // Update the frame owner so that a correct frame offset is computed in
                // Frame.accept().
                Some(current_basic_block) => {
                    if let Some(frame) = self.labels.get(current_basic_block).frame {
                        self.labels.frame_mut(frame).frame.owner = Some(block);
                    }
                }
            },
            Compute::MaxStackAndLocal => {
                if let Some(current_basic_block) = self.current_basic_block {
                    // End the current basic block (with one new successor).
                    self.labels.get_mut(current_basic_block).output_stack_max =
                        self.max_relative_stack_size as i16;
                    self.add_successor_to_current_basic_block(self.relative_stack_size, block);
                }
                // Start a new current basic block, and reset the current and maximum relative
                // stack sizes.
                self.current_basic_block = Some(block);
                self.relative_stack_size = 0;
                self.max_relative_stack_size = 0;
                // Append the new basic block at the end of the basic block list.
                if let Some(last_basic_block) = self.last_basic_block {
                    self.labels.get_mut(last_basic_block).next_basic_block = Some(block);
                }
                self.last_basic_block = Some(block);
            }
            Compute::MaxStackAndLocalFromFrames if self.current_basic_block.is_none() => {
                // This case should happen only once, for the visitLabel call in the constructor.
                // Indeed, if compute is equal to COMPUTE_MAX_STACK_AND_LOCAL_FROM_FRAMES,
                // currentBasicBlock stays unchanged.
                self.current_basic_block = Some(block);
            }
            Compute::MaxStackAndLocalFromFrames | Compute::Nothing => {}
        }
    }

    /// Adds a successor to the current basic block in the control flow graph (ASM's
    /// `addSuccessorToCurrentBasicBlock`).
    ///
    /// - `info`: information about the control flow edge to be added.
    /// - `successor`: the successor block to be added to the current basic block.
    pub(super) fn add_successor_to_current_basic_block(&mut self, info: i32, successor: BlockId) {
        if let Some(current_basic_block) = self.current_basic_block {
            self.labels
                .add_outgoing_edge(current_basic_block, info, successor);
        }
    }

    /// Ends the current basic block. This method must be used in the case where the current
    /// basic block does not have any successor (ASM's `endCurrentBasicBlockWithNoSuccessor`).
    ///
    /// WARNING: this method must be called after the currently visited instruction has been put
    /// in `code` (if frames are computed, this method inserts a new label to start a new basic
    /// block after the current instruction).
    pub(super) fn end_current_basic_block_with_no_successor(&mut self) {
        match self.compute {
            Compute::AllFrames => {
                let next_basic_block = Label::new();
                let block = self.labels.id(next_basic_block);
                let frame = self.labels.add_frame(Frame::new(Some(block)));
                self.labels.get_mut(block).frame = Some(frame);
                let code_length = self.code_length();
                self.labels.resolve(
                    next_basic_block,
                    &mut self.code,
                    self.stack_map_table_entries.as_mut(),
                    code_length,
                );
                if let Some(last_basic_block) = self.last_basic_block {
                    self.labels.get_mut(last_basic_block).next_basic_block = Some(block);
                }
                self.last_basic_block = Some(block);
                self.current_basic_block = None;
            }
            Compute::MaxStackAndLocal => {
                if let Some(current_basic_block) = self.current_basic_block {
                    self.labels.get_mut(current_basic_block).output_stack_max =
                        self.max_relative_stack_size as i16;
                }
                self.current_basic_block = None;
            }
            _ => {}
        }
    }

    /// ASM's `visitSwitchInsn`: the control flow graph and stack size part of the switch
    /// instructions.
    pub(super) fn visit_switch_insn(
        &mut self,
        symbol_table: &mut SymbolTable,
        dflt: Label,
        labels: &[Label],
    ) {
        if self.current_basic_block.is_none() {
            return;
        }
        match self.compute {
            Compute::AllFrames => {
                self.execute(symbol_table, opcodes::LOOKUPSWITCH, 0, None);
                // Add all the labels as successors of the current basic block.
                for &label in std::iter::once(&dflt).chain(labels) {
                    let block = self.labels.id(label);
                    self.add_successor_to_current_basic_block(EDGE_JUMP, block);
                    let canonical = self.labels.canonical(block);
                    self.labels.get_mut(canonical).flags |= FLAG_JUMP_TARGET;
                }
            }
            Compute::MaxStackAndLocal => {
                // No need to update maxRelativeStackSize (the stack size delta is always
                // negative).
                self.relative_stack_size = self.relative_stack_size.wrapping_sub(1);
                // Add all the labels as successors of the current basic block.
                for &label in std::iter::once(&dflt).chain(labels) {
                    let block = self.labels.id(label);
                    self.add_successor_to_current_basic_block(self.relative_stack_size, block);
                }
            }
            _ => {}
        }
        // End the current basic block.
        self.end_current_basic_block_with_no_successor();
    }

    /// Updates the relative stack size, and its maximum, after an instruction with the given
    /// stack size variation.
    pub(super) fn update_relative_stack_size(&mut self, delta: i32) {
        let size = self.relative_stack_size.wrapping_add(delta);
        if size > self.max_relative_stack_size {
            self.max_relative_stack_size = size;
        }
        self.relative_stack_size = size;
    }

    /// ASM's `visitMaxs`.
    pub(super) fn visit_maxs(
        &mut self,
        symbol_table: &mut SymbolTable,
        hierarchy: Option<&dyn ClassHierarchy>,
        max_stack: u16,
        max_locals: u16,
    ) {
        match self.compute {
            Compute::AllFrames | Compute::MaxStackAndLocal => {
                // After an error, which ASM would have thrown, the class cannot be written: skip
                // the computation (ASM stops at its first exception).
                if symbol_table.error().is_some() {
                    return;
                }
                let result = if self.compute == Compute::AllFrames {
                    self.compute_all_frames(symbol_table, hierarchy)
                } else {
                    self.compute_max_stack_and_local()
                };
                if let Err(error) = result {
                    symbol_table.fail(error);
                }
            }
            Compute::MaxStackAndLocalFromFrames => {
                self.max_stack = self.max_relative_stack_size;
            }
            Compute::Nothing | Compute::InsertedFrames => {
                self.max_stack = i32::from(max_stack);
                self.max_locals = i32::from(max_locals);
            }
        }
    }

    /// Computes all the stack map frames of the method, from scratch (ASM's
    /// `computeAllFrames`).
    #[allow(clippy::too_many_lines)] // as in ASM
    fn compute_all_frames(
        &mut self,
        symbol_table: &mut SymbolTable,
        hierarchy: Option<&dyn ClassHierarchy>,
    ) -> Result<(), WriteError> {
        let Some(first_basic_block) = self.first_basic_block else {
            return Ok(());
        };
        // Complete the control flow graph with exception handler blocks.
        let labels = &mut self.labels;
        for handler in &self.handlers {
            let catch_type_descriptor = handler
                .catch_type_descriptor
                .as_deref()
                .unwrap_or(crate::jstr!("java/lang/Throwable"));
            let catch_type =
                frame::get_abstract_type_from_internal_name(symbol_table, catch_type_descriptor);
            // Mark handlerBlock as an exception handler.
            let handler_block = labels.id(handler.handler_pc);
            let handler_block = labels.canonical(handler_block);
            labels.get_mut(handler_block).flags |= FLAG_JUMP_TARGET;
            // Add handlerBlock as a successor of all the basic blocks in the exception handler
            // range.
            let handler_range_block = labels.id(handler.start_pc);
            let mut handler_range_block = labels.canonical(handler_range_block);
            let handler_range_end = labels.id(handler.end_pc);
            let handler_range_end = labels.canonical(handler_range_end);
            while handler_range_block != handler_range_end {
                labels.add_outgoing_edge(handler_range_block, catch_type, handler_block);
                handler_range_block = labels
                    .get(handler_range_block)
                    .next_basic_block
                    .ok_or_else(|| null_pointer("handlerRangeBlock"))?;
            }
        }

        // Create and visit the first (implicit) frame.
        let first_frame = labels
            .get(first_basic_block)
            .frame
            .expect("set by visit_label");
        let max_locals = usize::try_from(self.max_locals).unwrap_or(0);
        let descriptor = descriptor_copy(symbol_table, self.descriptor);
        labels
            .frame_mut(first_frame)
            .set_input_frame_from_descriptor(
                symbol_table,
                self.access_flags,
                &descriptor,
                max_locals,
            )?;
        self.accept_computed_frame(symbol_table, first_frame);

        // Fix point algorithm: add the first basic block to a list of blocks to process (i.e.
        // blocks whose stack map frame has changed) and, while there are blocks to process,
        // remove one from the list and update the stack map frames of its successor blocks in the
        // control flow graph (which might change them, in which case these blocks must be
        // processed too, and are thus added to the list of blocks to process). Also compute the
        // maximum stack size of the method, as a by-product.
        let labels = &mut self.labels;
        let mut list_of_blocks_to_process = first_basic_block;
        labels.get_mut(first_basic_block).next_list_element = Some(EMPTY_LIST);
        let mut max_stack_size = 0;
        while list_of_blocks_to_process != EMPTY_LIST {
            // Remove a basic block from the list of blocks to process.
            let basic_block = list_of_blocks_to_process;
            list_of_blocks_to_process = labels.take_next_list_element(basic_block);
            // By definition, basicBlock is reachable.
            labels.get_mut(basic_block).flags |= FLAG_REACHABLE;
            let basic_block_frame = labels
                .get(basic_block)
                .frame
                .ok_or_else(|| null_pointer("basicBlock.frame"))?;
            // Update the (absolute) maximum stack size.
            let frame = labels.frame(basic_block_frame);
            let max_block_stack_size =
                frame.get_input_stack_size() as i32 + i32::from(frame.output_stack_max);
            if max_block_stack_size > max_stack_size {
                max_stack_size = max_block_stack_size;
            }
            // Update the successor blocks of basicBlock in the control flow graph.
            let mut outgoing_edge = labels.get(basic_block).outgoing_edges;
            while let Some(edge) = outgoing_edge {
                let edge = *labels.edge(edge);
                let successor_block = labels.canonical(edge.successor);
                let successor_frame = labels
                    .get(successor_block)
                    .frame
                    .ok_or_else(|| null_pointer("successorBlock.frame"))?;
                let successor_block_changed = if successor_frame == basic_block_frame {
                    labels.frame_mut(successor_frame).merge_into_self(
                        symbol_table,
                        hierarchy,
                        edge.info,
                    )?
                } else {
                    let source = *labels.frame(basic_block_frame);
                    labels.frame_mut(successor_frame).merge_from(
                        &source,
                        symbol_table,
                        hierarchy,
                        edge.info,
                    )?
                };
                if successor_block_changed
                    && labels.get(successor_block).next_list_element.is_none()
                {
                    // If successorBlock has changed it must be processed. Thus, if it is not
                    // already in the list of blocks to process, add it to this list.
                    labels.get_mut(successor_block).next_list_element =
                        Some(list_of_blocks_to_process);
                    list_of_blocks_to_process = successor_block;
                }
                outgoing_edge = edge.next_edge;
            }
        }

        // Loop over all the basic blocks and visit the stack map frames that must be stored in
        // the StackMapTable attribute. Also replace unreachable code with NOP* ATHROW, and remove
        // it from exception handler ranges.
        let mut basic_block = Some(first_basic_block);
        while let Some(block) = basic_block {
            let flags = self.labels.get(block).flags;
            if flags & (FLAG_JUMP_TARGET | FLAG_REACHABLE) == FLAG_JUMP_TARGET | FLAG_REACHABLE {
                let Some(block_frame) = self.labels.get(block).frame else {
                    return Err(null_pointer("basicBlock.frame"));
                };
                self.accept_computed_frame(symbol_table, block_frame);
            }
            let next_basic_block = self.labels.get(block).next_basic_block;
            if flags & FLAG_REACHABLE == 0 {
                // Find the start and end bytecode offsets of this unreachable block.
                let start_offset = self.labels.get(block).bytecode_offset;
                let end_offset = next_basic_block.map_or(self.code_length(), |next| {
                    self.labels.get(next).bytecode_offset
                }) - 1;
                if end_offset >= start_offset {
                    // Replace its instructions with NOP ... NOP ATHROW.
                    let code = self.code.data_mut();
                    let (Ok(start), Ok(end)) =
                        (usize::try_from(start_offset), usize::try_from(end_offset))
                    else {
                        return Err(WriteError::IndexOutOfBounds(format!(
                            "Index {start_offset} out of bounds for length {}",
                            code.len()
                        )));
                    };
                    if end >= code.len() {
                        return Err(WriteError::IndexOutOfBounds(format!(
                            "Index {end} out of bounds for length {}",
                            code.len()
                        )));
                    }
                    code[start..end].fill(opcodes::NOP);
                    code[end] = opcodes::ATHROW;
                    // Emit a frame for this unreachable block, with no local and a Throwable on
                    // the stack (so that the ATHROW could consume this Throwable if it were
                    // reachable).
                    let frame_index = self.visit_frame_start(start_offset, 0, 1);
                    self.current_frame[frame_index] = frame::get_abstract_type_from_internal_name(
                        symbol_table,
                        crate::jstr!("java/lang/Throwable"),
                    );
                    self.visit_frame_end(symbol_table);
                    // Remove this unreachable basic block from the exception handler ranges.
                    let start = self.labels.get(block).label;
                    let end = next_basic_block.map(|next| self.labels.get(next).label);
                    self.handlers = handler::remove_range(
                        std::mem::take(&mut self.handlers),
                        &self.labels,
                        start,
                        end,
                    );
                    // The maximum stack size is now at least one, because of the Throwable
                    // declared above.
                    max_stack_size = max_stack_size.max(1);
                }
            }
            basic_block = next_basic_block;
        }

        self.max_stack = max_stack_size;
        Ok(())
    }

    /// The control flow graph of this method, from the basic block of `start` on: for each basic
    /// block, its bytecode offset and those of its successors, less the virtual successor of the
    /// blocks that end with a jsr (ASM's `ClassWriterComputeMaxsTest.TestCaseBuilder.visitMaxs`).
    pub(crate) fn control_flow_graph(&self, start: Label) -> Vec<(i32, Vec<i32>)> {
        let mut graph = Vec::new();
        let mut current_label = self.labels.find(start);
        while let Some(block) = current_label {
            let b = self.labels.get(block);
            let mut outgoing_edge = b.outgoing_edges;
            if b.flags & FLAG_SUBROUTINE_CALLER != 0 {
                // Ignore the first outgoing edge of the basic blocks ending with a jsr: these are
                // virtual edges which lead to the instruction just after the jsr.
                outgoing_edge = outgoing_edge.and_then(|edge| self.labels.edge(edge).next_edge);
            }
            let mut successors = Vec::new();
            while let Some(edge) = outgoing_edge {
                let edge = self.labels.edge(edge);
                successors.push(self.labels.get(edge.successor).bytecode_offset);
                outgoing_edge = edge.next_edge;
            }
            graph.push((b.bytecode_offset, successors));
            current_label = b.next_basic_block;
        }
        graph
    }

    /// Computes the maximum stack size of the method (ASM's `computeMaxStackAndLocal`).
    fn compute_max_stack_and_local(&mut self) -> Result<(), WriteError> {
        let Some(first_basic_block) = self.first_basic_block else {
            return Ok(());
        };
        let labels = &mut self.labels;
        // Complete the control flow graph with exception handler blocks.
        for handler in &self.handlers {
            let handler_block = labels.id(handler.handler_pc);
            let mut handler_range_block = labels.id(handler.start_pc);
            let handler_range_end = labels.id(handler.end_pc);
            // Add handlerBlock as a successor of all the basic blocks in the exception handler
            // range.
            while handler_range_block != handler_range_end {
                if labels.get(handler_range_block).flags & FLAG_SUBROUTINE_CALLER == 0 {
                    labels.add_outgoing_edge(handler_range_block, EDGE_EXCEPTION, handler_block);
                } else {
                    // If handlerRangeBlock is a JSR block, add handlerBlock after the first two
                    // outgoing edges to preserve the hypothesis about JSR block successors order
                    // (see visit_jump_insn).
                    let second_edge = labels
                        .get(handler_range_block)
                        .outgoing_edges
                        .and_then(|edge| labels.edge(edge).next_edge)
                        .ok_or_else(|| null_pointer("handlerRangeBlock.outgoingEdges.nextEdge"))?;
                    let third_edge = labels.edge(second_edge).next_edge;
                    let edge = labels.new_edge(EDGE_EXCEPTION, handler_block, third_edge);
                    labels.set_next_edge(second_edge, Some(edge));
                }
                handler_range_block = labels
                    .get(handler_range_block)
                    .next_basic_block
                    .ok_or_else(|| null_pointer("handlerRangeBlock"))?;
            }
        }

        // Complete the control flow graph with the successor blocks of subroutines, if needed.
        if self.has_subroutines {
            // First step: find the subroutines. This step determines, for each basic block, to
            // which subroutine(s) it belongs. Start with the main "subroutine":
            let mut num_subroutines: i16 = 1;
            labels.mark_subroutine(first_basic_block, num_subroutines);
            // Then, mark the subroutines called by the main subroutine, then the subroutines
            // called by those called by the main subroutine, etc.
            let mut current_subroutine: i16 = 1;
            while current_subroutine <= num_subroutines {
                let mut basic_block = Some(first_basic_block);
                while let Some(block) = basic_block {
                    let b = labels.get(block);
                    if b.flags & FLAG_SUBROUTINE_CALLER != 0
                        && b.subroutine_id == current_subroutine
                    {
                        let jsr_target = jsr_target(labels, block)?;
                        if labels.get(jsr_target).subroutine_id == 0 {
                            // If this subroutine has not been marked yet, find its basic blocks.
                            num_subroutines = num_subroutines.wrapping_add(1);
                            labels.mark_subroutine(jsr_target, num_subroutines);
                        }
                    }
                    basic_block = labels.get(block).next_basic_block;
                }
                current_subroutine = current_subroutine.wrapping_add(1);
                if current_subroutine == i16::MIN {
                    break;
                }
            }
            // Second step: find the successors in the control flow graph of each subroutine basic
            // block 'r' ending with a RET instruction. These successors are the virtual
            // successors of the basic blocks ending with JSR instructions (see visit_jump_insn)
            // that can reach 'r'.
            let mut basic_block = Some(first_basic_block);
            while let Some(block) = basic_block {
                if labels.get(block).flags & FLAG_SUBROUTINE_CALLER != 0 {
                    // By construction, jsr targets are stored in the second outgoing edge of
                    // basic blocks that ends with a jsr instruction (see FLAG_SUBROUTINE_CALLER).
                    let subroutine = jsr_target(labels, block)?;
                    labels.add_subroutine_ret_successors(subroutine, block);
                }
                basic_block = labels.get(block).next_basic_block;
            }
        }

        // Data flow algorithm: put the first basic block in a list of blocks to process (i.e.
        // blocks whose input stack size has changed) and, while there are blocks to process,
        // remove one from the list, update the input stack size of its successor blocks in the
        // control flow graph, and add these blocks to the list of blocks to process (if not
        // already done).
        let mut list_of_blocks_to_process = first_basic_block;
        labels.get_mut(first_basic_block).next_list_element = Some(EMPTY_LIST);
        let mut max_stack_size = self.max_stack;
        while list_of_blocks_to_process != EMPTY_LIST {
            // Remove a basic block from the list of blocks to process. Note that we don't reset
            // basicBlock.nextListElement to null on purpose, to make sure we don't reprocess
            // already processed basic blocks.
            let basic_block = list_of_blocks_to_process;
            list_of_blocks_to_process = labels.next_list_element(basic_block);
            // Compute the (absolute) input stack size and maximum stack size of this block.
            let block = labels.get(basic_block);
            let input_stack_top = i32::from(block.input_stack_size);
            let max_block_stack_size = input_stack_top + i32::from(block.output_stack_max);
            // Update the absolute maximum stack size of the method.
            if max_block_stack_size > max_stack_size {
                max_stack_size = max_block_stack_size;
            }
            // Update the input stack size of the successor blocks of basicBlock in the control
            // flow graph, and add these blocks to the list of blocks to process, if not already
            // done.
            let mut outgoing_edge = block.outgoing_edges;
            if block.flags & FLAG_SUBROUTINE_CALLER != 0 {
                // Ignore the first outgoing edge of the basic blocks ending with a jsr: these are
                // virtual edges which lead to the instruction just after the jsr, and do not
                // correspond to a possible execution path (see visit_jump_insn and
                // FLAG_SUBROUTINE_CALLER).
                outgoing_edge = outgoing_edge.and_then(|edge| labels.edge(edge).next_edge);
            }
            while let Some(edge) = outgoing_edge {
                let edge = *labels.edge(edge);
                let successor_block = edge.successor;
                if labels.get(successor_block).next_list_element.is_none() {
                    labels.get_mut(successor_block).input_stack_size =
                        if edge.info == EDGE_EXCEPTION {
                            1
                        } else {
                            input_stack_top.wrapping_add(edge.info) as i16
                        };
                    labels.get_mut(successor_block).next_list_element =
                        Some(list_of_blocks_to_process);
                    list_of_blocks_to_process = successor_block;
                }
                outgoing_edge = edge.next_edge;
            }
        }
        self.max_stack = max_stack_size;
        Ok(())
    }
}

/// The target of the jsr instruction that ends the given basic block: by construction, the
/// successor of its second outgoing edge (see `FLAG_SUBROUTINE_CALLER`).
fn jsr_target(labels: &LabelTable, block: BlockId) -> Result<BlockId, WriteError> {
    labels
        .get(block)
        .outgoing_edges
        .and_then(|edge| labels.edge(edge).next_edge)
        .map(|edge| labels.edge(edge).successor)
        .ok_or_else(|| null_pointer("basicBlock.outgoingEdges.nextEdge"))
}
