//! ASM's `ClassReader` section "Methods to parse a Code attribute".

use std::cell::Cell;

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
    CONSTANT_INTERFACE_METHODREF_TAG, ClassReader, CursorCycles, ParsingOptions, ReadError,
    ReadErrorKind, required,
};

/// The name of the attribute of CLDC's preverified classes (ASM's `"StackMap"`).
const STACK_MAP: &JavaStr = crate::jstr!("StackMap");

/// The error of a bytecode offset outside of the method code.
#[cold]
fn invalid_bytecode_offset(bytecode_offset: i64) -> ReadError {
    ReadError::new(0, ReadErrorKind::InvalidBytecodeOffset(bytecode_offset))
}

/// The labels of the current method, indexed by bytecode offset (ASM's `Label[] labels` of
/// `readCode`, `new Label[codeLength + 1]`): only the bytecode offsets for which a label is
/// needed have one.
///
/// ASM allocates the array for each method. Here the slots stay allocated, at the length of the
/// longest method so far, and a new method only empties the slots the previous one filled: every
/// slot not listed in `filled` is `None`, including those past the current method's offsets. A
/// thread's table also outlives the class, with its line number list: a reader borrows it with
/// [`take`](Self::take) and gives it back with [`put_back`](Self::put_back).
#[derive(Debug, Default)]
pub(super) struct MethodLabels {
    /// The label of each bytecode offset, if any; at least `len` slots.
    slots: Vec<Option<Label>>,
    /// The number of bytecode offsets of the current method (its code length + 1).
    len: usize,
    /// The bytecode offsets whose slot holds a label.
    filled: Vec<usize>,
    /// The `(start_pc, line_number)` entries of the current method's line number tables (ASM's
    /// `Label.lineNumber` and `Label.otherLineNumbers`), sorted by `start_pc` once read, and in
    /// the order they were read for each `start_pc`.
    pub(super) line_numbers: Vec<(u16, u16)>,
}

/// The most slots a thread's table keeps between classes: one per bytecode offset of the
/// longest method the JVMS allows (`code_length` < 65536), plus one.
const MAX_KEPT_SLOTS: usize = 1 << 16;

/// The most line number entries a thread's table keeps between classes, as many as it keeps
/// slots: javac writes at most one per instruction.
const MAX_KEPT_LINE_NUMBERS: usize = MAX_KEPT_SLOTS;

thread_local! {
    /// The label table of the thread's last class reader.
    static THREAD_METHOD_LABELS: Cell<MethodLabels> = const {
        Cell::new(MethodLabels {
            slots: Vec::new(),
            len: 0,
            filled: Vec::new(),
            line_numbers: Vec::new(),
        })
    };
}

impl MethodLabels {
    /// The thread's label table (a new one if another reader of this thread has it), with no
    /// bytecode offset until the class's first Code attribute: ASM's `null`, which the type
    /// annotations of the class, or of a method before it, may still read.
    pub(super) fn take() -> MethodLabels {
        let mut labels = THREAD_METHOD_LABELS
            .try_with(Cell::take)
            .unwrap_or_default();
        labels.reset(0);
        labels
    }

    /// Gives `labels` back to the thread, for the next [`take`](Self::take), unless it is larger
    /// than a valid method needs (a line number list longer than javac writes is dropped alone).
    pub(super) fn put_back(mut labels: MethodLabels) {
        if labels.line_numbers.capacity() > MAX_KEPT_LINE_NUMBERS {
            labels.line_numbers = Vec::new();
        }
        if labels.slots.len() <= MAX_KEPT_SLOTS {
            // During the thread's exit there is nothing to give it back to.
            let _ = THREAD_METHOD_LABELS.try_with(|kept| kept.set(labels));
        }
    }

    /// Empties the table and its line number list for a method with `len` bytecode offsets (its
    /// code length + 1).
    pub(super) fn reset(&mut self, len: usize) {
        for &i in &self.filled {
            self.slots[i] = None;
        }
        self.filled.clear();
        self.line_numbers.clear();
        if self.slots.len() < len {
            self.slots.resize(len, None);
        }
        self.len = len;
    }

    /// The label of the given bytecode offset, if any.
    #[inline]
    pub(super) fn get(&self, bytecode_offset: usize) -> Option<Label> {
        // The slots past `len` are `None`.
        self.slots.get(bytecode_offset).copied().flatten()
    }

    /// The index of `bytecode_offset`'s slot, which must exist.
    #[inline]
    fn index(&self, bytecode_offset: i64) -> Result<usize, ReadError> {
        match usize::try_from(bytecode_offset) {
            Ok(i) if i < self.len && i < self.slots.len() => Ok(i),
            _ => Err(invalid_bytecode_offset(bytecode_offset)),
        }
    }

    /// The label of `bytecode_offset`, whose slot must exist (ASM's `labels[bytecodeOffset]`).
    pub(super) fn slot(&self, bytecode_offset: i64) -> Result<Option<Label>, ReadError> {
        Ok(self.slots[self.index(bytecode_offset)?])
    }

    /// The label of `bytecode_offset`'s slot `i`, created (as `new_label`) if there is none.
    #[inline]
    fn get_or_create(&mut self, i: usize, new_label: impl FnOnce() -> Label) -> &mut Label {
        let MethodLabels { slots, filled, .. } = self;
        slots[i].get_or_insert_with(|| {
            filled.push(i);
            new_label()
        })
    }

    /// Returns the label corresponding to the given bytecode offset. The default implementation
    /// of this method creates a label for the given offset if it has not been already created
    /// (ASM's `readLabel`).
    pub(super) fn read_label(&mut self, bytecode_offset: i64) -> Result<Label, ReadError> {
        let i = self.index(bytecode_offset)?;
        Ok(*self.get_or_create(i, Label::new))
    }

    /// Creates a label without the `FLAG_DEBUG_ONLY` flag set, for the given bytecode offset.
    /// The label is created as with [`read_label`](Self::read_label) and its `FLAG_DEBUG_ONLY`
    /// flag is cleared.
    #[inline]
    pub(super) fn create_label(&mut self, bytecode_offset: i64) -> Result<Label, ReadError> {
        let i = self.index(bytecode_offset)?;
        let label = self.get_or_create(i, Label::new);
        *label = label.with_debug_only(false);
        Ok(*label)
    }

    /// Creates a label with the `FLAG_DEBUG_ONLY` flag set, if there is no already existing
    /// label for the given bytecode offset (otherwise does nothing). The label is created with a
    /// call to [`read_label`](Self::read_label).
    #[inline]
    fn create_debug_label(&mut self, bytecode_offset: i64) -> Result<(), ReadError> {
        let i = self.index(bytecode_offset)?;
        self.get_or_create(i, || Label::new().with_debug_only(true));
        Ok(())
    }

    /// `labels[bytecode_offset]` in the second pass, where the first pass created every label
    /// that an instruction refers to.
    #[inline]
    fn label_at(&self, bytecode_offset: i64) -> Result<Label, ReadError> {
        // The slots past `len` are `None`.
        match usize::try_from(bytecode_offset)
            .ok()
            .and_then(|i| self.get(i))
        {
            Some(label) => Ok(label),
            None => Err(invalid_bytecode_offset(bytecode_offset)),
        }
    }
}

// The kinds of instructions of the first pass over the code (`FIRST_PASS_KINDS`): 1 to 5 is the
// length of an instruction that refers to no label, 0 an invalid opcode.
const FIRST_PASS_JUMP: u8 = 6;
const FIRST_PASS_ASM_JUMP: u8 = 7;
const FIRST_PASS_WIDE_JUMP: u8 = 8;
const FIRST_PASS_WIDE: u8 = 9;
const FIRST_PASS_TABLESWITCH: u8 = 10;
const FIRST_PASS_LOOKUPSWITCH: u8 = 11;

/// The kind of each opcode in the first pass over the code, which only looks for the labels
/// (ASM's switch, as a table the loop indexes).
const FIRST_PASS_KINDS: [u8; 256] = {
    const fn kind(opcode: u8) -> u8 {
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
            | ISTORE_0..=ASTORE_3 => 1,
            IFEQ..=IF_ACMPNE | GOTO | JSR | IFNULL | IFNONNULL => FIRST_PASS_JUMP,
            ASM_IFEQ..=ASM_JSR | ASM_IFNULL | ASM_IFNONNULL => FIRST_PASS_ASM_JUMP,
            GOTO_W | JSR_W | ASM_GOTO_W => FIRST_PASS_WIDE_JUMP,
            WIDE => FIRST_PASS_WIDE,
            TABLESWITCH => FIRST_PASS_TABLESWITCH,
            LOOKUPSWITCH => FIRST_PASS_LOOKUPSWITCH,
            ILOAD..=ALOAD | ISTORE..=ASTORE | RET | BIPUSH | NEWARRAY | LDC => 2,
            SIPUSH
            | LDC_W
            | LDC2_W
            | GETSTATIC..=INVOKESTATIC
            | NEW
            | ANEWARRAY
            | CHECKCAST
            | INSTANCEOF
            | IINC => 3,
            INVOKEINTERFACE | INVOKEDYNAMIC => 5,
            MULTIANEWARRAY => 4,
            _ => 0,
        }
    }
    let mut kinds = [0; 256];
    let mut opcode = 0;
    while opcode < 256 {
        kinds[opcode] = kind(opcode as u8);
        opcode += 1;
    }
    kinds
};

/// How the second pass over the code visits an instruction: the arms of ASM's switch.
#[derive(Clone, Copy)]
enum InsnKind {
    Invalid,
    Insn,
    LoadN,
    StoreN,
    Jump,
    WideJump,
    AsmJump,
    AsmGotoW,
    Wide,
    TableSwitch,
    LookupSwitch,
    Var,
    Int8,
    Int16,
    Ldc,
    LdcW,
    Member,
    InvokeDynamic,
    Type,
    Iinc,
    MultiANewArray,
}

/// The kind of each opcode in the second pass over the code (ASM's switch, as a table the loop
/// indexes).
const SECOND_PASS_KINDS: [InsnKind; 256] = {
    const fn kind(opcode: u8) -> InsnKind {
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
            | MONITOREXIT => InsnKind::Insn,
            ILOAD_0..=ALOAD_3 => InsnKind::LoadN,
            ISTORE_0..=ASTORE_3 => InsnKind::StoreN,
            IFEQ..=IF_ACMPNE | GOTO | JSR | IFNULL | IFNONNULL => InsnKind::Jump,
            GOTO_W | JSR_W => InsnKind::WideJump,
            ASM_IFEQ..=ASM_JSR | ASM_IFNULL | ASM_IFNONNULL => InsnKind::AsmJump,
            ASM_GOTO_W => InsnKind::AsmGotoW,
            WIDE => InsnKind::Wide,
            TABLESWITCH => InsnKind::TableSwitch,
            LOOKUPSWITCH => InsnKind::LookupSwitch,
            ILOAD..=ALOAD | ISTORE..=ASTORE | RET => InsnKind::Var,
            BIPUSH | NEWARRAY => InsnKind::Int8,
            SIPUSH => InsnKind::Int16,
            LDC => InsnKind::Ldc,
            LDC_W | LDC2_W => InsnKind::LdcW,
            GETSTATIC..=INVOKEINTERFACE => InsnKind::Member,
            INVOKEDYNAMIC => InsnKind::InvokeDynamic,
            NEW | ANEWARRAY | CHECKCAST | INSTANCEOF => InsnKind::Type,
            IINC => InsnKind::Iinc,
            MULTIANEWARRAY => InsnKind::MultiANewArray,
            _ => InsnKind::Invalid,
        }
    }
    let mut kinds = [InsnKind::Invalid; 256];
    let mut opcode = 0;
    while opcode < 256 {
        kinds[opcode] = kind(opcode as u8);
        opcode += 1;
    }
    kinds
};

/// The instructions of a method: the class file from the start of its code array (the operands
/// of the last instruction may extend past the array). Reads take bytecode offsets and fail like
/// the [`ClassReader`]'s, at the same class file offset, but stay inline and in `usize`.
#[derive(Clone, Copy)]
struct Bytecode<'b> {
    /// The class file, from the first byte of the code array.
    bytes: &'b [u8],
    /// The class file offset of the code array.
    start: usize,
}

impl Bytecode<'_> {
    /// The `N` bytes at `bytecode_offset`.
    #[inline(always)]
    fn bytes_at<const N: usize>(&self, bytecode_offset: usize) -> Result<[u8; N], ReadError> {
        match self
            .bytes
            .get(bytecode_offset..)
            .and_then(<[u8]>::first_chunk::<N>)
        {
            Some(bytes) => Ok(*bytes),
            None => Err(self.unexpected_end(bytecode_offset)),
        }
    }

    #[cold]
    fn unexpected_end(&self, bytecode_offset: usize) -> ReadError {
        ReadError::at(self.start + bytecode_offset, ReadErrorKind::UnexpectedEnd)
    }

    #[inline(always)]
    fn u8_at(&self, bytecode_offset: usize) -> Result<u8, ReadError> {
        Ok(self.bytes_at::<1>(bytecode_offset)?[0])
    }

    #[inline(always)]
    fn i8_at(&self, bytecode_offset: usize) -> Result<i8, ReadError> {
        Ok(self.u8_at(bytecode_offset)? as i8)
    }

    #[inline(always)]
    fn u16_at(&self, bytecode_offset: usize) -> Result<u16, ReadError> {
        Ok(u16::from_be_bytes(self.bytes_at(bytecode_offset)?))
    }

    #[inline(always)]
    fn i16_at(&self, bytecode_offset: usize) -> Result<i16, ReadError> {
        Ok(i16::from_be_bytes(self.bytes_at(bytecode_offset)?))
    }

    #[inline(always)]
    fn i32_at(&self, bytecode_offset: usize) -> Result<i32, ReadError> {
        Ok(i32::from_be_bytes(self.bytes_at(bytecode_offset)?))
    }
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
        // `new Label[codeLength + 1]`.
        let labels_length = usize::try_from(code_length + 1).map_err(|_| {
            ReadError::new(
                current_offset - 4,
                ReadErrorKind::InvalidLength(code_length),
            )
        })?;
        // The code array is in the class file (checked above), so its offsets are `usize`s. A
        // code_length of -1 (`new Label[0]`) has no instruction, as in ASM's loops.
        let bytecode_length = usize::try_from(code_length).unwrap_or(0);
        let code = Bytecode {
            bytes: &self.class_file[bytecode_start_offset as usize..],
            start: bytecode_start_offset as usize,
        };
        method_visitor.visit_code_length(u32::try_from(code_length).unwrap_or(0));
        context.current_method_labels.reset(labels_length);
        let labels = &mut context.current_method_labels;
        // The bytecode offset of the current instruction.
        let mut pc: usize = 0;
        while pc < bytecode_length {
            let bytecode_offset = pc as i64;
            let opcode = code.u8_at(pc)?;
            match FIRST_PASS_KINDS[usize::from(opcode)] {
                length @ 1..=5 => {
                    pc += usize::from(length);
                }
                FIRST_PASS_JUMP => {
                    let branch = i64::from(code.i16_at(pc + 1)?);
                    labels.create_label(bytecode_offset + branch)?;
                    pc += 3;
                }
                FIRST_PASS_ASM_JUMP => {
                    let branch = i64::from(code.u16_at(pc + 1)?);
                    labels.create_label(bytecode_offset + branch)?;
                    pc += 3;
                }
                FIRST_PASS_WIDE_JUMP => {
                    let branch = i64::from(code.i32_at(pc + 1)?);
                    labels.create_label(bytecode_offset + branch)?;
                    pc += 5;
                }
                FIRST_PASS_WIDE => match code.u8_at(pc + 1)? {
                    ILOAD..=ALOAD | ISTORE..=ASTORE | RET => {
                        pc += 4;
                    }
                    IINC => {
                        pc += 6;
                    }
                    wide_opcode => {
                        return Err(ReadError::at(
                            code.start + pc + 1,
                            ReadErrorKind::InvalidWideOpcode(wide_opcode),
                        ));
                    }
                },
                FIRST_PASS_TABLESWITCH => {
                    // Skip 0 to 3 padding bytes.
                    pc += 4 - (pc & 3);
                    // Read the default label and the number of table entries.
                    let default = i64::from(code.i32_at(pc)?);
                    labels.create_label(bytecode_offset + default)?;
                    let high = code.i32_at(pc + 8)?;
                    let low = code.i32_at(pc + 4)?;
                    let mut num_table_entries = high.wrapping_sub(low).wrapping_add(1);
                    pc += 12;
                    // Read the table labels.
                    while num_table_entries > 0 {
                        num_table_entries -= 1;
                        let branch = i64::from(code.i32_at(pc)?);
                        labels.create_label(bytecode_offset + branch)?;
                        pc += 4;
                    }
                }
                FIRST_PASS_LOOKUPSWITCH => {
                    // Skip 0 to 3 padding bytes.
                    pc += 4 - (pc & 3);
                    // Read the default label and the number of switch cases.
                    let default = i64::from(code.i32_at(pc)?);
                    labels.create_label(bytecode_offset + default)?;
                    let mut num_switch_cases = code.i32_at(pc + 4)?;
                    pc += 8;
                    // Read the switch labels.
                    while num_switch_cases > 0 {
                        num_switch_cases -= 1;
                        let branch = i64::from(code.i32_at(pc + 4)?);
                        labels.create_label(bytecode_offset + branch)?;
                        pc += 8;
                    }
                }
                _ => {
                    return Err(ReadError::at(
                        code.start + pc,
                        ReadErrorKind::InvalidOpcode(opcode),
                    ));
                }
            }
        }
        current_offset = bytecode_start_offset + pc as i64;

        // Read the 'exception_table_length' and 'exception_table' field to create a label for
        // each referenced instruction, and to make method_visitor visit the corresponding try
        // catch blocks.
        let exception_table_length = self.u16_at(current_offset)?;
        current_offset += 2;
        for _ in 0..exception_table_length {
            let start = labels.create_label(self.u16_at(current_offset)?.into())?;
            let end = labels.create_label(self.u16_at(current_offset + 2)?.into())?;
            let handler = labels.create_label(self.u16_at(current_offset + 4)?.into())?;
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
        let mut cycles = CursorCycles::new();
        let mut effects = 0;
        let mut remaining = self.u16_at(current_offset)?;
        current_offset += 2;
        while remaining > 0 {
            remaining -= 1;
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
                            context.current_method_labels.create_debug_label(start_pc)?;
                            let length =
                                i64::from(self.u16_at(current_local_variable_table_offset + 2)?);
                            context
                                .current_method_labels
                                .create_debug_label(start_pc + length)?;
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
                        effects += 1;
                        // Parse the attribute to find the corresponding (debug only) labels.
                        let mut current_line_number_table_offset = current_offset;
                        let line_number_table_length =
                            self.u16_at(current_line_number_table_offset)?;
                        current_line_number_table_offset += 2;
                        for _ in 0..line_number_table_length {
                            let start_pc = self.u16_at(current_line_number_table_offset)?;
                            let line_number = self.u16_at(current_line_number_table_offset + 2)?;
                            current_line_number_table_offset += 4;
                            context
                                .current_method_labels
                                .create_debug_label(i64::from(start_pc))?;
                            context
                                .current_method_labels
                                .line_numbers
                                .push((start_pc, line_number));
                        }
                    }
                }
                Some(n) if n == constants::RUNTIME_VISIBLE_TYPE_ANNOTATIONS => {
                    // Visits the exception parameters' annotations.
                    effects += 1;
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
                    // Visits the exception parameters' annotations.
                    effects += 1;
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
                    effects += 1;
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
            if attribute_length <= -6 {
                remaining = cycles.step_back(current_offset, remaining, effects);
            }
        }
        // Javac's tables are mostly in bytecode order already. The sort is stable, so the line
        // numbers of an offset stay in the order they were read.
        let line_numbers = &mut context.current_method_labels.line_numbers;
        if !line_numbers.is_sorted_by_key(|&(start_pc, _)| start_pc) {
            line_numbers.sort_by_key(|&(start_pc, _)| start_pc);
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
                        context
                            .current_method_labels
                            .create_label(potential_bytecode_offset)?;
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

        // Whether the instructions have type annotations to visit.
        let has_insn_annotations = visible_type_annotation_offsets.is_some()
            || invisible_type_annotation_offsets.is_some();

        // Whether a F_INSERT stack map frame must be inserted before the current instruction.
        let mut insert_frame = false;

        // The index of the next line number entry to visit, at its start_pc.
        let mut line_number_index = 0;

        // The delta to subtract from a goto_w or jsr_w opcode to get the corresponding goto or
        // jsr opcode, or 0 if goto_w and jsr_w must be left unchanged (i.e. when expanding ASM
        // specific instructions).
        let wide_jump_opcode_delta = if parsing_options.contains(ParsingOptions::EXPAND_ASM_INSNS) {
            0
        } else {
            WIDE_JUMP_OPCODE_DELTA
        };

        pc = 0;
        while pc < bytecode_length {
            let current_bytecode_offset = pc as i64;
            // The class file offset of the instruction.
            let current_offset = bytecode_start_offset + current_bytecode_offset;

            // Visit the label and the line number(s) for this bytecode offset, if any.
            let labels = &context.current_method_labels;
            if let Some(current_label) = labels.get(pc) {
                method_visitor.visit_label(current_label);
                // Skip the entries of offsets that are not an instruction's, which ASM's
                // readCode never visits either.
                let line_numbers = &labels.line_numbers;
                while let Some(&(start_pc, line_number)) = line_numbers.get(line_number_index)
                    && usize::from(start_pc) <= pc
                {
                    if usize::from(start_pc) == pc {
                        method_visitor.visit_line_number(line_number, current_label);
                    }
                    line_number_index += 1;
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
            let opcode = code.u8_at(pc)?;
            match SECOND_PASS_KINDS[usize::from(opcode)] {
                InsnKind::Insn => {
                    method_visitor.visit_insn(opcode);
                    pc += 1;
                }
                InsnKind::LoadN => {
                    let opcode = opcode - ILOAD_0;
                    method_visitor.visit_var_insn(ILOAD + (opcode >> 2), u16::from(opcode & 0x3));
                    pc += 1;
                }
                InsnKind::StoreN => {
                    let opcode = opcode - ISTORE_0;
                    method_visitor.visit_var_insn(ISTORE + (opcode >> 2), u16::from(opcode & 0x3));
                    pc += 1;
                }
                InsnKind::Jump => {
                    let branch = i64::from(code.i16_at(pc + 1)?);
                    let label = labels.label_at(current_bytecode_offset + branch)?;
                    method_visitor.visit_jump_insn(opcode, label);
                    pc += 3;
                }
                InsnKind::WideJump => {
                    let branch = i64::from(code.i32_at(pc + 1)?);
                    let label = labels.label_at(current_bytecode_offset + branch)?;
                    method_visitor.visit_jump_insn(opcode - wide_jump_opcode_delta, label);
                    pc += 5;
                }
                InsnKind::AsmJump => {
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
                    let branch = i64::from(code.u16_at(pc + 1)?);
                    let target = labels.label_at(current_bytecode_offset + branch)?;
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
                        let endif = context
                            .current_method_labels
                            .create_label(current_bytecode_offset + 3)?;
                        method_visitor.visit_jump_insn(opcode, endif);
                        method_visitor.visit_jump_insn(GOTO_W, target);
                        // endif designates the instruction just after GOTO_W, and is visited as
                        // part of the next instruction. Since it is a jump target, we need to
                        // insert a frame here.
                        insert_frame = true;
                    }
                    pc += 3;
                }
                InsnKind::AsmGotoW => {
                    // Replace ASM_GOTO_W with GOTO_W.
                    let branch = i64::from(code.i32_at(pc + 1)?);
                    let label = labels.label_at(current_bytecode_offset + branch)?;
                    method_visitor.visit_jump_insn(GOTO_W, label);
                    // The instruction just after is a jump target (because ASM_GOTO_W is used in
                    // patterns IFNOTxxx <L> ASM_GOTO_W <l> L:..., see MethodWriter), so we need
                    // to insert a frame here.
                    insert_frame = true;
                    pc += 5;
                }
                InsnKind::Wide => {
                    let opcode = code.u8_at(pc + 1)?;
                    if opcode == IINC {
                        method_visitor.visit_iinc_insn(code.u16_at(pc + 2)?, code.i16_at(pc + 4)?);
                        pc += 6;
                    } else {
                        method_visitor.visit_var_insn(opcode, code.u16_at(pc + 2)?);
                        pc += 4;
                    }
                }
                InsnKind::TableSwitch => {
                    // Skip 0 to 3 padding bytes.
                    pc += 4 - (pc & 3);
                    // Read the instruction.
                    let default = i64::from(code.i32_at(pc)?);
                    let default_label = labels.label_at(current_bytecode_offset + default)?;
                    let low = code.i32_at(pc + 4)?;
                    let high = code.i32_at(pc + 8)?;
                    pc += 12;
                    let length = high.wrapping_sub(low).wrapping_add(1);
                    let mut table = Vec::with_capacity(array_capacity(
                        self,
                        length,
                        4,
                        bytecode_start_offset + pc as i64,
                    )?);
                    for _ in 0..length {
                        let branch = i64::from(code.i32_at(pc)?);
                        table.push(labels.label_at(current_bytecode_offset + branch)?);
                        pc += 4;
                    }
                    method_visitor.visit_table_switch_insn(low, high, default_label, &table);
                }
                InsnKind::LookupSwitch => {
                    // Skip 0 to 3 padding bytes.
                    pc += 4 - (pc & 3);
                    // Read the instruction.
                    let default = i64::from(code.i32_at(pc)?);
                    let default_label = labels.label_at(current_bytecode_offset + default)?;
                    let num_pairs = code.i32_at(pc + 4)?;
                    pc += 8;
                    let capacity =
                        array_capacity(self, num_pairs, 8, bytecode_start_offset + pc as i64)?;
                    let mut keys = Vec::with_capacity(capacity);
                    let mut values = Vec::with_capacity(capacity);
                    for _ in 0..num_pairs {
                        keys.push(code.i32_at(pc)?);
                        let branch = i64::from(code.i32_at(pc + 4)?);
                        values.push(labels.label_at(current_bytecode_offset + branch)?);
                        pc += 8;
                    }
                    method_visitor.visit_lookup_switch_insn(default_label, &keys, &values);
                }
                InsnKind::Var => {
                    method_visitor.visit_var_insn(opcode, u16::from(code.u8_at(pc + 1)?));
                    pc += 2;
                }
                InsnKind::Int8 => {
                    method_visitor.visit_int_insn(opcode, i32::from(code.i8_at(pc + 1)?));
                    pc += 2;
                }
                InsnKind::Int16 => {
                    method_visitor.visit_int_insn(opcode, i32::from(code.i16_at(pc + 1)?));
                    pc += 3;
                }
                InsnKind::Ldc => {
                    let index = usize::from(code.u8_at(pc + 1)?);
                    method_visitor.visit_ldc_insn(&self.const_at(index)?);
                    pc += 2;
                }
                InsnKind::LdcW => {
                    let index = usize::from(code.u16_at(pc + 1)?);
                    method_visitor.visit_ldc_insn(&self.const_at(index)?);
                    pc += 3;
                }
                InsnKind::Member => {
                    let cp_info_offset = self.cp_info_offset(usize::from(code.u16_at(pc + 1)?))?;
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
                        pc += 5;
                    } else {
                        pc += 3;
                    }
                }
                InsnKind::InvokeDynamic => {
                    let cp_info_offset = self.cp_info_offset(usize::from(code.u16_at(pc + 1)?))?;
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
                    pc += 5;
                }
                InsnKind::Type => {
                    let type_ = self.read_class_at(current_offset + 1)?;
                    method_visitor.visit_type_insn(opcode, required(type_, current_offset + 1)?);
                    pc += 3;
                }
                InsnKind::Iinc => {
                    method_visitor.visit_iinc_insn(
                        u16::from(code.u8_at(pc + 1)?),
                        i16::from(code.i8_at(pc + 2)?),
                    );
                    pc += 3;
                }
                InsnKind::MultiANewArray => {
                    let descriptor = self.read_class_at(current_offset + 1)?;
                    method_visitor.visit_multi_a_new_array_insn(
                        required(descriptor, current_offset + 1)?,
                        code.u8_at(pc + 3)?,
                    );
                    pc += 4;
                }
                // The first pass rejected the other opcodes (ASM: AssertionError).
                InsnKind::Invalid => {
                    return Err(ReadError::new(
                        current_offset,
                        ReadErrorKind::InvalidOpcode(opcode),
                    ));
                }
            }

            // Visit the runtime visible instruction annotations, if any, then the invisible
            // ones.
            if has_insn_annotations {
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
        }
        if let Some(label) = context.current_method_labels.slot(code_length)? {
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
            method_visitor.visit_local_variable_count(local_variable_table_length);
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
                let start = labels.label_at(start_pc.into())?;
                let end = labels.label_at(i64::from(start_pc) + i64::from(length))?;
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
