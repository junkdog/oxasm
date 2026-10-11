//! The low level parsing methods of ASM's `ClassReader` (its "Utility methods: low level parsing"
//! section): bounds-checked reads of the class file, and the constant pool decoding.
//!
//! Offsets are signed (`i64`), as ASM computes them with Java `int`s that a malformed class can
//! make negative (e.g. with a negative `attribute_length`). Like an array access in Java, a read
//! only fails if it happens *at* an out of bounds offset.

use std::borrow::Cow;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

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

/// The bits of a `cp_info_entries` entry that hold its `cp_info` offset plus one (a class file is
/// shorter than 2 GiB).
const CP_INFO_OFFSET: u64 = 0xFFFF_FFFF;
/// The flag of a `cp_info_entries` entry whose string is borrowed from the class file. The
/// entry's bits 32-47 are the string's length, and the string is at its offset in `utf8_bytes`.
const UTF8_BORROWED: u64 = 1 << 62;
/// The flag of a `cp_info_entries` entry whose string is decoded into `constant_utf8_decoded`.
const UTF8_DECODED: u64 = 1 << 63;

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
        Ok(tri!(self.bytes_at::<1>(offset))[0])
    }

    /// The signed byte at `offset` (ASM's `classFileBuffer[offset]`).
    #[inline]
    pub(crate) fn i8_at(&self, offset: i64) -> Result<i8, ReadError> {
        Ok(tri!(self.u8_at(offset)) as i8)
    }

    /// ASM's `readUnsignedShort`.
    #[inline]
    pub(crate) fn u16_at(&self, offset: i64) -> Result<u16, ReadError> {
        Ok(u16::from_be_bytes(tri!(self.bytes_at(offset))))
    }

    /// ASM's `readShort`.
    #[inline]
    pub(crate) fn i16_at(&self, offset: i64) -> Result<i16, ReadError> {
        Ok(i16::from_be_bytes(tri!(self.bytes_at(offset))))
    }

    /// ASM's `readInt`.
    #[inline]
    pub(crate) fn i32_at(&self, offset: i64) -> Result<i32, ReadError> {
        Ok(i32::from_be_bytes(tri!(self.bytes_at(offset))))
    }

    /// ASM's `readLong`.
    #[inline]
    pub(crate) fn i64_at(&self, offset: i64) -> Result<i64, ReadError> {
        Ok(i64::from_be_bytes(tri!(self.bytes_at(offset))))
    }

    /// `cpInfoOffsets[index]`: the offset of constant pool entry `index`, plus one (0 for entry 0
    /// and the second slot of long and double entries).
    #[inline]
    pub(crate) fn cp_info_offset(&self, index: usize) -> Result<i64, ReadError> {
        match self.cp_info_entries.get(index) {
            // The offset bits never change, so any ordering reads them.
            Some(entry) => Ok((entry.load(Ordering::Relaxed) & CP_INFO_OFFSET) as i64),
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
        self.cp_info_entries.len()
    }

    /// Returns the start offset in this class file of a JVMS 'cp_info' structure (i.e. a
    /// constant pool entry), *plus one*. *This method is intended for attribute codecs, and is
    /// normally not needed by users*.
    ///
    /// # Errors
    ///
    /// If `constant_pool_entry_index` is out of the constant pool.
    pub fn get_item(&self, constant_pool_entry_index: usize) -> Result<usize, ReadError> {
        Ok(tri!(self.cp_info_offset(constant_pool_entry_index)) as usize)
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
    #[inline]
    pub(crate) fn utf8_at(&self, offset: i64) -> Result<Option<&JavaStr>, ReadError> {
        let constant_pool_entry_index = tri!(self.u16_at(offset));
        if offset == 0 || constant_pool_entry_index == 0 {
            return Ok(None);
        }
        self.utf(usize::from(constant_pool_entry_index)).map(Some)
    }

    /// ASM's `readUtf(int, char[])`: the `CONSTANT_Utf8` entry of the given index, decoded on
    /// first use.
    #[inline]
    pub(crate) fn utf(&self, constant_pool_entry_index: usize) -> Result<&JavaStr, ReadError> {
        // The common case, a string borrowed from the class file, stays inline: the rest is
        // out of line, so that this path does not pay for its registers.
        if let Some(cell) = self.cp_info_entries.get(constant_pool_entry_index) {
            let state = cell.load(Ordering::Acquire);
            if state & UTF8_BORROWED != 0 {
                return Ok(self.borrowed_utf(state));
            }
        }
        self.utf_out_of_line(constant_pool_entry_index)
    }

    /// The string of a `cp_info_entries` entry borrowed from the class file.
    #[inline(always)]
    fn borrowed_utf(&self, state: u64) -> &'b JavaStr {
        let offset = (state & CP_INFO_OFFSET) as usize;
        let length = ((state >> 32) & 0xFFFF) as usize;
        let bytes = &self.utf8_bytes[offset..offset + length];
        // SAFETY: the entry was stored by `utf_out_of_line`, from a string that `read_utf`
        // borrowed from these bytes, so they are valid UTF-8.
        unsafe { JavaStr::from_semi_utf8_unchecked(bytes) }
    }

    /// [`ClassReader::utf`], out of line: for an entry that is not a borrowed string already,
    /// and for callers that read each entry once (e.g. a constant pool copy), which would
    /// mostly miss the inline path.
    #[inline(never)]
    pub(crate) fn utf_out_of_line(
        &self,
        constant_pool_entry_index: usize,
    ) -> Result<&JavaStr, ReadError> {
        let Some(cell) = self.cp_info_entries.get(constant_pool_entry_index) else {
            return Err(ReadError::new(
                0,
                ReadErrorKind::InvalidConstantPoolIndex(constant_pool_entry_index as u16),
            ));
        };
        let state = cell.load(Ordering::Acquire);
        if state & UTF8_BORROWED != 0 {
            return Ok(self.borrowed_utf(state));
        }
        if state & UTF8_DECODED != 0 {
            // The release store of UTF8_DECODED happened after both cells were initialized.
            let decoded = self.constant_utf8_decoded.get().expect("initialized");
            return Ok(decoded[constant_pool_entry_index]
                .get()
                .expect("initialized"));
        }
        let cp_info_offset = (state & CP_INFO_OFFSET) as i64;
        let utf_offset = cp_info_offset + 2;
        let value = tri!(self.read_utf(
            utf_offset,
            i32::from(tri!(self.u16_at(cp_info_offset))),
            self.max_string_length,
        ));
        match value {
            Cow::Borrowed(value) => {
                debug_assert!(std::ptr::eq(
                    value.as_bytes(),
                    &self.class_file[utf_offset as usize..utf_offset as usize + value.len()]
                ));
                // Racing threads store the same value.
                cell.store(
                    UTF8_BORROWED | (value.len() as u64) << 32 | cp_info_offset as u64,
                    Ordering::Release,
                );
                Ok(value)
            }
            Cow::Owned(value) => {
                let decoded = self.constant_utf8_decoded.get_or_init(|| {
                    (0..self.cp_info_entries.len())
                        .map(|_| OnceLock::new())
                        .collect()
                });
                // Another thread may have decoded the same string meanwhile: either value is fine.
                let value = decoded[constant_pool_entry_index].get_or_init(|| value);
                cell.store(UTF8_DECODED | cp_info_offset as u64, Ordering::Release);
                Ok(value)
            }
        }
    }

    /// ASM's `readUtf(int, int, char[])`: decodes the `utf_length` bytes at `utf_offset` as
    /// modified UTF-8, into at most `capacity` UTF-16 units (the size of ASM's `charBuffer`).
    ///
    /// ASM's decoder never rejects a byte sequence: it decodes any lead byte that is not ASCII
    /// nor `110xxxxx` as a 3 bytes sequence, without checking the continuation bytes (and may
    /// read past `utf_length` for a truncated sequence). This is ported as is, into UTF-16 units
    /// that become a `JavaString`. When the bytes are valid UTF-8 without 4 bytes sequences,
    /// both decodings agree, and the string is borrowed from the class file instead.
    #[inline(always)]
    pub(crate) fn read_utf(
        &self,
        utf_offset: i64,
        utf_length: i32,
        capacity: usize,
    ) -> Result<Cow<'b, JavaStr>, ReadError> {
        if utf_length >= 0
            && utf_length as usize <= capacity
            && let Ok(bytes) = self.slice_at(utf_offset, utf_length)
        {
            if bytes.is_ascii() {
                // SAFETY: ASCII is canonical semi-UTF-8.
                return Ok(Cow::Borrowed(unsafe {
                    JavaStr::from_semi_utf8_unchecked(bytes)
                }));
            }
            if !bytes.iter().any(|&b| b >= 0xF0)
                && let Ok(s) = std::str::from_utf8(bytes)
            {
                return Ok(Cow::Borrowed(JavaStr::from_str(s)));
            }
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
            let current_byte = i32::from(tri!(self.i8_at(current_offset)));
            current_offset += 1;
            let c = if (current_byte & 0x80) == 0 {
                (current_byte & 0x7F) as u16
            } else if (current_byte & 0xE0) == 0xC0 {
                let b1 = i32::from(tri!(self.u8_at(current_offset)));
                current_offset += 1;
                (((current_byte & 0x1F) << 6) + (b1 & 0x3F)) as u16
            } else {
                let b1 = i32::from(tri!(self.u8_at(current_offset)));
                let b2 = i32::from(tri!(self.u8_at(current_offset + 1)));
                current_offset += 2;
                (((current_byte & 0xF) << 12) + ((b1 & 0x3F) << 6) + (b2 & 0x3F)) as u16
            };
            chars.push(c);
        }
        Ok(Cow::Owned(JavaString::from_utf16(&chars)))
    }

    /// ASM's `readStringish` (and `readClass`, `readModule`, `readPackage`): the `CONSTANT_Utf8`
    /// entry referenced by the entry whose index is at `offset`.
    #[inline(always)]
    pub(crate) fn read_class_at(&self, offset: i64) -> Result<Option<&JavaStr>, ReadError> {
        // Get the start offset of the cp_info structure (plus one), and read the CONSTANT_Utf8
        // entry designated by the first two bytes of this cp_info.
        let cp_info_offset = tri!(self.cp_info_offset(usize::from(tri!(self.u16_at(offset)))));
        self.utf8_at(cp_info_offset)
    }

    // ---------------------------------------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------------------------------------

    /// ASM's `readConst`.
    #[inline(always)]
    pub(crate) fn const_at(
        &self,
        constant_pool_entry_index: usize,
    ) -> Result<Constant<'_>, ReadError> {
        let cp_info_offset = tri!(self.cp_info_offset(constant_pool_entry_index));
        let tag = tri!(self.u8_at(cp_info_offset - 1));
        Ok(match tag {
            CONSTANT_INTEGER_TAG => Constant::Integer(tri!(self.i32_at(cp_info_offset))),
            CONSTANT_FLOAT_TAG => {
                Constant::Float(f32::from_bits(tri!(self.i32_at(cp_info_offset)) as u32))
            }
            CONSTANT_LONG_TAG => Constant::Long(tri!(self.i64_at(cp_info_offset))),
            CONSTANT_DOUBLE_TAG => {
                Constant::Double(f64::from_bits(tri!(self.i64_at(cp_info_offset)) as u64))
            }
            CONSTANT_CLASS_TAG => {
                let internal_name = tri!(self.utf8_at(cp_info_offset));
                // Type.getObjectType reads internalName.charAt(0).
                match internal_name {
                    Some(name) if !name.is_empty() => Constant::Type(Type::get_object_type(name)),
                    _ => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
                }
            }
            CONSTANT_STRING_TAG => match tri!(self.utf8_at(cp_info_offset)) {
                Some(value) => Constant::String(Cow::Borrowed(value)),
                None => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
            },
            CONSTANT_METHOD_TYPE_TAG => match tri!(self.utf8_at(cp_info_offset)) {
                Some(descriptor) => Constant::Type(Type::get_method_type(descriptor)),
                None => return Err(invalid_constant(cp_info_offset, constant_pool_entry_index)),
            },
            CONSTANT_METHOD_HANDLE_TAG => Constant::Handle(tri!(self.read_handle(cp_info_offset))),
            CONSTANT_DYNAMIC_TAG => {
                let (constant_dynamic, tree_size) =
                    tri!(self.read_constant_dynamic(
                        constant_pool_entry_index,
                        MAX_CONSTANT_DYNAMIC_DEPTH
                    ));
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
        let reference_kind = tri!(self.u8_at(cp_info_offset));
        let reference_cp_info_offset =
            tri!(self.cp_info_offset(usize::from(tri!(self.u16_at(cp_info_offset + 1)))));
        let name_and_type_cp_info_offset =
            tri!(self.cp_info_offset(usize::from(tri!(self.u16_at(reference_cp_info_offset + 2)))));
        let owner = tri!(self.read_class_at(reference_cp_info_offset));
        let name = tri!(self.utf8_at(name_and_type_cp_info_offset));
        let descriptor = tri!(self.utf8_at(name_and_type_cp_info_offset + 2));
        let is_interface =
            tri!(self.u8_at(reference_cp_info_offset - 1)) == CONSTANT_INTERFACE_METHODREF_TAG;
        // ADR-09: ASM accepts any reference kind, and fails later on the invalid ones.
        let tag = tri!(HandleKind::from_tag(reference_kind).ok_or_else(|| {
            ReadError::new(
                cp_info_offset,
                ReadErrorKind::InvalidHandleKind(reference_kind),
            )
        }));
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
        let cp_info_offset = tri!(self.cp_info_offset(constant_pool_entry_index));
        Ok(tri!(self.u8_at(cp_info_offset - 1)) == CONSTANT_DYNAMIC_TAG)
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
        let cp_info_offset = tri!(self.cp_info_offset(constant_pool_entry_index));
        let name_and_type_cp_info_offset =
            tri!(self.cp_info_offset(usize::from(tri!(self.u16_at(cp_info_offset + 2)))));
        let name = tri!(self.utf8_at(name_and_type_cp_info_offset));
        let descriptor = tri!(self.utf8_at(name_and_type_cp_info_offset + 2));
        let mut bootstrap_method_offset =
            tri!(self.bootstrap_method_offset(tri!(self.u16_at(cp_info_offset)), cp_info_offset));
        let handle_cp_index = usize::from(tri!(self.u16_at(bootstrap_method_offset)));
        if tri!(self.is_constant_dynamic(handle_cp_index)) {
            // ASM 9.11: "ConstantDynamic cannot be cast to Handle".
            return Err(invalid_constant(bootstrap_method_offset, handle_cp_index));
        }
        let Constant::Handle(handle) = tri!(self.const_at(handle_cp_index)) else {
            return Err(invalid_constant(bootstrap_method_offset, handle_cp_index));
        };
        let num_bootstrap_arguments = tri!(self.u16_at(bootstrap_method_offset + 2));
        let mut bootstrap_method_arguments = Vec::with_capacity(num_bootstrap_arguments.into());
        let mut tree_size = 1;
        bootstrap_method_offset += 4;
        for _ in 0..num_bootstrap_arguments {
            let argument_cp_index = usize::from(tri!(self.u16_at(bootstrap_method_offset)));
            let argument = if tri!(self.is_constant_dynamic(argument_cp_index)) {
                let (argument, size) =
                    tri!(self.read_constant_dynamic(argument_cp_index, depth_limit - 1));
                tree_size += size;
                if tree_size > MAX_CONSTANT_DYNAMIC_TREE_SIZE {
                    return Err(ReadError::new(
                        cp_info_offset,
                        ReadErrorKind::RecursionLimit,
                    ));
                }
                Constant::Dynamic(Box::new(argument))
            } else {
                tri!(self.const_at(argument_cp_index)).into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jstr;

    /// A class file prefix whose constant pool holds the given `CONSTANT_Utf8` entries.
    fn class_file(strings: &[&[u8]]) -> Vec<u8> {
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        bytes.extend_from_slice(&(strings.len() as u16 + 1).to_be_bytes());
        for string in strings {
            bytes.push(super::super::CONSTANT_UTF8_TAG);
            bytes.extend_from_slice(&(string.len() as u16).to_be_bytes());
            bytes.extend_from_slice(string);
        }
        // access_flags, which ASM's decoding of a truncated sequence reads.
        bytes.extend_from_slice(&[0, 0]);
        bytes
    }

    fn borrows_from(reader: &ClassReader<'_>, value: &JavaStr) -> bool {
        reader
            .class_file
            .as_ptr_range()
            .contains(&value.as_bytes().as_ptr())
    }

    const STRINGS: [&[u8]; 5] = [
        b"java/lang/Object",
        "caf\u{e9}".as_bytes(),
        b"",
        // Modified UTF-8's NUL, which is not UTF-8.
        &[b'a', 0xC0, 0x80],
        // A 4 bytes sequence, which ASM decodes as a 3 bytes one plus a continuation byte.
        "\u{1F600}".as_bytes(),
    ];

    #[test]
    fn utf_borrows_utf8_and_decodes_the_rest_once() {
        let bytes = class_file(&STRINGS);
        let reader = ClassReader::new(&bytes).unwrap();
        for index in 1..=STRINGS.len() {
            let first = reader.utf(index).unwrap();
            let second = reader.utf(index).unwrap();
            assert!(std::ptr::eq(first, second), "entry {index} is cached");
            assert_eq!(borrows_from(&reader, first), index <= 3, "entry {index}");
            let uncached = reader
                .read_utf(
                    reader.cp_info_offset(index).unwrap() + 2,
                    STRINGS[index - 1].len() as i32,
                    reader.max_string_length,
                )
                .unwrap();
            assert_eq!(first, &*uncached, "entry {index}");
        }
        assert_eq!(reader.utf(1).unwrap(), jstr!("java/lang/Object"));
        assert_eq!(reader.utf(2).unwrap(), jstr!("caf\u{e9}"));
        assert_eq!(reader.utf(3).unwrap(), jstr!(""));
        assert_eq!(reader.utf(4).unwrap(), jstr!("a\0"));
        assert_eq!(
            reader.utf(STRINGS.len() + 1).unwrap_err().kind,
            ReadErrorKind::InvalidConstantPoolIndex(STRINGS.len() as u16 + 1)
        );
    }

    #[test]
    fn cp_info_offset_is_kept_by_cached_strings() {
        let bytes = class_file(&STRINGS);
        let reader = ClassReader::new(&bytes).unwrap();
        let offsets: Vec<i64> = (0..=STRINGS.len())
            .map(|index| reader.cp_info_offset(index).unwrap())
            .collect();
        assert_eq!(offsets[..2], [0, 11]);
        for index in 1..=STRINGS.len() {
            reader.utf(index).unwrap();
        }
        for (index, &offset) in offsets.iter().enumerate() {
            assert_eq!(
                reader.cp_info_offset(index).unwrap(),
                offset,
                "entry {index}"
            );
            assert_eq!(
                reader.get_item(index).unwrap(),
                offset as usize,
                "entry {index}"
            );
        }
    }

    /// The constant pool entries of every tag but the dynamic ones (which need a
    /// `BootstrapMethods` attribute), as tags and sizes.
    const ENTRIES: [(u8, usize); 15] = [
        (super::super::CONSTANT_UTF8_TAG, 3 + 4),
        (super::super::CONSTANT_INTEGER_TAG, 5),
        (super::super::CONSTANT_FLOAT_TAG, 5),
        (super::super::CONSTANT_LONG_TAG, 9),
        (super::super::CONSTANT_DOUBLE_TAG, 9),
        (super::super::CONSTANT_CLASS_TAG, 3),
        (super::super::CONSTANT_STRING_TAG, 3),
        (super::super::CONSTANT_FIELDREF_TAG, 5),
        (super::super::CONSTANT_METHODREF_TAG, 5),
        (super::super::CONSTANT_INTERFACE_METHODREF_TAG, 5),
        (super::super::CONSTANT_NAME_AND_TYPE_TAG, 5),
        (super::super::CONSTANT_METHOD_HANDLE_TAG, 4),
        (super::super::CONSTANT_METHOD_TYPE_TAG, 3),
        (super::super::CONSTANT_MODULE_TAG, 3),
        (super::super::CONSTANT_PACKAGE_TAG, 3),
    ];

    /// A class file prefix with the given constant_pool_count and entries (each a tag, then
    /// zeros but for a `CONSTANT_Utf8`'s length of 4), and `tail` after them.
    fn class_file_with(count: u16, entries: &[(u8, usize)], tail: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        bytes.extend_from_slice(&count.to_be_bytes());
        for &(tag, size) in entries {
            let start = bytes.len();
            bytes.resize(start + size, 0);
            bytes[start] = tag;
            if tag == super::super::CONSTANT_UTF8_TAG {
                bytes[start + 2] = 4;
            }
        }
        bytes.extend_from_slice(tail);
        bytes
    }

    #[test]
    fn new_finds_every_entry() {
        // A long or double takes two indices, the second with offset 0.
        let count = ENTRIES.len() + 2 + 1;
        let bytes = class_file_with(count as u16, &ENTRIES, &[]);
        let reader = ClassReader::new(&bytes).unwrap();
        assert_eq!(reader.get_item_count(), count);
        let mut expected = vec![0];
        let mut offset = 10;
        for &(tag, size) in &ENTRIES {
            expected.push(offset + 1);
            if tag == super::super::CONSTANT_LONG_TAG || tag == super::super::CONSTANT_DOUBLE_TAG {
                expected.push(0);
            }
            offset += size;
        }
        let offsets: Vec<usize> = (0..count).map(|i| reader.get_item(i).unwrap()).collect();
        assert_eq!(offsets, expected);
        assert_eq!(reader.header(), offset);
        assert_eq!(reader.get_max_string_length(), 3 + 4);
        assert_eq!(
            reader.get_item(count).unwrap_err().kind,
            ReadErrorKind::InvalidConstantPoolIndex(count as u16)
        );
    }

    #[test]
    fn new_sizes_entries_by_their_tag_whatever_their_bytes() {
        // Every entry's bytes are 0xFF where another tag's CONSTANT_Utf8 length would be, and
        // the strings' lengths need both bytes.
        let lengths = [0, 1, 255, 256, 257, 4096, usize::from(u16::MAX)];
        let mut entries: Vec<(u8, usize)> = ENTRIES
            .iter()
            .copied()
            .filter(|&(tag, _)| tag != super::super::CONSTANT_UTF8_TAG)
            .collect();
        for (i, &length) in lengths.iter().enumerate() {
            entries.insert(2 * i, (super::super::CONSTANT_UTF8_TAG, 3 + length));
        }
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52, 0, 0];
        let mut expected = vec![0];
        let mut strings = Vec::new();
        for &(tag, size) in &entries {
            if tag == super::super::CONSTANT_UTF8_TAG {
                strings.push(expected.len());
            }
            expected.push(bytes.len() + 1);
            if tag == super::super::CONSTANT_LONG_TAG || tag == super::super::CONSTANT_DOUBLE_TAG {
                expected.push(0);
            }
            let start = bytes.len();
            bytes.resize(start + size, 0xFF);
            bytes[start] = tag;
            if tag == super::super::CONSTANT_UTF8_TAG {
                let length = (size - 3) as u16;
                bytes[start + 1..start + 3].copy_from_slice(&length.to_be_bytes());
                bytes[start + 3..].fill(b'a');
            }
        }
        let header = bytes.len();
        bytes[8..10].copy_from_slice(&(expected.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&[0; 8]);
        let reader = ClassReader::new(&bytes).unwrap();
        let offsets: Vec<usize> = (0..expected.len())
            .map(|i| reader.get_item(i).unwrap())
            .collect();
        assert_eq!(offsets, expected);
        assert_eq!(reader.header(), header);
        assert_eq!(reader.get_max_string_length(), 3 + usize::from(u16::MAX));
        for (&index, &length) in strings.iter().zip(&lengths) {
            assert_eq!(reader.utf(index).unwrap().len(), length, "entry {index}");
        }
    }

    #[test]
    fn new_with_a_long_as_the_last_entry_or_no_entry() {
        // The long's second index is past the constant pool.
        let long = (super::super::CONSTANT_LONG_TAG, 9);
        let bytes = class_file_with(2, &[long], &[]);
        let reader = ClassReader::new(&bytes).unwrap();
        assert_eq!(reader.get_item_count(), 2);
        assert_eq!(reader.get_item(1).unwrap(), 11);
        assert_eq!(reader.header(), 19);

        for count in [0, 1] {
            let bytes = class_file_with(count, &[], &[]);
            let reader = ClassReader::new(&bytes).unwrap();
            assert_eq!(reader.get_item_count(), usize::from(count));
            assert_eq!(reader.header(), 10);
        }
    }

    #[test]
    fn new_rejects_truncated_constant_pools_and_unknown_tags() {
        let utf8 = (super::super::CONSTANT_UTF8_TAG, 3 + 4);
        let class = (super::super::CONSTANT_CLASS_TAG, 3);
        let error = |bytes: &[u8]| {
            let error = ClassReader::new(bytes).unwrap_err();
            (error.offset, error.kind.clone())
        };
        // No tag for the third entry.
        let bytes = class_file_with(4, &[utf8, class], &[]);
        assert_eq!(error(&bytes), (20, ReadErrorKind::UnexpectedEnd));
        // A CONSTANT_Utf8 tag without its length, or with only one byte of it.
        for tail in [&[][..], &[0]] {
            let mut bytes = class_file_with(3, &[class], &[super::super::CONSTANT_UTF8_TAG]);
            bytes.extend_from_slice(tail);
            assert_eq!(error(&bytes), (14, ReadErrorKind::UnexpectedEnd));
        }
        // Another entry cut short is the business of what reads it, as in ASM.
        for (tag, size) in [
            (super::super::CONSTANT_CLASS_TAG, 3),
            (super::super::CONSTANT_LONG_TAG, 9),
        ] {
            for tail in [&[tag][..], &[tag, 0]] {
                let bytes = class_file_with(2, &[], tail);
                let reader = ClassReader::new(&bytes).unwrap();
                assert_eq!(reader.get_item(1).unwrap(), 11);
                assert_eq!(reader.header(), 10 + size);
            }
        }
        for tag in [0, 2, 13, 14, 21, 255] {
            let bytes = class_file_with(3, &[class, (tag, 3)], &[]);
            assert_eq!(
                error(&bytes),
                (13, ReadErrorKind::InvalidConstantPoolTag(tag))
            );
        }
    }

    #[test]
    fn utf_is_decoded_consistently_by_racing_threads() {
        let bytes = class_file(&STRINGS);
        let reader = ClassReader::new(&bytes).unwrap();
        let results: Vec<Vec<&JavaStr>> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..4)
                .map(|thread| {
                    let reader = &reader;
                    scope.spawn(move || {
                        // Each thread visits the entries in a different order.
                        (0..STRINGS.len())
                            .map(|i| (i + thread) % STRINGS.len() + 1)
                            .map(|index| (index, reader.utf(index).unwrap()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| {
                    let mut values = thread.join().unwrap();
                    values.sort_by_key(|&(index, _)| index);
                    values.into_iter().map(|(_, value)| value).collect()
                })
                .collect()
        });
        // Every thread got the cached value, borrowed or decoded.
        for values in &results {
            for (i, value) in values.iter().enumerate() {
                assert!(
                    std::ptr::eq(*value, reader.utf(i + 1).unwrap()),
                    "entry {}",
                    i + 1
                );
            }
        }
    }
}
