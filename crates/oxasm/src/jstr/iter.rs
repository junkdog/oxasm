//! Iterators over a [`JavaStr`](super::JavaStr).

use std::iter::FusedIterator;

use super::JavaCodePoint;

/// Decodes the code point starting at the front of `bytes` (which must be non-empty canonical
/// semi-UTF-8) and returns it with its width.
#[inline]
fn decode_front(bytes: &[u8]) -> (u32, usize) {
    let b0 = u32::from(bytes[0]);
    match bytes[0] {
        0..0x80 => (b0, 1),
        0xc0..0xe0 => (((b0 & 0x1f) << 6) | (u32::from(bytes[1]) & 0x3f), 2),
        0xe0..0xf0 => (
            ((b0 & 0x0f) << 12)
                | ((u32::from(bytes[1]) & 0x3f) << 6)
                | (u32::from(bytes[2]) & 0x3f),
            3,
        ),
        _ => (
            ((b0 & 0x07) << 18)
                | ((u32::from(bytes[1]) & 0x3f) << 12)
                | ((u32::from(bytes[2]) & 0x3f) << 6)
                | (u32::from(bytes[3]) & 0x3f),
            4,
        ),
    }
}

/// The width of the code point ending at the back of `bytes`.
#[inline]
fn width_back(bytes: &[u8]) -> usize {
    let mut w = 1;
    while bytes[bytes.len() - w] & 0xc0 == 0x80 {
        w += 1;
    }
    w
}

/// The code points of a `JavaStr` ([`JavaStr::chars`](super::JavaStr::chars)).
#[derive(Clone, Debug)]
pub struct Chars<'a> {
    bytes: &'a [u8],
}

impl<'a> Chars<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Chars { bytes }
    }

    /// The rest of the string.
    #[must_use]
    pub fn as_java_str(&self) -> &'a super::JavaStr {
        // SAFETY: the iterator only ever advances by whole code points.
        unsafe { super::JavaStr::from_semi_utf8_unchecked(self.bytes) }
    }
}

impl Iterator for Chars<'_> {
    type Item = JavaCodePoint;

    #[inline]
    fn next(&mut self) -> Option<JavaCodePoint> {
        if self.bytes.is_empty() {
            return None;
        }
        let (c, w) = decode_front(self.bytes);
        self.bytes = &self.bytes[w..];
        Some(JavaCodePoint(c))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.bytes.len().div_ceil(4), Some(self.bytes.len()))
    }
}

impl DoubleEndedIterator for Chars<'_> {
    fn next_back(&mut self) -> Option<JavaCodePoint> {
        if self.bytes.is_empty() {
            return None;
        }
        let w = width_back(self.bytes);
        let (rest, last) = self.bytes.split_at(self.bytes.len() - w);
        self.bytes = rest;
        Some(JavaCodePoint(decode_front(last).0))
    }
}

impl FusedIterator for Chars<'_> {}

/// The code points of a `JavaStr` with their byte offsets
/// ([`JavaStr::char_indices`](super::JavaStr::char_indices)).
#[derive(Clone, Debug)]
pub struct CharIndices<'a> {
    offset: usize,
    chars: Chars<'a>,
}

impl<'a> CharIndices<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        CharIndices {
            offset: 0,
            chars: Chars::new(bytes),
        }
    }
}

impl Iterator for CharIndices<'_> {
    type Item = (usize, JavaCodePoint);

    #[inline]
    fn next(&mut self) -> Option<(usize, JavaCodePoint)> {
        let before = self.chars.bytes.len();
        let c = self.chars.next()?;
        let index = self.offset;
        self.offset += before - self.chars.bytes.len();
        Some((index, c))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chars.size_hint()
    }
}

impl DoubleEndedIterator for CharIndices<'_> {
    fn next_back(&mut self) -> Option<(usize, JavaCodePoint)> {
        let c = self.chars.next_back()?;
        Some((self.offset + self.chars.bytes.len(), c))
    }
}

impl FusedIterator for CharIndices<'_> {}

/// The UTF-16 code units of a `JavaStr` ([`JavaStr::encode_utf16`](super::JavaStr::encode_utf16)).
#[derive(Clone, Debug)]
pub struct EncodeUtf16<'a> {
    chars: Chars<'a>,
    pending_low: Option<u16>,
}

impl<'a> EncodeUtf16<'a> {
    pub(super) fn new(chars: Chars<'a>) -> Self {
        EncodeUtf16 {
            chars,
            pending_low: None,
        }
    }
}

impl Iterator for EncodeUtf16<'_> {
    type Item = u16;

    #[inline]
    #[allow(clippy::cast_possible_truncation)] // every value is below 0x10000 by construction
    fn next(&mut self) -> Option<u16> {
        if let Some(low) = self.pending_low.take() {
            return Some(low);
        }
        let c = self.chars.next()?.as_u32();
        if c >= 0x10000 {
            let c = c - 0x10000;
            self.pending_low = Some(0xdc00 | (c & 0x3ff) as u16);
            Some(0xd800 | (c >> 10) as u16)
        } else {
            Some(c as u16)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.chars.bytes.len();
        let pending = usize::from(self.pending_low.is_some());
        (n.div_ceil(3) + pending, Some(n + pending))
    }
}

impl FusedIterator for EncodeUtf16<'_> {}
