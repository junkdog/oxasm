//! ASM's `MethodWriter`, which generates the `method_info` structure of a method, and its Code
//! attribute.
//!
//! [`MethodWriterState`] holds what ASM's `MethodWriter` holds (the class writer owns one per
//! method, ADR-08), and [`MethodWriter`], the method visitor a class writer returns, borrows it
//! with the symbol table. `MethodWriter.java` is split along its sections:
//!
//! - this module: the constructor, and the implementation of the method visitor;
//! - `control_flow`: the control flow graph (`visitLabel`), and the computation of the maximum
//!   stack size and of the stack map frames from it (`visitMaxs`);
//! - `frames`: the stack map frames;
//! - `attributes`: the method copy optimization, and the size and content of the `method_info`.

mod attributes;
mod control_flow;
mod frames;

use crate::attribute::Attribute;
use crate::constant::{Constant, FrameValue};
use crate::constants::{
    ACC_CONSTRUCTOR, ASM_GOTO_W, GOTO_W, ILOAD_0, ISTORE_0, JSR_W, LDC_W, LDC2_W, WIDE,
    WIDE_JUMP_OPCODE_DELTA,
};
use crate::error::WriteError;
use crate::handle::Handle;
use crate::hierarchy::ClassHierarchy;
use crate::jstr::{JavaCompactStr, JavaStr};
use crate::label::Label;
use crate::opcodes::{self, FrameKind};
use crate::ty::Type;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::{AnnotationVisitor, MethodCopySource, MethodVisitor};
use crate::writer::annotation_writer::{AnnotationList, AnnotationWriter, MemberAnnotations};
use crate::writer::attributes::AttributeList;
use crate::writer::byte_vector::ByteVector;
use crate::writer::frame::SymbolArg;
use crate::writer::handler::{Handler, put_exception_table};
use crate::writer::label_table::{
    BlockId, EDGE_JUMP, FLAG_JUMP_TARGET, FLAG_SUBROUTINE_CALLER, FLAG_SUBROUTINE_END,
    FLAG_SUBROUTINE_START, LabelTable,
};
use crate::writer::symbol_table::{
    CONSTANT_DOUBLE_TAG, CONSTANT_DYNAMIC_TAG, CONSTANT_LONG_TAG, Span, Symbol, SymbolTable,
};

/// What a method writer must compute (ASM's `COMPUTE_*` constants of `MethodWriter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Compute {
    /// Nothing must be computed.
    Nothing,
    /// The maximum stack size and the maximum number of local variables must be computed, from
    /// the control flow graph.
    MaxStackAndLocal,
    /// The maximum stack size and the maximum number of local variables must be computed, from
    /// the stack map frames.
    MaxStackAndLocalFromFrames,
    /// The stack map frames inserted by the expansion of ASM specific instructions must be
    /// computed.
    InsertedFrames,
    /// All the stack map frames must be computed, as well as the maximum stack size and number
    /// of local variables.
    AllFrames,
}

/// The size in bytes of a `line_number_table` entry.
pub(super) const LINE_NUMBER_SIZE: usize = 4;

/// The size in bytes of a `local_variable_table` or `local_variable_type_table` entry.
pub(super) const LOCAL_VARIABLE_SIZE: usize = 10;

/// The LineNumberTable, LocalVariableTable and LocalVariableTypeTable entries of the methods of
/// a class, each table's end to end in one byte vector (not in ASM, whose method writers each
/// hold their own byte vectors: an allocation or more per table and method). A method is
/// visited to its end before the next one starts (its method writer borrows the class writer),
/// so a method's entries are contiguous in each vector, and the method keeps where they start
/// ([`TableEntries`]).
#[derive(Debug, Default)]
pub(crate) struct DebugTables {
    /// The `line_number_table` arrays.
    line_numbers: ByteVector,
    /// The `local_variable_table` arrays.
    local_variables: ByteVector,
    /// The `local_variable_type_table` arrays.
    local_variable_types: ByteVector,
}

/// A method's entries in one of the vectors of the [`DebugTables`].
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TableEntries {
    /// The offset of the first entry in the vector.
    start: usize,
    /// The number of entries (the attribute's `*_table_length` field).
    length: i32,
}

impl TableEntries {
    /// Counts a new entry, and returns `table`, to put it at its end.
    fn add<'t>(&mut self, table: &'t mut ByteVector) -> &'t mut ByteVector {
        if self.length == 0 {
            self.start = table.size();
        }
        self.length += 1;
        table
    }

    /// Whether there are no entries (and so no attribute).
    pub(super) fn is_empty(self) -> bool {
        self.length == 0
    }

    /// The number of entries, for the attribute's `*_table_length` field.
    pub(super) fn length(self) -> i32 {
        self.length
    }

    /// The size in bytes of the entries, each of `entry_size` bytes.
    #[allow(clippy::cast_sign_loss)] // a count, never negative
    pub(super) fn size(self, entry_size: usize) -> usize {
        self.length as usize * entry_size
    }

    /// The entries, each of `entry_size` bytes, in `table`.
    pub(super) fn data(self, table: &ByteVector, entry_size: usize) -> &[u8] {
        &table.data()[self.start..self.start + self.size(entry_size)]
    }
}

/// A copy of a method's descriptor, given by its span in `symbol_table`, which stays valid while
/// the table changes (computing the frames adds types to it while walking the descriptor).
fn descriptor_copy(symbol_table: &SymbolTable, descriptor: Span) -> JavaCompactStr {
    JavaCompactStr::new(symbol_table.text(descriptor))
}

/// A `method_info` structure being written (ASM's `MethodWriter` fields).
#[derive(Debug)]
pub(crate) struct MethodWriterState {
    // Note: fields are ordered as in the method_info structure, and those related to attributes
    // are ordered as in Section 4.7 of the JVMS.
    /// The `access_flags` field of the `method_info` JVMS structure. This field can contain ASM
    /// specific access flags, such as `ACC_DEPRECATED` or `ACC_CONSTRUCTOR`, which are removed
    /// when generating the ClassFile structure.
    access_flags: u32,
    /// The `name_index` field of the `method_info` JVMS structure.
    name_index: i32,
    /// The name of this method, in the symbol table's strings.
    name: Span,
    /// The `descriptor_index` field of the `method_info` JVMS structure.
    descriptor_index: i32,
    /// The descriptor of this method, in the symbol table's strings.
    descriptor: Span,

    // Code attribute fields and sub attributes:
    /// The `max_stack` field of the Code attribute.
    max_stack: i32,
    /// The `max_locals` field of the Code attribute.
    max_locals: i32,
    /// The `code` field of the Code attribute.
    code: ByteVector,
    /// The `exception_table` of the Code attribute (ASM's `firstHandler` list).
    handlers: Vec<Handler>,
    /// The `line_number_table` array of the LineNumberTable code attribute, in the class's
    /// [`DebugTables`], and its length (none if 0).
    line_number_table: TableEntries,
    /// The `local_variable_table` array of the LocalVariableTable code attribute, in the class's
    /// [`DebugTables`], and its length (none if 0).
    local_variable_table: TableEntries,
    /// The `local_variable_type_table` array of the LocalVariableTypeTable code attribute, in the
    /// class's [`DebugTables`], and its length (none if 0).
    local_variable_type_table: TableEntries,
    /// The `number_of_entries` field of the StackMapTable code attribute.
    stack_map_table_number_of_entries: i32,
    /// The `entries` array of the StackMapTable code attribute, or `None`.
    stack_map_table_entries: Option<ByteVector>,
    /// The runtime visible type annotations of the code of this method.
    code_visible_type_annotations: AnnotationList,
    /// The runtime invisible type annotations of the code of this method.
    code_invisible_type_annotations: AnnotationList,
    /// The non standard attributes of the Code attribute.
    code_attributes: AttributeList,

    // Other method_info attributes:
    /// The `exception_index_table` array of the Exceptions attribute (empty for none).
    exception_index_table: Vec<i32>,
    /// The `signature_index` field of the Signature attribute, or 0.
    signature_index: i32,
    /// The runtime (in)visible (type) annotations of this method.
    annotations: MemberAnnotations,
    /// The number of method parameters that can have runtime visible annotations, or 0.
    visible_annotable_parameter_count: i32,
    /// The runtime visible parameter annotations of this method, or `None`.
    visible_parameter_annotations: Option<Vec<AnnotationList>>,
    /// The number of method parameters that can have runtime invisible annotations, or 0.
    invisible_annotable_parameter_count: i32,
    /// The runtime invisible parameter annotations of this method, or `None`.
    invisible_parameter_annotations: Option<Vec<AnnotationList>>,
    /// The `default_value` field of the AnnotationDefault attribute, or `None`.
    default_value: Option<ByteVector>,
    /// The `parameters_count` field of the MethodParameters attribute.
    parameters_count: i32,
    /// The `parameters` array of the MethodParameters attribute, or `None`.
    parameters: Option<ByteVector>,
    /// The non standard attributes of the method.
    attributes: AttributeList,

    // Fields used to compute the maximum stack size and number of locals, and the stack map
    // frames.
    /// Indicates what must be computed.
    compute: Compute,
    /// The labels of this method (what ASM stores in its `Label`s), and its control flow graph.
    labels: LabelTable,
    /// The first basic block of the method. The next ones (in bytecode offset order) can be
    /// accessed with the `next_basic_block` field of the basic block states.
    first_basic_block: Option<BlockId>,
    /// The last basic block of the method (in bytecode offset order). This field is updated each
    /// time a basic block is encountered, and is used to append it at the end of the basic block
    /// list.
    last_basic_block: Option<BlockId>,
    /// The current basic block, i.e. the basic block of the last visited instruction. When
    /// `compute` is `MaxStackAndLocal` or `AllFrames`, this field is `None` for unreachable
    /// code. When `compute` is `MaxStackAndLocalFromFrames` or `InsertedFrames`, this field
    /// stays unchanged throughout the whole method (i.e. the whole code is seen as a single
    /// basic block; indeed, the existing frames are sufficient by hypothesis to compute any
    /// intermediate frame - and the maximum stack size as well - without using any control flow
    /// graph).
    current_basic_block: Option<BlockId>,
    /// The relative stack size after the last visited instruction. This size is relative to the
    /// beginning of `current_basic_block`, i.e. the true stack size after the last visited
    /// instruction is equal to the `input_stack_size` of the current basic block plus
    /// `relative_stack_size`. When `compute` is `MaxStackAndLocalFromFrames`,
    /// `current_basic_block` is always the start of the method, so this relative size is also
    /// equal to the absolute stack size after the last visited instruction.
    relative_stack_size: i32,
    /// The maximum relative stack size after the last visited instruction. This size is relative
    /// to the beginning of `current_basic_block`, i.e. the true maximum stack size after the
    /// last visited instruction is equal to the `input_stack_size` of the current basic block
    /// plus `max_relative_stack_size`. When `compute` is `MaxStackAndLocalFromFrames`,
    /// `current_basic_block` is always the start of the method, so this relative size is also
    /// equal to the absolute maximum stack size after the last visited instruction.
    max_relative_stack_size: i32,
    /// The number of local variables in the last visited stack map frame.
    current_locals: i32,
    /// The bytecode offset of the last frame that was written in `stack_map_table_entries`.
    previous_frame_offset: i32,
    /// The last frame that was written in `stack_map_table_entries`. This field has the same
    /// format as `current_frame`.
    previous_frame: Option<Vec<i32>>,
    /// The current stack map frame. The first element contains the bytecode offset of the
    /// instruction to which the frame corresponds, the second element is the number of
    /// locals and the third one is the number of stack elements. The local variables start at
    /// index 3 and are followed by the operand stack elements. In summary `frame[0] = offset`,
    /// `frame[1] = numLocal`, `frame[2] = numStack`. Local variables and operand stack entries
    /// contain abstract types, as defined in ASM's `Frame`, but restricted to CONSTANT_KIND,
    /// REFERENCE_KIND, UNINITIALIZED_KIND or FORWARD_UNINITIALIZED_KIND abstract types. Long and
    /// double types use only one array entry.
    current_frame: Vec<i32>,

    // Other miscellaneous status fields
    /// Whether this method contains subroutines.
    has_subroutines: bool,
    /// Whether this method contains ASM specific instructions.
    has_asm_instructions: bool,
    /// The start offset of the last visited instruction. Used to set the offset field of type
    /// annotations of type 'offset_target' (see JVMS 4.7.20.1).
    last_bytecode_offset: i32,
    /// The `method_info` attributes of the method, less its first 6 bytes, if they are copied as
    /// is from a class reader (ASM's `sourceOffset` and `sourceLength`, copied at once,
    /// ADR-08).
    copied_method_info: Option<Vec<u8>>,
    /// The exception table, with its length, if it was written at the end of the visit of the
    /// method, when `labels` was released for the next method writer (see
    /// [`release_labels`](Self::release_labels)).
    exception_table: Option<ByteVector>,
    /// Whether `labels` was released: writing the method no longer reads it.
    labels_released: bool,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodWriterState {
    /// Constructs a new method writer state (ASM's `MethodWriter` constructor), with the label
    /// table of `spare_labels`, emptied, if any: the [`released_labels`](Self::released_labels)
    /// of the previous method, to reuse their memory.
    #[allow(clippy::too_many_arguments)] // ASM's constructor, and the table
    pub(crate) fn new(
        symbol_table: &mut SymbolTable,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
        compute: Compute,
        spare_labels: Option<&mut LabelTable>,
    ) -> MethodWriterState {
        let access_flags = if name == "<init>" {
            access | ACC_CONSTRUCTOR
        } else {
            access
        };
        let (name_index, name_span) = symbol_table.add_constant_utf8_span(name);
        let (descriptor_index, descriptor_span) = symbol_table.add_constant_utf8_span(descriptor);
        let signature_index = signature.map_or(0, |s| symbol_table.add_constant_utf8(s));
        let exception_index_table = exceptions
            .iter()
            .map(|exception| symbol_table.add_constant_class(exception).index)
            .collect();
        let mut state = MethodWriterState {
            access_flags,
            name_index,
            name: name_span,
            descriptor_index,
            descriptor: descriptor_span,
            max_stack: 0,
            max_locals: 0,
            code: ByteVector::new(),
            handlers: Vec::new(),
            line_number_table: TableEntries::default(),
            local_variable_table: TableEntries::default(),
            local_variable_type_table: TableEntries::default(),
            stack_map_table_number_of_entries: 0,
            stack_map_table_entries: None,
            code_visible_type_annotations: Vec::new(),
            code_invisible_type_annotations: Vec::new(),
            code_attributes: AttributeList::default(),
            exception_index_table,
            signature_index,
            annotations: MemberAnnotations::default(),
            visible_annotable_parameter_count: 0,
            visible_parameter_annotations: None,
            invisible_annotable_parameter_count: 0,
            invisible_parameter_annotations: None,
            default_value: None,
            parameters_count: 0,
            parameters: None,
            attributes: AttributeList::default(),
            compute,
            labels: LabelTable::default(),
            first_basic_block: None,
            last_basic_block: None,
            current_basic_block: None,
            relative_stack_size: 0,
            max_relative_stack_size: 0,
            current_locals: 0,
            previous_frame_offset: 0,
            previous_frame: None,
            current_frame: Vec::new(),
            has_subroutines: false,
            has_asm_instructions: false,
            last_bytecode_offset: 0,
            copied_method_info: None,
            exception_table: None,
            labels_released: false,
        };
        if let Some(spare_labels) = spare_labels {
            spare_labels.clear();
            std::mem::swap(&mut state.labels, spare_labels);
        }
        if compute != Compute::Nothing {
            // Update maxLocals and currentLocals.
            let mut arguments_size = match Type::try_get_arguments_and_return_sizes_of(descriptor) {
                Ok(sizes) => (sizes >> 2) as i32,
                Err(error) => {
                    symbol_table.fail(error.into());
                    0
                }
            };
            if access & opcodes::ACC_STATIC != 0 {
                arguments_size -= 1;
            }
            state.max_locals = arguments_size;
            state.current_locals = arguments_size;
            // Create and visit the label for the first basic block.
            let first_basic_block = Label::new();
            state.visit_label(first_basic_block);
            state.first_basic_block = Some(state.labels.id(first_basic_block));
        }
        state
    }

    /// Releases the labels of this method, for the next method writer to take, empty and reuse
    /// ([`released_labels`](Self::released_labels)), if writing the method needs nothing more of
    /// them: the exception table, which refers to them, is written now. Non-standard code
    /// attributes, which resolve their labels when they are written, keep them, as does a table
    /// that has no memory to reuse (a copied method, or one without code). Called at the end of
    /// the visit of the method.
    pub(crate) fn release_labels(&mut self) {
        if !self.code_attributes.is_empty() || !self.labels.has_memory() {
            return;
        }
        if !self.handlers.is_empty() {
            let mut exception_table = ByteVector::with_capacity(2 + 8 * self.handlers.len());
            put_exception_table(&self.handlers, &self.labels, &mut exception_table);
            self.exception_table = Some(exception_table);
        }
        self.labels_released = true;
    }

    /// The labels of this method, if it released them (they stay here until the next method
    /// takes them, as the control flow graph of the last method is read from them in tests).
    pub(crate) fn released_labels(&mut self) -> Option<&mut LabelTable> {
        self.labels_released.then_some(&mut self.labels)
    }

    /// Whether the stack map frames are computed, i.e. the instructions are executed on the
    /// frame of the current basic block.
    fn computes_frames(&self) -> bool {
        matches!(self.compute, Compute::AllFrames | Compute::InsertedFrames)
    }

    /// Whether the relative stack size is tracked after each instruction: when the maximum stack
    /// size is computed without computing frames, in reachable code.
    fn tracks_stack_size(&self) -> bool {
        self.current_basic_block.is_some() && !self.computes_frames()
    }

    /// Updates the relative stack size, and its maximum, after an instruction with the stack
    /// size variation of the given opcode (if the stack size is tracked).
    fn update_relative_stack_size_by_opcode(&mut self, symbol_table: &mut SymbolTable, opcode: u8) {
        if self.tracks_stack_size() {
            match control_flow::stack_size_delta(opcode) {
                Ok(delta) => self.update_relative_stack_size(delta),
                Err(error) => symbol_table.fail(error),
            }
        }
    }

    /// Simulates the given instruction on the frame of the current basic block, if the stack
    /// map frames are computed (ASM's `currentBasicBlock.frame.execute`).
    fn execute(
        &mut self,
        symbol_table: &mut SymbolTable,
        opcode: u8,
        arg: i32,
        arg_symbol: Option<&SymbolArg<'_>>,
    ) {
        let Some(current_basic_block) = self.current_basic_block else {
            return;
        };
        // After an error, which ASM would have thrown, the class cannot be written: skip the
        // frame computation, the costly part of the visit ASM never does (it stops at its first
        // exception).
        if !self.computes_frames() || symbol_table.error().is_some() {
            return;
        }
        let inserted = self.compute == Compute::InsertedFrames;
        let frame = self.labels.get(current_basic_block).frame;
        let result = match frame.map(|frame| self.labels.frame_mut(frame)) {
            Some(mut frame) if inserted => {
                frame.execute_current(opcode, arg, arg_symbol, symbol_table)
            }
            Some(mut frame) => frame.execute(opcode, arg, arg_symbol, symbol_table),
            // ASM's NullPointerException (no frame visited before the instruction).
            None => Err(WriteError::IllegalState(
                "no frame before the first instruction".to_owned(),
            )),
        };
        if let Err(error) = result {
            symbol_table.fail(error);
        }
    }

    /// Updates the maximum number of locals, if computed.
    fn update_max_locals(&mut self, current_max_locals: i32) {
        if self.compute != Compute::Nothing && current_max_locals > self.max_locals {
            self.max_locals = current_max_locals;
        }
    }

    /// The symbol operand of an instruction for the frame computation: the symbol's tag, and
    /// its name and value given by the instruction.
    fn symbol_arg<'a>(symbol: &Symbol, name: &'a JavaStr, value: &'a JavaStr) -> SymbolArg<'a> {
        SymbolArg {
            tag: symbol.tag,
            name,
            value,
        }
    }

    /// Whether this method has stack map frames.
    pub(crate) fn has_frames(&self) -> bool {
        self.stack_map_table_number_of_entries > 0
    }

    /// Whether this method contains ASM specific instructions.
    pub(crate) fn has_asm_instructions(&self) -> bool {
        self.has_asm_instructions
    }

    /// The bytecode offset of the given label of this method, if it was visited.
    #[allow(dead_code)] // used by CheckClassAdapter (P7)
    pub(crate) fn label_offset(&self, label: Label) -> Option<i32> {
        self.labels.resolved_offset(label)
    }

    /// The code length, as a Java int.
    fn code_length(&self) -> i32 {
        self.code.size() as i32
    }

    /// Sizes the code and the label table for a method of `code_length` bytes of bytecode (the
    /// hint of [`MethodVisitor::visit_code_length`]), so that rewriting a method grows neither.
    fn reserve_code(&mut self, code_length: u32) {
        // A larger code array is not one a method can have (ASM's MethodTooLargeException).
        let code_length = code_length.min(65535) as usize;
        self.code.reserve_total(code_length);
        // Methods have about one label per 12 bytes of bytecode; one per 4, plus 2, covers 99.9%
        // of a corpus' methods.
        self.labels.reserve(code_length / 4 + 2);
        if self.compute == Compute::AllFrames {
            // The frames of a method have about 1.6 abstract types per byte of bytecode; 4 per
            // byte, plus 16, cover 99.7% of a corpus' methods.
            self.labels.reserve_frame_values(code_length * 4 + 16);
        }
    }
}

/// A [`MethodVisitor`] that generates a corresponding `method_info` structure, as defined in the
/// Java Virtual Machine Specification (JVMS).
pub(crate) struct MethodWriter<'a> {
    pub(crate) symbol_table: &'a mut SymbolTable,
    pub(crate) state: &'a mut MethodWriterState,
    /// The debug tables of the class, where the method's entries are put.
    pub(crate) debug_tables: &'a mut DebugTables,
    /// The class hierarchy that computing stack map frames uses (ASM's
    /// `ClassWriter.getCommonSuperClass`), if any.
    pub(crate) hierarchy: Option<&'a dyn ClassHierarchy>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodVisitor for MethodWriter<'_> {
    fn try_copy_method(&mut self, source: &MethodCopySource<'_, '_>) -> bool {
        self.state.try_copy_method(self.symbol_table, source)
    }

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        let state = &mut *self.state;
        let parameters = state.parameters.get_or_insert_with(ByteVector::new);
        state.parameters_count += 1;
        let name_index = name.map_or(0, |n| self.symbol_table.add_constant_utf8(n));
        parameters.put_short(name_index).put_short(access as i32);
    }

    fn visit_annotation_default<'b>(&'b mut self) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let default_value = self.state.default_value.insert(ByteVector::new());
        Some(Box::new(AnnotationWriter::new(
            self.symbol_table,
            false,
            default_value,
            false,
        )))
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.state
            .annotations
            .visit_annotation(self.symbol_table, descriptor, visible)
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.state.annotations.visit_type_annotation(
            self.symbol_table,
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        if visible {
            self.state.visible_annotable_parameter_count = i32::from(parameter_count);
        } else {
            self.state.invisible_annotable_parameter_count = i32::from(parameter_count);
        }
    }

    fn visit_parameter_annotation<'b>(
        &'b mut self,
        parameter: u8,
        annotation_descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let state = &mut *self.state;
        let parameter_annotations = if visible {
            &mut state.visible_parameter_annotations
        } else {
            &mut state.invisible_parameter_annotations
        };
        if parameter_annotations.is_none() {
            match Type::try_get_argument_count_of(self.symbol_table.text(state.descriptor)) {
                Ok(count) => *parameter_annotations = Some(vec![Vec::new(); count]),
                Err(error) => {
                    self.symbol_table.fail(error.into());
                    return None;
                }
            }
        }
        let parameter_annotations = parameter_annotations.as_mut().expect("created above");
        let length = parameter_annotations.len();
        let Some(list) = parameter_annotations.get_mut(usize::from(parameter)) else {
            self.symbol_table.fail(WriteError::IndexOutOfBounds(format!(
                "Index {parameter} out of bounds for length {length}"
            )));
            return None;
        };
        Some(Box::new(AnnotationWriter::create(
            self.symbol_table,
            annotation_descriptor,
            list,
        )))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        // Store the attributes in the <i>reverse</i> order of their visit by this method.
        if attribute.is_code_attribute() {
            self.state.code_attributes.push(attribute);
        } else {
            self.state.attributes.push(attribute);
        }
    }

    fn visit_code(&mut self) {
        // Nothing to do.
    }

    fn visit_code_length(&mut self, code_length: u32) {
        self.state.reserve_code(code_length);
    }

    fn visit_local_variable_count(&mut self, count: u16) {
        self.debug_tables
            .local_variables
            .reserve(usize::from(count) * LOCAL_VARIABLE_SIZE);
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        if let Err(error) = self
            .state
            .visit_frame(self.symbol_table, kind, locals, stack)
        {
            self.symbol_table.fail(error);
        }
    }

    fn visit_insn(&mut self, opcode: u8) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        state.code.put_byte(i32::from(opcode));
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode, 0, None);
        state.update_relative_stack_size_by_opcode(self.symbol_table, opcode);
        if state.current_basic_block.is_some()
            && ((opcodes::IRETURN..=opcodes::RETURN).contains(&opcode) || opcode == opcodes::ATHROW)
        {
            state.end_current_basic_block_with_no_successor();
        }
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        if opcode == opcodes::SIPUSH {
            state.code.put12(i32::from(opcode), operand);
        } else {
            // BIPUSH or NEWARRAY
            state.code.put11(i32::from(opcode), operand);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode, operand, None);
        if state.tracks_stack_size() && opcode != opcodes::NEWARRAY {
            // The stack size delta is 1 for BIPUSH or SIPUSH, and 0 for NEWARRAY.
            state.update_relative_stack_size(1);
        }
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let opcode = i32::from(opcode);
        let var_index = i32::from(var_index);
        if var_index < 4 && opcode != i32::from(opcodes::RET) {
            let optimized_opcode = if opcode < i32::from(opcodes::ISTORE) {
                i32::from(ILOAD_0) + ((opcode - i32::from(opcodes::ILOAD)) << 2) + var_index
            } else {
                i32::from(ISTORE_0) + ((opcode - i32::from(opcodes::ISTORE)) << 2) + var_index
            };
            state.code.put_byte(optimized_opcode);
        } else if var_index >= 256 {
            state
                .code
                .put_byte(i32::from(WIDE))
                .put12(opcode, var_index);
        } else {
            state.code.put11(opcode, var_index);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode as u8, var_index, None);
        if let Some(current_basic_block) = state.current_basic_block
            && state.tracks_stack_size()
        {
            if opcode == i32::from(opcodes::RET) {
                // No stack size delta.
                let block = state.labels.get_mut(current_basic_block);
                block.flags |= FLAG_SUBROUTINE_END;
                block.output_stack_size = state.relative_stack_size as i16;
                state.end_current_basic_block_with_no_successor();
            } else {
                // xLOAD or xSTORE
                state.update_relative_stack_size_by_opcode(self.symbol_table, opcode as u8);
            }
        }
        let long_or_double = [
            opcodes::LLOAD,
            opcodes::DLOAD,
            opcodes::LSTORE,
            opcodes::DSTORE,
        ]
        .contains(&(opcode as u8));
        state.update_max_locals(var_index + if long_or_double { 2 } else { 1 });
        if opcode >= i32::from(opcodes::ISTORE)
            && state.compute == Compute::AllFrames
            && !state.handlers.is_empty()
        {
            // If there are exception handler blocks, each instruction within a handler range is,
            // in theory, a basic block (since execution can jump from this instruction to the
            // exception handler). As a consequence, the local variable types at the beginning of
            // the handler block should be the merge of the local variable types at all the
            // instructions within the handler range. However, instead of creating a basic block
            // for each instruction, we can get the same result in a more efficient way. Namely,
            // by starting a new basic block after each xSTORE instruction, which is what we do
            // here.
            state.visit_label(Label::new());
        }
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let type_symbol = self.symbol_table.add_constant_class(type_);
        let (type_index, type_symbol) = (
            type_symbol.index,
            MethodWriterState::symbol_arg(type_symbol, JavaStr::from_str(""), type_),
        );
        state.code.put12(i32::from(opcode), type_index);
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        let last_bytecode_offset = state.last_bytecode_offset;
        state.execute(
            self.symbol_table,
            opcode,
            last_bytecode_offset,
            Some(&type_symbol),
        );
        if state.tracks_stack_size() && opcode == opcodes::NEW {
            // The stack size delta is 1 for NEW, and 0 for ANEWARRAY, CHECKCAST, or INSTANCEOF.
            state.update_relative_stack_size(1);
        }
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let fieldref_symbol = self
            .symbol_table
            .add_constant_fieldref(owner, name, descriptor);
        let (fieldref_index, fieldref_symbol) = (
            fieldref_symbol.index,
            MethodWriterState::symbol_arg(fieldref_symbol, name, descriptor),
        );
        state.code.put12(i32::from(opcode), fieldref_index);
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode, 0, Some(&fieldref_symbol));
        if state.tracks_stack_size() {
            let Some(&first_desc_char) = descriptor.as_bytes().first() else {
                // The descriptor's charAt(0).
                self.symbol_table.fail(WriteError::IndexOutOfBounds(
                    "Index 0 out of bounds for length 0".to_owned(),
                ));
                return;
            };
            let long_or_double = first_desc_char == b'D' || first_desc_char == b'J';
            let delta = match opcode {
                opcodes::GETSTATIC => {
                    if long_or_double {
                        2
                    } else {
                        1
                    }
                }
                opcodes::PUTSTATIC => {
                    if long_or_double {
                        -2
                    } else {
                        -1
                    }
                }
                opcodes::GETFIELD => i32::from(long_or_double),
                // PUTFIELD
                _ => {
                    if long_or_double {
                        -3
                    } else {
                        -2
                    }
                }
            };
            state.update_relative_stack_size(delta);
        }
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let methodref_symbol =
            self.symbol_table
                .add_constant_methodref(owner, name, descriptor, is_interface);
        let (methodref_index, methodref_symbol) = (
            methodref_symbol.index,
            MethodWriterState::symbol_arg(methodref_symbol, name, descriptor),
        );
        if opcode == opcodes::INVOKEINTERFACE {
            let arguments_and_return_sizes =
                match Type::try_get_arguments_and_return_sizes_of(descriptor) {
                    Ok(sizes) => sizes as i32,
                    Err(error) => {
                        self.symbol_table.fail(error.into());
                        0
                    }
                };
            state
                .code
                .put12(i32::from(opcodes::INVOKEINTERFACE), methodref_index)
                .put11(arguments_and_return_sizes >> 2, 0);
        } else {
            state.code.put12(i32::from(opcode), methodref_index);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode, 0, Some(&methodref_symbol));
        if state.tracks_stack_size() {
            let argument_and_return_sizes =
                match Type::try_get_arguments_and_return_sizes_of(descriptor) {
                    Ok(sizes) => sizes as i32,
                    Err(error) => {
                        self.symbol_table.fail(error.into());
                        return;
                    }
                };
            let stack_size_delta =
                (argument_and_return_sizes & 3) - (argument_and_return_sizes >> 2);
            if opcode == opcodes::INVOKESTATIC {
                state.update_relative_stack_size(stack_size_delta + 1);
            } else {
                state.update_relative_stack_size(stack_size_delta);
            }
        }
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let invoke_dynamic_symbol = self.symbol_table.add_constant_invoke_dynamic(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        let (invoke_dynamic_index, invoke_dynamic_symbol) = (
            invoke_dynamic_symbol.index,
            MethodWriterState::symbol_arg(invoke_dynamic_symbol, name, descriptor),
        );
        state
            .code
            .put12(i32::from(opcodes::INVOKEDYNAMIC), invoke_dynamic_index);
        state.code.put_short(0);
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(
            self.symbol_table,
            opcodes::INVOKEDYNAMIC,
            0,
            Some(&invoke_dynamic_symbol),
        );
        if state.tracks_stack_size() {
            let argument_and_return_sizes =
                match Type::try_get_arguments_and_return_sizes_of(descriptor) {
                    Ok(sizes) => sizes as i32,
                    Err(error) => {
                        self.symbol_table.fail(error.into());
                        return;
                    }
                };
            let stack_size_delta =
                (argument_and_return_sizes & 3) - (argument_and_return_sizes >> 2) + 1;
            state.update_relative_stack_size(stack_size_delta);
        }
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        // Compute the 'base' opcode, i.e. GOTO or JSR if opcode is GOTO_W or JSR_W, otherwise
        // opcode.
        let base_opcode = if opcode >= GOTO_W {
            opcode.wrapping_sub(WIDE_JUMP_OPCODE_DELTA)
        } else {
            opcode
        };
        let mut next_insn_is_jump_target = false;
        let code_length = state.code_length();
        let backward_offset = state
            .labels
            .resolved_offset(label)
            .map(|offset| offset.wrapping_sub(code_length));
        if backward_offset.is_some_and(|offset| offset < i32::from(i16::MIN)) {
            // Case of a backward jump with an offset < -32768. In this case we automatically
            // replace GOTO with GOTO_W, JSR with JSR_W and IFxxx <l> with IFNOTxxx <L> GOTO_W <l>
            // L:..., where IFNOTxxx is the "opposite" opcode of IFxxx (e.g. IFNE for IFEQ) and
            // where <L> designates the instruction just after the GOTO_W.
            if base_opcode == opcodes::GOTO {
                state.code.put_byte(i32::from(GOTO_W));
            } else if base_opcode == opcodes::JSR {
                state.code.put_byte(i32::from(JSR_W));
            } else {
                // Put the "opposite" opcode of baseOpcode. This can be done by flipping the least
                // significant bit for IFNULL and IFNONNULL, and similarly for IFEQ ... IF_ACMPEQ
                // (with a pre and post offset by 1). The jump offset is 8 bytes (3 for IFNOTxxx,
                // 5 for GOTO_W).
                let base_opcode = i32::from(base_opcode);
                state
                    .code
                    .put_byte(if base_opcode >= i32::from(opcodes::IFNULL) {
                        base_opcode ^ 1
                    } else {
                        ((base_opcode + 1) ^ 1) - 1
                    });
                state.code.put_short(8);
                // Here we could put a GOTO_W in theory, but if ASM specific instructions are used
                // in this method or another one, and if the class has frames, we will need to
                // insert a frame after this GOTO_W during the additional ClassReader ->
                // ClassWriter round trip to remove the ASM specific instructions. To not miss
                // this additional frame, we need to use an ASM_GOTO_W here, which has the
                // unfortunate effect of forcing this additional round trip (which in some case
                // would not have been really necessary, but we can't know this at this point).
                state.code.put_byte(i32::from(ASM_GOTO_W));
                state.has_asm_instructions = true;
                // The instruction after the GOTO_W becomes the target of the IFNOT instruction.
                next_insn_is_jump_target = true;
            }
            let source = state.code_length() - 1;
            state.labels.put(label, &mut state.code, source, true);
        } else if base_opcode != opcode {
            // Case of a GOTO_W or JSR_W specified by the user (normally ClassReader when used to
            // remove ASM specific instructions). In this case we keep the original instruction.
            state.code.put_byte(i32::from(opcode));
            let source = state.code_length() - 1;
            state.labels.put(label, &mut state.code, source, true);
        } else {
            // Case of a jump with an offset >= -32768, or of a jump with an unknown offset. In
            // these cases we store the offset in 2 bytes (which will be increased via a
            // ClassReader -> ClassWriter round trip if it turns out that 2 bytes are not
            // sufficient).
            state.code.put_byte(i32::from(base_opcode));
            let source = state.code_length() - 1;
            state.labels.put(label, &mut state.code, source, false);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        let Some(current_basic_block) = state.current_basic_block else {
            return;
        };
        let mut next_basic_block = None;
        match state.compute {
            Compute::AllFrames => {
                state.execute(self.symbol_table, base_opcode, 0, None);
                // Record the fact that 'label' is the target of a jump instruction.
                let target = state.labels.id(label);
                let canonical = state.labels.canonical(target);
                state.labels.get_mut(canonical).flags |= FLAG_JUMP_TARGET;
                // Add 'label' as a successor of the current basic block.
                state.add_successor_to_current_basic_block(EDGE_JUMP, target);
                if base_opcode != opcodes::GOTO {
                    // The next instruction starts a new basic block (except for GOTO: by default
                    // the code following a goto is unreachable - unless there is an explicit
                    // label for it - and we should not compute stack frame types for its
                    // instructions).
                    next_basic_block = Some(Label::new());
                }
            }
            Compute::InsertedFrames => {
                state.execute(self.symbol_table, base_opcode, 0, None);
            }
            Compute::MaxStackAndLocalFromFrames => {
                // No need to update maxRelativeStackSize (the stack size delta is always
                // negative).
                match control_flow::stack_size_delta(base_opcode) {
                    Ok(delta) => {
                        state.relative_stack_size = state.relative_stack_size.wrapping_add(delta);
                    }
                    Err(error) => self.symbol_table.fail(error),
                }
            }
            Compute::MaxStackAndLocal | Compute::Nothing => {
                let target = state.labels.id(label);
                if base_opcode == opcodes::JSR {
                    // Record the fact that 'label' designates a subroutine, if not already done.
                    let target_block = state.labels.get_mut(target);
                    if target_block.flags & FLAG_SUBROUTINE_START == 0 {
                        target_block.flags |= FLAG_SUBROUTINE_START;
                        state.has_subroutines = true;
                    }
                    state.labels.get_mut(current_basic_block).flags |= FLAG_SUBROUTINE_CALLER;
                    // Note that, by construction in this method, a block which calls a
                    // subroutine has at least two successors in the control flow graph: the first
                    // one (added below) leads to the instruction after the JSR, while the second
                    // one (added here) leads to the JSR target. Note that the first successor is
                    // virtual (it does not correspond to a possible execution path): it is only
                    // used to compute the successors of the basic blocks ending with a ret, in
                    // add_subroutine_ret_successors.
                    state.add_successor_to_current_basic_block(
                        state.relative_stack_size.wrapping_add(1),
                        target,
                    );
                    // The instruction after the JSR starts a new basic block.
                    next_basic_block = Some(Label::new());
                } else {
                    // No need to update maxRelativeStackSize (the stack size delta is always
                    // negative).
                    match control_flow::stack_size_delta(base_opcode) {
                        Ok(delta) => {
                            state.relative_stack_size =
                                state.relative_stack_size.wrapping_add(delta);
                        }
                        Err(error) => self.symbol_table.fail(error),
                    }
                    state.add_successor_to_current_basic_block(state.relative_stack_size, target);
                }
            }
        }
        // If the next instruction starts a new basic block, call visitLabel to add the label of
        // this instruction as a successor of the current block, and to start a new basic block.
        if let Some(next_basic_block) = next_basic_block {
            if next_insn_is_jump_target {
                state.labels.block_mut(next_basic_block).flags |= FLAG_JUMP_TARGET;
            }
            state.visit_label(next_basic_block);
        }
        if base_opcode == opcodes::GOTO {
            state.end_current_basic_block_with_no_successor();
        }
    }

    fn visit_label(&mut self, label: Label) {
        self.state.visit_label(label);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let constant_symbol = self.symbol_table.add_constant(value);
        let constant_index = constant_symbol.index;
        // The strings of the symbol that the frame computation reads: a dynamic constant's.
        let (name, descriptor) = match value {
            Constant::Dynamic(d) => (d.get_name(), d.get_descriptor()),
            _ => (JavaStr::from_str(""), JavaStr::from_str("")),
        };
        let constant_arg = MethodWriterState::symbol_arg(constant_symbol, name, descriptor);
        let is_long_or_double = match constant_arg.tag {
            CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => Ok(true),
            CONSTANT_DYNAMIC_TAG => match descriptor.as_bytes().first() {
                Some(b'J' | b'D') => Ok(true),
                Some(_) => Ok(false),
                // The descriptor's charAt(0).
                None => Err(WriteError::IndexOutOfBounds(
                    "Index 0 out of bounds for length 0".to_owned(),
                )),
            },
            _ => Ok(false),
        };
        let is_long_or_double = is_long_or_double.unwrap_or_else(|error| {
            self.symbol_table.fail(error);
            false
        });
        if is_long_or_double {
            state.code.put12(i32::from(LDC2_W), constant_index);
        } else if constant_index >= 256 {
            state.code.put12(i32::from(LDC_W), constant_index);
        } else {
            state.code.put11(i32::from(opcodes::LDC), constant_index);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcodes::LDC, 0, Some(&constant_arg));
        if state.tracks_stack_size() {
            state.update_relative_stack_size(if is_long_or_double { 2 } else { 1 });
        }
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let var_index = i32::from(var_index);
        let increment = i32::from(increment);
        if var_index > 255 || !(-128..=127).contains(&increment) {
            state
                .code
                .put_byte(i32::from(WIDE))
                .put12(i32::from(opcodes::IINC), var_index)
                .put_short(increment);
        } else {
            state
                .code
                .put_byte(i32::from(opcodes::IINC))
                .put11(var_index, increment);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcodes::IINC, var_index, None);
        state.update_max_locals(var_index + 1);
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        state.code.put_byte(i32::from(opcodes::TABLESWITCH));
        state.code.put_zeros((4 - state.code.size() % 4) % 4);
        let source = state.last_bytecode_offset;
        state.labels.put(dflt, &mut state.code, source, true);
        state.code.put_int(min).put_int(max);
        for &label in labels {
            state.labels.put(label, &mut state.code, source, true);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.visit_switch_insn(self.symbol_table, dflt, labels);
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        state.code.put_byte(i32::from(opcodes::LOOKUPSWITCH));
        state.code.put_zeros((4 - state.code.size() % 4) % 4);
        let source = state.last_bytecode_offset;
        state.labels.put(dflt, &mut state.code, source, true);
        state.code.put_int(labels.len() as i32);
        for (i, &label) in labels.iter().enumerate() {
            let Some(&key) = keys.get(i) else {
                self.symbol_table.fail(WriteError::IndexOutOfBounds(format!(
                    "Index {i} out of bounds for length {}",
                    keys.len()
                )));
                return;
            };
            state.code.put_int(key);
            state.labels.put(label, &mut state.code, source, true);
        }
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.visit_switch_insn(self.symbol_table, dflt, labels);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let desc_symbol = self.symbol_table.add_constant_class(descriptor);
        let (desc_index, desc_symbol) = (
            desc_symbol.index,
            MethodWriterState::symbol_arg(desc_symbol, JavaStr::from_str(""), descriptor),
        );
        state
            .code
            .put12(i32::from(opcodes::MULTIANEWARRAY), desc_index)
            .put_byte(i32::from(num_dimensions));
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(
            self.symbol_table,
            opcodes::MULTIANEWARRAY,
            i32::from(num_dimensions),
            Some(&desc_symbol),
        );
        if state.tracks_stack_size() {
            // No need to update maxRelativeStackSize (the stack size delta is always negative).
            state.relative_stack_size = state
                .relative_stack_size
                .wrapping_add(1 - i32::from(num_dimensions));
        }
    }

    fn visit_insn_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let state = &mut *self.state;
        let type_ref = TypeReference::new(
            (type_ref.get_value() & 0xFF00_00FF) | ((state.last_bytecode_offset as u32) << 8),
        );
        let list = if visible {
            &mut state.code_visible_type_annotations
        } else {
            &mut state.code_invisible_type_annotations
        };
        AnnotationWriter::create_type_annotation(
            self.symbol_table,
            type_ref,
            type_path,
            descriptor,
            list,
        )
        .map(|w| Box::new(w) as Box<dyn AnnotationVisitor + 'b>)
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        let catch_type = type_.map_or(0, |t| self.symbol_table.add_constant_class(t).index);
        self.state.handlers.push(Handler {
            start_pc: start,
            end_pc: end,
            handler_pc: handler,
            catch_type,
            catch_type_descriptor: type_.map(JavaCompactStr::new),
        });
    }

    fn visit_try_catch_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let state = &mut *self.state;
        let list = if visible {
            &mut state.code_visible_type_annotations
        } else {
            &mut state.code_invisible_type_annotations
        };
        AnnotationWriter::create_type_annotation(
            self.symbol_table,
            type_ref,
            type_path,
            descriptor,
            list,
        )
        .map(|w| Box::new(w) as Box<dyn AnnotationVisitor + 'b>)
    }

    fn visit_local_variable(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        let state = &mut *self.state;
        let start_offset = state.labels.offset(start);
        let length = state.labels.offset(end).wrapping_sub(start_offset);
        if let Some(signature) = signature {
            let name_index = self.symbol_table.add_constant_utf8(name);
            let signature_index = self.symbol_table.add_constant_utf8(signature);
            state
                .local_variable_type_table
                .add(&mut self.debug_tables.local_variable_types)
                .put_short(start_offset)
                .put_short(length)
                .put_short(name_index)
                .put_short(signature_index)
                .put_short(i32::from(index));
        }
        let name_index = self.symbol_table.add_constant_utf8(name);
        let descriptor_index = self.symbol_table.add_constant_utf8(descriptor);
        state
            .local_variable_table
            .add(&mut self.debug_tables.local_variables)
            .put_short(start_offset)
            .put_short(length)
            .put_short(name_index)
            .put_short(descriptor_index)
            .put_short(i32::from(index));
        if state.compute != Compute::Nothing {
            match descriptor.as_bytes().first() {
                Some(b'J' | b'D') => state.update_max_locals(i32::from(index) + 2),
                Some(_) => state.update_max_locals(i32::from(index) + 1),
                // The descriptor's charAt(0).
                None => self.symbol_table.fail(WriteError::IndexOutOfBounds(
                    "Index 0 out of bounds for length 0".to_owned(),
                )),
            }
        }
    }

    fn visit_local_variable_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let state = &mut *self.state;
        // Create a ByteVector to hold a 'type_annotation' JVMS structure.
        // See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.20.
        let mut type_annotation = ByteVector::new();
        // Write target_type, target_info, and target_path.
        type_annotation
            .put_byte((type_ref.get_value() >> 24) as i32)
            .put_short(start.len() as i32);
        for (i, &start) in start.iter().enumerate() {
            let (Some(&end), Some(&index)) = (end.get(i), index.get(i)) else {
                self.symbol_table.fail(WriteError::IndexOutOfBounds(format!(
                    "Index {i} out of bounds for length {}",
                    end.len().min(index.len())
                )));
                return None;
            };
            let start_offset = state.labels.offset(start);
            type_annotation
                .put_short(start_offset)
                .put_short(state.labels.offset(end).wrapping_sub(start_offset))
                .put_short(i32::from(index));
        }
        crate::writer::annotation_writer::put_type_path(type_path, &mut type_annotation);
        // Write type_index and reserve space for num_element_value_pairs.
        type_annotation
            .put_short(self.symbol_table.add_constant_utf8(descriptor))
            .put_short(0);
        let list = if visible {
            &mut state.code_visible_type_annotations
        } else {
            &mut state.code_invisible_type_annotations
        };
        list.push(type_annotation);
        let annotation = list.last_mut().expect("pushed above");
        Some(Box::new(AnnotationWriter::new(
            self.symbol_table,
            true,
            annotation,
            true,
        )))
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        let state = &mut *self.state;
        let start_offset = state.labels.offset(start);
        state
            .line_number_table
            .add(&mut self.debug_tables.line_numbers)
            .put_short(start_offset)
            .put_short(i32::from(line));
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.state
            .visit_maxs(self.symbol_table, self.hierarchy, max_stack, max_locals);
    }

    fn visit_end(&mut self) {
        // Not in ASM: release the label table, for the next method writer.
        self.state.release_labels();
    }
}

#[cfg(test)]
mod tests;
