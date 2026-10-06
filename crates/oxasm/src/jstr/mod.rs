//! Java strings: [`JavaStr`], [`JavaString`] and [`JavaArcStr`], which hold any sequence of UTF-16 code units,
//! including unpaired surrogates (ADR-04).
//!
//! Class files store strings as *modified UTF-8*, and Java strings may contain lone surrogates,
//! which Rust's `str` cannot represent. Obfuscated jars use them in names, and string constants
//! can carry arbitrary UTF-16. These types keep such strings intact, so a class can be read and
//! written back without loss.
//!
//! # Encoding
//!
//! The bytes are *semi-UTF-8* in its canonical (WTF-8) form: UTF-8, where a surrogate code point
//! that is not half of a pair is encoded as a 3-byte sequence (`ED A0 80`..`ED BF BF`), and a
//! surrogate *pair* is always encoded as the 4-byte UTF-8 sequence of its supplementary code
//! point. Every sequence of UTF-16 code units therefore has exactly one encoding, so bytewise
//! `Eq` and `Hash` agree with Java's `String.equals`, and any valid `str` is a valid `JavaStr`
//! with the same bytes (`&str → &JavaStr` is free).
//!
//! The class file codec is [`JavaStr::from_modified_utf8`] / [`JavaStr::to_modified_utf8`];
//! UTF-16 conversions are [`JavaString::from_utf16`] and [`JavaStr::encode_utf16`].
//!
//! # Provenance
//!
//! Ported from the subset of the [`java_string`](https://crates.io/crates/java_string) crate
//! (valence-rs, MIT license, Copyright (c) 2022 Ryan Johnson; see `NOTICE`) that oxasm needs. The
//! one semantic difference is that `java_string` admits a surrogate pair written as two 3-byte
//! sequences, while `jstr` requires the canonical 4-byte form, which is what makes bytewise
//! equality sound.

mod arc;
mod codec;
mod iter;
mod pattern;

use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut, Index, Range, RangeFrom, RangeFull, RangeTo};
use std::rc::Rc;
use std::sync::Arc;

pub use arc::JavaArcStr;
pub use iter::{CharIndices, Chars, EncodeUtf16};
pub use pattern::Pattern;
use pattern::{find_bytes, rfind_bytes};

/// A borrowed Java string: canonical semi-UTF-8 (see the [module docs](self)).
///
/// This is to [`JavaString`] what `str` is to `String`.
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaStr {
    inner: [u8],
}

/// An owned Java string: canonical semi-UTF-8 (see the [module docs](self)).
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct JavaString {
    inner: Vec<u8>,
}

/// A Unicode code point, or a surrogate code point (`U+D800..=U+DFFF`): one element of a Java
/// string's code point sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaCodePoint(u32);

/// The error returned when bytes are not valid (canonical) semi-UTF-8 or modified UTF-8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utf8Error {
    valid_up_to: usize,
    error_len: Option<u8>,
}

/// Creates a `&'static JavaStr` from a string literal; usable in `const` items.
///
/// ```
/// use oxasm::jstr::JavaStr;
/// const OBJECT: &JavaStr = oxasm::jstr!("java/lang/Object");
/// assert_eq!(OBJECT, "java/lang/Object");
/// ```
#[macro_export]
macro_rules! jstr {
    ($s:expr) => {
        $crate::jstr::JavaStr::from_str($s)
    };
}

// -------------------------------------------------------------------------------------------------
// The two DST casts (ADR-18: the only unsafe in this module)
// -------------------------------------------------------------------------------------------------

impl JavaStr {
    /// Reinterprets bytes as a `JavaStr` without checking them.
    ///
    /// # Safety
    ///
    /// `bytes` must be canonical semi-UTF-8 (see the [module docs](self)).
    #[inline]
    #[must_use]
    pub const unsafe fn from_semi_utf8_unchecked(bytes: &[u8]) -> &JavaStr {
        // SAFETY: `JavaStr` is `repr(transparent)` over `[u8]`, so the two unsized types have the
        // same layout and pointer metadata; the caller guarantees the content invariant.
        unsafe { &*(bytes as *const [u8] as *const JavaStr) }
    }

    /// Reinterprets mutable bytes as a `JavaStr` without checking them.
    ///
    /// # Safety
    ///
    /// `bytes` must be canonical semi-UTF-8, and stay so when the borrow ends.
    #[inline]
    #[must_use]
    pub const unsafe fn from_semi_utf8_unchecked_mut(bytes: &mut [u8]) -> &mut JavaStr {
        // SAFETY: as in `from_semi_utf8_unchecked`; the caller keeps the content invariant.
        unsafe { &mut *(bytes as *mut [u8] as *mut JavaStr) }
    }
}

// -------------------------------------------------------------------------------------------------
// JavaStr
// -------------------------------------------------------------------------------------------------

impl JavaStr {
    /// Views a `str` as a `JavaStr`. Free: every `str` is valid canonical semi-UTF-8.
    #[inline]
    #[must_use]
    #[allow(clippy::should_implement_trait)] // borrows; `FromStr` would have to allocate
    pub const fn from_str(s: &str) -> &JavaStr {
        // SAFETY: valid UTF-8 contains no surrogates, so it is canonical semi-UTF-8.
        unsafe { JavaStr::from_semi_utf8_unchecked(s.as_bytes()) }
    }

    /// Validates and views bytes as a `JavaStr`.
    ///
    /// # Errors
    ///
    /// If `bytes` is not canonical semi-UTF-8 (this includes a surrogate pair written as two
    /// 3-byte sequences).
    pub fn from_semi_utf8(bytes: &[u8]) -> Result<&JavaStr, Utf8Error> {
        validate_semi_utf8(bytes)?;
        // SAFETY: just validated.
        Ok(unsafe { JavaStr::from_semi_utf8_unchecked(bytes) })
    }

    /// The semi-UTF-8 bytes of this string.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.inner
    }

    /// The length in bytes (of the semi-UTF-8 encoding).
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the string is empty.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether every character is ASCII.
    #[inline]
    #[must_use]
    pub const fn is_ascii(&self) -> bool {
        self.inner.is_ascii()
    }

    /// Views this string as a `str`, if it contains no unpaired surrogate.
    ///
    /// # Errors
    ///
    /// If the string contains an unpaired surrogate; the error points at the first one.
    #[inline]
    pub fn as_str(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(&self.inner).map_err(Utf8Error::from)
    }

    /// Converts to a `str`, replacing every unpaired surrogate with `U+FFFD`.
    #[must_use]
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        match self.as_str() {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => {
                let mut out = String::with_capacity(self.len());
                for segment in self.segments() {
                    match segment {
                        Segment::Str(s) => out.push_str(s),
                        Segment::Surrogate(_) => out.push(char::REPLACEMENT_CHARACTER),
                    }
                }
                Cow::Owned(out)
            }
        }
    }

    /// The bytes of the semi-UTF-8 encoding.
    #[inline]
    pub fn bytes(&self) -> std::iter::Copied<std::slice::Iter<'_, u8>> {
        self.inner.iter().copied()
    }

    /// The code points of this string; unpaired surrogates are yielded as themselves.
    #[inline]
    pub fn chars(&self) -> Chars<'_> {
        Chars::new(&self.inner)
    }

    /// The code points of this string with their byte offsets.
    #[inline]
    pub fn char_indices(&self) -> CharIndices<'_> {
        CharIndices::new(&self.inner)
    }

    /// The UTF-16 code units of this string: Java's `String.charAt` sequence.
    #[inline]
    pub fn encode_utf16(&self) -> EncodeUtf16<'_> {
        EncodeUtf16::new(self.chars())
    }

    /// The number of UTF-16 code units: Java's `String.length()`.
    #[must_use]
    pub fn len_utf16(&self) -> usize {
        // Every non-continuation byte starts a code point; a 4-byte one is two UTF-16 units.
        self.inner.iter().fold(0, |n, &b| match b {
            0x80..=0xbf => n,
            0xf0..=0xff => n + 2,
            _ => n + 1,
        })
    }

    /// Whether `index` is on a code point boundary (or at either end).
    #[inline]
    #[must_use]
    pub fn is_char_boundary(&self, index: usize) -> bool {
        match self.inner.get(index) {
            None => index == self.len(),
            Some(&b) => !is_continuation_byte(b),
        }
    }

    /// The substring for a byte range, or `None` if it is out of bounds or not on code point
    /// boundaries.
    #[must_use]
    pub fn get(&self, range: Range<usize>) -> Option<&JavaStr> {
        if range.start <= range.end
            && range.end <= self.len()
            && self.is_char_boundary(range.start)
            && self.is_char_boundary(range.end)
        {
            // SAFETY: a slice of canonical semi-UTF-8 on code point boundaries is canonical
            // semi-UTF-8: dropping a prefix or suffix cannot create a new high-low surrogate pair.
            Some(unsafe { JavaStr::from_semi_utf8_unchecked(&self.inner[range]) })
        } else {
            None
        }
    }

    /// Splits at a byte index.
    ///
    /// # Panics
    ///
    /// If `mid` is out of bounds or not on a code point boundary.
    #[must_use]
    #[track_caller]
    pub fn split_at(&self, mid: usize) -> (&JavaStr, &JavaStr) {
        (&self[..mid], &self[mid..])
    }

    /// Whether the string starts with `pattern`.
    #[inline]
    pub fn starts_with<P: Pattern>(&self, pattern: P) -> bool {
        pattern.with_bytes(|p| self.inner.starts_with(p))
    }

    /// Whether the string ends with `pattern`.
    #[inline]
    pub fn ends_with<P: Pattern>(&self, pattern: P) -> bool {
        pattern.with_bytes(|p| self.inner.ends_with(p))
    }

    /// Whether the string contains `pattern`.
    #[inline]
    pub fn contains<P: Pattern>(&self, pattern: P) -> bool {
        self.find(pattern).is_some()
    }

    /// The byte index of the first match of `pattern`.
    pub fn find<P: Pattern>(&self, pattern: P) -> Option<usize> {
        pattern.with_bytes(|p| find_bytes(&self.inner, p))
    }

    /// The byte index of the last match of `pattern`.
    pub fn rfind<P: Pattern>(&self, pattern: P) -> Option<usize> {
        pattern.with_bytes(|p| rfind_bytes(&self.inner, p))
    }

    /// The rest of the string after `prefix`, if it starts with it.
    pub fn strip_prefix<P: Pattern>(&self, prefix: P) -> Option<&JavaStr> {
        let n = prefix.with_bytes(|p| self.inner.starts_with(p).then_some(p.len()))?;
        Some(&self[n..])
    }

    /// The string before `suffix`, if it ends with it.
    pub fn strip_suffix<P: Pattern>(&self, suffix: P) -> Option<&JavaStr> {
        let n = suffix.with_bytes(|p| self.inner.ends_with(p).then_some(p.len()))?;
        Some(&self[..self.len() - n])
    }

    /// Replaces every occurrence of `from` with `to` (Java's `String.replace(char, char)`).
    #[must_use]
    pub fn replace(&self, from: char, to: char) -> JavaString {
        if from.is_ascii() && to.is_ascii() {
            let (from, to) = (from as u8, to as u8);
            let bytes = self
                .inner
                .iter()
                .map(|&b| if b == from { to } else { b })
                .collect();
            // SAFETY: swapping one ASCII byte for another keeps the encoding canonical.
            return unsafe { JavaString::from_semi_utf8_unchecked(bytes) };
        }
        let from = JavaCodePoint::from_char(from);
        let to = JavaCodePoint::from_char(to);
        self.chars()
            .map(|c| if c == from { to } else { c })
            .collect()
    }

    /// Compares two strings as Java's `String.compareTo` does: by UTF-16 code units.
    ///
    /// This differs from the bytewise `Ord`: in UTF-16, supplementary characters (surrogate
    /// pairs) sort *below* `U+E000..=U+FFFF`, while in UTF-8 they sort above.
    #[must_use]
    pub fn cmp_utf16(&self, other: &JavaStr) -> Ordering {
        self.encode_utf16().cmp(other.encode_utf16())
    }

    /// Copies this string into a `Box<JavaStr>`.
    #[must_use]
    pub fn into_boxed(&self) -> Box<JavaStr> {
        self.to_owned().into_boxed_java_str()
    }

    /// Splits the string into runs of valid UTF-8 and single unpaired surrogates.
    fn segments(&self) -> impl Iterator<Item = Segment<'_>> {
        let bytes = &self.inner;
        let mut pos = 0;
        std::iter::from_fn(move || {
            if pos >= bytes.len() {
                return None;
            }
            if is_surrogate_at(bytes, pos) {
                let c = decode_3(bytes[pos], bytes[pos + 1], bytes[pos + 2]);
                pos += 3;
                return Some(Segment::Surrogate(c));
            }
            let start = pos;
            while pos < bytes.len() && !is_surrogate_at(bytes, pos) {
                pos += 1;
            }
            let s = std::str::from_utf8(&bytes[start..pos])
                .expect("semi-UTF-8 without surrogates is UTF-8");
            Some(Segment::Str(s))
        })
    }
}

enum Segment<'a> {
    Str(&'a str),
    Surrogate(u32),
}

impl Default for &JavaStr {
    fn default() -> Self {
        jstr!("")
    }
}

impl ToOwned for JavaStr {
    type Owned = JavaString;

    fn to_owned(&self) -> JavaString {
        JavaString {
            inner: self.inner.to_vec(),
        }
    }
}

impl fmt::Display for JavaStr {
    /// Writes the string, with each unpaired surrogate replaced by `U+FFFD`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Ok(s) => fmt::Display::fmt(s, f),
            Err(_) => fmt::Display::fmt(&self.to_string_lossy(), f),
        }
    }
}

impl fmt::Debug for JavaStr {
    /// Like `str`'s `Debug`, with each unpaired surrogate escaped as `\u{d800}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        f.write_char('"')?;
        for segment in self.segments() {
            match segment {
                Segment::Str(s) => {
                    for c in s.chars() {
                        match c {
                            '\'' => f.write_char(c)?,
                            _ => write!(f, "{}", c.escape_debug())?,
                        }
                    }
                }
                Segment::Surrogate(c) => write!(f, "\\u{{{c:x}}}")?,
            }
        }
        f.write_char('"')
    }
}

impl AsRef<JavaStr> for JavaStr {
    fn as_ref(&self) -> &JavaStr {
        self
    }
}

impl AsRef<JavaStr> for str {
    fn as_ref(&self) -> &JavaStr {
        JavaStr::from_str(self)
    }
}

impl AsRef<JavaStr> for String {
    fn as_ref(&self) -> &JavaStr {
        JavaStr::from_str(self)
    }
}

impl AsRef<[u8]> for JavaStr {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

impl<'a> From<&'a str> for &'a JavaStr {
    fn from(s: &'a str) -> Self {
        JavaStr::from_str(s)
    }
}

macro_rules! impl_index {
    ($($range:ty),*) => {$(
        impl Index<$range> for JavaStr {
            type Output = JavaStr;

            #[track_caller]
            fn index(&self, range: $range) -> &JavaStr {
                let (start, end) = range_bounds(&range, self.len());
                self.get(start..end).unwrap_or_else(|| slice_error(self, start, end))
            }
        }

        impl Index<$range> for JavaString {
            type Output = JavaStr;

            #[track_caller]
            fn index(&self, range: $range) -> &JavaStr {
                &self.as_java_str()[range]
            }
        }
    )*};
}

impl_index!(Range<usize>, RangeFrom<usize>, RangeTo<usize>, RangeFull);

fn range_bounds(range: &impl std::ops::RangeBounds<usize>, len: usize) -> (usize, usize) {
    use std::ops::Bound;
    let start = match range.start_bound() {
        Bound::Included(&s) => s,
        Bound::Excluded(&s) => s.saturating_add(1),
        Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        Bound::Included(&e) => e.saturating_add(1),
        Bound::Excluded(&e) => e,
        Bound::Unbounded => len,
    };
    (start, end)
}

#[cold]
#[track_caller]
fn slice_error(s: &JavaStr, start: usize, end: usize) -> ! {
    if start > s.len() || end > s.len() {
        panic!(
            "byte range {start}..{end} is out of bounds of a JavaStr of length {}",
            s.len()
        );
    }
    if start > end {
        panic!("JavaStr slice index starts at {start} but ends at {end}");
    }
    panic!("byte range {start}..{end} is not on code point boundaries of {s:?}");
}

// -------------------------------------------------------------------------------------------------
// JavaString
// -------------------------------------------------------------------------------------------------

impl JavaString {
    /// An empty string.
    #[inline]
    #[must_use]
    pub const fn new() -> JavaString {
        JavaString { inner: Vec::new() }
    }

    /// An empty string with room for `capacity` bytes.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> JavaString {
        JavaString {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// Validates bytes as canonical semi-UTF-8.
    ///
    /// # Errors
    ///
    /// If `bytes` is not canonical semi-UTF-8; the bytes are returned with the error.
    pub fn from_semi_utf8(bytes: Vec<u8>) -> Result<JavaString, (Vec<u8>, Utf8Error)> {
        match validate_semi_utf8(&bytes) {
            Ok(()) => Ok(JavaString { inner: bytes }),
            Err(e) => Err((bytes, e)),
        }
    }

    /// Wraps bytes without checking them.
    ///
    /// # Safety
    ///
    /// `bytes` must be canonical semi-UTF-8 (see the [module docs](self)).
    #[inline]
    #[must_use]
    pub const unsafe fn from_semi_utf8_unchecked(bytes: Vec<u8>) -> JavaString {
        JavaString { inner: bytes }
    }

    /// Borrows this string as a [`JavaStr`].
    #[inline]
    #[must_use]
    pub fn as_java_str(&self) -> &JavaStr {
        // SAFETY: `JavaString` maintains the canonical semi-UTF-8 invariant.
        unsafe { JavaStr::from_semi_utf8_unchecked(&self.inner) }
    }

    /// The semi-UTF-8 bytes.
    #[inline]
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.inner
    }

    /// Converts into a `String`, if there is no unpaired surrogate.
    ///
    /// # Errors
    ///
    /// If the string contains an unpaired surrogate; the string is returned with the error.
    pub fn into_string(self) -> Result<String, (JavaString, Utf8Error)> {
        String::from_utf8(self.inner).map_err(|e| {
            (
                JavaString {
                    inner: e.as_bytes().to_vec(),
                },
                e.utf8_error().into(),
            )
        })
    }

    /// Converts into a `Box<JavaStr>`.
    #[must_use]
    pub fn into_boxed_java_str(self) -> Box<JavaStr> {
        let boxed: Box<[u8]> = self.inner.into_boxed_slice();
        // SAFETY: `JavaStr` is `repr(transparent)` over `[u8]`, so `Box<[u8]>` and `Box<JavaStr>`
        // have the same layout, metadata and allocation; the bytes are canonical semi-UTF-8.
        unsafe { Box::from_raw(Box::into_raw(boxed) as *mut JavaStr) }
    }

    /// The capacity in bytes.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves room for `additional` more bytes.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Empties the string.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Shortens the string to `new_len` bytes.
    ///
    /// # Panics
    ///
    /// If `new_len` is not on a code point boundary.
    #[track_caller]
    pub fn truncate(&mut self, new_len: usize) {
        if new_len < self.len() {
            assert!(
                self.is_char_boundary(new_len),
                "truncate: {new_len} is not a char boundary"
            );
            self.inner.truncate(new_len);
        }
    }

    /// Appends a `char`.
    #[inline]
    pub fn push(&mut self, c: char) {
        let mut buf = [0; 4];
        self.inner
            .extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }

    /// Appends a code point, which may be a surrogate. A low surrogate that follows a trailing
    /// high surrogate joins it into one supplementary code point, as in UTF-16.
    pub fn push_java(&mut self, c: JavaCodePoint) {
        let c = c.as_u32();
        if (0xdc00..=0xdfff).contains(&c)
            && let Some(high) = self.trailing_high_surrogate()
        {
            self.inner.truncate(self.inner.len() - 3);
            let supplementary = 0x10000 + ((high - 0xd800) << 10) + (c - 0xdc00);
            encode_semi_utf8(supplementary, &mut self.inner);
            return;
        }
        encode_semi_utf8(c, &mut self.inner);
    }

    /// Appends a `str`.
    #[inline]
    pub fn push_str(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    /// Appends a `JavaStr`, joining a trailing high surrogate and a leading low surrogate into
    /// one supplementary code point, as concatenation does in UTF-16.
    pub fn push_java_str(&mut self, s: &JavaStr) {
        let bytes = s.as_bytes();
        if is_surrogate_at(bytes, 0) && bytes[1] >= 0xb0 && self.trailing_high_surrogate().is_some()
        {
            let low = decode_3(bytes[0], bytes[1], bytes[2]);
            self.push_java(JavaCodePoint(low));
            self.inner.extend_from_slice(&bytes[3..]);
        } else {
            self.inner.extend_from_slice(bytes);
        }
    }

    fn trailing_high_surrogate(&self) -> Option<u32> {
        let n = self.inner.len();
        if n >= 3 && self.inner[n - 3] == 0xed && (0xa0..=0xaf).contains(&self.inner[n - 2]) {
            Some(decode_3(
                self.inner[n - 3],
                self.inner[n - 2],
                self.inner[n - 1],
            ))
        } else {
            None
        }
    }
}

impl Deref for JavaString {
    type Target = JavaStr;

    #[inline]
    fn deref(&self) -> &JavaStr {
        self.as_java_str()
    }
}

impl DerefMut for JavaString {
    #[inline]
    fn deref_mut(&mut self) -> &mut JavaStr {
        // SAFETY: `JavaStr` exposes no mutation that could break the invariant.
        unsafe { JavaStr::from_semi_utf8_unchecked_mut(&mut self.inner) }
    }
}

impl Hash for JavaString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Must match `JavaStr`'s derived `Hash` (`Borrow<JavaStr>`).
        self.as_java_str().hash(state);
    }
}

impl Borrow<JavaStr> for JavaString {
    fn borrow(&self) -> &JavaStr {
        self
    }
}

impl AsRef<JavaStr> for JavaString {
    fn as_ref(&self) -> &JavaStr {
        self
    }
}

impl AsRef<[u8]> for JavaString {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

impl fmt::Display for JavaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_java_str(), f)
    }
}

impl fmt::Debug for JavaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_java_str(), f)
    }
}

impl fmt::Write for JavaString {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl From<&str> for JavaString {
    fn from(s: &str) -> Self {
        JavaString {
            inner: s.as_bytes().to_vec(),
        }
    }
}

impl From<String> for JavaString {
    fn from(s: String) -> Self {
        JavaString {
            inner: s.into_bytes(),
        }
    }
}

impl From<&String> for JavaString {
    fn from(s: &String) -> Self {
        JavaString::from(s.as_str())
    }
}

impl From<&JavaStr> for JavaString {
    fn from(s: &JavaStr) -> Self {
        s.to_owned()
    }
}

impl From<char> for JavaString {
    fn from(c: char) -> Self {
        let mut s = JavaString::new();
        s.push(c);
        s
    }
}

impl From<Cow<'_, JavaStr>> for JavaString {
    fn from(s: Cow<'_, JavaStr>) -> Self {
        s.into_owned()
    }
}

impl From<JavaString> for Cow<'_, JavaStr> {
    fn from(s: JavaString) -> Self {
        Cow::Owned(s)
    }
}

impl<'a> From<&'a JavaStr> for Cow<'a, JavaStr> {
    fn from(s: &'a JavaStr) -> Self {
        Cow::Borrowed(s)
    }
}

impl From<JavaString> for Box<JavaStr> {
    fn from(s: JavaString) -> Self {
        s.into_boxed_java_str()
    }
}

impl From<Box<JavaStr>> for JavaString {
    fn from(s: Box<JavaStr>) -> Self {
        // SAFETY: `JavaStr` is `repr(transparent)` over `[u8]` (same layout, metadata and
        // allocation), and the content stays canonical semi-UTF-8.
        let bytes: Box<[u8]> = unsafe { Box::from_raw(Box::into_raw(s) as *mut [u8]) };
        JavaString {
            inner: bytes.into_vec(),
        }
    }
}

impl Clone for Box<JavaStr> {
    fn clone(&self) -> Self {
        self.into_boxed()
    }
}

impl From<&JavaStr> for Arc<JavaStr> {
    fn from(s: &JavaStr) -> Self {
        let arc: Arc<[u8]> = Arc::from(s.as_bytes());
        // SAFETY: `JavaStr` is `repr(transparent)` over `[u8]`, so `Arc<[u8]>` and `Arc<JavaStr>`
        // have the same layout and metadata; the bytes are canonical semi-UTF-8.
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const JavaStr) }
    }
}

impl From<JavaString> for Arc<JavaStr> {
    fn from(s: JavaString) -> Self {
        Arc::from(s.as_java_str())
    }
}

impl From<&JavaStr> for Rc<JavaStr> {
    fn from(s: &JavaStr) -> Self {
        let rc: Rc<[u8]> = Rc::from(s.as_bytes());
        // SAFETY: as for `Arc<JavaStr>`.
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const JavaStr) }
    }
}

impl FromIterator<JavaCodePoint> for JavaString {
    fn from_iter<I: IntoIterator<Item = JavaCodePoint>>(iter: I) -> Self {
        let mut s = JavaString::new();
        s.extend(iter);
        s
    }
}

impl FromIterator<char> for JavaString {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
        let mut s = JavaString::new();
        iter.into_iter().for_each(|c| s.push(c));
        s
    }
}

impl<'a> FromIterator<&'a JavaStr> for JavaString {
    fn from_iter<I: IntoIterator<Item = &'a JavaStr>>(iter: I) -> Self {
        let mut s = JavaString::new();
        iter.into_iter().for_each(|part| s.push_java_str(part));
        s
    }
}

impl Extend<JavaCodePoint> for JavaString {
    fn extend<I: IntoIterator<Item = JavaCodePoint>>(&mut self, iter: I) {
        iter.into_iter().for_each(|c| self.push_java(c));
    }
}

impl<'a> Extend<&'a JavaStr> for JavaString {
    fn extend<I: IntoIterator<Item = &'a JavaStr>>(&mut self, iter: I) {
        iter.into_iter().for_each(|s| self.push_java_str(s));
    }
}

impl<'a> Extend<&'a str> for JavaString {
    fn extend<I: IntoIterator<Item = &'a str>>(&mut self, iter: I) {
        iter.into_iter().for_each(|s| self.push_str(s));
    }
}

impl std::ops::Add<&JavaStr> for JavaString {
    type Output = JavaString;

    fn add(mut self, rhs: &JavaStr) -> JavaString {
        self.push_java_str(rhs);
        self
    }
}

impl std::ops::Add<&str> for JavaString {
    type Output = JavaString;

    fn add(mut self, rhs: &str) -> JavaString {
        self.push_str(rhs);
        self
    }
}

impl std::ops::AddAssign<&JavaStr> for JavaString {
    fn add_assign(&mut self, rhs: &JavaStr) {
        self.push_java_str(rhs);
    }
}

impl std::ops::AddAssign<&str> for JavaString {
    fn add_assign(&mut self, rhs: &str) {
        self.push_str(rhs);
    }
}

// Cross-type equality, as `str`/`String` have it.
macro_rules! impl_eq {
    ($a:ty, $b:ty) => {
        impl PartialEq<$b> for $a {
            #[inline]
            fn eq(&self, other: &$b) -> bool {
                AsRef::<[u8]>::as_ref(self) == AsRef::<[u8]>::as_ref(other)
            }
        }

        impl PartialEq<$a> for $b {
            #[inline]
            fn eq(&self, other: &$a) -> bool {
                AsRef::<[u8]>::as_ref(self) == AsRef::<[u8]>::as_ref(other)
            }
        }
    };
}

impl_eq!(JavaStr, str);
impl_eq!(JavaStr, String);
impl_eq!(JavaStr, JavaString);
impl_eq!(JavaString, str);
impl_eq!(JavaString, String);
impl_eq!(&JavaStr, str);
impl_eq!(&JavaStr, String);
impl_eq!(&JavaStr, JavaString);
impl_eq!(JavaStr, &str);
impl_eq!(JavaString, &str);
impl_eq!(JavaArcStr, str);
impl_eq!(JavaArcStr, &str);
impl_eq!(JavaArcStr, String);
impl_eq!(JavaArcStr, JavaStr);
impl_eq!(JavaArcStr, &JavaStr);
impl_eq!(JavaArcStr, JavaString);

impl PartialEq<Cow<'_, JavaStr>> for JavaStr {
    fn eq(&self, other: &Cow<'_, JavaStr>) -> bool {
        *self == **other
    }
}

impl PartialEq<Cow<'_, JavaStr>> for &JavaStr {
    fn eq(&self, other: &Cow<'_, JavaStr>) -> bool {
        **self == **other
    }
}

// -------------------------------------------------------------------------------------------------
// JavaCodePoint
// -------------------------------------------------------------------------------------------------

impl JavaCodePoint {
    /// The code point `c`, if it is at most `U+10FFFF` (surrogates included).
    #[inline]
    #[must_use]
    pub const fn from_u32(c: u32) -> Option<JavaCodePoint> {
        if c <= 0x10ffff {
            Some(JavaCodePoint(c))
        } else {
            None
        }
    }

    /// The code point of a `char`.
    #[inline]
    #[must_use]
    pub const fn from_char(c: char) -> JavaCodePoint {
        JavaCodePoint(c as u32)
    }

    /// The numeric value.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// The `char`, unless this is a surrogate.
    #[inline]
    #[must_use]
    pub const fn as_char(self) -> Option<char> {
        char::from_u32(self.0)
    }

    /// Whether this is a surrogate code point (`U+D800..=U+DFFF`).
    #[inline]
    #[must_use]
    pub const fn is_surrogate(self) -> bool {
        matches!(self.0, 0xd800..=0xdfff)
    }

    /// The number of UTF-16 code units (1 or 2).
    #[inline]
    #[must_use]
    pub const fn len_utf16(self) -> usize {
        if self.0 >= 0x10000 { 2 } else { 1 }
    }

    /// The number of semi-UTF-8 bytes (1 to 4).
    #[inline]
    #[must_use]
    pub const fn len_semi_utf8(self) -> usize {
        match self.0 {
            0..0x80 => 1,
            0x80..0x800 => 2,
            0x800..0x10000 => 3,
            _ => 4,
        }
    }
}

impl From<char> for JavaCodePoint {
    fn from(c: char) -> Self {
        JavaCodePoint::from_char(c)
    }
}

impl PartialEq<char> for JavaCodePoint {
    fn eq(&self, other: &char) -> bool {
        self.0 == *other as u32
    }
}

// -------------------------------------------------------------------------------------------------
// Utf8Error
// -------------------------------------------------------------------------------------------------

impl Utf8Error {
    /// The length of the valid prefix.
    #[inline]
    #[must_use]
    pub const fn valid_up_to(&self) -> usize {
        self.valid_up_to
    }

    /// The length of the invalid sequence, or `None` if the input ended inside a sequence.
    #[inline]
    #[must_use]
    pub const fn error_len(&self) -> Option<usize> {
        match self.error_len {
            Some(n) => Some(n as usize),
            None => None,
        }
    }
}

impl From<std::str::Utf8Error> for Utf8Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Utf8Error {
            valid_up_to: e.valid_up_to(),
            #[allow(clippy::cast_possible_truncation)] // at most 3
            error_len: e.error_len().map(|n| n as u8),
        }
    }
}

impl fmt::Display for Utf8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.error_len {
            Some(n) => {
                write!(
                    f,
                    "invalid semi-UTF-8 sequence of {n} bytes from index {}",
                    self.valid_up_to
                )
            }
            None => write!(
                f,
                "incomplete semi-UTF-8 byte sequence from index {}",
                self.valid_up_to
            ),
        }
    }
}

impl std::error::Error for Utf8Error {}

// -------------------------------------------------------------------------------------------------
// Encoding helpers
// -------------------------------------------------------------------------------------------------

#[inline]
const fn is_continuation_byte(b: u8) -> bool {
    b & 0xc0 == 0x80
}

/// Whether a 3-byte surrogate sequence starts at `pos`. `0xED` is always a lead byte, so this
/// needs no boundary check.
#[inline]
fn is_surrogate_at(bytes: &[u8], pos: usize) -> bool {
    bytes.len() >= pos + 3 && bytes[pos] == 0xed && bytes[pos + 1] >= 0xa0
}

#[inline]
const fn decode_3(b0: u8, b1: u8, b2: u8) -> u32 {
    ((b0 as u32 & 0x0f) << 12) | ((b1 as u32 & 0x3f) << 6) | (b2 as u32 & 0x3f)
}

/// Appends the semi-UTF-8 encoding of `c` (at most `U+10FFFF`).
#[allow(clippy::cast_possible_truncation)] // every shifted value is masked to fit a byte
pub(crate) fn encode_semi_utf8(c: u32, out: &mut Vec<u8>) {
    match c {
        0..0x80 => out.push(c as u8),
        0x80..0x800 => out.extend_from_slice(&[0xc0 | (c >> 6) as u8, 0x80 | (c & 0x3f) as u8]),
        0x800..0x10000 => out.extend_from_slice(&[
            0xe0 | (c >> 12) as u8,
            0x80 | ((c >> 6) & 0x3f) as u8,
            0x80 | (c & 0x3f) as u8,
        ]),
        _ => out.extend_from_slice(&[
            0xf0 | (c >> 18) as u8,
            0x80 | ((c >> 12) & 0x3f) as u8,
            0x80 | ((c >> 6) & 0x3f) as u8,
            0x80 | (c & 0x3f) as u8,
        ]),
    }
}

/// Validates canonical semi-UTF-8: UTF-8 plus 3-byte surrogates, where a high surrogate is never
/// directly followed by a low one (a pair must use the 4-byte form).
fn validate_semi_utf8(v: &[u8]) -> Result<(), Utf8Error> {
    // Fast path: std's validator; it stops at the first surrogate (or real error).
    let mut i = match std::str::from_utf8(v) {
        Ok(_) => return Ok(()),
        Err(e) => e.valid_up_to(),
    };
    // The end of the last high surrogate, to reject a split pair.
    let mut high_end = usize::MAX;
    while i < v.len() {
        let first = v[i];
        if first < 0x80 {
            i += 1;
            continue;
        }
        let err = |len: Option<u8>| {
            Err(Utf8Error {
                valid_up_to: i,
                error_len: len,
            })
        };
        let cont = |k: usize| -> Result<u8, Option<u8>> {
            match v.get(i + k) {
                None => Err(None),
                #[allow(clippy::cast_possible_truncation)] // k <= 3
                Some(&b) if !is_continuation_byte(b) => Err(Some(k as u8)),
                Some(&b) => Ok(b),
            }
        };
        let width = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return err(Some(1)),
        };
        let second = match cont(1) {
            Ok(b) => b,
            Err(len) => return err(len),
        };
        match (first, second) {
            (0xc2..=0xdf, _)
            | (0xe0, 0xa0..=0xbf)
            | (0xe1..=0xef, 0x80..=0xbf)
            | (0xf0, 0x90..=0xbf)
            | (0xf1..=0xf3, 0x80..=0xbf)
            | (0xf4, 0x80..=0x8f) => {}
            _ => return err(Some(1)),
        }
        for k in 2..width {
            if let Err(len) = cont(k) {
                return err(len);
            }
        }
        if first == 0xed && second >= 0xa0 {
            if second >= 0xb0 && high_end == i {
                // A surrogate pair written as two 3-byte sequences: not canonical.
                return Err(Utf8Error {
                    valid_up_to: i - 3,
                    error_len: Some(6),
                });
            }
            if second < 0xb0 {
                high_end = i + 3;
            }
        }
        i += width;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
