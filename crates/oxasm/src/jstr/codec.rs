//! The class file codec (modified UTF-8) and UTF-16 conversion.

use std::borrow::Cow;

use super::{JavaCodePoint, JavaStr, JavaString, Utf8Error, decode_3, encode_semi_utf8};

impl JavaStr {
    /// Decodes Java's [modified UTF-8], the class file encoding of `CONSTANT_Utf8` entries.
    ///
    /// Borrows when the bytes are already canonical semi-UTF-8, which is the case for any string
    /// without NUL and without supplementary characters (almost every name and descriptor).
    /// Otherwise `C0 80` becomes NUL and each 6-byte surrogate pair becomes one 4-byte code point.
    ///
    /// # Errors
    ///
    /// If the bytes are not modified UTF-8: a raw NUL byte, a 4-byte sequence, a malformed or
    /// truncated sequence.
    ///
    /// [modified UTF-8]: https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.7
    pub fn from_modified_utf8(bytes: &[u8]) -> Result<Cow<'_, JavaStr>, Utf8Error> {
        if !has_nul_or_4_byte_lead(bytes)
            && let Ok(s) = std::str::from_utf8(bytes)
        {
            return Ok(Cow::Borrowed(JavaStr::from_str(s)));
        }
        decode_modified_utf8(bytes).map(Cow::Owned)
    }

    /// Encodes as Java's modified UTF-8. Borrows when the encoding equals the semi-UTF-8 bytes,
    /// i.e. when the string has neither NUL nor supplementary characters.
    #[must_use]
    pub fn to_modified_utf8(&self) -> Cow<'_, [u8]> {
        let bytes = self.as_bytes();
        if !has_nul_or_4_byte_lead(bytes) {
            return Cow::Borrowed(bytes);
        }
        let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 2);
        for c in self.chars() {
            match c.as_u32() {
                0 => out.extend_from_slice(&[0xc0, 0x80]),
                c @ 0x10000.. => {
                    let c = c - 0x10000;
                    encode_semi_utf8(0xd800 | (c >> 10), &mut out);
                    encode_semi_utf8(0xdc00 | (c & 0x3ff), &mut out);
                }
                c => encode_semi_utf8(c, &mut out),
            }
        }
        Cow::Owned(out)
    }

    /// The length of the modified UTF-8 encoding, in bytes (what a `CONSTANT_Utf8` entry's
    /// `length` field holds).
    #[must_use]
    pub fn modified_utf8_len(&self) -> usize {
        self.bytes().fold(0, |n, b| match b {
            0 => n + 2,
            0xf0.. => n + 6,
            0x80..=0xbf => n,
            0xc0..=0xdf => n + 2,
            0xe0..=0xef => n + 3,
            _ => n + 1,
        })
    }
}

impl JavaString {
    /// Decodes Java's modified UTF-8 into an owned string (see
    /// [`JavaStr::from_modified_utf8`]).
    ///
    /// # Errors
    ///
    /// If the bytes are not modified UTF-8.
    pub fn from_modified_utf8(bytes: &[u8]) -> Result<JavaString, Utf8Error> {
        JavaStr::from_modified_utf8(bytes).map(Cow::into_owned)
    }

    /// Builds a string from UTF-16 code units without loss: valid surrogate pairs become
    /// supplementary code points, and unpaired surrogates are kept as they are. This is the
    /// inverse of [`JavaStr::encode_utf16`].
    #[must_use]
    pub fn from_utf16(units: &[u16]) -> JavaString {
        let mut out = Vec::with_capacity(units.len());
        let mut i = 0;
        while i < units.len() {
            let u = u32::from(units[i]);
            i += 1;
            if (0xd800..0xdc00).contains(&u)
                && let Some(&low) = units.get(i)
                && (0xdc00..0xe000).contains(&low)
            {
                i += 1;
                encode_semi_utf8(
                    0x10000 + ((u - 0xd800) << 10) + (u32::from(low) - 0xdc00),
                    &mut out,
                );
            } else {
                encode_semi_utf8(u, &mut out);
            }
        }
        // SAFETY: every pair was joined, so no high surrogate is followed by a low one.
        unsafe { JavaString::from_semi_utf8_unchecked(out) }
    }
}

impl FromIterator<u16> for JavaString {
    /// Collects UTF-16 code units, as [`JavaString::from_utf16`].
    fn from_iter<I: IntoIterator<Item = u16>>(iter: I) -> Self {
        let mut s = JavaString::new();
        for u in iter {
            // `push_java` joins a low surrogate onto a trailing high one.
            s.push_java(JavaCodePoint(u32::from(u)));
        }
        s
    }
}

/// Whether the bytes hold a NUL or a byte of `F0` and above (which starts a 4-byte sequence in
/// semi-UTF-8, and is invalid in modified UTF-8): the only bytes on which a string's semi-UTF-8
/// and modified UTF-8 encodings differ. Checks a word at a time, as most class file strings are
/// only a few words long.
#[inline]
pub(crate) fn has_nul_or_4_byte_lead(bytes: &[u8]) -> bool {
    /// Whether a byte of `w` is NUL or `F0..=FF`.
    #[inline(always)]
    fn word(w: u64) -> bool {
        const LO: u64 = 0x0101_0101_0101_0101;
        const HI: u64 = 0x8080_8080_8080_8080;
        // A byte is F0..=FF exactly when the high nibble of its complement is 0.
        let x = !w & 0xf0f0_f0f0_f0f0_f0f0;
        // `(v - LO) & !v & HI` is non-zero exactly when `v` has a zero byte.
        (w.wrapping_sub(LO) & !w | x.wrapping_sub(LO) & !x) & HI != 0
    }
    if let Some(last) = bytes.last_chunk::<8>() {
        let (words, rest) = bytes.as_chunks::<8>();
        // The rest, if any, is checked with the last 8 bytes, which overlap the last word.
        words.iter().any(|w| word(u64::from_le_bytes(*w)))
            || (!rest.is_empty() && word(u64::from_le_bytes(*last)))
    } else if let (Some(first), Some(last)) = (bytes.first_chunk::<4>(), bytes.last_chunk::<4>()) {
        word(u64::from(u32::from_le_bytes(*first)) | u64::from(u32::from_le_bytes(*last)) << 32)
    } else {
        bytes.iter().any(|&b| b == 0 || b >= 0xf0)
    }
}

/// The strict modified UTF-8 decoder, after `java_string`'s `from_modified_utf8_internal`.
fn decode_modified_utf8(slice: &[u8]) -> Result<JavaString, Utf8Error> {
    let mut decoded = Vec::with_capacity(slice.len());
    let mut offset = 0;
    while let Some(&first) = slice.get(offset) {
        let start = offset;
        let err = |len: Option<u8>| {
            Err(Utf8Error {
                valid_up_to: start,
                error_len: len,
            })
        };
        let cont = |k: usize| -> Result<u8, Option<u8>> {
            match slice.get(start + k) {
                None => Err(None),
                #[allow(clippy::cast_possible_truncation)] // k <= 2
                Some(&b) if b & 0xc0 != 0x80 => Err(Some(k as u8)),
                Some(&b) => Ok(b),
            }
        };
        match first {
            // Modified UTF-8 never contains a raw NUL.
            0 => return err(Some(1)),
            0x01..=0x7f => {
                decoded.push(first);
                offset += 1;
            }
            // The 2-byte encoding of NUL.
            0xc0 => match slice.get(offset + 1) {
                Some(0x80) => {
                    decoded.push(0);
                    offset += 2;
                }
                None => return err(None),
                Some(_) => return err(Some(1)),
            },
            0xc2..=0xdf => {
                let second = match cont(1) {
                    Ok(b) => b,
                    Err(len) => return err(len),
                };
                decoded.extend_from_slice(&[first, second]);
                offset += 2;
            }
            0xe0..=0xef => {
                let (second, third) = match (cont(1), cont(2)) {
                    (Ok(b1), Ok(b2)) => (b1, b2),
                    (Err(len), _) | (_, Err(len)) => return err(len),
                };
                match (first, second) {
                    (0xe0, 0xa0..=0xbf)
                    | (0xe1..=0xec | 0xee..=0xef, 0x80..=0xbf)
                    | (0xed, 0x80..=0x9f)
                    // A low surrogate without a preceding high one: kept as is.
                    | (0xed, 0xb0..=0xbf) => decoded.extend_from_slice(&[first, second, third]),
                    // A high surrogate: join it with a following low surrogate, if any.
                    (0xed, 0xa0..=0xaf) => match &slice[offset + 3..] {
                        [0xed, fifth @ 0xb0..=0xbf, sixth, ..] if sixth & 0xc0 == 0x80 => {
                            let high = decode_3(first, second, third);
                            let low = decode_3(0xed, *fifth, *sixth);
                            let c = 0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00);
                            encode_semi_utf8(c, &mut decoded);
                            offset += 3;
                        }
                        _ => decoded.extend_from_slice(&[first, second, third]),
                    },
                    _ => return err(Some(1)),
                }
                offset += 3;
            }
            // Overlong 2-byte forms other than C0 80, continuation bytes, and 4-byte sequences
            // (supplementary characters are surrogate pairs in modified UTF-8).
            _ => return err(Some(1)),
        }
    }
    // SAFETY: built from valid UTF-8 sequences and 3-byte surrogates, joining every pair.
    Ok(unsafe { JavaString::from_semi_utf8_unchecked(decoded) })
}
