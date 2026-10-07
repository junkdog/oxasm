//! The stack map frames of a method writer: `visitFrame`, and ASM's "utility methods: stack map
//! frames" (`visitFrameStart`, `visitAbstractType`, `visitFrameEnd`, `putFrame`, ...).

use crate::constant::FrameValue;
use crate::error::WriteError;
use crate::opcodes::{self, FrameKind};
use crate::ty::Type;
use crate::writer::byte_vector::ByteVector;
use crate::writer::frame::{
    self, APPEND_FRAME, CHOP_FRAME, FULL_FRAME, Frame, ITEM_OBJECT, ITEM_UNINITIALIZED, SAME_FRAME,
    SAME_FRAME_EXTENDED, SAME_LOCALS_1_STACK_ITEM_FRAME, SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED,
};
use crate::writer::label_table::LabelTable;
use crate::writer::method_writer::{Compute, MethodWriterState};
use crate::writer::symbol_table::SymbolTable;

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodWriterState {
    /// ASM's `visitFrame`.
    pub(super) fn visit_frame(
        &mut self,
        symbol_table: &mut SymbolTable,
        kind: FrameKind,
        local: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) -> Result<(), WriteError> {
        // A chop frame has no locals, only their number.
        let num_local = match kind {
            FrameKind::Chop(count) => i32::from(count),
            _ => local.len() as i32,
        };
        let num_stack = stack.len() as i32;
        if self.compute == Compute::AllFrames {
            return Ok(());
        }
        if self.compute == Compute::InsertedFrames {
            self.visit_inserted_frame(symbol_table, kind, local, stack)?;
        } else if kind == FrameKind::New {
            if self.previous_frame.is_none() {
                // In expanded mode, the implicit first frame is not visited: compute it from the
                // method descriptor (it is the previous frame of the first visited one).
                let arguments_size =
                    (Type::try_get_arguments_and_return_sizes_of(&*self.descriptor)? >> 2) as usize;
                let input_locals = frame::input_locals_from_descriptor(
                    symbol_table,
                    self.access_flags,
                    &self.descriptor,
                    arguments_size,
                )?;
                // The implicit first frame's owner is a new label, at bytecode offset 0.
                let locals = frame::frame_elements(&input_locals, true);
                self.accept_frame(symbol_table, 0, &locals, &[]);
            }
            self.current_locals = num_local;
            let frame_index = self.visit_frame_start(self.code_length(), num_local, num_stack);
            let labels = &self.labels;
            for (i, value) in local.iter().chain(stack).enumerate() {
                self.current_frame[frame_index + i] =
                    frame::get_abstract_type_from_api_format(symbol_table, value, |label| {
                        labels.resolved_offset(label)
                    })?;
            }
            self.visit_frame_end(symbol_table);
        } else {
            if symbol_table.major_version() < opcodes::V1_6 as i32 {
                return Err(WriteError::InvalidArgument(
                    "Class versions V1_5 or less must use F_NEW frames.".to_owned(),
                ));
            }
            let offset_delta;
            if self.stack_map_table_entries.is_none() {
                self.stack_map_table_entries = Some(ByteVector::new());
                offset_delta = self.code_length();
            } else {
                offset_delta = self.code_length() - self.previous_frame_offset - 1;
                if offset_delta < 0 {
                    if kind == FrameKind::Same {
                        return Ok(());
                    }
                    return Err(WriteError::IllegalState(String::new()));
                }
            }
            let entries = self
                .stack_map_table_entries
                .as_mut()
                .expect("created above");
            match kind {
                FrameKind::Full => {
                    self.current_locals = num_local;
                    entries
                        .put_byte(FULL_FRAME)
                        .put_short(offset_delta)
                        .put_short(num_local);
                    for value in local {
                        put_frame_type(symbol_table, &mut self.labels, entries, value);
                    }
                    entries.put_short(num_stack);
                    for value in stack {
                        put_frame_type(symbol_table, &mut self.labels, entries, value);
                    }
                }
                FrameKind::Append => {
                    self.current_locals += num_local;
                    entries
                        .put_byte(SAME_FRAME_EXTENDED + num_local)
                        .put_short(offset_delta);
                    for value in local {
                        put_frame_type(symbol_table, &mut self.labels, entries, value);
                    }
                }
                FrameKind::Chop(_) => {
                    self.current_locals -= num_local;
                    entries
                        .put_byte(SAME_FRAME_EXTENDED - num_local)
                        .put_short(offset_delta);
                }
                FrameKind::Same => {
                    if offset_delta < 64 {
                        entries.put_byte(offset_delta);
                    } else {
                        entries
                            .put_byte(SAME_FRAME_EXTENDED)
                            .put_short(offset_delta);
                    }
                }
                FrameKind::Same1 => {
                    if offset_delta < 64 {
                        entries.put_byte(SAME_LOCALS_1_STACK_ITEM_FRAME + offset_delta);
                    } else {
                        entries
                            .put_byte(SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED)
                            .put_short(offset_delta);
                    }
                    let Some(value) = stack.first() else {
                        return Err(WriteError::IndexOutOfBounds(
                            "Index 0 out of bounds for length 0".to_owned(),
                        ));
                    };
                    put_frame_type(symbol_table, &mut self.labels, entries, value);
                }
                FrameKind::New | FrameKind::Insert => {
                    return Err(WriteError::InvalidArgument(String::new()));
                }
            }
            self.previous_frame_offset = self.code_length();
            self.stack_map_table_number_of_entries += 1;
        }
        // COMPUTE_MAX_STACK_AND_LOCAL_FROM_FRAMES updates the relative stack size here (P6).
        self.max_stack = self.max_stack.max(num_stack);
        self.max_locals = self.max_locals.max(self.current_locals);
        Ok(())
    }

    /// The `COMPUTE_INSERTED_FRAMES` case of `visitFrame`: sets the frame of the current basic
    /// block, for an expanded frame, and writes it, for an expanded and an inserted frame.
    fn visit_inserted_frame(
        &mut self,
        symbol_table: &mut SymbolTable,
        kind: FrameKind,
        local: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) -> Result<(), WriteError> {
        let current_basic_block = self
            .current_basic_block
            .expect("set by the constructor when computing frames");
        let labels = &mut self.labels;
        let block = labels.block_mut(current_basic_block);
        if block.frame.is_none() {
            // This should happen only once, for the implicit first frame (which is explicitly
            // visited in ClassReader if the EXPAND_ASM_INSNS option is used - and
            // COMPUTE_INSERTED_FRAMES can't be set if EXPAND_ASM_INSNS is not used). Its number of
            // locals is the length of `local`.
            let mut frame = Frame::new(Some(current_basic_block));
            frame.set_input_frame_from_descriptor(
                symbol_table,
                self.access_flags,
                &self.descriptor,
                local.len(),
            )?;
            block.frame = Some(Box::new(frame));
        } else if kind == FrameKind::New {
            let mut frame = block.frame.take().expect("checked above");
            let result =
                frame.set_input_frame_from_api_format(symbol_table, local, stack, |label| {
                    labels.resolved_offset(label)
                });
            labels.block_mut(current_basic_block).frame = Some(frame);
            result?;
        }
        // If type is not F_NEW then it is F_INSERT by hypothesis, and currentBlock.frame
        // contains the stack map frame at the current instruction, computed from the last F_NEW
        // frame and the bytecode instructions in between (via calls to CurrentFrame#execute).
        let frame = self
            .labels
            .block(current_basic_block)
            .and_then(|block| block.frame.as_deref())
            .expect("set above");
        let owner_offset = frame.owner.map_or(0, |owner| self.labels.offset(owner));
        let (locals, stack) = frame.accept();
        self.accept_frame(symbol_table, owner_offset, &locals, &stack);
        Ok(())
    }

    /// Visits a frame computed by the frame computation (the end of ASM's `Frame.accept`).
    pub(crate) fn accept_frame(
        &mut self,
        symbol_table: &mut SymbolTable,
        offset: i32,
        locals: &[i32],
        stack: &[i32],
    ) {
        let frame_index = self.visit_frame_start(offset, locals.len() as i32, stack.len() as i32);
        for (i, &abstract_type) in locals.iter().chain(stack).enumerate() {
            self.current_frame[frame_index + i] = abstract_type;
        }
        self.visit_frame_end(symbol_table);
    }

    /// Starts the visit of a new stack map frame, stored in `current_frame`. Returns the index
    /// of the next element to be written in this frame.
    pub(crate) fn visit_frame_start(
        &mut self,
        offset: i32,
        num_local: i32,
        num_stack: i32,
    ) -> usize {
        self.current_frame.clear();
        self.current_frame
            .extend_from_slice(&[offset, num_local, num_stack]);
        self.current_frame
            .resize(3 + num_local.max(0) as usize + num_stack.max(0) as usize, 0);
        3
    }

    /// Ends the visit of `current_frame` by writing it in the StackMapTable entries and by
    /// updating the StackMapTable `number_of_entries` (except if the current frame is the first
    /// one, which is implicit in StackMapTable). Then resets `current_frame` to `None`.
    pub(crate) fn visit_frame_end(&mut self, symbol_table: &mut SymbolTable) {
        if self.previous_frame.is_some() {
            if self.stack_map_table_entries.is_none() {
                self.stack_map_table_entries = Some(ByteVector::new());
            }
            self.put_frame(symbol_table);
            self.stack_map_table_number_of_entries += 1;
        }
        self.previous_frame = Some(std::mem::take(&mut self.current_frame));
    }

    /// Compresses and writes `current_frame` in a new StackMapTable entry.
    fn put_frame(&mut self, symbol_table: &mut SymbolTable) {
        let current_frame = &self.current_frame;
        let previous_frame = self
            .previous_frame
            .as_deref()
            .expect("checked by the caller");
        let entries = self
            .stack_map_table_entries
            .as_mut()
            .expect("created by the caller");
        let labels = &mut self.labels;
        let num_local = current_frame[1];
        let num_stack = current_frame[2];
        let put_abstract_types = |entries: &mut ByteVector,
                                  labels: &mut LabelTable,
                                  st: &mut SymbolTable,
                                  start: i32,
                                  end: i32| {
            for &abstract_type in &current_frame[start as usize..end as usize] {
                if let Err(error) =
                    frame::put_abstract_type(st, abstract_type, entries, |label, output| {
                        labels.put_stack_map(label, output);
                    })
                {
                    st.fail(error);
                }
            }
        };
        if symbol_table.major_version() < opcodes::V1_6 as i32 {
            // Generate a StackMap attribute entry, which are always uncompressed.
            entries.put_short(current_frame[0]).put_short(num_local);
            put_abstract_types(entries, labels, symbol_table, 3, 3 + num_local);
            entries.put_short(num_stack);
            put_abstract_types(
                entries,
                labels,
                symbol_table,
                3 + num_local,
                3 + num_local + num_stack,
            );
            return;
        }
        let offset_delta = if self.stack_map_table_number_of_entries == 0 {
            current_frame[0]
        } else {
            current_frame[0] - previous_frame[0] - 1
        };
        let previous_num_local = previous_frame[1];
        let num_local_delta = num_local - previous_num_local;
        let mut frame_type = FULL_FRAME;
        if num_stack == 0 {
            match num_local_delta {
                -3..=-1 => frame_type = CHOP_FRAME,
                0 => {
                    frame_type = if offset_delta < 64 {
                        SAME_FRAME
                    } else {
                        SAME_FRAME_EXTENDED
                    };
                }
                1..=3 => frame_type = APPEND_FRAME,
                _ => {
                    // Keep the FULL_FRAME type.
                }
            }
        } else if num_local_delta == 0 && num_stack == 1 {
            frame_type = if offset_delta < 63 {
                SAME_LOCALS_1_STACK_ITEM_FRAME
            } else {
                SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED
            };
        }
        if frame_type != FULL_FRAME {
            // Verify if locals are the same as in the previous frame.
            let mut frame_index = 3;
            let mut i = 0;
            while i < previous_num_local && i < num_local {
                if current_frame[frame_index] != previous_frame[frame_index] {
                    frame_type = FULL_FRAME;
                    break;
                }
                frame_index += 1;
                i += 1;
            }
        }
        match frame_type {
            SAME_FRAME => {
                entries.put_byte(offset_delta);
            }
            SAME_LOCALS_1_STACK_ITEM_FRAME => {
                entries.put_byte(SAME_LOCALS_1_STACK_ITEM_FRAME + offset_delta);
                put_abstract_types(entries, labels, symbol_table, 3 + num_local, 4 + num_local);
            }
            SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED => {
                entries
                    .put_byte(SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED)
                    .put_short(offset_delta);
                put_abstract_types(entries, labels, symbol_table, 3 + num_local, 4 + num_local);
            }
            SAME_FRAME_EXTENDED => {
                entries
                    .put_byte(SAME_FRAME_EXTENDED)
                    .put_short(offset_delta);
            }
            CHOP_FRAME => {
                entries
                    .put_byte(SAME_FRAME_EXTENDED + num_local_delta)
                    .put_short(offset_delta);
            }
            APPEND_FRAME => {
                entries
                    .put_byte(SAME_FRAME_EXTENDED + num_local_delta)
                    .put_short(offset_delta);
                put_abstract_types(
                    entries,
                    labels,
                    symbol_table,
                    3 + previous_num_local,
                    3 + num_local,
                );
            }
            _ => {
                entries
                    .put_byte(FULL_FRAME)
                    .put_short(offset_delta)
                    .put_short(num_local);
                put_abstract_types(entries, labels, symbol_table, 3, 3 + num_local);
                entries.put_short(num_stack);
                put_abstract_types(
                    entries,
                    labels,
                    symbol_table,
                    3 + num_local,
                    3 + num_local + num_stack,
                );
            }
        }
    }
}

/// Puts the given public API frame element type in the StackMapTable entries (ASM's
/// `putFrameType`).
fn put_frame_type(
    symbol_table: &mut SymbolTable,
    labels: &mut LabelTable,
    entries: &mut ByteVector,
    value: &FrameValue<'_>,
) {
    match value {
        FrameValue::Object(internal_name) => {
            let index = symbol_table.add_constant_class(internal_name).index;
            entries.put_byte(ITEM_OBJECT).put_short(index);
        }
        FrameValue::Uninitialized(label) => {
            entries.put_byte(ITEM_UNINITIALIZED);
            labels.put_stack_map(*label, entries);
        }
        _ => {
            entries.put_byte(frame::api_integer_value(value).expect("a constant frame value"));
        }
    }
}
