//! [`ByteVector`]: ASM's `ByteVector`, a growable byte buffer with big endian writers.

use crate::error::WriteError;
use crate::jstr::{JavaStr, has_nul_or_4_byte_lead};

/// A dynamically extensible vector of bytes. This class is roughly equivalent to a
/// `DataOutputStream` on top of a `ByteArrayOutputStream`, but is more efficient.
///
/// The writers take Java `int`s and keep their low order bytes, as ASM's do: `put_short(-1)`
/// writes `FF FF`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ByteVector {
    /// The content of this vector.
    data: Vec<u8>,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Java's narrowing casts
impl ByteVector {
    /// Constructs a new byte vector with a default initial capacity.
    #[must_use]
    pub fn new() -> ByteVector {
        ByteVector {
            data: Vec::with_capacity(64),
        }
    }

    /// Constructs a new byte vector with the given initial capacity.
    #[must_use]
    pub fn with_capacity(initial_capacity: usize) -> ByteVector {
        ByteVector {
            data: Vec::with_capacity(initial_capacity),
        }
    }

    /// Constructs a new byte vector from the given initial data.
    #[must_use]
    pub fn from_vec(data: Vec<u8>) -> ByteVector {
        ByteVector { data }
    }

    /// Returns the actual number of bytes in this vector.
    #[inline]
    #[must_use]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// The content of this vector.
    #[inline]
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The content of this vector, mutably (to patch already written values).
    #[inline]
    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// The content of this vector.
    #[inline]
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Reserves room for at least `total` bytes in this vector, and no more (a size known in
    /// advance).
    #[inline]
    pub(crate) fn reserve_total(&mut self, total: usize) {
        self.data
            .reserve_exact(total.saturating_sub(self.data.len()));
    }

    /// Truncates this vector to the given length (ASM sets `length` directly).
    #[inline]
    pub(crate) fn truncate(&mut self, length: usize) {
        self.data.truncate(length);
    }

    /// Puts a byte into this byte vector.
    #[inline]
    pub fn put_byte(&mut self, byte_value: i32) -> &mut ByteVector {
        self.data.push(byte_value as u8);
        self
    }

    /// Puts two bytes into this byte vector.
    #[inline]
    pub(crate) fn put11(&mut self, byte_value1: i32, byte_value2: i32) -> &mut ByteVector {
        self.data
            .extend_from_slice(&[byte_value1 as u8, byte_value2 as u8]);
        self
    }

    /// Puts a short into this byte vector.
    #[inline]
    pub fn put_short(&mut self, short_value: i32) -> &mut ByteVector {
        self.data
            .extend_from_slice(&[(short_value >> 8) as u8, short_value as u8]);
        self
    }

    /// Puts a byte and a short into this byte vector.
    #[inline]
    pub(crate) fn put12(&mut self, byte_value: i32, short_value: i32) -> &mut ByteVector {
        self.data.extend_from_slice(&[
            byte_value as u8,
            (short_value >> 8) as u8,
            short_value as u8,
        ]);
        self
    }

    /// Puts two bytes and a short into this byte vector.
    #[inline]
    pub(crate) fn put112(
        &mut self,
        byte_value1: i32,
        byte_value2: i32,
        short_value: i32,
    ) -> &mut ByteVector {
        self.data.extend_from_slice(&[
            byte_value1 as u8,
            byte_value2 as u8,
            (short_value >> 8) as u8,
            short_value as u8,
        ]);
        self
    }

    /// Puts an int into this byte vector.
    #[inline]
    pub fn put_int(&mut self, int_value: i32) -> &mut ByteVector {
        self.data.extend_from_slice(&int_value.to_be_bytes());
        self
    }

    /// Puts one byte and two shorts into this byte vector.
    #[inline]
    pub(crate) fn put122(
        &mut self,
        byte_value: i32,
        short_value1: i32,
        short_value2: i32,
    ) -> &mut ByteVector {
        self.data.extend_from_slice(&[
            byte_value as u8,
            (short_value1 >> 8) as u8,
            short_value1 as u8,
            (short_value2 >> 8) as u8,
            short_value2 as u8,
        ]);
        self
    }

    /// Puts a long into this byte vector.
    #[inline]
    pub fn put_long(&mut self, long_value: i64) -> &mut ByteVector {
        self.data.extend_from_slice(&long_value.to_be_bytes());
        self
    }

    /// Puts a string into this byte vector, in modified UTF-8, preceded by its encoded length
    /// (a `CONSTANT_Utf8_info`'s `length` and `bytes`).
    ///
    /// # Errors
    ///
    /// If the string has more than 65535 UTF-16 units, or if its encoding has more than 65535
    /// bytes (ASM's `IllegalArgumentException("UTF8 string too large")`). Nothing is written
    /// then.
    pub fn put_utf8(&mut self, string_value: &JavaStr) -> Result<&mut ByteVector, WriteError> {
        // ASM checks the UTF-16 length first, then the encoded length; the encoded length is at
        // least the UTF-16 length, so the second check covers the first.
        let bytes = string_value.as_bytes();
        if !has_nul_or_4_byte_lead(bytes) {
            // The encoding is the bytes themselves.
            if bytes.len() > 65535 {
                return Err(WriteError::Utf8TooLarge);
            }
            self.data.reserve(2 + bytes.len());
            self.put_short(bytes.len() as i32);
            self.data.extend_from_slice(bytes);
            return Ok(self);
        }
        let encoded = string_value.to_modified_utf8();
        if encoded.len() > 65535 {
            return Err(WriteError::Utf8TooLarge);
        }
        self.put_short(encoded.len() as i32);
        self.data.extend_from_slice(&encoded);
        Ok(self)
    }

    /// Puts a string into this byte vector, in modified UTF-8, without its length (ASM's
    /// `encodeUtf8(stringValue, 0, maxByteLength)` on an empty vector, as used for the
    /// `SourceDebugExtension` attribute).
    pub(crate) fn put_modified_utf8_bytes(&mut self, string_value: &JavaStr) -> &mut ByteVector {
        self.data
            .extend_from_slice(&string_value.to_modified_utf8());
        self
    }

    /// Puts an array of bytes into this byte vector.
    #[inline]
    pub fn put_byte_array(&mut self, byte_array_value: &[u8]) -> &mut ByteVector {
        self.data.extend_from_slice(byte_array_value);
        self
    }

    /// Puts `byte_length` zero bytes into this byte vector (ASM's `putByteArray(null, 0,
    /// byteLength)`).
    #[inline]
    pub(crate) fn put_zeros(&mut self, byte_length: usize) -> &mut ByteVector {
        self.data.resize(self.data.len() + byte_length, 0);
        self
    }
}

/// The tests of ASM's `ByteVectorTest` for its package-private methods (the others are in
/// `tests/byte_vector_test.rs`).
#[cfg(test)]
mod tests {
    use super::ByteVector;

    #[test]
    fn test_put11() {
        let mut byte_vector = ByteVector::with_capacity(0);
        byte_vector.put11(1, 2);
        assert_eq!(byte_vector.data(), [1, 2]);
        assert_eq!(byte_vector.size(), 2);
    }

    #[test]
    fn test_put12() {
        let mut byte_vector = ByteVector::with_capacity(0);
        byte_vector.put12(1, 0x0203);
        assert_eq!(byte_vector.data(), [1, 2, 3]);
        assert_eq!(byte_vector.size(), 3);
    }

    #[test]
    fn test_put112() {
        let mut byte_vector = ByteVector::with_capacity(0);
        byte_vector.put112(1, 2, 0x0304);
        assert_eq!(byte_vector.data(), [1, 2, 3, 4]);
        assert_eq!(byte_vector.size(), 4);
    }

    #[test]
    fn test_put122() {
        let mut byte_vector = ByteVector::with_capacity(0);
        byte_vector.put122(1, 0x0203, 0x0405);
        assert_eq!(byte_vector.data(), [1, 2, 3, 4, 5]);
        assert_eq!(byte_vector.size(), 5);
    }
}
