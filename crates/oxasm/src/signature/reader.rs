//! [`SignatureReader`]: ASM's `SignatureReader`.

use crate::jstr::JavaStr;
use crate::signature::SignatureVisitor;

/// The maximum nesting depth of type arguments (ASM 9.11's `MAX_DEPTH`).
const MAX_DEPTH: u32 = 256;

/// Why a signature could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// The signature is malformed at this byte offset: an unexpected character (ASM's
    /// `IllegalArgumentException`), or an end of input or missing delimiter (ASM's
    /// `StringIndexOutOfBoundsException`).
    #[error("invalid signature at offset {0}")]
    Malformed(usize),
    /// The signature has more than 256 levels of nested type arguments. ASM 9.10.1 recurses until
    /// it overflows the stack; this is ASM 9.11's limit and message (ADR-06).
    #[error("Too many nested type arguments")]
    TooManyNestedTypeArguments,
}

/// A parser for signature literals, as defined in the Java Virtual Machine Specification (JVMS),
/// to visit them with a [`SignatureVisitor`].
///
/// See [JVMS 4.7.9.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7.9.1).
///
/// Like ASM, the reader does not validate signatures: it reads just enough to visit them, and
/// accepts some malformed ones. On a malformed signature it returns an error, after the visitor
/// has seen the events of the signature's valid prefix.
#[derive(Clone, Copy, Debug)]
pub struct SignatureReader<'s> {
    /// The JVMS signature to be read.
    signature_value: &'s JavaStr,
}

impl<'s> SignatureReader<'s> {
    /// Constructs a reader for the given signature: a *JavaTypeSignature*, *ClassSignature* or
    /// *MethodSignature*.
    #[must_use]
    pub fn new(signature: &'s (impl AsRef<JavaStr> + ?Sized)) -> SignatureReader<'s> {
        SignatureReader {
            signature_value: signature.as_ref(),
        }
    }

    /// Makes the given visitor visit the signature of this reader. This method is intended to be
    /// called on a reader created with a *ClassSignature* (such as the `signature` parameter of
    /// `ClassVisitor::visit`) or a *MethodSignature* (such as the `signature` parameter of
    /// `ClassVisitor::visit_method`).
    ///
    /// # Errors
    ///
    /// If the signature is malformed, or nests type arguments more than 256 levels deep.
    pub fn accept(
        &self,
        signature_visitor: &mut dyn SignatureVisitor,
    ) -> Result<(), SignatureError> {
        let signature = self.signature_value;
        let s = signature.as_bytes();
        let length = s.len();
        let char_at = |i: usize| s.get(i).copied().ok_or(SignatureError::Malformed(i));
        let mut offset; // Current offset in the parsed signature (parsed from left to right).
        let mut current_char; // The signature character at 'offset', or just before.

        // If the signature starts with '<', it starts with TypeParameters, i.e. a formal type
        // parameter identifier, followed by one or more pair ':',ReferenceTypeSignature (for its
        // class bound and interface bounds).
        if char_at(0)? == b'<' {
            // Invariant: offset points to the second character of a formal type parameter name at
            // the beginning of each iteration of the loop below.
            offset = 2;
            loop {
                // The formal type parameter name is everything between offset - 1 and the first
                // ':'.
                let class_bound_start_offset =
                    index_of(s, b':', offset).ok_or(SignatureError::Malformed(offset))?;
                let name = signature
                    .get(offset - 1..class_bound_start_offset)
                    .ok_or(SignatureError::Malformed(offset - 1))?;
                signature_visitor.visit_formal_type_parameter(name);

                // If the character after the ':' class bound marker is not the start of a
                // ReferenceTypeSignature, it means the class bound is empty (which is a valid
                // case).
                offset = class_bound_start_offset + 1;
                current_char = char_at(offset)?;
                if current_char == b'L' || current_char == b'[' || current_char == b'T' {
                    offset = parse_type(
                        signature,
                        offset,
                        signature_visitor.visit_class_bound(),
                        MAX_DEPTH,
                    )?;
                }

                // While the character after the class bound or after the last parsed interface
                // bound is ':', we need to parse another interface bound.
                loop {
                    current_char = char_at(offset)?;
                    offset += 1;
                    if current_char != b':' {
                        break;
                    }
                    offset = parse_type(
                        signature,
                        offset,
                        signature_visitor.visit_interface_bound(),
                        MAX_DEPTH,
                    )?;
                }

                // At this point a TypeParameter has been fully parsed, and we need to parse the
                // next one (note that current_char is now the first character of the next
                // TypeParameter, and that offset points to the second character), unless the
                // character just after this TypeParameter signals the end of the
                // TypeParameters.
                if current_char == b'>' {
                    break;
                }
            }
        } else {
            offset = 0;
        }

        // If the (optional) TypeParameters is followed by '(' this means we are parsing a
        // MethodSignature, which has JavaTypeSignature type inside parentheses, followed by a
        // Result type and optional ThrowsSignature types.
        if char_at(offset)? == b'(' {
            offset += 1;
            while char_at(offset)? != b')' {
                offset = parse_type(
                    signature,
                    offset,
                    signature_visitor.visit_parameter_type(),
                    MAX_DEPTH,
                )?;
            }
            // Use offset + 1 to skip ')'.
            offset = parse_type(
                signature,
                offset + 1,
                signature_visitor.visit_return_type(),
                MAX_DEPTH,
            )?;
            while offset < length {
                // Use offset + 1 to skip the first character of a ThrowsSignature, i.e. '^'.
                offset = parse_type(
                    signature,
                    offset + 1,
                    signature_visitor.visit_exception_type(),
                    MAX_DEPTH,
                )?;
            }
        } else {
            // Otherwise we are parsing a ClassSignature (by hypothesis on the method input), which
            // has one or more ClassTypeSignature for the super class and the implemented
            // interfaces.
            offset = parse_type(
                signature,
                offset,
                signature_visitor.visit_superclass(),
                MAX_DEPTH,
            )?;
            while offset < length {
                offset = parse_type(
                    signature,
                    offset,
                    signature_visitor.visit_interface(),
                    MAX_DEPTH,
                )?;
            }
        }
        Ok(())
    }

    /// Makes the given visitor visit the signature of this reader. This method is intended to be
    /// called on a reader created with a *JavaTypeSignature*, such as the `signature` parameter
    /// of `ClassVisitor::visit_field` or `MethodVisitor::visit_local_variable`.
    ///
    /// # Errors
    ///
    /// If the signature is malformed, or nests type arguments more than 256 levels deep.
    pub fn accept_type(
        &self,
        signature_visitor: &mut dyn SignatureVisitor,
    ) -> Result<(), SignatureError> {
        parse_type(self.signature_value, 0, signature_visitor, MAX_DEPTH).map(|_| ())
    }
}

/// Java's `String.indexOf(char, from)`, on bytes (`c` is ASCII, so byte offsets are char
/// boundaries).
fn index_of(s: &[u8], c: u8, from: usize) -> Option<usize> {
    s.get(from..)?
        .iter()
        .position(|&b| b == c)
        .map(|i| from + i)
}

/// Parses a JavaTypeSignature starting at `offset` and makes the given visitor visit it. Returns
/// the index of the first character after the parsed signature.
///
/// As in ASM 9.11, array types are parsed iteratively and only type arguments recurse, at most
/// `depth_limit` levels deep.
fn parse_type(
    signature: &JavaStr,
    mut offset: usize,
    mut visitor: &mut dyn SignatureVisitor,
    depth_limit: u32,
) -> Result<usize, SignatureError> {
    if depth_limit == 0 {
        return Err(SignatureError::TooManyNestedTypeArguments);
    }
    let s = signature.as_bytes();
    let char_at = |i: usize| s.get(i).copied().ok_or(SignatureError::Malformed(i));
    // Every name below starts after and ends before an ASCII delimiter, so it is on code point
    // boundaries; `get` checks it anyway.
    let name = |begin: usize, end: usize| {
        signature
            .get(begin..end)
            .ok_or(SignatureError::Malformed(begin))
    };
    loop {
        let mut current_char = char_at(offset)?; // The signature character at 'offset'.
        offset += 1;

        // Switch based on the first character of the JavaTypeSignature, which indicates its kind.
        match current_char {
            b'Z' | b'C' | b'B' | b'S' | b'I' | b'F' | b'J' | b'D' | b'V' => {
                // Case of a BaseType or a VoidDescriptor.
                visitor.visit_base_type(char::from(current_char));
                return Ok(offset);
            }

            b'[' => {
                // Case of an ArrayTypeSignature, a '[' followed by a JavaTypeSignature. Do another
                // iteration of the loop to parse the element type.
                visitor = visitor.visit_array_type();
            }

            b'T' => {
                // Case of TypeVariableSignature, an identifier between 'T' and ';'.
                let end_offset =
                    index_of(s, b';', offset).ok_or(SignatureError::Malformed(offset))?;
                visitor.visit_type_variable(name(offset, end_offset)?);
                return Ok(end_offset + 1);
            }

            b'L' => {
                // Case of a ClassTypeSignature, which ends with ';'.
                // These signatures have a main class type followed by zero or more inner class
                // types (separated by '.'). Each can have type arguments, inside '<' and '>'.
                let mut start = offset; // The start offset of the currently parsed class name.
                let mut visited = false; // Whether the current class name has been visited.
                let mut inner = false; // Whether we are currently parsing an inner class type.

                // Parses the signature, one character at a time.
                loop {
                    current_char = char_at(offset)?;
                    offset += 1;
                    if current_char == b'.' || current_char == b';' {
                        // If a '.' or ';' is encountered, this means we have fully parsed the main
                        // class name or an inner class name. This name may already have been
                        // visited it is was followed by type arguments between '<' and '>'. If
                        // not, we need to visit it here.
                        if !visited {
                            let class_name = name(start, offset - 1)?;
                            if inner {
                                visitor.visit_inner_class_type(class_name);
                            } else {
                                visitor.visit_class_type(class_name);
                            }
                        }
                        // If we reached the end of the ClassTypeSignature return, otherwise start
                        // the parsing of a new class name, which is necessarily an inner class
                        // name.
                        if current_char == b';' {
                            visitor.visit_end();
                            break;
                        }
                        start = offset;
                        visited = false;
                        inner = true;
                    } else if current_char == b'<' {
                        // If a '<' is encountered, this means we have fully parsed the main class
                        // name or an inner class name, and that we now need to parse
                        // TypeArguments. First, we need to visit the parsed class name.
                        let class_name = name(start, offset - 1)?;
                        if inner {
                            visitor.visit_inner_class_type(class_name);
                        } else {
                            visitor.visit_class_type(class_name);
                        }
                        visited = true;
                        // Now, parse the TypeArgument(s), one at a time.
                        loop {
                            current_char = char_at(offset)?;
                            if current_char == b'>' {
                                break;
                            }
                            match current_char {
                                b'*' => {
                                    // Unbounded TypeArgument.
                                    offset += 1;
                                    visitor.visit_type_argument();
                                }
                                b'+' | b'-' => {
                                    // Extends or Super TypeArgument. Use offset + 1 to skip the
                                    // '+' or '-'.
                                    offset = parse_type(
                                        signature,
                                        offset + 1,
                                        visitor
                                            .visit_type_argument_wildcard(char::from(current_char)),
                                        depth_limit - 1,
                                    )?;
                                }
                                _ => {
                                    // Instanceof TypeArgument. The '=' is implicit.
                                    offset = parse_type(
                                        signature,
                                        offset,
                                        visitor.visit_type_argument_wildcard('='),
                                        depth_limit - 1,
                                    )?;
                                }
                            }
                        }
                    }
                }
                return Ok(offset);
            }

            _ => return Err(SignatureError::Malformed(offset - 1)),
        }
    }
}
