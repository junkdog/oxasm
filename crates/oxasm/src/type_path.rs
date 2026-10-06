//! [`TypePath`]: the path to a type argument, wildcard bound, array element type, or static inner
//! type within an enclosing type.

use std::fmt;

/// The path to a type argument, wildcard bound, array element type, or static inner type within
/// an enclosing type.
///
/// Holds a copy of the `type_path` structure of the JVMS, rather than (as ASM does) a window into
/// the class file buffer, so a `TypePath` does not borrow the class it was read from.
///
/// See [JVMS 4.7.20.2](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7.20.2).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TypePath {
    /// The `type_path` structure: `path_length`, then `path_length` pairs of
    /// (`type_path_kind`, `type_argument_index`).
    bytes: Vec<u8>,
}

/// The error of [`TypePath::from_string`] (ASM's `IllegalArgumentException`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid type path: {0:?}")]
pub struct InvalidTypePath(pub String);

impl TypePath {
    /// A type path step that steps into the element type of an array type. See
    /// [`TypePath::get_step`].
    pub const ARRAY_ELEMENT: u8 = 0;
    /// A type path step that steps into the nested type of a class type.
    pub const INNER_TYPE: u8 = 1;
    /// A type path step that steps into the bound of a wildcard type.
    pub const WILDCARD_BOUND: u8 = 2;
    /// A type path step that steps into a type argument of a generic type.
    pub const TYPE_ARGUMENT: u8 = 3;

    /// Creates a type path from a `type_path` structure: its first byte is `path_length`, which
    /// must be followed by at least `path_length` pairs of bytes (extra bytes are ignored).
    ///
    /// Returns `None` if `structure` is too short.
    #[must_use]
    pub fn from_structure(structure: &[u8]) -> Option<TypePath> {
        let length = usize::from(*structure.first()?) * 2 + 1;
        Some(TypePath {
            bytes: structure.get(..length)?.to_vec(),
        })
    }

    /// Returns the `type_path` structure of this path (as written in class files).
    #[inline]
    #[must_use]
    pub fn as_structure(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the length of this path, i.e. its number of steps.
    #[inline]
    #[must_use]
    pub fn get_length(&self) -> usize {
        // path_length is stored in the first byte of a type_path.
        usize::from(self.bytes[0])
    }

    /// Returns the value of the given step of this path: one of [`TypePath::ARRAY_ELEMENT`],
    /// [`TypePath::INNER_TYPE`], [`TypePath::WILDCARD_BOUND`] or [`TypePath::TYPE_ARGUMENT`].
    ///
    /// # Panics
    ///
    /// If `index` is not less than [`TypePath::get_length`].
    #[inline]
    #[must_use]
    #[track_caller]
    pub fn get_step(&self, index: usize) -> u8 {
        // Returns the type_path_kind of the path element of the given index.
        self.bytes[2 * index + 1]
    }

    /// Returns the index of the type argument that the given step is stepping into. This method
    /// should only be used for steps whose value is [`TypePath::TYPE_ARGUMENT`].
    ///
    /// # Panics
    ///
    /// If `index` is not less than [`TypePath::get_length`].
    #[inline]
    #[must_use]
    #[track_caller]
    pub fn get_step_argument(&self, index: usize) -> u8 {
        // Returns the type_argument_index of the path element of the given index.
        self.bytes[2 * index + 2]
    }

    /// Converts a type path in string form, in the format used by [`TypePath`]'s `Display`, into
    /// a `TypePath`. Returns `Ok(None)` for an empty string (Java's `null`).
    ///
    /// Like ASM, a type argument index is truncated to a byte, and so is the number of steps.
    ///
    /// # Errors
    ///
    /// If the string contains anything but `[`, `.`, `*` and `<digits>;` steps.
    pub fn from_string(type_path: &str) -> Result<Option<TypePath>, InvalidTypePath> {
        if type_path.is_empty() {
            return Ok(None);
        }
        let invalid = || InvalidTypePath(type_path.to_owned());
        let chars = type_path.as_bytes();
        let mut output = Vec::with_capacity(chars.len() + 1);
        output.push(0);
        let mut type_path_index = 0;
        while type_path_index < chars.len() {
            let mut c = chars[type_path_index];
            type_path_index += 1;
            match c {
                b'[' => output.extend_from_slice(&[TypePath::ARRAY_ELEMENT, 0]),
                b'.' => output.extend_from_slice(&[TypePath::INNER_TYPE, 0]),
                b'*' => output.extend_from_slice(&[TypePath::WILDCARD_BOUND, 0]),
                b'0'..=b'9' => {
                    let mut type_arg = i32::from(c - b'0');
                    while type_path_index < chars.len() {
                        c = chars[type_path_index];
                        type_path_index += 1;
                        match c {
                            b'0'..=b'9' => {
                                type_arg =
                                    type_arg.wrapping_mul(10).wrapping_add(i32::from(c - b'0'));
                            }
                            b';' => break,
                            _ => return Err(invalid()),
                        }
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    // ASM's put11
                    output.extend_from_slice(&[TypePath::TYPE_ARGUMENT, type_arg as u8]);
                }
                _ => return Err(invalid()),
            }
        }
        #[allow(clippy::cast_possible_truncation)] // ASM's `(byte) (output.length / 2)`
        let length = (output.len() / 2) as u8;
        output[0] = length;
        output.truncate(usize::from(length) * 2 + 1);
        Ok(Some(TypePath { bytes: output }))
    }

    /// Puts the `type_path` structure corresponding to the given type path (`None` for an empty
    /// path) into `output`.
    #[allow(dead_code)] // the writer's (P5)
    pub(crate) fn put(type_path: Option<&TypePath>, output: &mut Vec<u8>) {
        match type_path {
            None => output.push(0),
            Some(type_path) => output.extend_from_slice(&type_path.bytes),
        }
    }
}

/// [`TypePath::ARRAY_ELEMENT`] steps are represented with '[', [`TypePath::INNER_TYPE`] steps
/// with '.', [`TypePath::WILDCARD_BOUND`] steps with '*' and [`TypePath::TYPE_ARGUMENT`] steps
/// with their type argument index in decimal form followed by ';'. (A step kind that is none of
/// these, which ASM rejects with an `AssertionError`, prints as `?`.)
impl fmt::Display for TypePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for i in 0..self.get_length() {
            match self.get_step(i) {
                TypePath::ARRAY_ELEMENT => f.write_str("[")?,
                TypePath::INNER_TYPE => f.write_str(".")?,
                TypePath::WILDCARD_BOUND => f.write_str("*")?,
                TypePath::TYPE_ARGUMENT => write!(f, "{};", self.get_step_argument(i))?,
                _ => f.write_str("?")?,
            }
        }
        Ok(())
    }
}

impl fmt::Debug for TypePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypePath({self})")
    }
}
