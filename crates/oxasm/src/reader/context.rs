//! [`Context`]: ASM's `Context`, the information about a class being parsed in a class reader.

use crate::attribute::AttributeCodec;
use crate::constant::FrameValue;
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::type_path::TypePath;

use super::ParsingOptions;
use super::code::MethodLabels;

/// Information about a class being parsed in a [`ClassReader`](super::ClassReader). `'a` is the
/// lifetime of the reader borrow (the strings come from its cache) and of the codecs.
pub(super) struct Context<'a> {
    /// The prototypes of the attributes that must be parsed in this class.
    pub(super) codecs: &'a [&'a dyn AttributeCodec],

    /// The options used to parse this class.
    pub(super) parsing_options: ParsingOptions,

    // Information about the current method, i.e. the one read in the current (or latest) call
    // to `read_method`.
    /// The access flags of the current method.
    pub(super) current_method_access_flags: u32,
    /// The name of the current method.
    pub(super) current_method_name: Option<&'a JavaStr>,
    /// The descriptor of the current method.
    pub(super) current_method_descriptor: Option<&'a JavaStr>,
    /// The labels of the current method, indexed by bytecode offset: the thread's table,
    /// given back when the context is dropped.
    pub(super) current_method_labels: MethodLabels,
    /// The `(start_pc, line_number)` entries of the current method's line number tables (ASM's
    /// `Label.lineNumber` and `Label.otherLineNumbers`), sorted by `start_pc` once read, and in
    /// the order they were read for each `start_pc`.
    pub(super) current_method_line_numbers: Vec<(u16, u16)>,

    // Information about the current type annotation target, i.e. the one read in the current
    // (or latest) call to `read_type_annotation_target`.
    /// The target_type and target_info of the current type annotation target, encoded as
    /// described in [`TypeReference`](crate::TypeReference).
    pub(super) current_type_annotation_target: u32,
    /// The target_path of the current type annotation target.
    pub(super) current_type_annotation_target_path: Option<TypePath>,
    /// The start of each local variable range in the current local variable annotation.
    pub(super) current_local_variable_annotation_range_starts: Vec<Label>,
    /// The end of each local variable range in the current local variable annotation.
    pub(super) current_local_variable_annotation_range_ends: Vec<Label>,
    /// The local variable index of each local variable range in the current local variable
    /// annotation.
    pub(super) current_local_variable_annotation_range_indices: Vec<u16>,

    // Information about the current stack map frame, i.e. the one read in the current (or
    // latest) call to `read_stack_map_frame`.
    /// The bytecode offset of the current stack map frame.
    pub(super) current_frame_offset: i32,
    /// The type of the current stack map frame. One of `F_FULL`, `F_APPEND`, `F_CHOP`, `F_SAME`
    /// or `F_SAME1` (ASM's values).
    pub(super) current_frame_type: i32,
    /// The number of local variable types in the current stack map frame. Each type is
    /// represented with a single array element (even long and double).
    pub(super) current_frame_local_count: i32,
    /// The delta number of local variable types in the current stack map frame (each type is
    /// represented with a single array element - even long and double). This is the number of
    /// local variable types in this frame, minus the number of local variable types in the
    /// previous frame.
    pub(super) current_frame_local_count_delta: i32,
    /// The types of the local variables in the current stack map frame (the elements past the
    /// count are left over from previous frames). ASM allocates `max_locals` of them; this
    /// grows on demand up to `current_frame_local_capacity`.
    pub(super) current_frame_local_types: Vec<FrameValue<'a>>,
    /// The `max_locals` of the current method: the size of ASM's `currentFrameLocalTypes`.
    pub(super) current_frame_local_capacity: usize,
    /// The number of stack element types in the current stack map frame. Each type is
    /// represented with a single array element (even long and double).
    pub(super) current_frame_stack_count: i32,
    /// The types of the stack elements in the current stack map frame (growing on demand up to
    /// `current_frame_stack_capacity`).
    pub(super) current_frame_stack_types: Vec<FrameValue<'a>>,
    /// The `max_stack` of the current method: the size of ASM's `currentFrameStackTypes`.
    pub(super) current_frame_stack_capacity: usize,
}

impl<'a> Context<'a> {
    pub(super) fn new(
        codecs: &'a [&'a dyn AttributeCodec],
        parsing_options: ParsingOptions,
    ) -> Context<'a> {
        Context {
            codecs,
            parsing_options,
            current_method_access_flags: 0,
            current_method_name: None,
            current_method_descriptor: None,
            current_method_labels: MethodLabels::take(),
            current_method_line_numbers: Vec::new(),
            current_type_annotation_target: 0,
            current_type_annotation_target_path: None,
            current_local_variable_annotation_range_starts: Vec::new(),
            current_local_variable_annotation_range_ends: Vec::new(),
            current_local_variable_annotation_range_indices: Vec::new(),
            current_frame_offset: 0,
            current_frame_type: 0,
            current_frame_local_count: 0,
            current_frame_local_count_delta: 0,
            current_frame_local_types: Vec::new(),
            current_frame_local_capacity: 0,
            current_frame_stack_count: 0,
            current_frame_stack_types: Vec::new(),
            current_frame_stack_capacity: 0,
        }
    }
}

impl Drop for Context<'_> {
    fn drop(&mut self) {
        MethodLabels::put_back(std::mem::take(&mut self.current_method_labels));
    }
}
