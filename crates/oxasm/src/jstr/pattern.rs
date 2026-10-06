//! Search patterns for [`JavaStr`](super::JavaStr) methods.
//!
//! Matching is bytewise on the semi-UTF-8 encoding. Encodings are self-synchronizing, so a match
//! always starts and ends on code point boundaries. One consequence differs from Java: a lone
//! surrogate pattern never matches half of a surrogate *pair*, which `String.indexOf` would.

use super::{JavaCodePoint, JavaStr, JavaString, encode_semi_utf8};

/// Something [`JavaStr::find`] and friends can search for: a `char`, a [`JavaCodePoint`], or a
/// string (`&str`, `&JavaStr`, ...).
pub trait Pattern {
    /// Calls `f` with the encoded bytes of the pattern.
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R;
}

impl Pattern for char {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.encode_utf8(&mut [0; 4]).as_bytes())
    }
}

impl Pattern for JavaCodePoint {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let mut buf = Vec::with_capacity(4);
        encode_semi_utf8(self.as_u32(), &mut buf);
        f(&buf)
    }
}

impl Pattern for &str {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

impl Pattern for &String {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

impl Pattern for &JavaStr {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

impl Pattern for &JavaString {
    fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        f(self.as_bytes())
    }
}

pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    match needle {
        [] => Some(0),
        [b] => haystack.iter().position(|x| x == b),
        _ => haystack.windows(needle.len()).position(|w| w == needle),
    }
}

pub(super) fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    match needle {
        [] => Some(haystack.len()),
        [b] => haystack.iter().rposition(|x| x == b),
        _ => haystack.windows(needle.len()).rposition(|w| w == needle),
    }
}
