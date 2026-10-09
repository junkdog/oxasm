//! ASM's `Frame`: the abstract types of the stack map frames a method writer writes or computes.
//!
//! An abstract type is an `i32` with ASM's exact layout (DIM, KIND, FLAGS and VALUE fields), so
//! that type table indices and frame comparisons are those of ASM. This module holds what a
//! method writer needs to write frames given to it (the conversions from the API format, the
//! implicit first frame of an expanded method, and `putAbstractType`), and the frame computation
//! (`execute`, `merge`).

use std::borrow::Cow;

use crate::constant::FrameValue;
use crate::constants::ACC_CONSTRUCTOR;
use crate::error::WriteError;
use crate::hierarchy::ClassHierarchy;
use crate::jstr::{JavaCodePoint, JavaStr, JavaString};
use crate::label::Label;
use crate::opcodes;
use crate::ty::Type;
use crate::writer::byte_vector::ByteVector;
use crate::writer::label_table::BlockId;
use crate::writer::symbol_table::SymbolTable;

// Constants used in the StackMapTable attribute.
// See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.4.

pub(crate) const SAME_FRAME: i32 = 0;
pub(crate) const SAME_LOCALS_1_STACK_ITEM_FRAME: i32 = 64;
#[allow(dead_code)] // as in ASM
pub(crate) const RESERVED: i32 = 128;
pub(crate) const SAME_LOCALS_1_STACK_ITEM_FRAME_EXTENDED: i32 = 247;
pub(crate) const CHOP_FRAME: i32 = 248;
pub(crate) const SAME_FRAME_EXTENDED: i32 = 251;
pub(crate) const APPEND_FRAME: i32 = 252;
pub(crate) const FULL_FRAME: i32 = 255;

pub(crate) const ITEM_TOP: i32 = 0;
pub(crate) const ITEM_INTEGER: i32 = 1;
pub(crate) const ITEM_FLOAT: i32 = 2;
pub(crate) const ITEM_DOUBLE: i32 = 3;
pub(crate) const ITEM_LONG: i32 = 4;
pub(crate) const ITEM_NULL: i32 = 5;
pub(crate) const ITEM_UNINITIALIZED_THIS: i32 = 6;
pub(crate) const ITEM_OBJECT: i32 = 7;
pub(crate) const ITEM_UNINITIALIZED: i32 = 8;
// Additional, ASM specific constants used in abstract types below.
const ITEM_ASM_BOOLEAN: i32 = 9;
const ITEM_ASM_BYTE: i32 = 10;
const ITEM_ASM_CHAR: i32 = 11;
const ITEM_ASM_SHORT: i32 = 12;

// The size and offset in bits of each field of an abstract type.
const DIM_SIZE: i32 = 6;
const KIND_SIZE: i32 = 4;
const FLAGS_SIZE: i32 = 2;
const VALUE_SIZE: i32 = 32 - DIM_SIZE - KIND_SIZE - FLAGS_SIZE;

const DIM_SHIFT: i32 = KIND_SIZE + FLAGS_SIZE + VALUE_SIZE;
const KIND_SHIFT: i32 = FLAGS_SIZE + VALUE_SIZE;
const FLAGS_SHIFT: i32 = VALUE_SIZE;

// Bitmasks to get each field of an abstract type.
const DIM_MASK: i32 = ((1 << DIM_SIZE) - 1) << DIM_SHIFT;
const KIND_MASK: i32 = ((1 << KIND_SIZE) - 1) << KIND_SHIFT;
const VALUE_MASK: i32 = (1 << VALUE_SIZE) - 1;

// Constants to manipulate the KIND field of an abstract type.
pub(crate) const CONSTANT_KIND: i32 = 1 << KIND_SHIFT;
pub(crate) const REFERENCE_KIND: i32 = 2 << KIND_SHIFT;
pub(crate) const UNINITIALIZED_KIND: i32 = 3 << KIND_SHIFT;
pub(crate) const FORWARD_UNINITIALIZED_KIND: i32 = 4 << KIND_SHIFT;

// Useful predefined abstract types (all the possible CONSTANT_KIND types).
pub(crate) const TOP: i32 = CONSTANT_KIND | ITEM_TOP;
const BOOLEAN: i32 = CONSTANT_KIND | ITEM_ASM_BOOLEAN;
const BYTE: i32 = CONSTANT_KIND | ITEM_ASM_BYTE;
const CHAR: i32 = CONSTANT_KIND | ITEM_ASM_CHAR;
const SHORT: i32 = CONSTANT_KIND | ITEM_ASM_SHORT;
pub(crate) const INTEGER: i32 = CONSTANT_KIND | ITEM_INTEGER;
pub(crate) const FLOAT: i32 = CONSTANT_KIND | ITEM_FLOAT;
pub(crate) const LONG: i32 = CONSTANT_KIND | ITEM_LONG;
pub(crate) const DOUBLE: i32 = CONSTANT_KIND | ITEM_DOUBLE;
pub(crate) const NULL: i32 = CONSTANT_KIND | ITEM_NULL;
pub(crate) const UNINITIALIZED_THIS: i32 = CONSTANT_KIND | ITEM_UNINITIALIZED_THIS;

/// The `Integer` value of a frame type in ASM's API format (`Opcodes.TOP`, ... `Opcodes.NULL`,
/// `Opcodes.UNINITIALIZED_THIS`), or `None` for an object or uninitialized type.
pub(crate) fn api_integer_value(value: &FrameValue<'_>) -> Option<i32> {
    match value {
        FrameValue::Top => Some(ITEM_TOP),
        FrameValue::Integer => Some(ITEM_INTEGER),
        FrameValue::Float => Some(ITEM_FLOAT),
        FrameValue::Double => Some(ITEM_DOUBLE),
        FrameValue::Long => Some(ITEM_LONG),
        FrameValue::Null => Some(ITEM_NULL),
        FrameValue::UninitializedThis => Some(ITEM_UNINITIALIZED_THIS),
        FrameValue::Object(_) | FrameValue::Uninitialized(_) => None,
    }
}

/// Whether a frame type in ASM's API format is `Opcodes.LONG` or `Opcodes.DOUBLE`.
pub(crate) fn is_long_or_double(value: &FrameValue<'_>) -> bool {
    matches!(value, FrameValue::Long | FrameValue::Double)
}

// -------------------------------------------------------------------------------------------------
// Static methods to get abstract types from other type formats
// -------------------------------------------------------------------------------------------------

/// Returns the abstract type corresponding to the given public API frame element type.
///
/// `label_offset` returns the bytecode offset of a label, if it is resolved.
pub(crate) fn get_abstract_type_from_api_format(
    symbol_table: &mut SymbolTable,
    value: &FrameValue<'_>,
    label_offset: impl Fn(Label) -> Option<i32>,
) -> Result<i32, WriteError> {
    match value {
        FrameValue::Object(internal_name) => {
            // Type.getObjectType(type).getDescriptor().
            if internal_name.is_empty() {
                // Type.getObjectType's charAt(0).
                return Err(index_out_of_bounds(0, 0));
            }
            let object_type = Type::get_object_type(&**internal_name);
            get_abstract_type_from_descriptor(symbol_table, &object_type.get_descriptor(), 0)
        }
        FrameValue::Uninitialized(label) => Ok(match label_offset(*label) {
            Some(bytecode_offset) => {
                UNINITIALIZED_KIND
                    | symbol_table.add_uninitialized_type(JavaStr::from_str(""), bytecode_offset)
            }
            None => {
                FORWARD_UNINITIALIZED_KIND
                    | symbol_table.add_forward_uninitialized_type(JavaStr::from_str(""), *label)
            }
        }),
        _ => Ok(CONSTANT_KIND | api_integer_value(value).expect("a constant frame value")),
    }
}

/// Returns the abstract type corresponding to the internal name of a class.
pub(crate) fn get_abstract_type_from_internal_name(
    symbol_table: &mut SymbolTable,
    internal_name: &JavaStr,
) -> i32 {
    REFERENCE_KIND | symbol_table.add_type(internal_name)
}

/// Returns the abstract type corresponding to the given type descriptor, which starts at the
/// given byte offset of `buffer` (an ASCII character).
///
/// # Errors
///
/// ASM's `IllegalArgumentException` for an invalid descriptor, and its
/// `StringIndexOutOfBoundsException` for a descriptor that ends too early.
pub(crate) fn get_abstract_type_from_descriptor(
    symbol_table: &mut SymbolTable,
    buffer: &JavaStr,
    offset: usize,
) -> Result<i32, WriteError> {
    let bytes = buffer.as_bytes();
    let char_at = |i: usize| {
        bytes
            .get(i)
            .copied()
            .ok_or_else(|| index_out_of_bounds(utf16_index(buffer, i), buffer.len_utf16()))
    };
    match char_at(offset)? {
        b'V' => Ok(0),
        b'Z' | b'C' | b'B' | b'S' | b'I' => Ok(INTEGER),
        b'F' => Ok(FLOAT),
        b'J' => Ok(LONG),
        b'D' => Ok(DOUBLE),
        b'L' => {
            let internal_name = substring_to_last_char(buffer, offset + 1)?;
            Ok(REFERENCE_KIND | symbol_table.add_type(&internal_name))
        }
        b'[' => {
            let mut element_descriptor_offset = offset + 1;
            while char_at(element_descriptor_offset)? == b'[' {
                element_descriptor_offset += 1;
            }
            let type_value = match char_at(element_descriptor_offset)? {
                b'Z' => BOOLEAN,
                b'C' => CHAR,
                b'B' => BYTE,
                b'S' => SHORT,
                b'I' => INTEGER,
                b'F' => FLOAT,
                b'J' => LONG,
                b'D' => DOUBLE,
                b'L' => {
                    let internal_name =
                        substring_to_last_char(buffer, element_descriptor_offset + 1)?;
                    REFERENCE_KIND | symbol_table.add_type(&internal_name)
                }
                _ => {
                    return Err(WriteError::InvalidArgument(format!(
                        "Invalid descriptor fragment: {}",
                        tail(buffer, element_descriptor_offset)
                    )));
                }
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let dimensions = (element_descriptor_offset - offset) as i32;
            Ok((dimensions << DIM_SHIFT) | type_value)
        }
        _ => Err(WriteError::InvalidArgument(format!(
            "Invalid descriptor: {}",
            tail(buffer, offset)
        ))),
    }
}

/// Java's `buffer.substring(begin, buffer.length() - 1)`, for a `begin` just after an ASCII
/// character: the characters from `begin` to the end, less the last UTF-16 unit. Borrows
/// `buffer` unless its last character is a supplementary one.
fn substring_to_last_char(buffer: &JavaStr, begin: usize) -> Result<Cow<'_, JavaStr>, WriteError> {
    let bytes = buffer.as_bytes();
    if begin >= bytes.len() {
        // Java's message for substring(begin, end) with begin > end.
        let length_utf16 = buffer.len_utf16();
        let begin_utf16 = utf16_index(buffer, begin);
        return Err(WriteError::IndexOutOfBounds(format!(
            "begin {begin_utf16}, end {}, length {length_utf16}",
            length_utf16.wrapping_sub(1)
        )));
    }
    // The start of the last character (a descriptor's `;` is ASCII: the first test ends it).
    let mut last = bytes.len() - 1;
    while bytes[last] & 0xc0 == 0x80 {
        last -= 1;
    }
    let rest = buffer.get(begin..last).expect("code point boundaries");
    if bytes[last] < 0xf0 {
        // A BMP character is one UTF-16 unit.
        return Ok(Cow::Borrowed(rest));
    }
    // Removing the last UTF-16 unit of a supplementary character leaves its high surrogate.
    let c = buffer
        .get(last..bytes.len())
        .and_then(|c| c.chars().next())
        .expect("a supplementary character");
    let high = 0xd800 | ((c.as_u32() - 0x10000) >> 10);
    let mut result = JavaString::with_capacity(rest.len() + 3);
    result.push_java_str(rest);
    result.push_java(JavaCodePoint::from_u32(high).expect("a surrogate"));
    Ok(Cow::Owned(result))
}

/// The characters of `buffer` from the ASCII byte offset `begin` (for error messages).
fn tail(buffer: &JavaStr, begin: usize) -> String {
    buffer
        .get(begin..buffer.len())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The UTF-16 index of the given byte offset of `buffer` (which may be past its end, by bytes
/// that count as UTF-16 units).
fn utf16_index(buffer: &JavaStr, byte_offset: usize) -> usize {
    if byte_offset >= buffer.len() {
        buffer.len_utf16() + (byte_offset - buffer.len())
    } else {
        buffer
            .get(0..byte_offset)
            .map_or(byte_offset, JavaStr::len_utf16)
    }
}

/// Java's `StringIndexOutOfBoundsException` message for `charAt(index)`.
fn index_out_of_bounds(index: usize, length: usize) -> WriteError {
    WriteError::IndexOutOfBounds(format!("Index {index} out of bounds for length {length}"))
}

// -------------------------------------------------------------------------------------------------
// Methods related to the input frame
// -------------------------------------------------------------------------------------------------

/// Appends the input locals of the frame of the given method's entry point (ASM's
/// `setInputFrameFromDescriptor`), `max_locals` of them, to `input_locals`. Appends nothing on
/// error.
pub(crate) fn input_locals_from_descriptor(
    symbol_table: &mut SymbolTable,
    access: u32,
    descriptor: &JavaStr,
    max_locals: usize,
    input_locals: &mut Vec<i32>,
) -> Result<(), WriteError> {
    let start = input_locals.len();
    input_locals.reserve(max_locals);
    let result = append_input_locals(symbol_table, access, descriptor, max_locals, input_locals);
    match result {
        Ok(()) => input_locals.resize(start + max_locals, TOP),
        Err(_) => input_locals.truncate(start),
    }
    result
}

/// The body of [`input_locals_from_descriptor`], which may leave some locals appended on error,
/// and does not append the trailing TOPs.
fn append_input_locals(
    symbol_table: &mut SymbolTable,
    access: u32,
    descriptor: &JavaStr,
    max_locals: usize,
    input_locals: &mut Vec<i32>,
) -> Result<(), WriteError> {
    let end = input_locals.len() + max_locals;
    let mut set = |value: i32| {
        if input_locals.len() == end {
            return Err(WriteError::IndexOutOfBounds(format!(
                "Index {max_locals} out of bounds for length {max_locals}"
            )));
        }
        input_locals.push(value);
        Ok(())
    };
    if access & opcodes::ACC_STATIC == 0 {
        if access & ACC_CONSTRUCTOR == 0 {
            set(REFERENCE_KIND | symbol_table.add_type_of(symbol_table.class_name_span()))?;
        } else {
            set(UNINITIALIZED_THIS)?;
        }
    }
    Type::try_for_each_argument_type_of(descriptor, |argument_type| {
        let abstract_type =
            get_abstract_type_from_descriptor(symbol_table, &argument_type.get_descriptor(), 0)?;
        set(abstract_type)?;
        if abstract_type == LONG || abstract_type == DOUBLE {
            set(TOP)?;
        }
        Ok::<_, WriteError>(())
    })
}

/// The elements to visit of a frame with the given input locals or stack: the types, without
/// the TOP types that are just after a LONG or a DOUBLE, and without the trailing TOP locals if
/// `skip_trailing_top` (the first half of ASM's `Frame.accept`).
pub(crate) fn frame_elements(types: &[i32], skip_trailing_top: bool) -> Vec<i32> {
    let mut elements = Vec::with_capacity(types.len());
    push_frame_elements(types, skip_trailing_top, &mut elements);
    elements
}

/// Appends the [`frame_elements`] of the given types to `elements`, and returns their number.
fn push_frame_elements(types: &[i32], skip_trailing_top: bool, elements: &mut Vec<i32>) -> usize {
    let start = elements.len();
    let mut num_elements = 0;
    let mut num_trailing_top = 0;
    let mut i = 0;
    while i < types.len() {
        let abstract_type = types[i];
        i += if abstract_type == LONG || abstract_type == DOUBLE {
            2
        } else {
            1
        };
        elements.push(abstract_type);
        if skip_trailing_top && abstract_type == TOP {
            num_trailing_top += 1;
        } else {
            num_elements += num_trailing_top + 1;
            num_trailing_top = 0;
        }
    }
    elements.truncate(start + num_elements);
    num_elements
}

// -------------------------------------------------------------------------------------------------
// Frame output methods, to generate StackMapFrame attributes
// -------------------------------------------------------------------------------------------------

/// Puts the given abstract type in the given byte vector, using the JVMS
/// `verification_type_info` format used in StackMapTable attributes.
///
/// `put_label` puts the bytecode offset of a forward uninitialized type's label (ASM's
/// `Label.put(stackMapTableEntries)`).
///
/// # Errors
///
/// ASM's `AssertionError` for an abstract type that has no `verification_type_info` (which the
/// frame computation produces from invalid bytecode, e.g. an AALOAD on a non array value).
pub(crate) fn put_abstract_type(
    symbol_table: &mut SymbolTable,
    abstract_type: i32,
    output: &mut ByteVector,
    put_label: impl FnOnce(Label, &mut ByteVector),
) -> Result<(), WriteError> {
    let assertion_error = || WriteError::InvalidArgument("java.lang.AssertionError".to_owned());
    let type_value_of = |symbol_table: &SymbolTable, type_index: i32| {
        symbol_table
            .try_get_type(type_index)
            .map(|symbol| (symbol.value, symbol.data))
            .ok_or_else(assertion_error)
    };
    let mut array_dimensions = (abstract_type & DIM_MASK) >> DIM_SHIFT;
    if array_dimensions == 0 {
        let type_value = abstract_type & VALUE_MASK;
        match abstract_type & KIND_MASK {
            CONSTANT_KIND => {
                output.put_byte(type_value);
            }
            REFERENCE_KIND => {
                let (value, _) = type_value_of(symbol_table, type_value)?;
                let index = symbol_table.add_constant_class_of(value);
                output.put_byte(ITEM_OBJECT).put_short(index);
            }
            UNINITIALIZED_KIND => {
                let (_, data) = type_value_of(symbol_table, type_value)?;
                #[allow(clippy::cast_possible_truncation)]
                let bytecode_offset = data as i32;
                output
                    .put_byte(ITEM_UNINITIALIZED)
                    .put_short(bytecode_offset);
            }
            FORWARD_UNINITIALIZED_KIND => {
                type_value_of(symbol_table, type_value)?;
                output.put_byte(ITEM_UNINITIALIZED);
                put_label(
                    symbol_table.get_forward_uninitialized_label(type_value),
                    output,
                );
            }
            _ => return Err(assertion_error()),
        }
    } else {
        // Case of an array type, we need to build its descriptor first.
        let mut type_descriptor = JavaString::new();
        while array_dimensions > 0 {
            type_descriptor.push('[');
            array_dimensions -= 1;
        }
        if abstract_type & KIND_MASK == REFERENCE_KIND {
            let (value, _) = type_value_of(symbol_table, abstract_type & VALUE_MASK)?;
            type_descriptor.push('L');
            type_descriptor.push_java_str(symbol_table.text(value));
            type_descriptor.push(';');
        } else {
            type_descriptor.push(match abstract_type & VALUE_MASK {
                ITEM_ASM_BOOLEAN => 'Z',
                ITEM_ASM_BYTE => 'B',
                ITEM_ASM_CHAR => 'C',
                ITEM_ASM_SHORT => 'S',
                ITEM_INTEGER => 'I',
                ITEM_FLOAT => 'F',
                ITEM_LONG => 'J',
                ITEM_DOUBLE => 'D',
                _ => return Err(assertion_error()),
            });
        }
        let index = symbol_table.add_constant_class(&type_descriptor).index;
        output.put_byte(ITEM_OBJECT).put_short(index);
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------------
// The frame computation: ASM's Frame instances
// -------------------------------------------------------------------------------------------------

/// The ASM specific `LOCAL_KIND` and `STACK_KIND` kinds of abstract types, relative to the input
/// frame of a basic block.
const LOCAL_KIND: i32 = 5 << KIND_SHIFT;
const STACK_KIND: i32 = 6 << KIND_SHIFT;
/// A flag that can be set on a `LOCAL_KIND` or `STACK_KIND` abstract type, to make it TOP if the
/// type it designates is LONG or DOUBLE.
const TOP_IF_LONG_OR_DOUBLE_FLAG: i32 = 1 << FLAGS_SHIFT;
/// Constants to manipulate the DIM field of an abstract type.
const ARRAY_OF: i32 = 1 << DIM_SHIFT;
const ELEMENT_OF: i32 = -1 << DIM_SHIFT;

/// The input and output stack map frames of a basic block (ASM's `Frame`). See ASM's `Frame`
/// documentation for the abstract types and the two steps of the frame computation.
///
/// ASM keeps the maximum output stack size of a basic block in its label (`outputStackMax`),
/// which a frame updates through its `owner`; here the frame holds it. (This only differs for a
/// label visited twice in a method, which shares the frame of another label but keeps the stack
/// size of its first visit in ASM.)
///
/// ASM's `int[]` arrays of a frame are [`Span`]s of the method's [`FrameValues`], so a frame is
/// plain data, and its operations take a [`FrameMut`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Frame {
    /// The basic block to which these input and output stack map frames correspond.
    pub(crate) owner: Option<BlockId>,
    /// The maximum height reached by the output stack, relative to the top of the input stack,
    /// in the basic block of this frame (ASM's `owner.outputStackMax`).
    pub(crate) output_stack_max: i16,
    /// The input stack map frame locals, as abstract types, or `None` if not set yet.
    input_locals: Option<Span>,
    /// The input stack map frame stack, as abstract types, or `None` if not set yet.
    input_stack: Option<Span>,
    /// The output stack map frame locals, as abstract types (0 for a local that was not
    /// assigned).
    output_locals: Option<Span>,
    /// The output stack map frame stack, as abstract types.
    output_stack: Option<Span>,
    /// The start of the output stack, relative to the input stack: the number of input stack
    /// elements that have been popped (as a negative number).
    output_stack_start: i16,
    /// The index of the top stack element in `output_stack`.
    output_stack_top: i16,
    /// The abstract types that are initialized in the basic block (ASM's `initializations`, of
    /// which `initializationCount` elements are used): `len` is the count.
    initializations: Span,
    /// The number of values reserved for `initializations` (ASM's array length).
    initializations_capacity: usize,
}

/// An array of abstract types of a [`Frame`]: a range of its [`FrameValues`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    start: usize,
    len: usize,
}

impl Span {
    fn end(self) -> usize {
        self.start + self.len
    }
}

/// The arrays of the frames of a method, in one arena: ASM allocates an `int[]` for each. An
/// array that grows moves to the end of the arena, unless it is there already; the space it
/// leaves is not reused (the arena lives as long as the method's frames), except by
/// [`FrameMut::execute_current`], which reclaims its arrays after each instruction.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameValues {
    values: Vec<i32>,
}

impl FrameValues {
    /// Reserves room for `additional` more values.
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.values.reserve(additional);
    }

    /// Removes every array, keeping the memory.
    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }

    /// A new array of `len` zeros.
    fn alloc(&mut self, len: usize) -> Span {
        let start = self.values.len();
        self.values.resize(start + len, 0);
        Span { start, len }
    }

    /// The array of the given span.
    fn get(&self, span: Span) -> &[i32] {
        &self.values[span.start..span.end()]
    }

    /// The array of the given span, or an empty one.
    fn get_or_empty(&self, span: Option<Span>) -> &[i32] {
        span.map_or(&[], |span| self.get(span))
    }

    /// The array of the given span.
    fn get_mut(&mut self, span: Span) -> &mut [i32] {
        &mut self.values[span.start..span.end()]
    }

    /// The array of the given span grown to `len` values, the new ones zero.
    fn grow(&mut self, span: Span, len: usize) -> Span {
        if span.end() == self.values.len() {
            self.values.resize(span.start + len, 0);
            return Span {
                start: span.start,
                len,
            };
        }
        let grown = self.alloc(len);
        self.values.copy_within(span.start..span.end(), grown.start);
        grown
    }
}

/// The `i32` at the given index, or Java's `ArrayIndexOutOfBoundsException`.
fn element(array: &[i32], index: i64) -> Result<i32, WriteError> {
    usize::try_from(index)
        .ok()
        .and_then(|i| array.get(i).copied())
        .ok_or_else(|| array_index_out_of_bounds(index, array.len()))
}

/// Java's `ArrayIndexOutOfBoundsException` message.
fn array_index_out_of_bounds(index: i64, length: usize) -> WriteError {
    WriteError::IndexOutOfBounds(format!("Index {index} out of bounds for length {length}"))
}

/// The symbol an instruction refers to, for [`FrameMut::execute`]: the parts of ASM's `Symbol`
/// that the frame computation reads. Its strings are the instruction's own (equal to the
/// symbol's), so that it does not borrow the symbol table the frame computation adds types to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SymbolArg<'a> {
    pub(crate) tag: u8,
    pub(crate) name: &'a JavaStr,
    pub(crate) value: &'a JavaStr,
}

impl SymbolArg<'_> {
    fn value(&self) -> &JavaStr {
        self.value
    }
}

impl Frame {
    /// Constructs a new frame for the given basic block.
    pub(crate) fn new(owner: Option<BlockId>) -> Frame {
        Frame {
            owner,
            ..Frame::default()
        }
    }

    /// Returns the input stack size of this frame.
    pub(crate) fn get_input_stack_size(&self) -> usize {
        self.input_stack.map_or(0, |span| span.len)
    }

    /// Appends the elements to visit for this frame to `elements` (the first half of ASM's
    /// `accept`): its input locals (less the trailing TOPs) and its input stack, without the TOPs
    /// after LONGs and DOUBLEs. Returns the number of locals and of stack elements.
    pub(crate) fn accept(&self, values: &FrameValues, elements: &mut Vec<i32>) -> (usize, usize) {
        let input_locals = values.get_or_empty(self.input_locals);
        let input_stack = values.get_or_empty(self.input_stack);
        elements.reserve(input_locals.len() + input_stack.len());
        (
            push_frame_elements(input_locals, true, elements),
            push_frame_elements(input_stack, false, elements),
        )
    }
}

/// A frame and the arena of its arrays: what the frame operations that read or write the arrays
/// take (ASM's `Frame` methods).
pub(crate) struct FrameMut<'a> {
    pub(crate) frame: &'a mut Frame,
    pub(crate) values: &'a mut FrameValues,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
impl FrameMut<'_> {
    /// Sets this frame to the value of the given frame (ASM's `copyFrom`).
    fn copy_from(&mut self, frame: Frame) {
        let this = &mut *self.frame;
        this.input_locals = frame.input_locals;
        this.input_stack = frame.input_stack;
        this.output_stack_start = 0;
        this.output_locals = frame.output_locals;
        this.output_stack = frame.output_stack;
        this.output_stack_top = frame.output_stack_top;
        this.initializations = frame.initializations;
        this.initializations_capacity = frame.initializations_capacity;
    }

    /// Sets the input frame from the given method description (ASM's
    /// `setInputFrameFromDescriptor`).
    pub(crate) fn set_input_frame_from_descriptor(
        &mut self,
        symbol_table: &mut SymbolTable,
        access: u32,
        descriptor: &JavaStr,
        max_locals: usize,
    ) -> Result<(), WriteError> {
        let start = self.values.values.len();
        input_locals_from_descriptor(
            symbol_table,
            access,
            descriptor,
            max_locals,
            &mut self.values.values,
        )?;
        self.frame.input_locals = Some(Span {
            start,
            len: max_locals,
        });
        self.frame.input_stack = Some(self.values.alloc(0));
        Ok(())
    }

    /// Sets the input frame from the given public API frame description (ASM's
    /// `setInputFrameFromApiFormat`).
    pub(crate) fn set_input_frame_from_api_format(
        &mut self,
        symbol_table: &mut SymbolTable,
        local: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
        label_offset: impl Fn(Label) -> Option<i32>,
    ) -> Result<(), WriteError> {
        let input_locals = match self.frame.input_locals {
            Some(span) => span,
            None => *self.frame.input_locals.insert(self.values.alloc(0)),
        };
        let input_locals = self.values.get_mut(input_locals);
        let length = input_locals.len();
        let mut input_local_index = 0;
        let mut set = |index: &mut usize, value: i32| {
            let Some(slot) = input_locals.get_mut(*index) else {
                return Err(array_index_out_of_bounds(*index as i64, length));
            };
            *slot = value;
            *index += 1;
            Ok(())
        };
        for value in local {
            let abstract_type =
                get_abstract_type_from_api_format(symbol_table, value, &label_offset)?;
            set(&mut input_local_index, abstract_type)?;
            if is_long_or_double(value) {
                set(&mut input_local_index, TOP)?;
            }
        }
        while input_local_index < length {
            set(&mut input_local_index, TOP)?;
        }
        let values = &mut self.values.values;
        let start = values.len();
        values.reserve(stack.len() * 2);
        for value in stack {
            values.push(get_abstract_type_from_api_format(
                symbol_table,
                value,
                &label_offset,
            )?);
            if is_long_or_double(value) {
                values.push(TOP);
            }
        }
        self.frame.input_stack = Some(Span {
            start,
            len: values.len() - start,
        });
        self.frame.output_stack_top = 0;
        self.frame.initializations.len = 0;
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Methods related to the output frame
    // ---------------------------------------------------------------------------------------------

    /// Returns the abstract type stored at the given local variable index in the output frame.
    fn get_local(&mut self, local_index: i32) -> i32 {
        match self.frame.output_locals {
            Some(output_locals) if (local_index as usize) < output_locals.len => {
                let abstract_type = &mut self.values.get_mut(output_locals)[local_index as usize];
                if *abstract_type == 0 {
                    // If this local has never been assigned in this basic block, so it is still
                    // equal to its value in the input frame.
                    *abstract_type = LOCAL_KIND | local_index;
                }
                *abstract_type
            }
            // If this local has never been assigned in this basic block, it is still equal to
            // its value in the input frame.
            _ => LOCAL_KIND | local_index,
        }
    }

    /// Replaces the abstract type stored at the given local variable index in the output frame.
    fn set_local(&mut self, local_index: i32, abstract_type: i32) {
        // Create and/or resize the output local variables array if necessary.
        let mut output_locals = match self.frame.output_locals {
            Some(span) => span,
            None => self.values.alloc(10),
        };
        let index = local_index as usize;
        if index >= output_locals.len {
            let new_length = (index + 1).max(2 * output_locals.len);
            output_locals = self.values.grow(output_locals, new_length);
        }
        self.frame.output_locals = Some(output_locals);
        // Set the local variable.
        self.values.get_mut(output_locals)[index] = abstract_type;
    }

    /// Pushes the given abstract type on the output frame stack.
    fn push(&mut self, abstract_type: i32) -> Result<(), WriteError> {
        // Create and/or resize the output stack array if necessary.
        let mut output_stack = match self.frame.output_stack {
            Some(span) => span,
            None => *self.frame.output_stack.insert(self.values.alloc(10)),
        };
        let frame = &mut *self.frame;
        let Ok(top) = usize::try_from(frame.output_stack_top) else {
            // ASM's short outputStackTop overflowed.
            return Err(array_index_out_of_bounds(
                i64::from(frame.output_stack_top),
                output_stack.len,
            ));
        };
        if top >= output_stack.len {
            let new_length = (top + 1).max(2 * output_stack.len);
            output_stack = self.values.grow(output_stack, new_length);
            frame.output_stack = Some(output_stack);
        }
        // Pushes the abstract type on the output stack.
        self.values.get_mut(output_stack)[top] = abstract_type;
        frame.output_stack_top = frame.output_stack_top.wrapping_add(1);
        // Updates the maximum size reached by the output stack, if needed (note that this size
        // is relative to the input stack size, which is not known yet).
        let output_stack_size = frame
            .output_stack_start
            .wrapping_add(frame.output_stack_top);
        if output_stack_size > frame.output_stack_max {
            frame.output_stack_max = output_stack_size;
        }
        Ok(())
    }

    /// Pushes the abstract type corresponding to the given descriptor on the output frame stack
    /// (the return type for a method descriptor).
    fn push_descriptor(
        &mut self,
        symbol_table: &mut SymbolTable,
        descriptor: &JavaStr,
    ) -> Result<(), WriteError> {
        let type_descriptor_offset = if descriptor.as_bytes().first() == Some(&b'(') {
            Type::get_return_type_offset(descriptor)?
        } else {
            0
        };
        let abstract_type =
            get_abstract_type_from_descriptor(symbol_table, descriptor, type_descriptor_offset)?;
        if abstract_type != 0 {
            self.push(abstract_type)?;
            if abstract_type == LONG || abstract_type == DOUBLE {
                self.push(TOP)?;
            }
        }
        Ok(())
    }

    /// Pops an abstract type from the output frame stack and returns its value.
    fn pop(&mut self) -> i32 {
        let frame = &mut *self.frame;
        if frame.output_stack_top > 0 {
            frame.output_stack_top -= 1;
            self.values.get(frame.output_stack.expect("non empty"))[frame.output_stack_top as usize]
        } else {
            // If the output frame stack is empty, pop from the input stack.
            frame.output_stack_start = frame.output_stack_start.wrapping_sub(1);
            STACK_KIND | -i32::from(frame.output_stack_start)
        }
    }

    /// Pops the given number of abstract types from the output frame stack.
    fn pop_n(&mut self, elements: i32) {
        let frame = &mut *self.frame;
        if i32::from(frame.output_stack_top) >= elements {
            frame.output_stack_top = (i32::from(frame.output_stack_top) - elements) as i16;
        } else {
            // If the number of elements to be popped is greater than the number of elements in
            // the output stack, clear it, and pop the remaining elements from the input stack.
            frame.output_stack_start = (i32::from(frame.output_stack_start)
                - (elements - i32::from(frame.output_stack_top)))
                as i16;
            frame.output_stack_top = 0;
        }
    }

    /// Pops as many abstract types from the output frame stack as described by the given
    /// descriptor (the arguments for a method descriptor).
    fn pop_descriptor(&mut self, descriptor: &JavaStr) -> Result<(), WriteError> {
        match descriptor.as_bytes().first() {
            Some(b'(') => {
                let sizes = Type::try_get_arguments_and_return_sizes_of(descriptor)? as i32;
                self.pop_n((sizes >> 2) - 1);
            }
            Some(b'J' | b'D') => self.pop_n(2),
            Some(_) => self.pop_n(1),
            None => return Err(index_out_of_bounds(0, 0)),
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Methods to handle uninitialized types
    // ---------------------------------------------------------------------------------------------

    /// Adds an abstract type to the list of types on which a constructor is invoked in the basic
    /// block.
    fn add_initialized_type(&mut self, abstract_type: i32) {
        let frame = &mut *self.frame;
        if frame.initializations.len == frame.initializations_capacity {
            let reserved = Span {
                start: frame.initializations.start,
                len: frame.initializations_capacity,
            };
            let capacity = (2 * reserved.len).max(4);
            frame.initializations.start = self.values.grow(reserved, capacity).start;
            frame.initializations_capacity = capacity;
        }
        self.values.values[frame.initializations.end()] = abstract_type;
        frame.initializations.len += 1;
    }

    // ---------------------------------------------------------------------------------------------
    // Main method, to simulate the execution of each instruction on the output frame
    // ---------------------------------------------------------------------------------------------

    /// Simulates the action of the given instruction on the output stack frame (ASM's
    /// `execute`).
    ///
    /// - `arg`: the numeric operand of the instruction, if any.
    /// - `arg_symbol`: the symbol operand of the instruction, if any.
    #[allow(clippy::too_many_lines)] // ASM's switch
    pub(crate) fn execute(
        &mut self,
        opcode: u8,
        arg: i32,
        arg_symbol: Option<&SymbolArg<'_>>,
        symbol_table: &mut SymbolTable,
    ) -> Result<(), WriteError> {
        use crate::opcodes::*;
        use crate::writer::symbol_table::{
            CONSTANT_CLASS_TAG, CONSTANT_DOUBLE_TAG, CONSTANT_DYNAMIC_TAG, CONSTANT_FLOAT_TAG,
            CONSTANT_INTEGER_TAG, CONSTANT_LONG_TAG, CONSTANT_METHOD_HANDLE_TAG,
            CONSTANT_METHOD_TYPE_TAG, CONSTANT_STRING_TAG,
        };
        let symbol = || arg_symbol.ok_or_else(|| WriteError::InvalidArgument(String::new()));
        match opcode {
            NOP | INEG | LNEG | FNEG | DNEG | I2B | I2C | I2S | GOTO | RETURN => {}
            ACONST_NULL => self.push(NULL)?,
            ICONST_M1 | ICONST_0 | ICONST_1 | ICONST_2 | ICONST_3 | ICONST_4 | ICONST_5
            | BIPUSH | SIPUSH | ILOAD => self.push(INTEGER)?,
            LCONST_0 | LCONST_1 | LLOAD => {
                self.push(LONG)?;
                self.push(TOP)?;
            }
            FCONST_0 | FCONST_1 | FCONST_2 | FLOAD => self.push(FLOAT)?,
            DCONST_0 | DCONST_1 | DLOAD => {
                self.push(DOUBLE)?;
                self.push(TOP)?;
            }
            LDC => {
                let symbol = symbol()?;
                match symbol.tag {
                    CONSTANT_INTEGER_TAG => self.push(INTEGER)?,
                    CONSTANT_LONG_TAG => {
                        self.push(LONG)?;
                        self.push(TOP)?;
                    }
                    CONSTANT_FLOAT_TAG => self.push(FLOAT)?,
                    CONSTANT_DOUBLE_TAG => {
                        self.push(DOUBLE)?;
                        self.push(TOP)?;
                    }
                    CONSTANT_CLASS_TAG => {
                        self.push(
                            REFERENCE_KIND | symbol_table.add_type(crate::jstr!("java/lang/Class")),
                        )?;
                    }
                    CONSTANT_STRING_TAG => {
                        self.push(
                            REFERENCE_KIND
                                | symbol_table.add_type(crate::jstr!("java/lang/String")),
                        )?;
                    }
                    CONSTANT_METHOD_TYPE_TAG => self.push(
                        REFERENCE_KIND
                            | symbol_table.add_type(crate::jstr!("java/lang/invoke/MethodType")),
                    )?,
                    CONSTANT_METHOD_HANDLE_TAG => self.push(
                        REFERENCE_KIND
                            | symbol_table.add_type(crate::jstr!("java/lang/invoke/MethodHandle")),
                    )?,
                    CONSTANT_DYNAMIC_TAG => self.push_descriptor(symbol_table, symbol.value())?,
                    _ => {
                        return Err(WriteError::InvalidArgument(
                            "java.lang.AssertionError".to_owned(),
                        ));
                    }
                }
            }
            ALOAD => {
                let local = self.get_local(arg);
                self.push(local)?;
            }
            LALOAD | D2L => {
                self.pop_n(2);
                self.push(LONG)?;
                self.push(TOP)?;
            }
            DALOAD | L2D => {
                self.pop_n(2);
                self.push(DOUBLE)?;
                self.push(TOP)?;
            }
            AALOAD => {
                self.pop_n(1);
                let abstract_type1 = self.pop();
                self.push(if abstract_type1 == NULL {
                    abstract_type1
                } else {
                    ELEMENT_OF.wrapping_add(abstract_type1)
                })?;
            }
            ISTORE | FSTORE | ASTORE => {
                let abstract_type1 = self.pop();
                self.set_local(arg, abstract_type1);
                self.mark_previous_local(arg);
            }
            LSTORE | DSTORE => {
                self.pop_n(1);
                let abstract_type1 = self.pop();
                self.set_local(arg, abstract_type1);
                self.set_local(arg + 1, TOP);
                self.mark_previous_local(arg);
            }
            IASTORE | BASTORE | CASTORE | SASTORE | FASTORE | AASTORE => self.pop_n(3),
            LASTORE | DASTORE => self.pop_n(4),
            POP | IFEQ | IFNE | IFLT | IFGE | IFGT | IFLE | IRETURN | FRETURN | ARETURN
            | TABLESWITCH | LOOKUPSWITCH | ATHROW | MONITORENTER | MONITOREXIT | IFNULL
            | IFNONNULL => self.pop_n(1),
            POP2 | IF_ICMPEQ | IF_ICMPNE | IF_ICMPLT | IF_ICMPGE | IF_ICMPGT | IF_ICMPLE
            | IF_ACMPEQ | IF_ACMPNE | LRETURN | DRETURN => self.pop_n(2),
            DUP => {
                let abstract_type1 = self.pop();
                self.push(abstract_type1)?;
                self.push(abstract_type1)?;
            }
            DUP_X1 => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                self.push(abstract_type1)?;
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
            }
            DUP_X2 => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                let abstract_type3 = self.pop();
                self.push(abstract_type1)?;
                self.push(abstract_type3)?;
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
            }
            DUP2 => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
            }
            DUP2_X1 => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                let abstract_type3 = self.pop();
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
                self.push(abstract_type3)?;
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
            }
            DUP2_X2 => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                let abstract_type3 = self.pop();
                let abstract_type4 = self.pop();
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
                self.push(abstract_type4)?;
                self.push(abstract_type3)?;
                self.push(abstract_type2)?;
                self.push(abstract_type1)?;
            }
            SWAP => {
                let abstract_type1 = self.pop();
                let abstract_type2 = self.pop();
                self.push(abstract_type1)?;
                self.push(abstract_type2)?;
            }
            IALOAD | BALOAD | CALOAD | SALOAD | IADD | ISUB | IMUL | IDIV | IREM | IAND | IOR
            | IXOR | ISHL | ISHR | IUSHR | L2I | D2I | FCMPL | FCMPG => {
                self.pop_n(2);
                self.push(INTEGER)?;
            }
            LADD | LSUB | LMUL | LDIV | LREM | LAND | LOR | LXOR => {
                self.pop_n(4);
                self.push(LONG)?;
                self.push(TOP)?;
            }
            FALOAD | FADD | FSUB | FMUL | FDIV | FREM | L2F | D2F => {
                self.pop_n(2);
                self.push(FLOAT)?;
            }
            DADD | DSUB | DMUL | DDIV | DREM => {
                self.pop_n(4);
                self.push(DOUBLE)?;
                self.push(TOP)?;
            }
            LSHL | LSHR | LUSHR => {
                self.pop_n(3);
                self.push(LONG)?;
                self.push(TOP)?;
            }
            IINC => self.set_local(arg, INTEGER),
            I2L | F2L => {
                self.pop_n(1);
                self.push(LONG)?;
                self.push(TOP)?;
            }
            I2F => {
                self.pop_n(1);
                self.push(FLOAT)?;
            }
            I2D | F2D => {
                self.pop_n(1);
                self.push(DOUBLE)?;
                self.push(TOP)?;
            }
            F2I | ARRAYLENGTH | INSTANCEOF => {
                self.pop_n(1);
                self.push(INTEGER)?;
            }
            LCMP | DCMPL | DCMPG => {
                self.pop_n(4);
                self.push(INTEGER)?;
            }
            JSR | RET => {
                return Err(WriteError::InvalidArgument(
                    "JSR/RET are not supported with computeFrames option".to_owned(),
                ));
            }
            GETSTATIC => self.push_descriptor(symbol_table, symbol()?.value())?,
            PUTSTATIC => self.pop_descriptor(symbol()?.value())?,
            GETFIELD => {
                self.pop_n(1);
                self.push_descriptor(symbol_table, symbol()?.value())?;
            }
            PUTFIELD => {
                self.pop_descriptor(symbol()?.value())?;
                self.pop();
            }
            INVOKEVIRTUAL | INVOKESPECIAL | INVOKESTATIC | INVOKEINTERFACE => {
                let symbol = symbol()?;
                self.pop_descriptor(symbol.value())?;
                if opcode != INVOKESTATIC {
                    let abstract_type1 = self.pop();
                    let name = symbol.name;
                    if opcode == INVOKESPECIAL {
                        match name.as_bytes().first() {
                            Some(b'<') => self.add_initialized_type(abstract_type1),
                            Some(_) => {}
                            // The name's charAt(0).
                            None => return Err(index_out_of_bounds(0, 0)),
                        }
                    }
                }
                self.push_descriptor(symbol_table, symbol.value())?;
            }
            INVOKEDYNAMIC => {
                let symbol = symbol()?;
                self.pop_descriptor(symbol.value())?;
                self.push_descriptor(symbol_table, symbol.value())?;
            }
            NEW => {
                let value = symbol()?.value();
                self.push(UNINITIALIZED_KIND | symbol_table.add_uninitialized_type(value, arg))?;
            }
            NEWARRAY => {
                self.pop();
                let element_type = match arg {
                    T_BOOLEAN => BOOLEAN,
                    T_CHAR => CHAR,
                    T_BYTE => BYTE,
                    T_SHORT => SHORT,
                    T_INT => INTEGER,
                    T_FLOAT => FLOAT,
                    T_DOUBLE => DOUBLE,
                    T_LONG => LONG,
                    _ => return Err(WriteError::InvalidArgument(String::new())),
                };
                self.push(ARRAY_OF | element_type)?;
            }
            ANEWARRAY => {
                let array_element_type = symbol()?.value();
                self.pop();
                match array_element_type.as_bytes().first() {
                    Some(b'[') => {
                        let mut descriptor = JavaString::new();
                        descriptor.push('[');
                        descriptor.push_java_str(array_element_type);
                        self.push_descriptor(symbol_table, &descriptor)?;
                    }
                    Some(_) => self.push(
                        ARRAY_OF | REFERENCE_KIND | symbol_table.add_type(array_element_type),
                    )?,
                    None => return Err(index_out_of_bounds(0, 0)),
                }
            }
            CHECKCAST => {
                let cast_type = symbol()?.value();
                self.pop();
                match cast_type.as_bytes().first() {
                    Some(b'[') => self.push_descriptor(symbol_table, cast_type)?,
                    Some(_) => self.push(REFERENCE_KIND | symbol_table.add_type(cast_type))?,
                    None => return Err(index_out_of_bounds(0, 0)),
                }
            }
            MULTIANEWARRAY => {
                self.pop_n(arg);
                self.push_descriptor(symbol_table, symbol()?.value())?;
            }
            _ => return Err(WriteError::InvalidArgument(String::new())),
        }
        Ok(())
    }

    /// After a store in the given local, makes the previous local TOP if it held a long or a
    /// double (or marks it to become TOP, if its type is not known yet).
    fn mark_previous_local(&mut self, arg: i32) {
        if arg > 0 {
            let previous_local_type = self.get_local(arg - 1);
            if previous_local_type == LONG || previous_local_type == DOUBLE {
                self.set_local(arg - 1, TOP);
            } else if previous_local_type & KIND_MASK == LOCAL_KIND
                || previous_local_type & KIND_MASK == STACK_KIND
            {
                // The type of the previous local variable is not known yet, but if it later
                // appears to be LONG or DOUBLE, we should then use TOP instead.
                self.set_local(arg - 1, previous_local_type | TOP_IF_LONG_OR_DOUBLE_FLAG);
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Frame merging methods, used in the second step of the stack map frame computation algorithm
    // ---------------------------------------------------------------------------------------------

    /// Merges the input frame of this frame with the input and output frames of `source` (ASM's
    /// `source.merge(symbolTable, this, catchTypeIndex)`). Returns whether the input frame of
    /// this frame has been changed by this operation.
    ///
    /// - `hierarchy`: the class hierarchy that gives the common super class of two classes
    ///   (ASM's `ClassWriter.getCommonSuperClass`), if any.
    /// - `catch_type_index`: if this frame corresponds to an exception handler basic block, the
    ///   type table index of the caught exception type, otherwise 0.
    pub(crate) fn merge_from(
        &mut self,
        source: &Frame,
        symbol_table: &mut SymbolTable,
        hierarchy: Option<&dyn ClassHierarchy>,
        catch_type_index: i32,
    ) -> Result<bool, WriteError> {
        merge_frames(
            symbol_table,
            hierarchy,
            self.values,
            source,
            SourceInputs::Own,
            self.frame,
            catch_type_index,
        )
    }

    /// Merges the input frame of this frame with its own input and output frames: ASM's
    /// `frame.merge(symbolTable, frame, catchTypeIndex)`, for a basic block that is its own
    /// successor. ASM then reads the input frame as it updates it, and so does this method.
    pub(crate) fn merge_into_self(
        &mut self,
        symbol_table: &mut SymbolTable,
        hierarchy: Option<&dyn ClassHierarchy>,
        catch_type_index: i32,
    ) -> Result<bool, WriteError> {
        let source = *self.frame;
        merge_frames(
            symbol_table,
            hierarchy,
            self.values,
            &source,
            SourceInputs::Destination,
            self.frame,
            catch_type_index,
        )
    }

    /// Simulates the action of the given instruction, and makes the resulting output frame the
    /// input frame (ASM's `CurrentFrame.execute`: a frame of a method writer that computes the
    /// frames inserted by the expansion of ASM specific instructions).
    pub(crate) fn execute_current(
        &mut self,
        opcode: u8,
        arg: i32,
        arg_symbol: Option<&SymbolArg<'_>>,
        symbol_table: &mut SymbolTable,
    ) -> Result<(), WriteError> {
        let mark = self.values.values.len();
        self.execute(opcode, arg, arg_symbol, symbol_table)?;
        if self.frame.initializations.len == 0
            && let (Some(input_locals), Some(input_stack)) =
                (self.frame.input_locals, self.frame.input_stack)
            && input_stack.len as i64 + i64::from(self.frame.output_stack_start) >= 0
            && self.frame.output_stack_top >= 0
        {
            return self.update_inputs_in_place(input_locals, input_stack, mark);
        }
        self.merge_into_new_input_frame(symbol_table, mark)
    }

    /// The end of [`Self::execute_current`] in general: merges the frame into a new frame,
    /// which becomes its input frame (ASM's `merge(symbolTable, successor, 0)` and `copyFrom`).
    fn merge_into_new_input_frame(
        &mut self,
        symbol_table: &mut SymbolTable,
        mark: usize,
    ) -> Result<(), WriteError> {
        let source = *self.frame;
        let mut successor = Frame::new(None);
        // The successor's input frame is new: no reference types are merged.
        merge_frames(
            symbol_table,
            None,
            self.values,
            &source,
            SourceInputs::Own,
            &mut successor,
            0,
        )?;
        self.copy_from(successor);
        self.reclaim(&source, mark);
        Ok(())
    }

    /// [`Self::execute_current`]'s merge into a new frame and copy, for an instruction that
    /// initialized nothing and popped no more than the input stack has (otherwise the merge
    /// resolves every local, or fails), with a stack top that did not overflow.
    ///
    /// The new input locals are then the old ones but for those the instruction assigned, so only
    /// these are resolved, in place: an instruction costs what it touches, not `max_locals`.
    /// A merge into a new frame also turns an array of NULL into NULL (`merge_type`), which the
    /// input types never are: they come from the descriptor, an API frame, or a previous
    /// instruction's merge. The changed locals and the new stack are resolved against the old
    /// input frame, in the merge's order (so the first error is the same), before any is written.
    fn update_inputs_in_place(
        &mut self,
        input_locals: Span,
        input_stack: Span,
        mark: usize,
    ) -> Result<(), WriteError> {
        let frame = &mut *self.frame;
        let values = &mut self.values.values;
        let num_stack = input_stack.len;
        // The changed locals, as (index, type) pairs, then the new stack, after the instruction's
        // output arrays.
        let changed_locals = values.len();
        if let Some(output_locals) = frame.output_locals {
            for i in 0..output_locals.len.min(input_locals.len) {
                let output_local = values[output_locals.start + i];
                if output_local != 0 {
                    let concrete_output_type = get_concrete_output_type(
                        output_local,
                        num_stack,
                        &values[input_locals.start..input_locals.end()],
                        &values[input_stack.start..input_stack.end()],
                    )?;
                    values.extend([i as i32, null_merged(concrete_output_type)]);
                }
            }
        }
        let new_stack = values.len();
        let num_input_stack = (num_stack as i64 + i64::from(frame.output_stack_start)) as usize;
        values.extend_from_within(input_stack.start..input_stack.start + num_input_stack);
        let output_stack = frame.output_stack.unwrap_or_default();
        for i in 0..i64::from(frame.output_stack_top) {
            let abstract_output_type = element(&values[output_stack.start..output_stack.end()], i)?;
            let concrete_output_type = get_concrete_output_type(
                abstract_output_type,
                num_stack,
                &values[input_locals.start..input_locals.end()],
                &values[input_stack.start..input_stack.end()],
            )?;
            values.push(null_merged(concrete_output_type));
        }
        for pair in (changed_locals..new_stack).step_by(2) {
            let (index, abstract_type) = (values[pair] as usize, values[pair + 1]);
            values[input_locals.start + index] = abstract_type;
        }
        // The old input stack is not used anymore: the new one replaces it if it is the last
        // array before the instruction's (it always is, in practice).
        let stack_len = values.len() - new_stack;
        let stack_start = if input_stack.end() == mark {
            input_stack.start
        } else {
            mark
        };
        values.copy_within(new_stack.., stack_start);
        values.truncate(stack_start + stack_len);
        frame.input_stack = Some(Span {
            start: stack_start,
            len: stack_len,
        });
        frame.output_locals = None;
        frame.output_stack = None;
        frame.output_stack_start = 0;
        frame.output_stack_top = 0;
        frame.initializations = Span::default();
        frame.initializations_capacity = 0;
        Ok(())
    }

    /// Frees the arrays that [`Self::execute_current`] no longer uses, so that the arena does
    /// not grow with each instruction: everything from `mark` on, where the frame's new input
    /// arrays are moved, and the previous input arrays of `old` if they end at `mark`.
    fn reclaim(&mut self, old: &Frame, mark: usize) {
        let mut start = mark;
        for span in [old.input_stack, old.input_locals].into_iter().flatten() {
            if span.end() == start {
                start = span.start;
            }
        }
        let frame = &mut *self.frame;
        for span in [&mut frame.input_locals, &mut frame.input_stack]
            .into_iter()
            .flatten()
        {
            // The new input arrays are at the end of the arena, in this order, so moving them
            // down overwrites nothing they still hold.
            debug_assert!(span.start >= mark);
            self.values
                .values
                .copy_within(span.start..span.end(), start);
            span.start = start;
            start += span.len;
        }
        self.values.values.truncate(start);
    }
}

/// Where a merge reads the input frame of its source frame.
#[derive(Clone, Copy)]
enum SourceInputs {
    /// The source frame's own input locals and stack.
    Own,
    /// The input frame of the destination frame, which is the source frame: ASM's merge then
    /// reads the arrays it updates.
    Destination,
}

impl SourceInputs {
    /// The input locals and stack of the source frame, given the destination frame.
    fn spans(self, source: &Frame, dst_frame: &Frame) -> (Option<Span>, Option<Span>) {
        let frame = match self {
            SourceInputs::Own => source,
            SourceInputs::Destination => dst_frame,
        };
        (frame.input_locals, frame.input_stack)
    }
}

/// The arrays of a [`FrameValues`] but the one a merge writes (`dst`), which
/// [`FrameValues::split`] borrows apart.
struct OtherArrays<'a> {
    /// The values before `dst`.
    before: &'a [i32],
    /// The values after `dst`.
    after: &'a [i32],
    dst: Span,
}

/// An array that a merge reads: another array, or the one it writes (the input frame of a basic
/// block that is its own successor).
#[derive(Clone, Copy)]
enum MergeInput<'a> {
    Other(&'a [i32]),
    Dst,
}

impl<'a> OtherArrays<'a> {
    /// The array of the given span (empty if `None`).
    fn array(&self, span: Option<Span>) -> MergeInput<'a> {
        let Some(span) = span else {
            return MergeInput::Other(&[]);
        };
        if span.end() <= self.dst.start {
            MergeInput::Other(&self.before[span.start..span.end()])
        } else if span.start >= self.dst.end() {
            let base = self.dst.end();
            MergeInput::Other(&self.after[span.start - base..span.end() - base])
        } else {
            // Frames do not share arrays, and an array is not part of another.
            debug_assert_eq!(span, self.dst);
            MergeInput::Dst
        }
    }
}

impl<'a> MergeInput<'a> {
    /// The values of this array, given those of the array the merge writes.
    fn get<'b>(self, dst: &'b [i32]) -> &'b [i32]
    where
        'a: 'b,
    {
        match self {
            MergeInput::Other(values) => values,
            MergeInput::Dst => dst,
        }
    }
}

impl FrameValues {
    /// The array of the given span, to write, and the other arrays, to read.
    fn split(&mut self, dst: Span) -> (OtherArrays<'_>, &mut [i32]) {
        let (before, rest) = self.values.split_at_mut(dst.start);
        let (dst_values, after) = rest.split_at_mut(dst.len);
        (OtherArrays { before, after, dst }, dst_values)
    }
}

/// The type [`merge_type`] stores in an unassigned slot from the given type: NULL for an array
/// of NULL.
fn null_merged(abstract_type: i32) -> i32 {
    if abstract_type & !DIM_MASK == NULL {
        NULL
    } else {
        abstract_type
    }
}

/// Computes the concrete output type corresponding to a given abstract output type (ASM's
/// `getConcreteOutputType`), given the input frame of the basic block.
fn get_concrete_output_type(
    abstract_output_type: i32,
    num_stack: usize,
    input_locals: &[i32],
    input_stack: &[i32],
) -> Result<i32, WriteError> {
    let dim = abstract_output_type & DIM_MASK;
    let kind = abstract_output_type & KIND_MASK;
    let concrete_output_type = if kind == LOCAL_KIND {
        // By definition, a LOCAL_KIND type designates the concrete type of a local variable at
        // the beginning of the basic block corresponding to this frame (which is known when this
        // method is called, but was not when the abstract type was computed).
        dim.wrapping_add(element(
            input_locals,
            i64::from(abstract_output_type & VALUE_MASK),
        )?)
    } else if kind == STACK_KIND {
        // By definition, a STACK_KIND type designates the concrete type of a local variable at
        // the beginning of the basic block corresponding to this frame (which is known when this
        // method is called, but was not when the abstract type was computed).
        dim.wrapping_add(element(
            input_stack,
            num_stack as i64 - i64::from(abstract_output_type & VALUE_MASK),
        )?)
    } else {
        return Ok(abstract_output_type);
    };
    if abstract_output_type & TOP_IF_LONG_OR_DOUBLE_FLAG != 0
        && (concrete_output_type == LONG || concrete_output_type == DOUBLE)
    {
        return Ok(TOP);
    }
    Ok(concrete_output_type)
}

/// Returns the "initialized" abstract type corresponding to the given abstract type (ASM's
/// `getInitializedType`), given the initializations and the input frame of the basic block.
fn get_initialized_type(
    symbol_table: &mut SymbolTable,
    initializations: &[i32],
    input_locals: &[i32],
    input_stack: &[i32],
    abstract_type: i32,
) -> Result<i32, WriteError> {
    if abstract_type == UNINITIALIZED_THIS
        || abstract_type & (DIM_MASK | KIND_MASK) == UNINITIALIZED_KIND
        || abstract_type & (DIM_MASK | KIND_MASK) == FORWARD_UNINITIALIZED_KIND
    {
        for &initialization in initializations {
            let mut initialized_type = initialization;
            let dim = initialized_type & DIM_MASK;
            let kind = initialized_type & KIND_MASK;
            let value = initialized_type & VALUE_MASK;
            if kind == LOCAL_KIND {
                initialized_type = dim.wrapping_add(element(input_locals, i64::from(value))?);
            } else if kind == STACK_KIND {
                initialized_type = dim.wrapping_add(element(
                    input_stack,
                    input_stack.len() as i64 - i64::from(value),
                )?);
            }
            if abstract_type == initialized_type {
                let internal_name = if abstract_type == UNINITIALIZED_THIS {
                    symbol_table.class_name_span()
                } else {
                    symbol_table
                        .try_get_type(abstract_type & VALUE_MASK)
                        .ok_or_else(|| {
                            WriteError::InvalidArgument("java.lang.AssertionError".to_owned())
                        })?
                        .value
                };
                return Ok(REFERENCE_KIND | symbol_table.add_type_of(internal_name));
            }
        }
    }
    Ok(abstract_type)
}

/// Merges the input frame of `dst_frame` with the input and output frames of a source frame
/// (ASM's `merge`). Returns whether the input frame of `dst_frame` has been changed.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's int arithmetic
fn merge_frames(
    symbol_table: &mut SymbolTable,
    hierarchy: Option<&dyn ClassHierarchy>,
    values: &mut FrameValues,
    source: &Frame,
    inputs: SourceInputs,
    dst_frame: &mut Frame,
    catch_type_index: i32,
) -> Result<bool, WriteError> {
    let mut frame_changed = false;
    // Compute the concrete types of the local variables at the end of the basic block
    // corresponding to the source frame, by resolving its abstract output types, and merge these
    // concrete types with those of the local variables in the input frame of dst_frame.
    let (num_local, num_stack) = {
        let (input_locals, input_stack) = inputs.spans(source, dst_frame);
        let len = |span: Option<Span>| span.map_or(0, |span| span.len);
        (len(input_locals), len(input_stack))
    };
    let dst_locals = match dst_frame.input_locals {
        Some(span) => span,
        None => {
            frame_changed = true;
            *dst_frame.input_locals.insert(values.alloc(num_local))
        }
    };
    let (input_locals, input_stack) = inputs.spans(source, dst_frame);
    {
        let (arrays, dst_types) = values.split(dst_locals);
        let input_locals = arrays.array(input_locals);
        let input_stack = arrays.array(input_stack);
        let output_locals = arrays.array(source.output_locals);
        let initializations = arrays.array(Some(source.initializations));
        for i in 0..num_local {
            let input_locals = input_locals.get(dst_types);
            let input_stack = input_stack.get(dst_types);
            let output_local = output_locals.get(dst_types).get(i).copied().unwrap_or(0);
            let mut concrete_output_type = if output_local != 0 {
                get_concrete_output_type(output_local, num_stack, input_locals, input_stack)?
            } else {
                // If the local variable has never been assigned in this basic block, it is
                // equal to its value at the beginning of the block.
                input_locals[i]
            };
            // concreteOutputType might be an uninitialized type from the input locals or from
            // the input stack. However, if a constructor has been called for this class type in
            // the basic block, then this type is no longer uninitialized at the end of basic
            // block.
            let initializations = initializations.get(dst_types);
            if !initializations.is_empty() {
                concrete_output_type = get_initialized_type(
                    symbol_table,
                    initializations,
                    input_locals,
                    input_stack,
                    concrete_output_type,
                )?;
            }
            frame_changed |= merge_type(
                symbol_table,
                hierarchy,
                concrete_output_type,
                dst_types,
                i as i64,
            )?;
        }
        // If dst_frame is an exception handler block, it can be reached from any instruction of
        // the basic block corresponding to the source frame, in particular from the first one.
        // Therefore, the input locals of dst_frame should be compatible (i.e. merged) with the
        // input locals of the source frame (and the input stack of dst_frame should be
        // compatible, i.e. merged, with a one element stack containing the caught exception
        // type).
        if catch_type_index > 0 {
            for i in 0..num_local {
                let input_local = input_locals.get(dst_types)[i];
                frame_changed |=
                    merge_type(symbol_table, hierarchy, input_local, dst_types, i as i64)?;
            }
        }
    }
    if catch_type_index > 0 {
        let dst_stack = match dst_frame.input_stack {
            Some(span) => span,
            None => {
                frame_changed = true;
                *dst_frame.input_stack.insert(values.alloc(1))
            }
        };
        frame_changed |= merge_type(
            symbol_table,
            hierarchy,
            catch_type_index,
            values.get_mut(dst_stack),
            0,
        )?;
        return Ok(frame_changed);
    }
    // Compute the concrete types of the stack operands at the end of the basic block
    // corresponding to the source frame, by resolving its abstract output types, and merge these
    // concrete types with those of the stack operands in the input frame of dst_frame.
    let num_input_stack = num_stack as i64 + i64::from(source.output_stack_start);
    let output_stack_top = i64::from(source.output_stack_top);
    let dst_stack = match dst_frame.input_stack {
        Some(span) => span,
        None => {
            let Ok(length) = usize::try_from(num_input_stack + output_stack_top) else {
                // ASM's NegativeArraySizeException (more elements popped than the input stack
                // has).
                return Err(WriteError::IndexOutOfBounds(format!(
                    "{}",
                    num_input_stack + output_stack_top
                )));
            };
            frame_changed = true;
            *dst_frame.input_stack.insert(values.alloc(length))
        }
    };
    let (input_locals, input_stack) = inputs.spans(source, dst_frame);
    let (arrays, dst_types) = values.split(dst_stack);
    let input_locals = arrays.array(input_locals);
    let input_stack = arrays.array(input_stack);
    let output_stack = arrays.array(source.output_stack);
    let initializations = arrays.array(Some(source.initializations));
    // First, do this for the stack operands that have not been popped in the basic block
    // corresponding to the source frame, and which are therefore equal to their value in the
    // input frame (except for uninitialized types, which may have been initialized).
    for i in 0..num_input_stack.max(0) {
        let input_locals = input_locals.get(dst_types);
        let input_stack = input_stack.get(dst_types);
        let mut concrete_output_type = element(input_stack, i)?;
        let initializations = initializations.get(dst_types);
        if !initializations.is_empty() {
            concrete_output_type = get_initialized_type(
                symbol_table,
                initializations,
                input_locals,
                input_stack,
                concrete_output_type,
            )?;
        }
        frame_changed |= merge_type(symbol_table, hierarchy, concrete_output_type, dst_types, i)?;
    }
    // Then, do this for the stack operands that have pushed in the basic block (this code is the
    // same as the one above for local variables).
    for i in 0..output_stack_top.max(0) {
        let input_locals = input_locals.get(dst_types);
        let input_stack = input_stack.get(dst_types);
        let abstract_output_type = element(output_stack.get(dst_types), i)?;
        let mut concrete_output_type =
            get_concrete_output_type(abstract_output_type, num_stack, input_locals, input_stack)?;
        let initializations = initializations.get(dst_types);
        if !initializations.is_empty() {
            concrete_output_type = get_initialized_type(
                symbol_table,
                initializations,
                input_locals,
                input_stack,
                concrete_output_type,
            )?;
        }
        frame_changed |= merge_type(
            symbol_table,
            hierarchy,
            concrete_output_type,
            dst_types,
            num_input_stack + i,
        )?;
    }
    Ok(frame_changed)
}

/// Merges the type at the given index in the given abstract type array with the given type
/// (ASM's static `merge`). Returns whether the type array has been modified by this operation.
fn merge_type(
    symbol_table: &mut SymbolTable,
    hierarchy: Option<&dyn ClassHierarchy>,
    source_type: i32,
    dst_types: &mut [i32],
    dst_index: i64,
) -> Result<bool, WriteError> {
    let length = dst_types.len();
    let Some(dst_type) = usize::try_from(dst_index)
        .ok()
        .and_then(|i| dst_types.get_mut(i))
    else {
        return Err(array_index_out_of_bounds(dst_index, length));
    };
    let dst_type_value = *dst_type;
    if dst_type_value == source_type {
        // If the types are equal, merge(sourceType, dstType) = dstType, so there is no change.
        return Ok(false);
    }
    let mut src_type = source_type;
    if source_type & !DIM_MASK == NULL {
        if dst_type_value == NULL {
            return Ok(false);
        }
        src_type = NULL;
    }
    if dst_type_value == 0 {
        // If dstTypes[dstIndex] has never been assigned, merge(srcType, dstType) = srcType.
        *dst_type = src_type;
        return Ok(true);
    }
    let dst_type_ref = dst_type;
    let dst_type = dst_type_value;
    let merged_type;
    if dst_type & DIM_MASK != 0 || dst_type & KIND_MASK == REFERENCE_KIND {
        // If dstType is a reference type of any array dimension.
        if src_type == NULL {
            // If srcType is the NULL type, merge(srcType, dstType) = dstType, so there is no
            // change.
            return Ok(false);
        } else if src_type & (DIM_MASK | KIND_MASK) == dst_type & (DIM_MASK | KIND_MASK) {
            // If srcType has the same array dimension and the same kind as dstType.
            if dst_type & KIND_MASK == REFERENCE_KIND {
                // If srcType and dstType are reference types with the same array dimension,
                // merge(srcType, dstType) = dim(srcType) | common super class of srcType and
                // dstType.
                merged_type = (src_type & DIM_MASK)
                    | REFERENCE_KIND
                    | symbol_table.add_merged_type(
                        hierarchy,
                        src_type & VALUE_MASK,
                        dst_type & VALUE_MASK,
                    )?;
            } else {
                // If srcType and dstType are array types of equal dimension but different
                // element types, merge(srcType, dstType) = dim(srcType) - 1 | java/lang/Object.
                let merged_dim = ELEMENT_OF.wrapping_add(src_type & DIM_MASK);
                merged_type = merged_dim
                    | REFERENCE_KIND
                    | symbol_table.add_type(crate::jstr!("java/lang/Object"));
            }
        } else if src_type & DIM_MASK != 0 || src_type & KIND_MASK == REFERENCE_KIND {
            // If srcType is any other reference or array type,
            // merge(srcType, dstType) = min(srcDdim, dstDim) | java/lang/Object
            // where srcDim is the array dimension of srcType, minus 1 if srcType is an array type
            // with a non reference element type (and similarly for dstDim).
            let mut src_dim = src_type & DIM_MASK;
            if src_dim != 0 && src_type & KIND_MASK != REFERENCE_KIND {
                src_dim = ELEMENT_OF.wrapping_add(src_dim);
            }
            let mut dst_dim = dst_type & DIM_MASK;
            if dst_dim != 0 && dst_type & KIND_MASK != REFERENCE_KIND {
                dst_dim = ELEMENT_OF.wrapping_add(dst_dim);
            }
            merged_type = src_dim.min(dst_dim)
                | REFERENCE_KIND
                | symbol_table.add_type(crate::jstr!("java/lang/Object"));
        } else {
            // If srcType is any other type, merge(srcType, dstType) = TOP.
            merged_type = TOP;
        }
    } else if dst_type == NULL {
        // If dstType is the NULL type, merge(srcType, dstType) = srcType, or TOP if srcType is
        // not a an array type or a reference type.
        merged_type = if src_type & DIM_MASK != 0 || src_type & KIND_MASK == REFERENCE_KIND {
            src_type
        } else {
            TOP
        };
    } else {
        // If dstType is any other type, merge(srcType, dstType) = TOP whatever srcType.
        merged_type = TOP;
    }
    if merged_type != dst_type {
        *dst_type_ref = merged_type;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        DOUBLE, Frame, FrameMut, FrameValues, INTEGER, LONG, REFERENCE_KIND, TOP,
        UNINITIALIZED_THIS, get_abstract_type_from_descriptor, input_locals_from_descriptor,
        substring_to_last_char,
    };
    use crate::constants::ACC_CONSTRUCTOR;
    use crate::error::WriteError;
    use crate::jstr::{JavaCodePoint, JavaStr, JavaString};
    use crate::opcodes;
    use crate::ty::Type;
    use crate::writer::symbol_table::SymbolTable;

    fn substring(buffer: &str, begin: usize) -> Result<Cow<'_, JavaStr>, String> {
        substring_to_last_char(JavaStr::from_str(buffer), begin).map_err(|e| e.to_string())
    }

    #[test]
    fn substring_to_last_char_borrows_up_to_a_bmp_character() {
        let internal_name = substring("Ljava/lang/String;", 1).unwrap();
        assert!(matches!(internal_name, Cow::Borrowed(_)));
        assert_eq!(*internal_name, *JavaStr::from_str("java/lang/String"));
        // Java's "Laé".substring(1, 2): é is one UTF-16 unit, of two bytes.
        assert_eq!(*substring("Laé", 1).unwrap(), *JavaStr::from_str("a"));
        assert_eq!(*substring("L€", 1).unwrap(), *JavaStr::from_str(""));
        assert_eq!(*substring("[[Lé;", 3).unwrap(), *JavaStr::from_str("é"));
    }

    #[test]
    fn substring_to_last_char_keeps_a_supplementary_characters_high_surrogate() {
        // U+1F600 is the surrogate pair D83D DE00: Java's substring keeps D83D.
        let mut expected = JavaString::from("a");
        expected.push_java(JavaCodePoint::from_u32(0xd83d).unwrap());
        let result = substring("La\u{1f600}", 1).unwrap();
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(*result, *expected);
    }

    #[test]
    fn substring_to_last_char_fails_at_the_end() {
        assert_eq!(
            substring("L", 1).unwrap_err(),
            substring_error("begin 1, end 0, length 1")
        );
        // The UTF-16 length of the buffer, not its byte length.
        assert_eq!(
            substring("\u{1f600}L", 5).unwrap_err(),
            substring_error("begin 3, end 2, length 3")
        );
    }

    fn substring_error(message: &str) -> String {
        crate::error::WriteError::IndexOutOfBounds(message.to_owned()).to_string()
    }

    /// ASM's `setInputFrameFromDescriptor` as it reads: every argument type is created before the
    /// first is used.
    fn input_locals_collecting_first(
        symbol_table: &mut SymbolTable,
        access: u32,
        descriptor: &JavaStr,
        max_locals: usize,
    ) -> Result<Vec<i32>, WriteError> {
        let mut input_locals = vec![0; max_locals];
        let mut index = 0;
        let mut set = |value: i32| {
            let slot = input_locals.get_mut(index).ok_or_else(|| {
                WriteError::IndexOutOfBounds(format!(
                    "Index {index} out of bounds for length {max_locals}"
                ))
            })?;
            *slot = value;
            index += 1;
            Ok::<_, WriteError>(())
        };
        if access & opcodes::ACC_STATIC == 0 {
            if access & ACC_CONSTRUCTOR == 0 {
                set(REFERENCE_KIND | symbol_table.add_type_of(symbol_table.class_name_span()))?;
            } else {
                set(UNINITIALIZED_THIS)?;
            }
        }
        for argument_type in Type::try_get_argument_types_of(descriptor)? {
            let abstract_type = get_abstract_type_from_descriptor(
                symbol_table,
                &argument_type.get_descriptor(),
                0,
            )?;
            set(abstract_type)?;
            if abstract_type == LONG || abstract_type == DOUBLE {
                set(TOP)?;
            }
        }
        input_locals[index..].fill(TOP);
        Ok(input_locals)
    }

    #[test]
    fn input_locals_report_the_errors_of_collecting_the_argument_types_first() {
        let descriptors = [
            "()V",
            "(IJ)V",
            "(DLa/B;[[I[La/C;)V",
            "(IIX)V",
            "([X)V",
            "([XI",
            "([XLa/B",
            "(J",
            "(L;)V",
            "(La/B)V",
            "(Ié)V",
            "(I[é)V",
            "(",
            "",
            "(IIIIIIII)V",
        ];
        let accesses = [opcodes::ACC_STATIC, 0, ACC_CONSTRUCTOR];
        for descriptor in descriptors {
            let descriptor = JavaStr::from_str(descriptor);
            for access in accesses {
                for max_locals in 0..8 {
                    type InputLocals =
                        fn(&mut SymbolTable, u32, &JavaStr, usize) -> Result<Vec<i32>, WriteError>;
                    let appending = |symbol_table: &mut SymbolTable,
                                     access,
                                     descriptor: &JavaStr,
                                     max_locals| {
                        // After other values, which must stay as they are.
                        let mut input_locals = vec![7, 8];
                        let result = input_locals_from_descriptor(
                            symbol_table,
                            access,
                            descriptor,
                            max_locals,
                            &mut input_locals,
                        );
                        assert_eq!(input_locals[..2], [7, 8]);
                        assert_eq!(
                            input_locals.len(),
                            2 + result.as_ref().map_or(0, |()| max_locals)
                        );
                        result.map(|()| input_locals[2..].to_vec())
                    };
                    let result = |f: InputLocals| {
                        let mut symbol_table = SymbolTable::new();
                        symbol_table.set_major_version_and_class_name(
                            opcodes::V1_8 as i32,
                            JavaStr::from_str("pkg/Owner"),
                        );
                        f(&mut symbol_table, access, descriptor, max_locals)
                            .map_err(|e| e.to_string())
                    };
                    assert_eq!(
                        result(appending),
                        result(input_locals_collecting_first),
                        "{descriptor} {access:#x} {max_locals}"
                    );
                }
            }
        }
    }

    #[test]
    fn execute_current_reuses_the_arena() {
        let mut symbol_table = SymbolTable::new();
        symbol_table
            .set_major_version_and_class_name(opcodes::V1_8 as i32, JavaStr::from_str("pkg/Owner"));
        let mut frame = Frame::new(None);
        let mut values = FrameValues::default();
        let mut current = FrameMut {
            frame: &mut frame,
            values: &mut values,
        };
        current
            .set_input_frame_from_descriptor(&mut symbol_table, 0, JavaStr::from_str("(J)V"), 5)
            .unwrap();
        let this = current.values.get(current.frame.input_locals.unwrap())[0];
        for i in 0..100_usize {
            for (opcode, arg) in [
                (opcodes::ALOAD, 0),
                (opcodes::ICONST_0, 0),
                (opcodes::ISTORE, 4),
                (opcodes::ILOAD, 4),
            ] {
                current
                    .execute_current(opcode, arg, None, &mut symbol_table)
                    .unwrap();
            }
            if i % 2 == 1 {
                // An odd stack height, so that the arrays change size.
                current
                    .execute_current(opcodes::POP, 0, None, &mut symbol_table)
                    .unwrap();
            }
            let locals = current.values.get(current.frame.input_locals.unwrap());
            assert_eq!(locals, [this, LONG, TOP, TOP, INTEGER]);
            let stack = current.values.get(current.frame.input_stack.unwrap());
            // Two pushes a round, and a pop every odd round.
            assert_eq!(stack.len(), 2 * (i + 1) - i.div_ceil(2));
            assert!(stack.iter().all(|&value| value == this || value == INTEGER));
            // The arrays of the instructions before are reclaimed: the arena holds the input
            // arrays, and the output arrays of the last instruction (of at least 10 values).
            let len = current.values.values.len();
            assert!(len <= locals.len() + stack.len() + 20, "{i}: {len}");
        }
    }

    /// `execute_current` resolves only the locals an instruction assigned, in place; ASM merges
    /// every local into a new frame. Both give the same frames and the same first error, on
    /// random instruction sequences (loads, stores, stack shuffles and pops below the input
    /// stack, NULLs and array elements of NULL).
    #[test]
    fn execute_current_in_place_is_the_merge() {
        let new_frame = |symbol_table: &mut SymbolTable| {
            symbol_table
                .set_major_version_and_class_name(opcodes::V1_8 as i32, JavaStr::from_str("pkg/C"));
            let mut frame = Frame::new(None);
            let mut values = FrameValues::default();
            FrameMut {
                frame: &mut frame,
                values: &mut values,
            }
            .set_input_frame_from_descriptor(
                symbol_table,
                0,
                JavaStr::from_str("(J[Ljava/lang/String;I)V"),
                12,
            )
            .unwrap();
            (frame, values)
        };
        let frame_of = |frame: &Frame, values: &FrameValues| {
            (
                values.get_or_empty(frame.input_locals).to_vec(),
                values.get_or_empty(frame.input_stack).to_vec(),
            )
        };
        let instructions = [
            (opcodes::ACONST_NULL, -1),
            (opcodes::ICONST_0, -1),
            (opcodes::LCONST_0, -1),
            (opcodes::ALOAD, 0),
            (opcodes::ALOAD, 3),
            (opcodes::ALOAD, 1),
            (opcodes::ASTORE, 1),
            (opcodes::ISTORE, 1),
            (opcodes::LSTORE, 1),
            (opcodes::DSTORE, 1),
            (opcodes::ILOAD, 1),
            (opcodes::LLOAD, 1),
            (opcodes::IINC, 1),
            (opcodes::AALOAD, -1),
            (opcodes::IADD, -1),
            (opcodes::POP, -1),
            (opcodes::POP2, -1),
            (opcodes::DUP, -1),
            (opcodes::DUP_X1, -1),
            (opcodes::DUP2_X2, -1),
            (opcodes::SWAP, -1),
            (opcodes::ARETURN, -1),
        ];
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut in_place_runs = 0;
        for _ in 0..2000 {
            let (mut frame, mut values) = new_frame(&mut SymbolTable::new());
            let (mut reference, mut reference_values) = (frame, values.clone());
            let mut symbol_table = SymbolTable::new();
            let mut reference_symbol_table = SymbolTable::new();
            new_frame(&mut symbol_table);
            new_frame(&mut reference_symbol_table);
            for _ in 0..30 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let (opcode, arg) =
                    instructions[usize::try_from(seed % instructions.len() as u64).unwrap()];
                // A local in 0..14, past the 12 locals too.
                let arg = if arg < 0 {
                    0
                } else {
                    i32::try_from((seed >> 32) % 14).unwrap()
                };
                let mut current = FrameMut {
                    frame: &mut frame,
                    values: &mut values,
                };
                let result = current.execute_current(opcode, arg, None, &mut symbol_table);
                let mut merged = FrameMut {
                    frame: &mut reference,
                    values: &mut reference_values,
                };
                let mark = merged.values.values.len();
                let reference_result = merged
                    .execute(opcode, arg, None, &mut reference_symbol_table)
                    .and_then(|()| {
                        merged.merge_into_new_input_frame(&mut reference_symbol_table, mark)
                    });
                assert_eq!(
                    format!("{result:?}"),
                    format!("{reference_result:?}"),
                    "{opcode} {arg}"
                );
                if result.is_err() {
                    break;
                }
                assert_eq!(
                    frame_of(&frame, &values),
                    frame_of(&reference, &reference_values),
                    "{opcode} {arg}"
                );
                in_place_runs += 1;
            }
        }
        assert!(in_place_runs > 4000, "{in_place_runs}");
    }
}
