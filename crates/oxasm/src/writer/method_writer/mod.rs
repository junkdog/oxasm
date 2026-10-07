//! ASM's `MethodWriter`, which generates the `method_info` structure of a method, and its Code
//! attribute.
//!
//! [`MethodWriterState`] holds what ASM's `MethodWriter` holds (the class writer owns one per
//! method, ADR-08), and [`MethodWriter`], the method visitor a class writer returns, borrows it
//! with the symbol table. `MethodWriter.java` is split along its sections:
//!
//! - this module: the constructor, and the implementation of the method visitor;
//! - `frames`: the stack map frames;
//! - `attributes`: the method copy optimization, and the size and content of the `method_info`.
//!
//! Only `COMPUTE_NOTHING` is implemented: the computation of the maximum stack size, of the
//! number of locals and of the stack map frames arrives in P6.

mod attributes;
mod frames;

use crate::attribute::Attribute;
use crate::constant::{Constant, FrameValue};
use crate::constants::{
    ACC_CONSTRUCTOR, ASM_GOTO_W, GOTO_W, ILOAD_0, ISTORE_0, JSR_W, LDC_W, LDC2_W, WIDE,
    WIDE_JUMP_OPCODE_DELTA,
};
use crate::error::WriteError;
use crate::handle::Handle;
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
use crate::writer::handler::Handler;
use crate::writer::label_table::LabelTable;
use crate::writer::symbol_table::{
    CONSTANT_DOUBLE_TAG, CONSTANT_DYNAMIC_TAG, CONSTANT_LONG_TAG, Symbol, SymbolTable,
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
    /// The name of this method.
    name: JavaCompactStr,
    /// The `descriptor_index` field of the `method_info` JVMS structure.
    descriptor_index: i32,
    /// The descriptor of this method.
    descriptor: JavaCompactStr,

    // Code attribute fields and sub attributes:
    /// The `max_stack` field of the Code attribute.
    max_stack: i32,
    /// The `max_locals` field of the Code attribute.
    max_locals: i32,
    /// The `code` field of the Code attribute.
    code: ByteVector,
    /// The `exception_table` of the Code attribute (ASM's `firstHandler` list).
    handlers: Vec<Handler>,
    /// The `line_number_table_length` field of the LineNumberTable code attribute.
    line_number_table_length: i32,
    /// The `line_number_table` array of the LineNumberTable code attribute, or `None`.
    line_number_table: Option<ByteVector>,
    /// The `local_variable_table_length` field of the LocalVariableTable code attribute.
    local_variable_table_length: i32,
    /// The `local_variable_table` array of the LocalVariableTable code attribute, or `None`.
    local_variable_table: Option<ByteVector>,
    /// The `local_variable_type_table_length` field of the LocalVariableTypeTable code
    /// attribute.
    local_variable_type_table_length: i32,
    /// The `local_variable_type_table` array of the LocalVariableTypeTable code attribute, or
    /// `None`.
    local_variable_type_table: Option<ByteVector>,
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
    /// The labels of this method (what ASM stores in its `Label`s).
    labels: LabelTable,
    /// The current basic block, i.e. the basic block of the last visited instruction. When
    /// `compute` is `InsertedFrames`, this field corresponds to the first basic block (the
    /// frame of the current instruction is computed in its frame, and its owner is updated).
    current_basic_block: Option<Label>,
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
    /// index 3 and are followed by the operand stack elements. In summary frame[0] = offset,
    /// frame[1] = numLocal, frame[2] = numStack. Local variables and operand stack entries
    /// contain abstract types, as defined in ASM's `Frame`, but restricted to CONSTANT_KIND,
    /// REFERENCE_KIND, UNINITIALIZED_KIND or FORWARD_UNINITIALIZED_KIND abstract types. Long and
    /// double types use only one array entry.
    current_frame: Vec<i32>,

    // Other miscellaneous status fields
    /// Whether this method contains ASM specific instructions.
    has_asm_instructions: bool,
    /// The start offset of the last visited instruction. Used to set the offset field of type
    /// annotations of type 'offset_target' (see JVMS 4.7.20.1).
    last_bytecode_offset: i32,
    /// The `method_info` attributes of the method, less its first 6 bytes, if they are copied as
    /// is from a class reader (ASM's `sourceOffset` and `sourceLength`, copied at once,
    /// ADR-08).
    copied_method_info: Option<Vec<u8>>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl MethodWriterState {
    /// Constructs a new method writer state (ASM's `MethodWriter` constructor).
    pub(crate) fn new(
        symbol_table: &mut SymbolTable,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
        compute: Compute,
    ) -> MethodWriterState {
        let access_flags = if name == "<init>" {
            access | ACC_CONSTRUCTOR
        } else {
            access
        };
        let name_index = symbol_table.add_constant_utf8(name);
        let descriptor_index = symbol_table.add_constant_utf8(descriptor);
        let signature_index = signature.map_or(0, |s| symbol_table.add_constant_utf8(s));
        let exception_index_table = exceptions
            .iter()
            .map(|exception| symbol_table.add_constant_class(exception).index)
            .collect();
        let compute = if matches!(compute, Compute::Nothing | Compute::InsertedFrames) {
            compute
        } else {
            symbol_table.fail(WriteError::NotYetPorted(
                "computing max stack, max locals or stack map frames (P6)",
            ));
            Compute::Nothing
        };
        let mut state = MethodWriterState {
            access_flags,
            name_index,
            name: JavaCompactStr::new(name),
            descriptor_index,
            descriptor: JavaCompactStr::new(descriptor),
            max_stack: 0,
            max_locals: 0,
            code: ByteVector::new(),
            handlers: Vec::new(),
            line_number_table_length: 0,
            line_number_table: None,
            local_variable_table_length: 0,
            local_variable_table: None,
            local_variable_type_table_length: 0,
            local_variable_type_table: None,
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
            current_basic_block: None,
            current_locals: 0,
            previous_frame_offset: 0,
            previous_frame: None,
            current_frame: Vec::new(),
            has_asm_instructions: false,
            last_bytecode_offset: 0,
            copied_method_info: None,
        };
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
            state.visit_label(Label::new());
        }
        state
    }

    /// ASM's `visitLabel`.
    fn visit_label(&mut self, label: Label) {
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
        if label.is_debug_only() {
            return;
        }
        if self.compute == Compute::InsertedFrames {
            match self.current_basic_block {
                // This case should happen only once, for the visitLabel call in the constructor.
                // Indeed, if compute is equal to COMPUTE_INSERTED_FRAMES, currentBasicBlock stays
                // unchanged.
                None => self.current_basic_block = Some(label),
                // Update the frame owner so that a correct frame offset is computed in
                // Frame.accept().
                Some(current_basic_block) => {
                    if let Some(frame) = self.labels.block_mut(current_basic_block).frame.as_mut() {
                        frame.owner = Some(label);
                    }
                }
            }
        }
        // The other computations (COMPUTE_ALL_FRAMES, COMPUTE_MAX_STACK_AND_LOCAL...) arrive in
        // P6.
    }

    /// Whether the stack map frames are computed, i.e. the instructions are executed on the
    /// frame of the current basic block.
    fn computes_frames(&self) -> bool {
        matches!(self.compute, Compute::AllFrames | Compute::InsertedFrames)
    }

    /// Simulates the given instruction on the frame of the current basic block, if the stack
    /// map frames are computed (ASM's `currentBasicBlock.frame.execute`).
    fn execute(
        &mut self,
        symbol_table: &mut SymbolTable,
        opcode: u8,
        arg: i32,
        arg_symbol: Option<&SymbolArg>,
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
        let result = match self
            .labels
            .block_mut(current_basic_block)
            .frame
            .as_deref_mut()
        {
            Some(frame) if inserted => frame.execute_current(opcode, arg, arg_symbol, symbol_table),
            Some(frame) => frame.execute(opcode, arg, arg_symbol, symbol_table),
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

    /// A copy of a symbol, if the stack map frames are computed.
    fn symbol_arg(&self, symbol: &Symbol) -> Option<SymbolArg> {
        self.computes_frames().then(|| SymbolArg::of(symbol))
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
}

/// A [`MethodVisitor`] that generates a corresponding `method_info` structure, as defined in the
/// Java Virtual Machine Specification (JVMS).
pub(crate) struct MethodWriter<'a> {
    pub(crate) symbol_table: &'a mut SymbolTable,
    pub(crate) state: &'a mut MethodWriterState,
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
            match Type::try_get_argument_count_of(&*state.descriptor) {
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
        // COMPUTE_MAX_STACK_AND_LOCAL and the end of the basic block arrive in P6.
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
        let long_or_double = [
            opcodes::LLOAD,
            opcodes::DLOAD,
            opcodes::LSTORE,
            opcodes::DSTORE,
        ]
        .contains(&(opcode as u8));
        state.update_max_locals(var_index + if long_or_double { 2 } else { 1 });
        // The new basic block after an xSTORE in a handler range (COMPUTE_ALL_FRAMES) arrives
        // in P6.
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let type_symbol = self.symbol_table.add_constant_class(type_);
        let (type_index, type_symbol) = (type_symbol.index, state.symbol_arg(type_symbol));
        state.code.put12(i32::from(opcode), type_index);
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        let last_bytecode_offset = state.last_bytecode_offset;
        state.execute(
            self.symbol_table,
            opcode,
            last_bytecode_offset,
            type_symbol.as_ref(),
        );
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
        let (fieldref_index, fieldref_symbol) =
            (fieldref_symbol.index, state.symbol_arg(fieldref_symbol));
        state.code.put12(i32::from(opcode), fieldref_index);
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(self.symbol_table, opcode, 0, fieldref_symbol.as_ref());
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
        let (methodref_index, methodref_symbol) =
            (methodref_symbol.index, state.symbol_arg(methodref_symbol));
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
        state.execute(self.symbol_table, opcode, 0, methodref_symbol.as_ref());
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
            state.symbol_arg(invoke_dynamic_symbol),
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
            invoke_dynamic_symbol.as_ref(),
        );
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
                // The instruction after the GOTO_W becomes the target of the IFNOT instruction
                // (which only matters to the frame computation, P6).
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
        // The control flow graph (COMPUTE_ALL_FRAMES, COMPUTE_MAX_STACK_AND_LOCAL...) arrives in
        // P6.
        if state.current_basic_block.is_some() && state.compute == Compute::InsertedFrames {
            state.execute(self.symbol_table, base_opcode, 0, None);
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
        let constant_arg = state.symbol_arg(constant_symbol);
        let is_long_or_double = match constant_symbol.tag {
            CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => Ok(true),
            CONSTANT_DYNAMIC_TAG => match constant_symbol.value().as_bytes().first() {
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
        state.execute(self.symbol_table, opcodes::LDC, 0, constant_arg.as_ref());
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
        // If needed, update the maximum stack size and number of locals, and stack map frames
        // (ASM's visitSwitchInsn, which does nothing for COMPUTE_INSERTED_FRAMES; the other
        // computations arrive in P6).
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
        // If needed, update the maximum stack size and number of locals, and stack map frames
        // (ASM's visitSwitchInsn, which does nothing for COMPUTE_INSERTED_FRAMES; the other
        // computations arrive in P6).
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        let state = &mut *self.state;
        state.last_bytecode_offset = state.code_length();
        // Add the instruction to the bytecode of the method.
        let desc_symbol = self.symbol_table.add_constant_class(descriptor);
        let (desc_index, desc_symbol) = (desc_symbol.index, state.symbol_arg(desc_symbol));
        state
            .code
            .put12(i32::from(opcodes::MULTIANEWARRAY), desc_index)
            .put_byte(i32::from(num_dimensions));
        // If needed, update the maximum stack size and number of locals, and stack map frames.
        state.execute(
            self.symbol_table,
            opcodes::MULTIANEWARRAY,
            i32::from(num_dimensions),
            desc_symbol.as_ref(),
        );
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
            state.local_variable_type_table_length += 1;
            let name_index = self.symbol_table.add_constant_utf8(name);
            let signature_index = self.symbol_table.add_constant_utf8(signature);
            state
                .local_variable_type_table
                .get_or_insert_with(ByteVector::new)
                .put_short(start_offset)
                .put_short(length)
                .put_short(name_index)
                .put_short(signature_index)
                .put_short(i32::from(index));
        }
        state.local_variable_table_length += 1;
        let name_index = self.symbol_table.add_constant_utf8(name);
        let descriptor_index = self.symbol_table.add_constant_utf8(descriptor);
        state
            .local_variable_table
            .get_or_insert_with(ByteVector::new)
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
        state.line_number_table_length += 1;
        let start_offset = state.labels.offset(start);
        state
            .line_number_table
            .get_or_insert_with(ByteVector::new)
            .put_short(start_offset)
            .put_short(i32::from(line));
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        // COMPUTE_ALL_FRAMES, COMPUTE_MAX_STACK_AND_LOCAL and
        // COMPUTE_MAX_STACK_AND_LOCAL_FROM_FRAMES arrive in P6.
        self.state.max_stack = i32::from(max_stack);
        self.state.max_locals = i32::from(max_locals);
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

#[cfg(test)]
mod tests;
