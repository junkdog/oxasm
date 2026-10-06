//! [`JavaCompactStr`]: an immutable Java string that is cheap to clone.

use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

use super::{JavaStr, JavaString};

/// The longest string stored inline.
const INLINE_CAP: usize = 23;

/// An immutable Java string that is cheap to clone, and to create when it is short.
///
/// This is to [`JavaStr`] what `SmolStr` or `CompactString` are to `str`: a string of up to 23
/// bytes is stored inline, without an allocation, and a longer one is a reference-counted
/// `Arc<JavaStr>`, so a clone never copies more than 24 bytes. Most of the names and descriptors
/// of a class are short (two thirds of those of the JDK's `java.base` are 23 bytes or less); the
/// tree nodes hold them as `JavaCompactStr`s, and share the long ones.
///
/// It dereferences to [`JavaStr`], and compares, orders and hashes like it (and like the other
/// string types it compares to), whatever its representation, so it can be looked up by
/// `&JavaStr` in hash maps and sets.
#[derive(Clone)]
pub struct JavaCompactStr(Repr);

#[derive(Clone)]
enum Repr {
    /// A string of at most [`INLINE_CAP`] bytes: `bytes[..len]`.
    Inline {
        len: InlineLen,
        bytes: [u8; INLINE_CAP],
    },
    /// A longer string (or one made from an `Arc<JavaStr>`).
    Heap(Arc<JavaStr>),
}

/// The length of an inline string. As an enum, its unused values are the niche that keeps
/// [`Repr`] at 24 bytes (as `SmolStr` does).
#[derive(Clone, Copy)]
#[repr(u8)]
#[rustfmt::skip]
enum InlineLen {
    L0, L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16, L17, L18, L19, L20,
    L21, L22, L23,
}

impl InlineLen {
    const ALL: [InlineLen; INLINE_CAP + 1] = {
        use InlineLen::*;
        [
            L0, L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16, L17, L18,
            L19, L20, L21, L22, L23,
        ]
    };
}

const _: () = assert!(size_of::<JavaCompactStr>() == 24);
const _: () = assert!(size_of::<Option<JavaCompactStr>>() == 24);

impl JavaCompactStr {
    /// A copy of the given string: inline if it is short enough, otherwise a new allocation.
    #[inline]
    #[must_use]
    pub fn new(s: &JavaStr) -> JavaCompactStr {
        let src = s.as_bytes();
        if src.len() <= INLINE_CAP {
            let mut bytes = [0; INLINE_CAP];
            bytes[..src.len()].copy_from_slice(src);
            JavaCompactStr(Repr::Inline {
                len: InlineLen::ALL[src.len()],
                bytes,
            })
        } else {
            JavaCompactStr(Repr::Heap(Arc::from(s)))
        }
    }

    /// Whether a copy of the given string is stored inline (if it has at most 23 bytes).
    #[inline]
    #[must_use]
    pub fn fits_inline(s: &JavaStr) -> bool {
        s.as_bytes().len() <= INLINE_CAP
    }

    /// Whether this string is stored inline, i.e. clones of it copy it rather than share it.
    #[inline]
    #[must_use]
    pub fn is_inline(&self) -> bool {
        matches!(self.0, Repr::Inline { .. })
    }

    /// The string, borrowed.
    #[inline]
    #[must_use]
    pub fn as_java_str(&self) -> &JavaStr {
        match &self.0 {
            Repr::Inline { len, bytes } => {
                let bytes = &bytes[..*len as usize];
                // SAFETY: the inline bytes are a copy of a `JavaStr`'s (in `new`), so they are
                // canonical semi-UTF-8.
                unsafe { JavaStr::from_semi_utf8_unchecked(bytes) }
            }
            Repr::Heap(s) => s,
        }
    }
}

impl Default for JavaCompactStr {
    /// The empty string (inline).
    fn default() -> JavaCompactStr {
        JavaCompactStr(Repr::Inline {
            len: InlineLen::L0,
            bytes: [0; INLINE_CAP],
        })
    }
}

impl PartialEq for JavaCompactStr {
    #[inline]
    fn eq(&self, other: &JavaCompactStr) -> bool {
        if let (Repr::Heap(a), Repr::Heap(b)) = (&self.0, &other.0)
            && Arc::ptr_eq(a, b)
        {
            return true;
        }
        self.as_java_str() == other.as_java_str()
    }
}

impl Eq for JavaCompactStr {}

impl PartialOrd for JavaCompactStr {
    #[inline]
    fn partial_cmp(&self, other: &JavaCompactStr) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JavaCompactStr {
    #[inline]
    fn cmp(&self, other: &JavaCompactStr) -> Ordering {
        self.as_java_str().cmp(other.as_java_str())
    }
}

impl Hash for JavaCompactStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Must match `JavaStr`'s derived `Hash` (`Borrow<JavaStr>`).
        self.as_java_str().hash(state);
    }
}

impl Deref for JavaCompactStr {
    type Target = JavaStr;

    #[inline]
    fn deref(&self) -> &JavaStr {
        self.as_java_str()
    }
}

impl Borrow<JavaStr> for JavaCompactStr {
    #[inline]
    fn borrow(&self) -> &JavaStr {
        self.as_java_str()
    }
}

impl AsRef<JavaStr> for JavaCompactStr {
    #[inline]
    fn as_ref(&self) -> &JavaStr {
        self.as_java_str()
    }
}

impl AsRef<[u8]> for JavaCompactStr {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_java_str().as_bytes()
    }
}

impl fmt::Display for JavaCompactStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_java_str(), f)
    }
}

impl fmt::Debug for JavaCompactStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_java_str(), f)
    }
}

impl From<&JavaStr> for JavaCompactStr {
    #[inline]
    fn from(s: &JavaStr) -> JavaCompactStr {
        JavaCompactStr::new(s)
    }
}

impl From<&str> for JavaCompactStr {
    #[inline]
    fn from(s: &str) -> JavaCompactStr {
        JavaCompactStr::from(JavaStr::from_str(s))
    }
}

impl From<String> for JavaCompactStr {
    #[inline]
    fn from(s: String) -> JavaCompactStr {
        JavaCompactStr::from(JavaStr::from_str(&s))
    }
}

impl From<&String> for JavaCompactStr {
    #[inline]
    fn from(s: &String) -> JavaCompactStr {
        JavaCompactStr::from(JavaStr::from_str(s))
    }
}

impl From<JavaString> for JavaCompactStr {
    #[inline]
    fn from(s: JavaString) -> JavaCompactStr {
        JavaCompactStr::from(s.as_java_str())
    }
}

impl From<&JavaString> for JavaCompactStr {
    #[inline]
    fn from(s: &JavaString) -> JavaCompactStr {
        JavaCompactStr::from(s.as_java_str())
    }
}

impl From<Cow<'_, JavaStr>> for JavaCompactStr {
    #[inline]
    fn from(s: Cow<'_, JavaStr>) -> JavaCompactStr {
        JavaCompactStr::from(&*s)
    }
}

impl From<&JavaCompactStr> for JavaCompactStr {
    /// Another reference to the same string.
    #[inline]
    fn from(s: &JavaCompactStr) -> JavaCompactStr {
        s.clone()
    }
}

impl From<Arc<JavaStr>> for JavaCompactStr {
    /// The given string, shared (not inline, even if it is short).
    #[inline]
    fn from(s: Arc<JavaStr>) -> JavaCompactStr {
        JavaCompactStr(Repr::Heap(s))
    }
}

impl From<JavaCompactStr> for Arc<JavaStr> {
    /// The shared string, or a new allocation for an inline one.
    #[inline]
    fn from(s: JavaCompactStr) -> Arc<JavaStr> {
        match s.0 {
            Repr::Inline { .. } => Arc::from(s.as_java_str()),
            Repr::Heap(s) => s,
        }
    }
}

impl From<JavaCompactStr> for JavaString {
    #[inline]
    fn from(s: JavaCompactStr) -> JavaString {
        s.as_java_str().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::hash::BuildHasher;

    use super::*;
    use crate::jstr;

    #[test]
    fn inline_up_to_23_bytes() {
        for len in 0..=40 {
            let string = "x".repeat(len);
            let s = JavaCompactStr::from(string.as_str());
            assert_eq!(s.is_inline(), len <= INLINE_CAP);
            assert_eq!(s, string.as_str());
            assert_eq!(s.as_java_str().as_bytes(), string.as_bytes());
            assert_eq!(s.clone(), s);
        }
        assert!(JavaCompactStr::default().is_inline());
        assert_eq!(JavaCompactStr::default(), "");
    }

    #[test]
    fn surrogates_and_multibyte_chars() {
        // A lone surrogate (3 bytes), NUL and a supplementary character (4 bytes), on both sides
        // of the inline capacity.
        let mut short = JavaString::from("a\0\u{1F600}");
        short.push_java(crate::jstr::JavaCodePoint::from_u32(0xD800).unwrap());
        let long = short.clone() + "0123456789abcdef";
        for s in [short, long] {
            let arc = JavaCompactStr::from(&s);
            assert_eq!(arc, s);
            assert_eq!(format!("{arc:?}"), format!("{s:?}"));
        }
    }

    #[test]
    fn eq_ord_hash_ignore_representation() {
        let inline = JavaCompactStr::from("java/lang/Object");
        let heap = JavaCompactStr::from(Arc::<JavaStr>::from(jstr!("java/lang/Object")));
        assert!(inline.is_inline() && !heap.is_inline());
        assert_eq!(inline, heap);
        assert_eq!(inline.cmp(&heap), Ordering::Equal);
        let hasher = std::hash::RandomState::new();
        assert_eq!(hasher.hash_one(&inline), hasher.hash_one(&heap));
        assert_eq!(
            hasher.hash_one(&inline),
            hasher.hash_one(jstr!("java/lang/Object"))
        );
        let set: HashSet<JavaCompactStr> = [inline].into_iter().collect();
        assert!(set.contains(jstr!("java/lang/Object")));
        let (short, long) = (
            JavaCompactStr::from("a"),
            JavaCompactStr::from("b".repeat(30)),
        );
        assert_eq!(short.cmp(&long), Ordering::Less);
    }

    #[test]
    fn arc_round_trip() {
        let long: Arc<JavaStr> = Arc::from(jstr!("Ljava/lang/invoke/MethodHandles$Lookup;"));
        let s = JavaCompactStr::from(long.clone());
        assert!(Arc::ptr_eq(&Arc::<JavaStr>::from(s.clone()), &long));
        assert_eq!(
            *Arc::<JavaStr>::from(JavaCompactStr::from("()V")),
            *jstr!("()V")
        );
    }
}
