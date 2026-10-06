//! The low level parsing methods of ASM's `ClassReader` (its "Utility methods: low level parsing"
//! section): bounds-checked reads of the class file, and the constant pool decoding.
//!
//! Offsets are signed (`i64`), as ASM computes them with Java `int`s that a malformed class can
//! make negative (e.g. with a negative `attribute_length`). Like an array access in Java, a read
//! only fails if it happens *at* an out of bounds offset.

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::constant::Constant;
use crate::constant_dynamic::ConstantDynamic;
use crate::handle::Handle;
use crate::jstr::{JavaStr, JavaString};
use crate::opcodes::HandleKind;
use crate::ty::Type;

use super::{
    CONSTANT_CLASS_TAG, CONSTANT_DOUBLE_TAG, CONSTANT_DYNAMIC_TAG, CONSTANT_FLOAT_TAG,
    CONSTANT_INTEGER_TAG, CONSTANT_INTERFACE_METHODREF_TAG, CONSTANT_LONG_TAG,
    CONSTANT_METHOD_HANDLE_TAG, CONSTANT_METHOD_TYPE_TAG, CONSTANT_STRING_TAG, ClassReader,
    ReadError, ReadErrorKind,
};

/// ASM 9.11's bound on nested `ConstantDynamic` bootstrap arguments (`depthLimit`).
const MAX_CONSTANT_DYNAMIC_DEPTH: u32 = 127;
/// ASM 9.11's bound on the number of `ConstantDynamic` in the tree of a constant
/// (`subtractTreeSize(4096) <= 0` fails).
const MAX_CONSTANT_DYNAMIC_TREE_SIZE: u32 = 4095;

impl<'b> ClassReader<'b> {
    // ---------------------------------------------------------------------------------------------
    // Bounds-checked reads, at signed offsets
    // ---------------------------------------------------------------------------------------------

    /// The `length` bytes at `offset`.
    pub(crate) fn slice_at(
        &self,
        offset: i64,
        length: impl Into<i64>,
    ) -> Result<&'b [u8], ReadError> {
        let length = length.into();
        let start = usize::try_from(offset).ok();
        let end = usize::try_from(offset + length).ok();
        match (start, end) {
            (Some(start), Some(end)) if start <= end && end <= self.class_file.len() => {
                Ok(&self.class_file[start..end])
            }
            _ if length < 0 => Err(ReadError::new(offset, ReadErrorKind::InvalidLength(length))),
            _ => Err(ReadError::new(offset, ReadErrorKind::UnexpectedEnd)),
        }
    }

    /// The `N` bytes at `offset`.
    #[inline]
    fn bytes_at<const N: usize>(&self, offset: i64) -> Result<[u8; N], ReadError> {
        usize::try_from(offset)
            .ok()
            .and_then(|start| self.class_file.get(start..start.checked_add(N)?))
            .map(|bytes| bytes.try_into().expect("N bytes"))
            .ok_or_else(|| ReadError::new(offset, ReadErrorKind::UnexpectedEnd))
    }

    /// The byte at `offset` (ASM's `readByte`, or `classFileBuffer[offset] & 0xFF`).
    #[inline]
    pub(crate) fn u8_at(&self, offset: i64) -> Result<u8, ReadError> {
        Ok(self.bytes_at::<1>(offset)?[0])
    }

    /// The signed byte at `offset` (ASM's `classFileBuffer[offset]`).
    #[inline]
    pub(crate) fn i8_at(&self, offset: i64) -> Result<i8, ReadError> {
        Ok(self.u8_at(offset)? as i8)
    }

    /// ASM's `readUnsignedShort`.
    #[inline]
    pub(crate) fn u16_at(&self, offset: i64) -> Result<u16, ReadError> {
        Ok(u16::from_be_bytes(self.bytes_at(offset)?))
    }

    /// ASM's `readShort`.
    #[inline]
    pub(crate) fn i16_at(&self, offset: i64) -> Result<i16, ReadError> {
        Ok(i16::from_be_bytes(self.bytes_at(offset)?))
    }

    /// ASM's `readInt`.
    #[inline]
    pub(crate) fn i32_at(&self, offset: i64) -> Result<i32, ReadError> {
        Ok(i32::from_be_bytes(self.bytes_at(offset)?))
    }

    /// ASM's `readLong`.
    #[inline]
    pub(crate) fn i64_at(&self, offset: i64) -> Result<i64, ReadError> {
        Ok(i64::from_be_bytes(self.bytes_at(offset)?))
    }

    /// `cpInfoOffsets[index]`: the offset of constant pool entry `index`, plus one (0 for entry 0
    /// and the second slot of long and double entries).
    #[inline]
    pub(crate) fn cp_info_offset(&self, index: usize) -> Result<i64, ReadError> {
        match self.cp_info_offsets.get(index) {
            Some(&offset) => Ok(i64::from(offset)),
            None => Err(ReadError::new(
                0,
                ReadErrorKind::InvalidConstantPoolIndex(index as u16),
            )),
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Public low level parsing methods
    // ---------------------------------------------------------------------------------------------

    /// Returns the number of entries in the class's constant pool table (the
    /// `constant_pool_count` field).
    #[must_use]
    pub fn get_item_count(&self) -> usize {
        self.cp_info_offsets.len()
    }

    /// Returns the start offset in this class file of a JVMS 'cp_info' structure (i.e. a
    /// constant pool entry), *plus one*. *This method is intended for attribute codecs, and is
    /// normally not needed by users*.
    ///
    /// # Errors
    ///
    /// If `constant_pool_entry_index` is out of the constant pool.
    pub fn get_item(&self, constant_pool_entry_index: usize) -> Result<usize, ReadError> {
        Ok(self.cp_info_offset(constant_pool_entry_index)? as usize)
    }

    /// Returns a conservative estimate of the maximum length of the strings contained in the
    /// class's constant pool table.
    #[must_use]
    pub fn get_max_string_length(&self) -> usize {
        self.max_string_length
    }

    /// Reads a byte value in this class file.
    ///
    /// # Errors
    ///
    /// If `offset` is outside of the class file.
    pub fn read_byte(&self, offset: usize) -> Result<u8, ReadError> {
        self.u8_at(offset as i64)
    }

    /// Reads `length` bytes of this class file, starting at `offset`.
    ///
    /// # Errors
    ///
    /// If the bytes are not all in the class file.
    pub fn read_bytes(&self, offset: usize, length: usize) -> Result<&'b [u8], ReadError> {
        self.slice_at(offset as i64, length as i64)
    }

    /// Reads an unsigned short value in this class file.
    ///
    /// # Errors
    ///
    /// If the value is not in the class file.
    pub fn read_unsigned_short(&self, offset: usize) -> Result<u16, ReadError> {
        self.u16_at(offset as i64)
    }

    /// Reads a signed short value in this class file.
    ///
    /// # Errors
    ///
    /// If the value is not in the class file.
    pub fn read_short(&self, offset: usize) -> Result<i16, ReadError> {
        self.i16_at(offset as i64)
    }

    /// Reads a signed int value in this class file.
    ///
    /// # Errors
    ///
    /// If the value is not in the class file.
    pub fn read_int(&self, offset: usize) -> Result<i32, ReadError> {
        self.i32_at(offset as i64)
    }

    /// Reads a signed long value in this class file.
    ///
    /// # Errors
    ///
    /// If the value is not in the class file.
    pub fn read_long(&self, offset: usize) -> Result<i64, ReadError> {
        self.i64_at(offset as i64)
    }

    /// Reads a `CONSTANT_Utf8` constant pool entry in this class file: `offset` is the offset of
    /// an unsigned short value whose value is the index of the entry. Returns `None` if that
    /// index (or `offset`) is 0. *This method is intended for attribute codecs, and is normally
    /// not needed by users* (ASM's `readUTF8`).
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn read_utf8(&self, offset: usize) -> Result<Option<&JavaStr>, ReadError> {
        self.utf8_at(offset as i64)
    }

    /// Reads a `CONSTANT_Class` constant pool entry in this class file: `offset` is the offset of
    /// an unsigned short value whose value is the index of the entry. Returns `None` if that
    /// index is 0. *This method is intended for attribute codecs*.
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn read_class(&self, offset: usize) -> Result<Option<&JavaStr>, ReadError> {
        self.read_class_at(offset as i64)
    }

    /// Reads a `CONSTANT_Module` constant pool entry in this class file (see
    /// [`ClassReader::read_class`]).
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn read_module(&self, offset: usize) -> Result<Option<&JavaStr>, ReadError> {
        self.read_class_at(offset as i64)
    }

    /// Reads a `CONSTANT_Package` constant pool entry in this class file (see
    /// [`ClassReader::read_class`]).
    ///
    /// # Errors
    ///
    /// If the class file is malformed.
    pub fn read_package(&self, offset: usize) -> Result<Option<&JavaStr>, ReadError> {
        self.read_class_at(offset as i64)
    }

    /// Reads a numeric or string constant pool entry in this class file: a `CONSTANT_Integer`,
    /// `CONSTANT_Float`, `CONSTANT_Long`, `CONSTANT_Double`, `CONSTANT_Class`,
    /// `CONSTANT_String`, `CONSTANT_MethodType`, `CONSTANT_MethodHandle` or `CONSTANT_Dynamic`
    /// entry. *This method is intended for attribute codecs*.
    ///
    /// # Errors
    ///
    /// If the entry is not a constant, or is malformed.
    pub fn read_const(&self, constant_pool_entry_index: usize) -> Result<Constant<'_>, ReadError> {
        self.const_at(constant_pool_entry_index)
    }

    // ---------------------------------------------------------------------------------------------
    // Strings
    // ---------------------------------------------------------------------------------------------

    /// ASM's `readUTF8`: the `CONSTANT_Utf8` entry whose index is at `offset`, or `None` if
    /// `offset` or this index is 0.
    pub(crate) fn utf8_at(&self, offset: i64) -> Result<Option<&JavaStr>, ReadError> {
        let constant_pool_entry_index = self.u16_at(offset)?;
        if offset == 0 || constant_pool_entry_index == 0 {
            return Ok(None);
        }
        self.utf(usize::from(constant_pool_entry_index)).map(Some)
    }

    /// ASM's `readUtf(int, char[])`: the `CONSTANT_Utf8` entry of the given index, decoded on
    /// first use.
    pub(crate) fn utf(&self, constant_pool_entry_index: usize) -> Result<&JavaStr, ReadError> {
        let Some(cell) = self.constant_utf8_values.get(constant_pool_entry_index) else {
            return Err(ReadError::new(
                0,
                ReadErrorKind::InvalidConstantPoolIndex(constant_pool_entry_index as u16),
            ));
        };
        if let Some(value) = cell.get() {
            return Ok(value);
        }
        let cp_info_offset = self.cp_info_offset(constant_pool_entry_index)?;
        let value = self.read_utf(
            cp_info_offset + 2,
            i32::from(self.u16_at(cp_info_offset)?),
            self.max_string_length,
        )?;
        // Another thread may have decoded the same string meanwhile: either value is fine.
        Ok(cell.get_or_init(|| value))
    }

    /// ASM's `readUtf(int, int, char[])`: decodes the `utf_length` bytes at `utf_offset` as
    /// modified UTF-8, into at most `capacity` UTF-16 units (the size of ASM's `charBuffer`).
    ///
    /// ASM's decoder never rejects a byte sequence: it decodes any lead byte that is not ASCII
    /// nor `110xxxxx` as a 3 bytes sequence, without checking the continuation bytes (and may
    /// read past `utf_length` for a truncated sequence). This is ported as is, into UTF-16 units
    /// that become a `JavaString`. When the bytes are valid UTF-8 without 4 bytes sequences,
    /// both decodings agree, and the string is borrowed from the class file instead.
    pub(crate) fn read_utf(
        &self,
        utf_offset: i64,
        utf_length: i32,
        capacity: usize,
    ) -> Result<Cow<'b, JavaStr>, ReadError> {
        if utf_length >= 0
            && utf_length as usize <= capacity
            && let Ok(bytes) = self.slice_at(utf_offset, utf_length)
            && (bytes.is_ascii() || (!bytes.iter().any(|&b| b >= 0xF0)))
            && let Ok(s) = std::str::from_utf8(bytes)
        {
            return Ok(Cow::Borrowed(JavaStr::from_str(s)));
        }
        let mut current_offset = utf_offset;
        let end_offset = utf_offset + i64::from(utf_length);
        let mut chars: Vec<u16> = Vec::with_capacity((utf_length.max(0) as usize).min(capacity));
        while current_offset < end_offset {
            if chars.len() == capacity {
                // ASM's charBuffer is full: ArrayIndexOutOfBoundsException.
                return Err(ReadError::new(
                    current_offset,
                    ReadErrorKind::InvalidLength(utf_length.into()),
                ));
            }
            let current_byte = i32::from(self.i8_at(current_offset)?);
            current_offset += 1;
            let c = if (current_byte & 0x80) == 0 {
                (current_byte & 0x7F) as u16
            } else if (current_byte & 0xE0) == 0xC0 {
                let b1 = i32::from(self.u8_at(current_offset)?);
                current_offset += 1;
                (((current_byte & 0x1F) << 6) + (b1 & 0x3F)) as u16
            } else {
                let b1 = i32::from(self.u8_at(current_offset)?);
                let b2 = i32::from(self.u8_at(current_offset + 1)?);
                current_offset += 2;
                (((current_byte & 0xF) << 12) + ((b1 & 0x3F) << 6) + (b2 & 0x3F)) as u16
            };
            chars.push(c);
        }
        Ok(Cow::Owned(JavaString::from_utf16(&chars)))
    }

    /// ASM's `readStringish` (and `readClass`, `readModule`, `readPackage`): the `CONSTANT_Utf8`
    /// entry referenced by the entry whose index is at `offset`.
    pub(crate) fn read_class_at(&self, offset: i64) -> Result<Option<&JavaStr>, ReadError> {
        // Get the start offset of the cp_info structure (plus one), and read the CONSTANT_Utf8
        // entry designated by the first two bytes of this cp_info.
        let cp_info_offset = self.cp_info_offset(usize::from(self.u16_at(offset)?))?;
        self.utf8_at(cp_info_offset)
    }

    // ---------------------------------------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------------------------------------

    /// ASM's `readConst`.
    pub(crate) fn const_at(
        &self,
        constant_pool_entry_index: usize,
    ) -> Result<Constant<'_>, ReadError> {
        let cp_info_offset = self.cp_info_offset(constant_pool_entry_index)?;
        let tag = self.u8_at(cp_info_offset - 1)?;
        Ok(match tag {
            CONSTANT_INTEGER_TAG => Constant::Integer(self.i32_at(cp_info_offset)?),
            CONSTANT_FLOAT_TAG => {
                Constant::Float(f32::from_bits(self.i32_at(cp_info_offset)? as u32))
            }
            CONSTANT_LONG_TAG => Constant::Long(self.i64_at(cp_info_offset)?),
            CONSTANT_DOUBLE_TAG => {
                Constant::Double(f64::from_bits(self.i64_at(cp_info_offset)? as u64))
            }
            CONSTANT_CLASS_TAG => {
                let internal_name = self.utf8_at(cp_info_offset)?;
                // Type.getObjectType reads internalName.charAt(0).
                match internal_name {
                    Some(name) if !name.is_empty() => Constant::Type(Type::get_object_type(name)),
                    _ => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
                }
            }
            CONSTANT_STRING_TAG => match self.utf8_at(cp_info_offset)? {
                Some(value) => Constant::String(Cow::Borrowed(value)),
                None => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
            },
            CONSTANT_METHOD_TYPE_TAG => match self.utf8_at(cp_info_offset)? {
                Some(descriptor) => Constant::Type(Type::get_method_type(descriptor)),
                None => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
            },
            CONSTANT_METHOD_HANDLE_TAG => Constant::Handle(self.read_handle(cp_info_offset)?),
            CONSTANT_DYNAMIC_TAG => {
                let (constant_dynamic, tree_size) = self
                    .read_constant_dynamic(constant_pool_entry_index, MAX_CONSTANT_DYNAMIC_DEPTH)?;
                if tree_size > MAX_CONSTANT_DYNAMIC_TREE_SIZE {
                    return Err(ReadError::new(
                        cp_info_offset,
                        ReadErrorKind::RecursionLimit,
                    ));
                }
                Constant::Dynamic(Box::new(constant_dynamic))
            }
            _ => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
        })
    }

    /// The `CONSTANT_MethodHandle` entry at `cp_info_offset` (the `CONSTANT_METHOD_HANDLE_TAG`
    /// case of ASM's `readConst`).
    fn read_handle(&self, cp_info_offset: i64) -> Result<Handle, ReadError> {
        let reference_kind = self.u8_at(cp_info_offset)?;
        let reference_cp_info_offset =
            self.cp_info_offset(usize::from(self.u16_at(cp_info_offset + 1)?))?;
        let name_and_type_cp_info_offset =
            self.cp_info_offset(usize::from(self.u16_at(reference_cp_info_offset + 2)?))?;
        let owner = self.read_class_at(reference_cp_info_offset)?;
        let name = self.utf8_at(name_and_type_cp_info_offset)?;
        let descriptor = self.utf8_at(name_and_type_cp_info_offset + 2)?;
        let is_interface =
            self.u8_at(reference_cp_info_offset - 1)? == CONSTANT_INTERFACE_METHODREF_TAG;
        // ADR-09: ASM accepts any reference kind, and fails later on the invalid ones.
        let tag = HandleKind::from_tag(reference_kind).ok_or_else(|| {
            ReadError::new(
                cp_info_offset,
                ReadErrorKind::InvalidHandleKind(reference_kind),
            )
        })?;
        let (Some(owner), Some(name), Some(descriptor)) = (owner, name, descriptor) else {
            return Err(ReadError::new(
                cp_info_offset,
                ReadErrorKind::InvalidConstantPoolIndex(0),
            ));
        };
        Ok(Handle::new(tag, owner, name, descriptor, is_interface))
    }

    /// Whether the constant pool entry of the given index is a `CONSTANT_Dynamic` (ASM 9.11's
    /// `isConstantDynamic`).
    fn is_constant_dynamic(&self, constant_pool_entry_index: usize) -> Result<bool, ReadError> {
        let cp_info_offset = self.cp_info_offset(constant_pool_entry_index)?;
        Ok(self.u8_at(cp_info_offset - 1)? == CONSTANT_DYNAMIC_TAG)
    }

    /// ASM's `readConstantDynamic`, with ASM 9.11's bounds: at most `depth_limit` more levels of
    /// nested `ConstantDynamic`, and a tree of at most 4095 `ConstantDynamic` (counted as it is
    /// built, since a shared subtree is copied here). Returns the constant and its tree size.
    fn read_constant_dynamic(
        &self,
        constant_pool_entry_index: usize,
        depth_limit: u32,
    ) -> Result<(ConstantDynamic, u32), ReadError> {
        if depth_limit == 0 {
            return Err(ReadError::new(0, ReadErrorKind::RecursionLimit));
        }
        let cell: Option<&OnceLock<(ConstantDynamic, u32)>> =
            self.constant_dynamic_values.get(constant_pool_entry_index);
        if let Some((value, size)) = cell.and_then(OnceLock::get) {
            return Ok((value.clone(), *size));
        }
        let cp_info_offset = self.cp_info_offset(constant_pool_entry_index)?;
        let name_and_type_cp_info_offset =
            self.cp_info_offset(usize::from(self.u16_at(cp_info_offset + 2)?))?;
        let name = self.utf8_at(name_and_type_cp_info_offset)?;
        let descriptor = self.utf8_at(name_and_type_cp_info_offset + 2)?;
        let mut bootstrap_method_offset =
            self.bootstrap_method_offset(self.u16_at(cp_info_offset)?, cp_info_offset)?;
        let handle_cp_index = usize::from(self.u16_at(bootstrap_method_offset)?);
        if self.is_constant_dynamic(handle_cp_index)? {
            // ASM 9.11: "ConstantDynamic cannot be cast to Handle".
            return Err(invalid_constant(bootstrap_method_offset, handle_cp_index));
        }
        let Constant::Handle(handle) = self.const_at(handle_cp_index)? else {
            return Err(invalid_constant(bootstrap_method_offset, handle_cp_index));
        };
        let num_bootstrap_arguments = self.u16_at(bootstrap_method_offset + 2)?;
        let mut bootstrap_method_arguments = Vec::with_capacity(num_bootstrap_arguments.into());
        let mut tree_size = 1;
        bootstrap_method_offset += 4;
        for _ in 0..num_bootstrap_arguments {
            let argument_cp_index = usize::from(self.u16_at(bootstrap_method_offset)?);
            let argument = if self.is_constant_dynamic(argument_cp_index)? {
                let (argument, size) =
                    self.read_constant_dynamic(argument_cp_index, depth_limit - 1)?;
                tree_size += size;
                if tree_size > MAX_CONSTANT_DYNAMIC_TREE_SIZE {
                    return Err(ReadError::new(
                        cp_info_offset,
                        ReadErrorKind::RecursionLimit,
                    ));
                }
                Constant::Dynamic(Box::new(argument))
            } else {
                self.const_at(argument_cp_index)?.into_owned()
            };
            bootstrap_method_arguments.push(argument);
            bootstrap_method_offset += 2;
        }
        let (Some(name), Some(descriptor)) = (name, descriptor) else {
            return Err(ReadError::new(
                cp_info_offset,
                ReadErrorKind::InvalidConstantPoolIndex(0),
            ));
        };
        let value = ConstantDynamic::new(name, descriptor, handle, bootstrap_method_arguments);
        if let Some(cell) = cell {
            // Another thread may have decoded the same constant meanwhile: either is fine.
            let _ = cell.set((value.clone(), tree_size));
        }
        Ok((value, tree_size))
    }

    /// `bootstrapMethodOffsets[index]`: the offset of a `bootstrap_methods` entry.
    pub(crate) fn bootstrap_method_offset(&self, index: u16, at: i64) -> Result<i64, ReadError> {
        // ASM: a NullPointerException without a BootstrapMethods attribute (which the
        // constructor requires if there are dynamic constants), or an out of bounds index.
        self.bootstrap_method_offsets
            .as_deref()
            .and_then(|offsets| offsets.get(usize::from(index)))
            .map(|&offset| i64::from(offset))
            .ok_or_else(|| ReadError::new(at, ReadErrorKind::InvalidConstantPoolIndex(index)))
    }
}

/// A constant pool entry that does not have the kind ASM expects there (where ASM throws an
/// `IllegalArgumentException`, a `ClassCastException` or a `NullPointerException`).
fn invalid_constant(offset: i64, constant_pool_entry_index: usize) -> ReadError {
    ReadError::new(
        offset,
        ReadErrorKind::InvalidConstant(constant_pool_entry_index as u16),
    )
}
