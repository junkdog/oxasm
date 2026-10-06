//! ASM's `ClassReader` section "Methods to parse stack map frames".

use std::borrow::Cow;

use crate::constant::FrameValue;
use crate::constants::{
    ITEM_DOUBLE, ITEM_FLOAT, ITEM_INTEGER, ITEM_LONG, ITEM_NULL, ITEM_OBJECT, ITEM_TOP,
    ITEM_UNINITIALIZED, ITEM_UNINITIALIZED_THIS,
};
use crate::jstr::{JavaStr, JavaString};
use crate::label::Label;
use crate::opcodes::ACC_STATIC;

use super::code::create_label;
use super::context::Context;
use super::{ClassReader, ReadError, ReadErrorKind, required};

// The stack map frame types (ASM's `Frame.*_FRAME` constants).
pub(super) const SAME_LOCALS_1_STACK_ITEM_FRAME: u8 = 64;
pub(super) const RESERVED: u8 = 128;
pub(super) const SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED: u8 = 247;
pub(super) const CHOP_FRAME: u8 = 248;
pub(super) const SAME_FRAME_EXTENDED: u8 = 251;
pub(super) const FULL_FRAME: u8 = 255;

// ASM's `Opcodes.F_*` values, which `Context::current_frame_type` holds.
pub(super) const F_FULL: i32 = 0;
pub(super) const F_APPEND: i32 = 1;
pub(super) const F_CHOP: i32 = 2;
pub(super) const F_SAME: i32 = 3;
pub(super) const F_SAME1: i32 = 4;

/// `array[index] = value` for ASM's `new Object[capacity]` array, failing as Java does for an
/// index out of the array. The array grows on demand: only the written prefix is ever visited.
fn store<'a>(
    array: &mut Vec<FrameValue<'a>>,
    capacity: usize,
    index: i32,
    value: FrameValue<'a>,
    offset: i64,
) -> Result<(), ReadError> {
    let index = usize::try_from(index)
        .ok()
        .filter(|&i| i < capacity)
        .ok_or_else(|| ReadError::new(offset, ReadErrorKind::InvalidLength(index.into())))?;
    if index >= array.len() {
        array.resize(index + 1, FrameValue::Top);
    }
    array[index] = value;
    Ok(())
}

impl<'b> ClassReader<'b> {
    /// Computes the implicit frame of the method currently being parsed (as defined in the
    /// given context) and stores it in the given context.
    pub(super) fn compute_implicit_frame<'s>(
        &'s self,
        context: &mut Context<'s>,
        code_offset: i64,
    ) -> Result<(), ReadError> {
        let method_descriptor = required(context.current_method_descriptor, code_offset)?;
        let descriptor = Descriptor::new(method_descriptor);
        let locals = &mut context.current_frame_local_types;
        let capacity = context.current_frame_local_capacity;
        let mut num_local = 0;
        if (context.current_method_access_flags & ACC_STATIC) == 0 {
            if context.current_method_name.is_some_and(|n| n == "<init>") {
                store(
                    locals,
                    capacity,
                    num_local,
                    FrameValue::UninitializedThis,
                    code_offset,
                )?;
            } else {
                let this_class = self.read_class_at(self.header + 2)?;
                let this_class = required(this_class, self.header + 2)?;
                let value = FrameValue::Object(Cow::Borrowed(this_class));
                store(locals, capacity, num_local, value, code_offset)?;
            }
            num_local += 1;
        }
        let char_at = |i: usize| descriptor.char_at(i, code_offset);
        // Parse the method descriptor, one argument type descriptor at each iteration. Start by
        // skipping the first method descriptor character, which is always '('.
        let mut current_method_descriptor_offset = 1;
        loop {
            let current_argument_descriptor_start_offset = current_method_descriptor_offset;
            let c = char_at(current_method_descriptor_offset)?;
            current_method_descriptor_offset += 1;
            let value = match c {
                Z | C | B | S | I => FrameValue::Integer,
                F => FrameValue::Float,
                J => FrameValue::Long,
                D => FrameValue::Double,
                ARRAY => {
                    while char_at(current_method_descriptor_offset)? == ARRAY {
                        current_method_descriptor_offset += 1;
                    }
                    if char_at(current_method_descriptor_offset)? == L {
                        current_method_descriptor_offset += 1;
                        while char_at(current_method_descriptor_offset)? != SEMICOLON {
                            current_method_descriptor_offset += 1;
                        }
                    }
                    current_method_descriptor_offset += 1;
                    FrameValue::Object(descriptor.substring(
                        current_argument_descriptor_start_offset,
                        current_method_descriptor_offset,
                    ))
                }
                L => {
                    while char_at(current_method_descriptor_offset)? != SEMICOLON {
                        current_method_descriptor_offset += 1;
                    }
                    let value = FrameValue::Object(descriptor.substring(
                        current_argument_descriptor_start_offset + 1,
                        current_method_descriptor_offset,
                    ));
                    current_method_descriptor_offset += 1;
                    value
                }
                _ => {
                    context.current_frame_local_count = num_local;
                    return Ok(());
                }
            };
            store(locals, capacity, num_local, value, code_offset)?;
            num_local += 1;
        }
    }

    /// Reads a JVMS 'stack_map_frame' structure and stores the result in the given context
    /// object. This method can also be used to read a full_frame structure, excluding its
    /// frame_type field (this is used to parse the legacy StackMap attributes).
    ///
    /// - `compressed`: whether to read a 'stack_map_frame' structure (`true`) or a 'full_frame'
    ///   structure without its frame_type field (`false`).
    /// - `expand`: whether the stack map frame must be expanded.
    ///
    /// Returns the end offset of the JVMS 'stack_map_frame' or 'full_frame' structure.
    pub(super) fn read_stack_map_frame<'s>(
        &'s self,
        stack_map_frame_offset: i64,
        compressed: bool,
        expand: bool,
        context: &mut Context<'s>,
    ) -> Result<i64, ReadError> {
        let mut current_offset = stack_map_frame_offset;
        let frame_type = if compressed {
            // Read the frame_type field.
            let frame_type = self.u8_at(current_offset)?;
            current_offset += 1;
            frame_type
        } else {
            context.current_frame_offset = -1;
            FULL_FRAME
        };
        let offset_delta: i32;
        context.current_frame_local_count_delta = 0;
        if frame_type < SAME_LOCALS_1_STACK_ITEM_FRAME {
            offset_delta = i32::from(frame_type);
            context.current_frame_type = F_SAME;
            context.current_frame_stack_count = 0;
        } else if frame_type < RESERVED {
            offset_delta = i32::from(frame_type - SAME_LOCALS_1_STACK_ITEM_FRAME);
            current_offset =
                self.read_verification_type_info(current_offset, FrameSlot::Stack, 0, context)?;
            context.current_frame_type = F_SAME1;
            context.current_frame_stack_count = 1;
        } else if frame_type >= SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED {
            offset_delta = i32::from(self.u16_at(current_offset)?);
            current_offset += 2;
            if frame_type == SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED {
                current_offset =
                    self.read_verification_type_info(current_offset, FrameSlot::Stack, 0, context)?;
                context.current_frame_type = F_SAME1;
                context.current_frame_stack_count = 1;
            } else if (CHOP_FRAME..SAME_FRAME_EXTENDED).contains(&frame_type) {
                context.current_frame_type = F_CHOP;
                context.current_frame_local_count_delta =
                    i32::from(SAME_FRAME_EXTENDED - frame_type);
                context.current_frame_local_count -= context.current_frame_local_count_delta;
                context.current_frame_stack_count = 0;
            } else if frame_type == SAME_FRAME_EXTENDED {
                context.current_frame_type = F_SAME;
                context.current_frame_stack_count = 0;
            } else if frame_type < FULL_FRAME {
                let first_local = if expand {
                    context.current_frame_local_count
                } else {
                    0
                };
                let count = i32::from(frame_type - SAME_FRAME_EXTENDED);
                for local in first_local..first_local + count {
                    current_offset = self.read_verification_type_info(
                        current_offset,
                        FrameSlot::Local,
                        local,
                        context,
                    )?;
                }
                context.current_frame_type = F_APPEND;
                context.current_frame_local_count_delta =
                    i32::from(frame_type - SAME_FRAME_EXTENDED);
                context.current_frame_local_count += context.current_frame_local_count_delta;
                context.current_frame_stack_count = 0;
            } else {
                let number_of_locals = i32::from(self.u16_at(current_offset)?);
                current_offset += 2;
                context.current_frame_type = F_FULL;
                context.current_frame_local_count_delta = number_of_locals;
                context.current_frame_local_count = number_of_locals;
                for local in 0..number_of_locals {
                    current_offset = self.read_verification_type_info(
                        current_offset,
                        FrameSlot::Local,
                        local,
                        context,
                    )?;
                }
                let number_of_stack_items = i32::from(self.u16_at(current_offset)?);
                current_offset += 2;
                context.current_frame_stack_count = number_of_stack_items;
                for stack in 0..number_of_stack_items {
                    current_offset = self.read_verification_type_info(
                        current_offset,
                        FrameSlot::Stack,
                        stack,
                        context,
                    )?;
                }
            }
        } else {
            return Err(ReadError::new(
                current_offset - 1,
                ReadErrorKind::InvalidFrameType(frame_type),
            ));
        }
        context.current_frame_offset = context.current_frame_offset.wrapping_add(offset_delta + 1);
        create_label(
            i64::from(context.current_frame_offset),
            &mut context.current_method_labels,
        )?;
        Ok(current_offset)
    }

    /// Reads a JVMS 'verification_type_info' structure and stores it at the given index in the
    /// given array.
    ///
    /// Returns the end offset of the JVMS 'verification_type_info' structure.
    fn read_verification_type_info<'s>(
        &'s self,
        verification_type_info_offset: i64,
        slot: FrameSlot,
        index: i32,
        context: &mut Context<'s>,
    ) -> Result<i64, ReadError> {
        let mut current_offset = verification_type_info_offset;
        let tag = self.u8_at(current_offset)?;
        current_offset += 1;
        let value = match tag {
            ITEM_TOP => FrameValue::Top,
            ITEM_INTEGER => FrameValue::Integer,
            ITEM_FLOAT => FrameValue::Float,
            ITEM_DOUBLE => FrameValue::Double,
            ITEM_LONG => FrameValue::Long,
            ITEM_NULL => FrameValue::Null,
            ITEM_UNINITIALIZED_THIS => FrameValue::UninitializedThis,
            ITEM_OBJECT => {
                let class = self.read_class_at(current_offset)?;
                current_offset += 2;
                FrameValue::Object(Cow::Borrowed(required(class, current_offset - 2)?))
            }
            ITEM_UNINITIALIZED => {
                let bytecode_offset = i64::from(self.u16_at(current_offset)?);
                let label: Label =
                    create_label(bytecode_offset, &mut context.current_method_labels)?;
                current_offset += 2;
                FrameValue::Uninitialized(label)
            }
            _ => {
                return Err(ReadError::new(
                    current_offset - 1,
                    ReadErrorKind::InvalidVerificationType(tag),
                ));
            }
        };
        let (frame, capacity) = match slot {
            FrameSlot::Local => (
                &mut context.current_frame_local_types,
                context.current_frame_local_capacity,
            ),
            FrameSlot::Stack => (
                &mut context.current_frame_stack_types,
                context.current_frame_stack_capacity,
            ),
        };
        store(frame, capacity, index, value, verification_type_info_offset)?;
        Ok(current_offset)
    }
}

/// Which array of the context `read_verification_type_info` stores into (ASM passes the array).
#[derive(Clone, Copy)]
enum FrameSlot {
    Local,
    Stack,
}

// The descriptor characters that `compute_implicit_frame` looks for, as UTF-16 units.
const Z: u16 = b'Z' as u16;
const C: u16 = b'C' as u16;
const B: u16 = b'B' as u16;
const S: u16 = b'S' as u16;
const I: u16 = b'I' as u16;
const F: u16 = b'F' as u16;
const J: u16 = b'J' as u16;
const D: u16 = b'D' as u16;
const L: u16 = b'L' as u16;
const ARRAY: u16 = b'[' as u16;
const SEMICOLON: u16 = b';' as u16;

/// A method descriptor as Java's `String`, indexed by UTF-16 unit: its bytes when it is ASCII
/// (almost always), and its UTF-16 units otherwise.
enum Descriptor<'s> {
    Ascii(&'s JavaStr),
    Utf16(Vec<u16>),
}

impl<'s> Descriptor<'s> {
    fn new(descriptor: &'s JavaStr) -> Descriptor<'s> {
        if descriptor.is_ascii() {
            Descriptor::Ascii(descriptor)
        } else {
            Descriptor::Utf16(descriptor.encode_utf16().collect())
        }
    }

    /// Java's `String.charAt`, which throws a `StringIndexOutOfBoundsException` past the end.
    fn char_at(&self, index: usize, offset: i64) -> Result<u16, ReadError> {
        let c = match self {
            Descriptor::Ascii(s) => s.as_bytes().get(index).map(|&b| u16::from(b)),
            Descriptor::Utf16(units) => units.get(index).copied(),
        };
        c.ok_or_else(|| ReadError::new(offset, ReadErrorKind::InvalidLength(index as i64)))
    }

    /// Java's `String.substring(begin, end)`, for indices that `char_at` has validated.
    fn substring(&self, begin: usize, end: usize) -> Cow<'s, JavaStr> {
        match self {
            Descriptor::Ascii(s) => Cow::Borrowed(s.get(begin..end).unwrap_or_default()),
            Descriptor::Utf16(units) => Cow::Owned(JavaString::from_utf16(
                units.get(begin..end).unwrap_or_default(),
            )),
        }
    }
}
