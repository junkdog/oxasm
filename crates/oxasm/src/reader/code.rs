//! ASM's `ClassReader` section "Methods to parse a Code attribute".

use crate::constant::{Constant, FrameValue};
use crate::constants::{
    self, ALOAD_3, ASM_GOTO_W, ASM_IFEQ, ASM_IFNONNULL, ASM_IFNULL, ASM_IFNULL_OPCODE_DELTA,
    ASM_JSR, ASM_OPCODE_DELTA, ASTORE_3, GOTO_W, ILOAD_0, ISTORE_0, ITEM_UNINITIALIZED, JSR_W,
    LDC_W, LDC2_W, WIDE, WIDE_JUMP_OPCODE_DELTA,
};
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::opcodes::{
    ALOAD, ANEWARRAY, ARRAYLENGTH, ASTORE, ATHROW, BIPUSH, CHECKCAST, DCMPG, DCONST_1, FrameKind,
    GETSTATIC, GOTO, I2L, I2S, IADD, IALOAD, IASTORE, IF_ACMPNE, IFEQ, IFNONNULL, IFNULL, IINC,
    ILOAD, INSTANCEOF, INVOKEDYNAMIC, INVOKEINTERFACE, INVOKESTATIC, INVOKEVIRTUAL, IRETURN,
    ISTORE, JSR, LCMP, LDC, LOOKUPSWITCH, LXOR, MONITORENTER, MONITOREXIT, MULTIANEWARRAY, NEW,
    NEWARRAY, NOP, POP, RET, RETURN, SALOAD, SASTORE, SIPUSH, SWAP, TABLESWITCH,
};
use crate::type_reference::TypeReference;
use crate::visitor::MethodVisitor;

use super::context::Context;
use super::frames::{F_APPEND, F_CHOP, F_FULL, F_SAME, F_SAME1};
use super::{
    CONSTANT_INTERFACE_METHODREF_TAG, ClassReader, ParsingOptions, ReadError, ReadErrorKind,
    required,
};

/// The name of the attribute of CLDC's preverified classes (ASM's `"StackMap"`).
const STACK_MAP: &JavaStr = crate::jstr!("StackMap");

/// `labels[bytecode_offset]`, which must exist.
fn label_slot(
    bytecode_offset: i64,
    labels: &mut [Option<Label>],
) -> Result<&mut Option<Label>, ReadError> {
    usize::try_from(bytecode_offset)
        .ok()
        .and_then(|i| labels.get_mut(i))
        .ok_or_else(|| ReadError::new(0, ReadErrorKind::InvalidBytecodeOffset(bytecode_offset)))
}

/// Returns the label corresponding to the given bytecode offset. The default implementation of
/// this method creates a label for the given offset if it has not been already created (ASM's
/// `readLabel`).
pub(super) fn read_label(
    bytecode_offset: i64,
    labels: &mut [Option<Label>],
) -> Result<Label, ReadError> {
    Ok(*label_slot(bytecode_offset, labels)?.get_or_insert_with(Label::new))
}

/// Creates a label without the `FLAG_DEBUG_ONLY` flag set, for the given bytecode offset. The
/// label is created with a call to [`read_label`] and its `FLAG_DEBUG_ONLY` flag is cleared.
pub(super) fn create_label(
    bytecode_offset: i64,
    labels: &mut [Option<Label>],
) -> Result<Label, ReadError> {
    let label = read_label(bytecode_offset, labels)?.with_debug_only(false);
    *label_slot(bytecode_offset, labels)? = Some(label);
    Ok(label)
}

/// Creates a label with the `FLAG_DEBUG_ONLY` flag set, if there is no already existing label
/// for the given bytecode offset (otherwise does nothing). The label is created with a call to
/// [`read_label`].
fn create_debug_label(bytecode_offset: i64, labels: &mut [Option<Label>]) -> Result<(), ReadError> {
    let slot = label_slot(bytecode_offset, labels)?;
    if slot.is_none() {
        *slot = Some(Label::new().with_debug_only(true));
    }
    Ok(())
}

/// `labels[bytecode_offset]` in the second pass, where the first pass created every label that
/// an instruction refers to.
fn label_at(bytecode_offset: i64, labels: &[Option<Label>]) -> Result<Label, ReadError> {
    usize::try_from(bytecode_offset)
        .ok()
        .and_then(|i| labels.get(i).copied().flatten())
        .ok_or_else(|| ReadError::new(0, ReadErrorKind::InvalidBytecodeOffset(bytecode_offset)))
}

/// The number of elements of a Java array whose size comes from the class file, if it can be
/// allocated: `count` elements of `element_size` bytes each must fit in the class file from
/// `offset` (otherwise reading them fails anyway, so allocating them is pointless).
fn array_capacity(
    reader: &ClassReader<'_>,
    count: i32,
    element_size: i64,
    offset: i64,
) -> Result<usize, ReadError> {
    // Java: NegativeArraySizeException.
    let count_usize = usize::try_from(count)
        .map_err(|_| ReadError::new(offset, ReadErrorKind::InvalidLength(count.into())))?;
    let available = (reader.class_file.len() as i64 - offset).max(0) / element_size;
    Ok(count_usize.min(available as usize))
}

impl<'b> ClassReader<'b> {
    /// Reads a JVMS 'Code' attribute and makes the given visitor visit it.
    ///
    /// - `code_offset`: the start offset in the class file of the Code attribute, excluding its
    ///   attribute_name_index and attribute_length fields.
    pub(super) fn read_code<'s>(
        &'s self,
        method_visitor: &mut dyn MethodVisitor,
        context: &mut Context<'s>,
        code_offset: i64,
    ) -> Result<(), ReadError> {
        let mut current_offset = code_offset;

        // Read the max_stack, max_locals and code_length fields.
        let max_stack = self.u16_at(current_offset)?;
        let max_locals = self.u16_at(current_offset + 2)?;
        let code_length = self.i32_at(current_offset + 4)?;
        current_offset += 8;
        if i64::from(code_length) > self.class_file.len() as i64 - current_offset {
            return Err(ReadError::new(
                current_offset - 4,
                ReadErrorKind::InvalidLength(code_length.into()),
            ));
        }
        let code_length = i64::from(code_length);

        // Read the bytecode 'code' array to create a label for each referenced instruction.
        let bytecode_start_offset = current_offset;
        let bytecode_end_offset = current_offset + code_length;
        // `new Label[codeLength + 1]`.
        let labels_length = usize::try_from(code_length + 1).map_err(|_| {
            ReadError::new(
                current_offset - 4,
                ReadErrorKind::InvalidLength(code_length),
            )
        })?;
        method_visitor.visit_code_length(u32::try_from(code_length).unwrap_or(0));
        context.current_method_labels.clear();
        context.current_method_labels.resize(labels_length, None);
        context.current_method_line_numbers.clear();
        let labels = &mut context.current_method_labels;
        while current_offset < bytecode_end_offset {
            let bytecode_offset = current_offset - bytecode_start_offset;
            let opcode = self.u8_at(current_offset)?;
            match opcode {
                NOP..=DCONST_1
                | IALOAD..=SALOAD
                | IASTORE..=SASTORE
                | POP..=SWAP
                | IADD..=LXOR
                | I2L..=I2S
                | LCMP..=DCMPG
                | IRETURN..=RETURN
                | ARRAYLENGTH
                | ATHROW
                | MONITORENTER
                | MONITOREXIT
                | ILOAD_0..=ALOAD_3
                | ISTORE_0..=ASTORE_3 => {
                    current_offset += 1;
                }
                IFEQ..=IF_ACMPNE | GOTO | JSR | IFNULL | IFNONNULL => {
                    let branch = i64::from(self.i16_at(current_offset + 1)?);
                    create_label(bytecode_offset + branch, labels)?;
                    current_offset += 3;
                }
                ASM_IFEQ..=ASM_JSR | ASM_IFNULL | ASM_IFNONNULL => {
                    let branch = i64::from(self.u16_at(current_offset + 1)?);
                    create_label(bytecode_offset + branch, labels)?;
                    current_offset += 3;
                }
                GOTO_W | JSR_W | ASM_GOTO_W => {
                    let branch = i64::from(self.i32_at(current_offset + 1)?);
                    create_label(bytecode_offset + branch, labels)?;
                    current_offset += 5;
                }
                WIDE => match self.u8_at(current_offset + 1)? {
                    ILOAD..=ALOAD | ISTORE..=ASTORE | RET => {
                        current_offset += 4;
                    }
                    IINC => {
                        current_offset += 6;
                    }
                    wide_opcode => {
                        return Err(ReadError::new(
                            current_offset + 1,
                            ReadErrorKind::InvalidWideOpcode(wide_opcode),
                        ));
                    }
                },
                TABLESWITCH => {
                    // Skip 0 to 3 padding bytes.
                    current_offset += 4 - (bytecode_offset & 3);
                    // Read the default label and the number of table entries.
                    let default = i64::from(self.i32_at(current_offset)?);
                    create_label(bytecode_offset + default, labels)?;
                    let high = self.i32_at(current_offset + 8)?;
                    let low = self.i32_at(current_offset + 4)?;
                    let mut num_table_entries = high.wrapping_sub(low).wrapping_add(1);
                    current_offset += 12;
                    // Read the table labels.
                    while num_table_entries > 0 {
                        num_table_entries -= 1;
                        let branch = i64::from(self.i32_at(current_offset)?);
                        create_label(bytecode_offset + branch, labels)?;
                        current_offset += 4;
                    }
                }
                LOOKUPSWITCH => {
                    // Skip 0 to 3 padding bytes.
                    current_offset += 4 - (bytecode_offset & 3);
                    // Read the default label and the number of switch cases.
                    let default = i64::from(self.i32_at(current_offset)?);
                    create_label(bytecode_offset + default, labels)?;
                    let mut num_switch_cases = self.i32_at(current_offset + 4)?;
                    current_offset += 8;
                    // Read the switch labels.
                    while num_switch_cases > 0 {
                        num_switch_cases -= 1;
                        let branch = i64::from(self.i32_at(current_offset + 4)?);
                        create_label(bytecode_offset + branch, labels)?;
                        current_offset += 8;
                    }
                }
                ILOAD..=ALOAD | ISTORE..=ASTORE | RET | BIPUSH | NEWARRAY | LDC => {
                    current_offset += 2;
                }
                SIPUSH
                | LDC_W
                | LDC2_W
                | GETSTATIC..=INVOKESTATIC
                | NEW
                | ANEWARRAY
                | CHECKCAST
                | INSTANCEOF
                | IINC => {
                    current_offset += 3;
                }
                INVOKEINTERFACE | INVOKEDYNAMIC => {
                    current_offset += 5;
                }
                MULTIANEWARRAY => {
                    current_offset += 4;
                }
                _ => {
                    return Err(ReadError::new(
                        current_offset,
                        ReadErrorKind::InvalidOpcode(opcode),
                    ));
                }
            }
        }

        // Read the 'exception_table_length' and 'exception_table' field to create a label for
        // each referenced instruction, and to make method_visitor visit the corresponding try
        // catch blocks.
        let exception_table_length = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..exception_table_length {
            let start = create_label(self.u16_at(current_offset)?.into(), labels)?;
            let end = create_label(self.u16_at(current_offset + 2)?.into(), labels)?;
            let handler = create_label(self.u16_at(current_offset + 4)?.into(), labels)?;
            let catch_type_index = usize::from(self.u16_at(current_offset + 6)?);
            let catch_type = self.utf8_at(self.cp_info_offset(catch_type_index)?)?;
            current_offset += 8;
            method_visitor.visit_try_catch_block(start, end, handler, catch_type);
        }

        // Read the Code attributes to create a label for each referenced instruction (the
        // variables are ordered as in Section 4.7 of the JVMS). Attribute offsets exclude the
        // attribute_name_index and attribute_length fields.
        // - The offset of the current 'stack_map_frame' in the StackMap[Table] attribute, or 0.
        // Initially, this is the offset of the first 'stack_map_frame' entry. Then this offset is
        // updated after each stack_map_frame is read.
        let mut stack_map_frame_offset: i64 = 0;
        // - The end offset of the StackMap[Table] attribute, or 0.
        let mut stack_map_table_end_offset: i64 = 0;
        // - Whether the stack map frames are compressed (i.e. in a StackMapTable) or not.
        let mut compressed_frames = true;
        // - The offset of the LocalVariableTable attribute, or 0.
        let mut local_variable_table_offset: i64 = 0;
        // - The offset of the LocalVariableTypeTable attribute, or 0.
        let mut local_variable_type_table_offset: i64 = 0;
        // - The offset of each 'type_annotation' entry in the RuntimeVisibleTypeAnnotations
        // attribute, or None.
        let mut visible_type_annotation_offsets: Option<Vec<i64>> = None;
        // - The offset of each 'type_annotation' entry in the RuntimeInvisibleTypeAnnotations
        // attribute, or None.
        let mut invisible_type_annotation_offsets: Option<Vec<i64>> = None;
        // - The non standard attributes, in the order of the ClassFile structure.
        let mut attributes = Vec::new();

        let parsing_options = context.parsing_options;
        let skip_debug = parsing_options.contains(ParsingOptions::SKIP_DEBUG);
        let skip_frames = parsing_options.contains(ParsingOptions::SKIP_FRAMES);
        let attributes_count = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..attributes_count {
            // Read the attribute_info's attribute_name and attribute_length fields.
            let attribute_name = self.utf8_at(current_offset)?;
            let attribute_length = self.i32_at(current_offset + 2)?;
            current_offset += 6;
            match attribute_name {
                Some(n) if n == constants::LOCAL_VARIABLE_TABLE => {
                    if !skip_debug {
                        local_variable_table_offset = current_offset;
                        // Parse the attribute to find the corresponding (debug only) labels.
                        let mut current_local_variable_table_offset = current_offset;
                        let local_variable_table_length =
                            self.u16_at(current_local_variable_table_offset)?;
                        current_local_variable_table_offset += 2;
                        for _ in 0..local_variable_table_length {
                            let start_pc =
                                i64::from(self.u16_at(current_local_variable_table_offset)?);
                            create_debug_label(start_pc, &mut context.current_method_labels)?;
                            let length =
                                i64::from(self.u16_at(current_local_variable_table_offset + 2)?);
                            create_debug_label(
                                start_pc + length,
                                &mut context.current_method_labels,
                            )?;
                            // Skip the name_index, descriptor_index and index fields (2 bytes
                            // each).
                            current_local_variable_table_offset += 10;
                        }
                    }
                }
                Some(n) if n == constants::LOCAL_VARIABLE_TYPE_TABLE => {
                    local_variable_type_table_offset = current_offset;
                    // Here we do not extract the labels corresponding to the attribute content.
                    // We assume they are the same or a subset of those of the
                    // LocalVariableTable attribute.
                }
                Some(n) if n == constants::LINE_NUMBER_TABLE => {
                    if !skip_debug {
                        // Parse the attribute to find the corresponding (debug only) labels.
                        let mut current_line_number_table_offset = current_offset;
                        let line_number_table_length =
                            self.u16_at(current_line_number_table_offset)?;
                        current_line_number_table_offset += 2;
                        for _ in 0..line_number_table_length {
                            let start_pc = self.u16_at(current_line_number_table_offset)?;
                            let line_number = self.u16_at(current_line_number_table_offset + 2)?;
                            current_line_number_table_offset += 4;
                            create_debug_label(
                                i64::from(start_pc),
                                &mut context.current_method_labels,
                            )?;
                            context
                                .current_method_line_numbers
                                .entry(usize::from(start_pc))
                                .or_default()
                                .push(line_number);
                        }
                    }
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    visible_type_annotation_offsets = Some(self.read_type_annotations(
                        method_visitor,
                        context,
                        current_offset,
                        /* visible= */ true,
                    )?);
                    // Here we do not extract the labels corresponding to the attribute content.
                    // This would require a full parsing of the attribute, which would need to be
                    // repeated when parsing the bytecode instructions (see below). Instead, the
                    // content of the attribute is read one type annotation at a time (i.e. after
                    // a type annotation has been visited, the next type annotation is read), and
                    // the labels it contains are also extracted one annotation at a time. This
                    // assumes that type annotations are ordered by increasing bytecode offset.
                }
                Some(n) if n == constants::RUNTIME_INVISIBLE_TYPE_ANNOTATIONS => {
                    invisible_type_annotation_offsets = Some(self.read_type_annotations(
                        method_visitor,
                        context,
                        current_offset,
                        /* visible= */ false,
                    )?);
                    // Same comment as above for the RuntimeVisibleTypeAnnotations attribute.
                }
                Some(n) if n == constants::STACK_MAP_TABLE => {
                    if !skip_frames {
                        stack_map_frame_offset = current_offset + 2;
                        stack_map_table_end_offset = current_offset + i64::from(attribute_length);
                    }
                    // Here we do not extract the labels corresponding to the attribute content.
                    // This would require a full parsing of the attribute, which would need to be
                    // repeated when parsing the bytecode instructions (see below). Instead, the
                    // content of the attribute is read one frame at a time (i.e. after a frame
                    // has been visited, the next frame is read), and the labels it contains are
                    // also extracted one frame at a time. Thanks to the ordering of frames,
                    // having only a "one frame lookahead" is not a problem, i.e. it is not
                    // possible to see an offset smaller than the offset of the current
                    // instruction and for which no Label exist. Except for UNINITIALIZED type
                    // offsets. We solve this by parsing the stack map table without a full
                    // decoding (see below).
                }
                Some(n) if n == STACK_MAP => {
                    if !skip_frames {
                        stack_map_frame_offset = current_offset + 2;
                        stack_map_table_end_offset = current_offset + i64::from(attribute_length);
                        compressed_frames = false;
                    }
                    // IMPORTANT! Here we assume that the frames are ordered, as in the
                    // StackMapTable attribute, although this is not guaranteed by the attribute
                    // format. This allows an incremental extraction of the labels corresponding
                    // to this attribute (see the comment above for the StackMapTable attribute).
                }
                _ => {
                    attributes.push(self.read_attribute(
                        context.codecs,
                        attribute_name,
                        current_offset,
                        attribute_length,
                        Some((code_offset, &mut context.current_method_labels)),
                    )?);
                }
            }
            current_offset += i64::from(attribute_length);
        }

        // Initialize the context fields related to stack map frames, and generate the first
        // (implicit) stack map frame, if needed.
        let expand_frames = parsing_options.contains(ParsingOptions::EXPAND_FRAMES);
        if stack_map_frame_offset != 0 {
            // The bytecode offset of the first explicit frame is not offset_delta + 1 but only
            // offset_delta. Setting the implicit frame offset to -1 allows us to use of the
            // "offset_delta + 1" rule in all cases.
            context.current_frame_offset = -1;
            context.current_frame_type = 0;
            context.current_frame_local_count = 0;
            context.current_frame_local_count_delta = 0;
            context.current_frame_local_types.clear();
            context.current_frame_local_capacity = usize::from(max_locals);
            context.current_frame_stack_count = 0;
            context.current_frame_stack_types.clear();
            context.current_frame_stack_capacity = usize::from(max_stack);
            if expand_frames {
                self.compute_implicit_frame(context, code_offset)?;
            }
            // Find the labels for UNINITIALIZED frame types. Instead of decoding each element of
            // the stack map table, we look for 3 consecutive bytes that "look like" an
            // UNINITIALIZED type (tag ITEM_Uninitialized, offset within bytecode bounds, NEW
            // instruction at this offset). We may find false positives (i.e. not real
            // UNINITIALIZED types), but this should be rare, and the only consequence will be
            // the creation of an unneeded label. This is better than creating a label for each
            // NEW instruction, and faster than fully decoding the whole stack map table.
            let mut offset = stack_map_frame_offset;
            while offset < stack_map_table_end_offset - 2 {
                if self.u8_at(offset)? == ITEM_UNINITIALIZED {
                    let potential_bytecode_offset = i64::from(self.u16_at(offset + 1)?);
                    if potential_bytecode_offset < code_length
                        && self.u8_at(bytecode_start_offset + potential_bytecode_offset)? == NEW
                    {
                        create_label(
                            potential_bytecode_offset,
                            &mut context.current_method_labels,
                        )?;
                    }
                }
                offset += 1;
            }
        }
        if expand_frames && parsing_options.contains(ParsingOptions::EXPAND_ASM_INSNS) {
            // Expanding the ASM specific instructions can introduce F_INSERT frames, even if the
            // method does not currently have any frame. These inserted frames must be computed by
            // simulating the effect of the bytecode instructions, one by one, starting from the
            // implicit first frame. For this, MethodWriter needs to know maxLocals before the
            // first instruction is visited. To ensure this, we visit the implicit first frame
            // here (passing only maxLocals - the rest is computed in MethodWriter). ASM passes
            // (F_NEW, maxLocals, null, 0, null); the count travels in the length of the locals.
            let locals = vec![FrameValue::Top; usize::from(max_locals)];
            method_visitor.visit_frame(FrameKind::New, &locals, &[]);
        }

        // Visit the bytecode instructions. First, introduce state variables for the incremental
        // parsing of the type annotations.

        // Index of the next runtime visible type annotation to read (in the
        // visible_type_annotation_offsets array).
        let mut current_visible_type_annotation_index = 0;
        // The bytecode offset of the next runtime visible type annotation to read, or -1.
        let mut current_visible_type_annotation_bytecode_offset = self
            .get_type_annotation_bytecode_offset(visible_type_annotation_offsets.as_deref(), 0)?;
        // Index of the next runtime invisible type annotation to read (in the
        // invisible_type_annotation_offsets array).
        let mut current_invisible_type_annotation_index = 0;
        // The bytecode offset of the next runtime invisible type annotation to read, or -1.
        let mut current_invisible_type_annotation_bytecode_offset = self
            .get_type_annotation_bytecode_offset(invisible_type_annotation_offsets.as_deref(), 0)?;

        // Whether a F_INSERT stack map frame must be inserted before the current instruction.
        let mut insert_frame = false;

        // The delta to subtract from a goto_w or jsr_w opcode to get the corresponding goto or
        // jsr opcode, or 0 if goto_w and jsr_w must be left unchanged (i.e. when expanding ASM
        // specific instructions).
        let wide_jump_opcode_delta = if parsing_options.contains(ParsingOptions::EXPAND_ASM_INSNS) {
            0
        } else {
            WIDE_JUMP_OPCODE_DELTA
        };

        current_offset = bytecode_start_offset;
        while current_offset < bytecode_end_offset {
            let current_bytecode_offset = current_offset - bytecode_start_offset;

            // Visit the label and the line number(s) for this bytecode offset, if any.
            let labels = &context.current_method_labels;
            if let Some(current_label) = labels
                .get(current_bytecode_offset as usize)
                .copied()
                .flatten()
            {
                method_visitor.visit_label(current_label);
                if !skip_debug
                    && let Some(line_numbers) = context
                        .current_method_line_numbers
                        .get(&(current_bytecode_offset as usize))
                {
                    for &line_number in line_numbers {
                        method_visitor.visit_line_number(line_number, current_label);
                    }
                }
            }

            // Visit the stack map frame for this bytecode offset, if any.
            while stack_map_frame_offset != 0
                && (i64::from(context.current_frame_offset) == current_bytecode_offset
                    || context.current_frame_offset == -1)
            {
                // If there is a stack map frame for this offset, make method_visitor visit it,
                // and read the next stack map frame if there is one.
                if context.current_frame_offset != -1 {
                    visit_frame(method_visitor, context, !compressed_frames || expand_frames)?;
                    // Since there is already a stack map frame for this bytecode offset, there is
                    // no need to insert a new one.
                    insert_frame = false;
                }
                if stack_map_frame_offset < stack_map_table_end_offset {
                    stack_map_frame_offset = self.read_stack_map_frame(
                        stack_map_frame_offset,
                        compressed_frames,
                        expand_frames,
                        context,
                    )?;
                } else {
                    stack_map_frame_offset = 0;
                }
            }

            // Insert a stack map frame for this bytecode offset, if requested by setting
            // insert_frame to true during the previous iteration. The actual frame content is
            // computed in MethodWriter.
            if insert_frame {
                if expand_frames {
                    method_visitor.visit_frame(FrameKind::Insert, &[], &[]);
                }
                insert_frame = false;
            }

            // Visit the instruction at this bytecode offset.
            let labels = &context.current_method_labels;
            let opcode = self.u8_at(current_offset)?;
            match opcode {
                NOP..=DCONST_1
                | IALOAD..=SALOAD
                | IASTORE..=SASTORE
                | POP..=SWAP
                | IADD..=LXOR
                | I2L..=I2S
                | LCMP..=DCMPG
                | IRETURN..=RETURN
                | ARRAYLENGTH
                | ATHROW
                | MONITORENTER
                | MONITOREXIT => {
                    method_visitor.visit_insn(opcode);
                    current_offset += 1;
                }
                ILOAD_0..=ALOAD_3 => {
                    let opcode = opcode - ILOAD_0;
                    method_visitor.visit_var_insn(ILOAD + (opcode >> 2), u16::from(opcode & 0x3));
                    current_offset += 1;
                }
                ISTORE_0..=ASTORE_3 => {
                    let opcode = opcode - ISTORE_0;
                    method_visitor.visit_var_insn(ISTORE + (opcode >> 2), u16::from(opcode & 0x3));
                    current_offset += 1;
                }
                IFEQ..=IF_ACMPNE | GOTO | JSR | IFNULL | IFNONNULL => {
                    let branch = i64::from(self.i16_at(current_offset + 1)?);
                    let label = label_at(current_bytecode_offset + branch, labels)?;
                    method_visitor.visit_jump_insn(opcode, label);
                    current_offset += 3;
                }
                GOTO_W | JSR_W => {
                    let branch = i64::from(self.i32_at(current_offset + 1)?);
                    let label = label_at(current_bytecode_offset + branch, labels)?;
                    method_visitor.visit_jump_insn(opcode - wide_jump_opcode_delta, label);
                    current_offset += 5;
                }
                ASM_IFEQ..=ASM_JSR | ASM_IFNULL | ASM_IFNONNULL => {
                    // A forward jump with an offset > 32767. In this case we automatically
                    // replace ASM_GOTO with GOTO_W, ASM_JSR with JSR_W and ASM_IFxxx <l> with
                    // IFNOTxxx <L> GOTO_W <l> L:..., where IFNOTxxx is the "opposite" opcode of
                    // ASMS_IFxxx (e.g. IFNE for ASM_IFEQ) and where <L> designates the
                    // instruction just after the GOTO_W.
                    // First, change the ASM specific opcodes ASM_IFEQ ... ASM_JSR, ASM_IFNULL and
                    // ASM_IFNONNULL to IFEQ ... JSR, IFNULL and IFNONNULL.
                    let mut opcode = if opcode < ASM_IFNULL {
                        opcode - ASM_OPCODE_DELTA
                    } else {
                        opcode - ASM_IFNULL_OPCODE_DELTA
                    };
                    let branch = i64::from(self.u16_at(current_offset + 1)?);
                    let target = label_at(current_bytecode_offset + branch, labels)?;
                    if opcode == GOTO || opcode == JSR {
                        // Replace GOTO with GOTO_W and JSR with JSR_W.
                        method_visitor.visit_jump_insn(opcode + WIDE_JUMP_OPCODE_DELTA, target);
                    } else {
                        // Compute the "opposite" of opcode. This can be done by flipping the
                        // least significant bit for IFNULL and IFNONNULL, and similarly for IFEQ
                        // ... IF_ACMPEQ (with a pre and post offset by 1).
                        opcode = if opcode < GOTO {
                            ((opcode + 1) ^ 1) - 1
                        } else {
                            opcode ^ 1
                        };
                        let endif = create_label(
                            current_bytecode_offset + 3,
                            &mut context.current_method_labels,
                        )?;
                        method_visitor.visit_jump_insn(opcode, endif);
                        method_visitor.visit_jump_insn(GOTO_W, target);
                        // endif designates the instruction just after GOTO_W, and is visited as
                        // part of the next instruction. Since it is a jump target, we need to
                        // insert a frame here.
                        insert_frame = true;
                    }
                    current_offset += 3;
                }
                ASM_GOTO_W => {
                    // Replace ASM_GOTO_W with GOTO_W.
                    let branch = i64::from(self.i32_at(current_offset + 1)?);
                    let label = label_at(current_bytecode_offset + branch, labels)?;
                    method_visitor.visit_jump_insn(GOTO_W, label);
                    // The instruction just after is a jump target (because ASM_GOTO_W is used in
                    // patterns IFNOTxxx <L> ASM_GOTO_W <l> L:..., see MethodWriter), so we need
                    // to insert a frame here.
                    insert_frame = true;
                    current_offset += 5;
                }
                WIDE => {
                    let opcode = self.u8_at(current_offset + 1)?;
                    if opcode == IINC {
                        method_visitor.visit_iinc_insn(
                            self.u16_at(current_offset + 2)?,
                            self.i16_at(current_offset + 4)?,
                        );
                        current_offset += 6;
                    } else {
                        method_visitor.visit_var_insn(opcode, self.u16_at(current_offset + 2)?);
                        current_offset += 4;
                    }
                }
                TABLESWITCH => {
                    // Skip 0 to 3 padding bytes.
                    current_offset += 4 - (current_bytecode_offset & 3);
                    // Read the instruction.
                    let default = i64::from(self.i32_at(current_offset)?);
                    let default_label = label_at(current_bytecode_offset + default, labels)?;
                    let low = self.i32_at(current_offset + 4)?;
                    let high = self.i32_at(current_offset + 8)?;
                    current_offset += 12;
                    let length = high.wrapping_sub(low).wrapping_add(1);
                    let mut table =
                        Vec::with_capacity(array_capacity(self, length, 4, current_offset)?);
                    for _ in 0..length {
                        let branch = i64::from(self.i32_at(current_offset)?);
                        table.push(label_at(current_bytecode_offset + branch, labels)?);
                        current_offset += 4;
                    }
                    method_visitor.visit_table_switch_insn(low, high, default_label, &table);
                }
                LOOKUPSWITCH => {
                    // Skip 0 to 3 padding bytes.
                    current_offset += 4 - (current_bytecode_offset & 3);
                    // Read the instruction.
                    let default = i64::from(self.i32_at(current_offset)?);
                    let default_label = label_at(current_bytecode_offset + default, labels)?;
                    let num_pairs = self.i32_at(current_offset + 4)?;
                    current_offset += 8;
                    let capacity = array_capacity(self, num_pairs, 8, current_offset)?;
                    let mut keys = Vec::with_capacity(capacity);
                    let mut values = Vec::with_capacity(capacity);
                    for _ in 0..num_pairs {
                        keys.push(self.i32_at(current_offset)?);
                        let branch = i64::from(self.i32_at(current_offset + 4)?);
                        values.push(label_at(current_bytecode_offset + branch, labels)?);
                        current_offset += 8;
                    }
                    method_visitor.visit_lookup_switch_insn(default_label, &keys, &values);
                }
                ILOAD..=ALOAD | ISTORE..=ASTORE | RET => {
                    method_visitor
                        .visit_var_insn(opcode, u16::from(self.u8_at(current_offset + 1)?));
                    current_offset += 2;
                }
                BIPUSH | NEWARRAY => {
                    method_visitor
                        .visit_int_insn(opcode, i32::from(self.i8_at(current_offset + 1)?));
                    current_offset += 2;
                }
                SIPUSH => {
                    method_visitor
                        .visit_int_insn(opcode, i32::from(self.i16_at(current_offset + 1)?));
                    current_offset += 3;
                }
                LDC => {
                    let index = usize::from(self.u8_at(current_offset + 1)?);
                    method_visitor.visit_ldc_insn(&self.const_at(index)?);
                    current_offset += 2;
                }
                LDC_W | LDC2_W => {
                    let index = usize::from(self.u16_at(current_offset + 1)?);
                    method_visitor.visit_ldc_insn(&self.const_at(index)?);
                    current_offset += 3;
                }
                GETSTATIC..=INVOKEINTERFACE => {
                    let cp_info_offset =
                        self.cp_info_offset(usize::from(self.u16_at(current_offset + 1)?))?;
                    let name_and_type_cp_info_offset =
                        self.cp_info_offset(usize::from(self.u16_at(cp_info_offset + 2)?))?;
                    let owner = self.read_class_at(cp_info_offset)?;
                    let name = self.utf8_at(name_and_type_cp_info_offset)?;
                    let descriptor = self.utf8_at(name_and_type_cp_info_offset + 2)?;
                    let owner = required(owner, cp_info_offset)?;
                    let name = required(name, name_and_type_cp_info_offset)?;
                    let descriptor = required(descriptor, name_and_type_cp_info_offset + 2)?;
                    if opcode < INVOKEVIRTUAL {
                        method_visitor.visit_field_insn(opcode, owner, name, descriptor);
                    } else {
                        let is_interface =
                            self.u8_at(cp_info_offset - 1)? == CONSTANT_INTERFACE_METHODREF_TAG;
                        method_visitor.visit_method_insn(
                            opcode,
                            owner,
                            name,
                            descriptor,
                            is_interface,
                        );
                    }
                    if opcode == INVOKEINTERFACE {
                        current_offset += 5;
                    } else {
                        current_offset += 3;
                    }
                }
                INVOKEDYNAMIC => {
                    let cp_info_offset =
                        self.cp_info_offset(usize::from(self.u16_at(current_offset + 1)?))?;
                    let name_and_type_cp_info_offset =
                        self.cp_info_offset(usize::from(self.u16_at(cp_info_offset + 2)?))?;
                    let name = self.utf8_at(name_and_type_cp_info_offset)?;
                    let descriptor = self.utf8_at(name_and_type_cp_info_offset + 2)?;
                    let mut bootstrap_method_offset =
                        self.bootstrap_method_offset(self.u16_at(cp_info_offset)?, current_offset)?;
                    let handle_index = usize::from(self.u16_at(bootstrap_method_offset)?);
                    let Constant::Handle(handle) = self.const_at(handle_index)? else {
                        return Err(ReadError::new(
                            bootstrap_method_offset,
                            ReadErrorKind::InvalidConstant(handle_index as u16),
                        ));
                    };
                    let num_arguments = self.u16_at(bootstrap_method_offset + 2)?;
                    let mut bootstrap_method_arguments =
                        Vec::with_capacity(usize::from(num_arguments));
                    bootstrap_method_offset += 4;
                    for _ in 0..num_arguments {
                        let argument_index = usize::from(self.u16_at(bootstrap_method_offset)?);
                        bootstrap_method_arguments.push(self.const_at(argument_index)?);
                        bootstrap_method_offset += 2;
                    }
                    method_visitor.visit_invoke_dynamic_insn(
                        required(name, name_and_type_cp_info_offset)?,
                        required(descriptor, name_and_type_cp_info_offset + 2)?,
                        &handle,
                        &bootstrap_method_arguments,
                    );
                    current_offset += 5;
                }
                NEW | ANEWARRAY | CHECKCAST | INSTANCEOF => {
                    let type_ = self.read_class_at(current_offset + 1)?;
                    method_visitor.visit_type_insn(opcode, required(type_, current_offset + 1)?);
                    current_offset += 3;
                }
                IINC => {
                    method_visitor.visit_iinc_insn(
                        u16::from(self.u8_at(current_offset + 1)?),
                        i16::from(self.i8_at(current_offset + 2)?),
                    );
                    current_offset += 3;
                }
                MULTIANEWARRAY => {
                    let descriptor = self.read_class_at(current_offset + 1)?;
                    method_visitor.visit_multi_a_new_array_insn(
                        required(descriptor, current_offset + 1)?,
                        self.u8_at(current_offset + 3)?,
                    );
                    current_offset += 4;
                }
                // The first pass rejected the other opcodes (ASM: AssertionError).
                _ => {
                    return Err(ReadError::new(
                        current_offset,
                        ReadErrorKind::InvalidOpcode(opcode),
                    ));
                }
            }

            // Visit the runtime visible instruction annotations, if any, then the invisible
            // ones.
            for (offsets, index, bytecode_offset, visible) in [
                (
                    visible_type_annotation_offsets.as_deref(),
                    &mut current_visible_type_annotation_index,
                    &mut current_visible_type_annotation_bytecode_offset,
                    true,
                ),
                (
                    invisible_type_annotation_offsets.as_deref(),
                    &mut current_invisible_type_annotation_index,
                    &mut current_invisible_type_annotation_bytecode_offset,
                    false,
                ),
            ] {
                while let Some(offsets) = offsets
                    && *index < offsets.len()
                    && i64::from(*bytecode_offset) <= current_bytecode_offset
                {
                    if i64::from(*bytecode_offset) == current_bytecode_offset {
                        // Parse the target_type, target_info and target_path fields.
                        let mut current_annotation_offset =
                            self.read_type_annotation_target(context, offsets[*index])?;
                        // Parse the type_index field.
                        let annotation_descriptor = self.utf8_at(current_annotation_offset)?;
                        let annotation_descriptor =
                            required(annotation_descriptor, current_annotation_offset)?;
                        current_annotation_offset += 2;
                        // Parse num_element_value_pairs and element_value_pairs and visit these
                        // values.
                        let mut annotation_visitor = method_visitor.visit_insn_annotation(
                            TypeReference::new(context.current_type_annotation_target),
                            context.current_type_annotation_target_path.as_ref(),
                            annotation_descriptor,
                            visible,
                        );
                        self.read_element_values(
                            annotation_visitor.as_deref_mut(),
                            current_annotation_offset,
                            /* named= */ true,
                            0,
                        )?;
                    }
                    *index += 1;
                    *bytecode_offset =
                        self.get_type_annotation_bytecode_offset(Some(offsets), *index)?;
                }
            }
        }
        if let Some(label) = *label_slot(code_length, &mut context.current_method_labels)? {
            method_visitor.visit_label(label);
        }

        // Visit LocalVariableTable and LocalVariableTypeTable attributes.
        if local_variable_table_offset != 0 && !skip_debug {
            // The (start_pc, index, signature_index offset) fields of each entry of the
            // LocalVariableTypeTable.
            let mut type_table = Vec::new();
            if local_variable_type_table_offset != 0 {
                let length = self.u16_at(local_variable_type_table_offset)?;
                type_table.reserve(usize::from(length));
                current_offset = local_variable_type_table_offset + 2;
                for _ in 0..length {
                    // Store the offset of 'signature_index', and the value of 'index' and
                    // 'start_pc'.
                    let index = self.u16_at(current_offset + 8)?;
                    let start_pc = self.u16_at(current_offset)?;
                    type_table.push((start_pc, index, current_offset + 6));
                    current_offset += 10;
                }
            }
            let local_variable_table_length = self.u16_at(local_variable_table_offset)?;
            current_offset = local_variable_table_offset + 2;
            for _ in 0..local_variable_table_length {
                let start_pc = self.u16_at(current_offset)?;
                let length = self.u16_at(current_offset + 2)?;
                let name = self.utf8_at(current_offset + 4)?;
                let descriptor = self.utf8_at(current_offset + 6)?;
                let index = self.u16_at(current_offset + 8)?;
                current_offset += 10;
                // ASM stores the type table in reverse order, and takes the first match.
                let mut signature = None;
                if let Some(&(_, _, signature_offset)) =
                    type_table
                        .iter()
                        .rev()
                        .find(|&&(type_start_pc, type_index, _)| {
                            type_start_pc == start_pc && type_index == index
                        })
                {
                    signature = self.utf8_at(signature_offset)?;
                }
                let labels = &context.current_method_labels;
                let start = label_at(start_pc.into(), labels)?;
                let end = label_at(i64::from(start_pc) + i64::from(length), labels)?;
                method_visitor.visit_local_variable(
                    required(name, current_offset - 6)?,
                    required(descriptor, current_offset - 4)?,
                    signature,
                    start,
                    end,
                    index,
                );
            }
        }

        // Visit the local variable type annotations of the RuntimeVisibleTypeAnnotations
        // attribute, then of the RuntimeInvisibleTypeAnnotations attribute.
        for (offsets, visible) in [
            (visible_type_annotation_offsets.as_deref(), true),
            (invisible_type_annotation_offsets.as_deref(), false),
        ] {
            for &type_annotation_offset in offsets.unwrap_or_default() {
                let target_type = self.u8_at(type_annotation_offset)?;
                if target_type == TypeReference::LOCAL_VARIABLE
                    || target_type == TypeReference::RESOURCE_VARIABLE
                {
                    // Parse the target_type, target_info and target_path fields.
                    current_offset =
                        self.read_type_annotation_target(context, type_annotation_offset)?;
                    // Parse the type_index field.
                    let annotation_descriptor = self.utf8_at(current_offset)?;
                    let annotation_descriptor = required(annotation_descriptor, current_offset)?;
                    current_offset += 2;
                    // Parse num_element_value_pairs and element_value_pairs and visit these
                    // values.
                    let mut annotation_visitor = method_visitor.visit_local_variable_annotation(
                        TypeReference::new(context.current_type_annotation_target),
                        context.current_type_annotation_target_path.as_ref(),
                        &context.current_local_variable_annotation_range_starts,
                        &context.current_local_variable_annotation_range_ends,
                        &context.current_local_variable_annotation_range_indices,
                        annotation_descriptor,
                        visible,
                    );
                    self.read_element_values(
                        annotation_visitor.as_deref_mut(),
                        current_offset,
                        /* named= */ true,
                        0,
                    )?;
                }
            }
        }

        // Visit the non standard attributes.
        for attribute in attributes.iter().rev() {
            method_visitor.visit_attribute(attribute);
        }

        // Visit the max stack and max locals values.
        method_visitor.visit_maxs(max_stack, max_locals);
        Ok(())
    }
}

/// Makes the method visitor visit the current stack map frame of the context: expanded
/// (`F_NEW`), or in its original compressed form.
fn visit_frame(
    method_visitor: &mut dyn MethodVisitor,
    context: &Context<'_>,
    expanded: bool,
) -> Result<(), ReadError> {
    let locals = &context.current_frame_local_types;
    let stack = &context.current_frame_stack_types;
    // ASM passes a count and the whole array; a count can be negative (e.g. after chopping more
    // locals than there are), which visitors treat as 0.
    let prefix = |values: &'_ [FrameValue<'_>], count: i32| -> usize {
        usize::try_from(count).unwrap_or(0).min(values.len())
    };
    let stack = &stack[..prefix(stack, context.current_frame_stack_count)];
    if expanded {
        let locals = &locals[..prefix(locals, context.current_frame_local_count)];
        method_visitor.visit_frame(FrameKind::New, locals, stack);
        return Ok(());
    }
    let delta = context.current_frame_local_count_delta;
    let kind = match context.current_frame_type {
        F_FULL => FrameKind::Full,
        F_APPEND => FrameKind::Append,
        F_CHOP => FrameKind::Chop(delta as u8),
        F_SAME => FrameKind::Same,
        F_SAME1 => FrameKind::Same1,
        frame_type => {
            return Err(ReadError::new(
                0,
                ReadErrorKind::InvalidFrameType(frame_type as u8),
            ));
        }
    };
    let locals = match kind {
        FrameKind::Chop(_) => &[],
        _ => &locals[..prefix(locals, delta)],
    };
    method_visitor.visit_frame(kind, locals, stack);
    Ok(())
}
