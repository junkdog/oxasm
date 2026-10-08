//! A Java field or method type ([`Type`]): ASM's `Type`, which makes it easier to manipulate type
//! and method descriptors (ADR-15).
//!
//! A `Type` is a window into a descriptor buffer, as in Java. The buffer is either `'static` (the
//! primitive types, usable in `const`s) or an `Arc<JavaStr>` shared by every `Type` derived from
//! it ([`Type::get_argument_types`] does not copy the method descriptor). Cloning is cheap.
//!
//! Java's reflection entry points (`getType(Class)`, `getType(Method)`,
//! `getType(Constructor)`, `getDescriptor(Class)`, ...) are not ported.
//!
//! # Malformed descriptors
//!
//! Like ASM, the parsing methods do not validate descriptors: they read just enough of them to do
//! their job, and accept some malformed ones. Where ASM throws (an unknown type character, or a
//! descriptor that ends too early), the plain methods panic with ASM's message, `Invalid
//! descriptor: <descriptor>`, and the `try_` methods return [`InvalidDescriptor`]. Code that
//! handles descriptors from untrusted class files uses the `try_` methods.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::jstr::{JavaStr, JavaString};
use crate::opcodes;

/// The sort of a [`Type`] (ASM's `Type.VOID` ... `Type.METHOD`; the values are the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TypeSort {
    /// The sort of the `void` type.
    Void = 0,
    /// The sort of the `boolean` type.
    Boolean = 1,
    /// The sort of the `char` type.
    Char = 2,
    /// The sort of the `byte` type.
    Byte = 3,
    /// The sort of the `short` type.
    Short = 4,
    /// The sort of the `int` type.
    Int = 5,
    /// The sort of the `float` type.
    Float = 6,
    /// The sort of the `long` type.
    Long = 7,
    /// The sort of the `double` type.
    Double = 8,
    /// The sort of array reference types.
    Array = 9,
    /// The sort of object reference types.
    Object = 10,
    /// The sort of method types.
    Method = 11,
}

/// The error of the `try_` methods of [`Type`]: the descriptor is malformed where ASM would throw.
///
/// Its message is that of ASM's exception: `Invalid descriptor: <the whole descriptor>` for an
/// invalid type character (an `IllegalArgumentException`), or Java's `Index <i> out of bounds
/// for length <n>` for a descriptor that ends too early (the `StringIndexOutOfBoundsException`
/// of `charAt`, with `i` and `n` in UTF-16 units). See [`InvalidDescriptor::message`] for the
/// exact text, which may have unpaired surrogates. The panicking methods of [`Type`] panic with
/// [`InvalidDescriptor::exception`], the `toString()` of ASM's exception (ADR-06).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub struct InvalidDescriptor {
    /// The whole descriptor (the buffer being parsed, as in ASM's message).
    pub descriptor: JavaString,
    /// For a descriptor that ends too early, the UTF-16 index past its end that ASM reads.
    pub out_of_bounds: Option<usize>,
}

impl InvalidDescriptor {
    /// The `toString()` of ASM's exception: its class, then its message.
    #[must_use]
    pub fn exception(&self) -> JavaString {
        let mut exception = JavaString::from(match self.out_of_bounds {
            Some(_) => "java.lang.StringIndexOutOfBoundsException: ",
            None => "java.lang.IllegalArgumentException: ",
        });
        exception.push_java_str(&self.message());
        exception
    }

    /// Panics as ASM throws (ADR-06): with the `toString()` of ASM's exception.
    #[track_caller]
    fn throw(&self) -> ! {
        panic!("{}", self.exception())
    }

    /// The message of ASM's exception.
    #[must_use]
    pub fn message(&self) -> JavaString {
        match self.out_of_bounds {
            Some(index) => JavaString::from(format!(
                "Index {index} out of bounds for length {}",
                self.descriptor.encode_utf16().count()
            )),
            None => {
                let mut message = JavaString::from("Invalid descriptor: ");
                message.push_java_str(&self.descriptor);
                message
            }
        }
    }
}

/// A Java field or method type.
///
/// Equality and hashing compare the sort and the value (the internal name of object types, the
/// descriptor of the others), as `Type.equals` does: `Type::get_object_type("I")` equals
/// `Type::get_type("LI;")`.
#[derive(Clone)]
pub struct Type {
    /// The sort of this type. `Object` both for types created from a descriptor and from an
    /// internal name; `internal` tells the two apart (ASM's private `INTERNAL` sort).
    sort: TypeSort,
    /// Whether this object type was created from an internal name (ASM's `INTERNAL` sort): its
    /// buffer holds no descriptor around the value.
    internal: bool,
    /// A buffer containing the value of this field or method type. This value is an internal name
    /// for object types, and a field or method descriptor in the other cases.
    ///
    /// For object types created from a descriptor, this buffer also contains the descriptor: the
    /// bytes in `begin..end` contain the internal name, and those in `begin - 1..end + 1` contain
    /// the descriptor.
    buf: TypeBuf,
    /// The beginning index, inclusive, of the value of this type in `buf`.
    begin: usize,
    /// The end index, exclusive, of the value of this type in `buf`.
    end: usize,
}

#[derive(Clone)]
enum TypeBuf {
    Static(&'static JavaStr),
    Shared(Arc<JavaStr>),
}

impl TypeBuf {
    #[inline]
    fn as_java_str(&self) -> &JavaStr {
        match self {
            TypeBuf::Static(s) => s,
            TypeBuf::Shared(s) => s,
        }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.as_java_str().as_bytes()
    }
}

/// The descriptors of the primitive types.
const PRIMITIVE_DESCRIPTORS: &JavaStr = crate::jstr!("VZCBSIFJD");

const fn primitive(sort: TypeSort) -> Type {
    let index = sort as usize;
    Type {
        sort,
        internal: false,
        buf: TypeBuf::Static(PRIMITIVE_DESCRIPTORS),
        begin: index,
        end: index + 1,
    }
}

impl Type {
    /// The `void` type.
    pub const VOID: Type = primitive(TypeSort::Void);
    /// The `boolean` type.
    pub const BOOLEAN: Type = primitive(TypeSort::Boolean);
    /// The `char` type.
    pub const CHAR: Type = primitive(TypeSort::Char);
    /// The `byte` type.
    pub const BYTE: Type = primitive(TypeSort::Byte);
    /// The `short` type.
    pub const SHORT: Type = primitive(TypeSort::Short);
    /// The `int` type.
    pub const INT: Type = primitive(TypeSort::Int);
    /// The `float` type.
    pub const FLOAT: Type = primitive(TypeSort::Float);
    /// The `long` type.
    pub const LONG: Type = primitive(TypeSort::Long);
    /// The `double` type.
    pub const DOUBLE: Type = primitive(TypeSort::Double);

    // ---------------------------------------------------------------------------------------------
    // Methods to get Type(s) from a descriptor, other types, etc.
    // ---------------------------------------------------------------------------------------------

    /// Returns the type corresponding to the given field or method type descriptor.
    ///
    /// # Panics
    ///
    /// If the descriptor does not start with a type character (see [`Type::try_get_type`]).
    #[must_use]
    #[track_caller]
    pub fn get_type(type_descriptor: impl AsRef<JavaStr>) -> Type {
        Type::try_get_type(type_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the given field or method type descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor does not start with a type character.
    pub fn try_get_type(type_descriptor: impl AsRef<JavaStr>) -> Result<Type, InvalidDescriptor> {
        let type_descriptor = type_descriptor.as_ref();
        // A primitive type needs no copy of the descriptor.
        if let Some(primitive) = type_descriptor
            .as_bytes()
            .first()
            .and_then(|&c| primitive_of(c))
        {
            return Ok(primitive);
        }
        let buf = shared(type_descriptor);
        let end = buf.bytes().len();
        get_type_internal(&buf, 0, end)
    }

    /// Returns the type of the elements of this array type. This method should only be used for
    /// an array type.
    ///
    /// # Panics
    ///
    /// If this type is a malformed array type such as `[` (see [`Type::try_get_element_type`]).
    #[must_use]
    #[track_caller]
    pub fn get_element_type(&self) -> Type {
        self.try_get_element_type().unwrap_or_else(|e| e.throw())
    }

    /// Returns the type of the elements of this array type. This method should only be used for
    /// an array type.
    ///
    /// # Errors
    ///
    /// If the element type descriptor is missing or invalid.
    pub fn try_get_element_type(&self) -> Result<Type, InvalidDescriptor> {
        let num_dimensions = self.get_dimensions();
        get_type_internal(&self.buf, self.begin + num_dimensions, self.end)
    }

    /// Returns the type corresponding to the given internal name (see
    /// [`Type::get_internal_name`]). A name starting with `[` gives an array type.
    #[must_use]
    pub fn get_object_type(internal_name: impl AsRef<JavaStr>) -> Type {
        let buf = shared(internal_name.as_ref());
        let end = buf.bytes().len();
        let is_array = buf.bytes().first() == Some(&b'[');
        Type {
            sort: if is_array {
                TypeSort::Array
            } else {
                TypeSort::Object
            },
            internal: !is_array,
            buf,
            begin: 0,
            end,
        }
    }

    /// [`Type::get_object_type`] of a `'static` internal name, usable in a `const` (no
    /// allocation): `Type::static_object_type(jstr!("java/lang/Object"))`.
    #[must_use]
    pub const fn static_object_type(internal_name: &'static JavaStr) -> Type {
        let bytes = internal_name.as_bytes();
        let is_array = !bytes.is_empty() && bytes[0] == b'[';
        Type {
            sort: if is_array {
                TypeSort::Array
            } else {
                TypeSort::Object
            },
            internal: !is_array,
            buf: TypeBuf::Static(internal_name),
            begin: 0,
            end: bytes.len(),
        }
    }

    /// Returns the type corresponding to the given method descriptor. Equivalent to
    /// `Type::get_type(method_descriptor)` for valid descriptors, but does not look at the
    /// descriptor at all.
    #[must_use]
    pub fn get_method_type(method_descriptor: impl AsRef<JavaStr>) -> Type {
        let buf = shared(method_descriptor.as_ref());
        let end = buf.bytes().len();
        Type {
            sort: TypeSort::Method,
            internal: false,
            buf,
            begin: 0,
            end,
        }
    }

    /// Returns the method type corresponding to the given argument and return types (Java's
    /// `getMethodType(Type, Type...)`).
    #[must_use]
    pub fn get_method_type_of(return_type: &Type, argument_types: &[Type]) -> Type {
        Type::get_type(Type::get_method_descriptor(return_type, argument_types))
    }

    /// Returns the argument types of methods of this type. This method should only be used for
    /// method types. The returned types share this type's buffer.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_argument_types_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_argument_types(&self) -> Vec<Type> {
        let descriptor = self.descriptor_buf();
        argument_types(&descriptor, 0).unwrap_or_else(|e| e.throw())
    }

    /// Returns the types corresponding to the argument types of the given method descriptor.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_argument_types_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_argument_types_of(method_descriptor: impl AsRef<JavaStr>) -> Vec<Type> {
        Type::try_get_argument_types_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the types corresponding to the argument types of the given method descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its `)`, or an argument type character is invalid.
    pub fn try_get_argument_types_of(
        method_descriptor: impl AsRef<JavaStr>,
    ) -> Result<Vec<Type>, InvalidDescriptor> {
        argument_types(&shared(method_descriptor.as_ref()), 0)
    }

    /// Returns the return type of methods of this type. This method should only be used for
    /// method types.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_return_type_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_return_type(&self) -> Type {
        let descriptor = self.descriptor_buf();
        return_type(&descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the return type of the given method descriptor.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_return_type_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_return_type_of(method_descriptor: impl AsRef<JavaStr>) -> Type {
        Type::try_get_return_type_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the return type of the given method descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its return type, or the return type is invalid.
    pub fn try_get_return_type_of(
        method_descriptor: impl AsRef<JavaStr>,
    ) -> Result<Type, InvalidDescriptor> {
        let method_descriptor = method_descriptor.as_ref();
        // A primitive return type needs no copy of the descriptor.
        let offset = Type::get_return_type_offset(method_descriptor)?;
        if let Some(primitive) = method_descriptor
            .as_bytes()
            .get(offset)
            .and_then(|&c| primitive_of(c))
        {
            return Ok(primitive);
        }
        get_type_internal(
            &shared(method_descriptor),
            offset,
            method_descriptor.as_bytes().len(),
        )
    }

    /// Returns the start index of the return type of the given method descriptor.
    pub(crate) fn get_return_type_offset(
        method_descriptor: &JavaStr,
    ) -> Result<usize, InvalidDescriptor> {
        let d = method_descriptor.as_bytes();
        let past_end = |offset| out_of_bounds(method_descriptor, offset);
        // Skip the first character, which is always a '('.
        let mut current_offset = char_len(d, 0);
        // Skip the argument types, one at a each loop iteration.
        while *d
            .get(current_offset)
            .ok_or_else(|| past_end(current_offset))?
            != b')'
        {
            current_offset = skip_argument(d, current_offset).map_err(past_end)?;
        }
        Ok(current_offset + 1)
    }

    // ---------------------------------------------------------------------------------------------
    // Methods to get class names, internal names or descriptors.
    // ---------------------------------------------------------------------------------------------

    /// Returns the binary name of the class corresponding to this type (`java.lang.Object`,
    /// `int[]`, ...). This method must not be used on method types.
    ///
    /// # Panics
    ///
    /// On a method type (ASM's `AssertionError`, ADR-06), or a malformed array type.
    #[must_use]
    #[track_caller]
    pub fn get_class_name(&self) -> JavaString {
        let name = match self.sort {
            TypeSort::Void => "void",
            TypeSort::Boolean => "boolean",
            TypeSort::Char => "char",
            TypeSort::Byte => "byte",
            TypeSort::Short => "short",
            TypeSort::Int => "int",
            TypeSort::Float => "float",
            TypeSort::Long => "long",
            TypeSort::Double => "double",
            TypeSort::Array => {
                let mut class_name = self.get_element_type().get_class_name();
                for _ in 0..self.get_dimensions() {
                    class_name.push_str("[]");
                }
                return class_name;
            }
            TypeSort::Object => return self.value().replace('/', '.'),
            TypeSort::Method => panic!("java.lang.AssertionError"),
        };
        JavaString::from(name)
    }

    /// Returns the internal name of the class corresponding to this object or array type. The
    /// internal name of a class is its fully qualified name, where '.' are replaced by '/'. This
    /// method should only be used for an object or array type (for others, it returns the
    /// descriptor).
    #[inline]
    #[must_use]
    pub fn get_internal_name(&self) -> &JavaStr {
        self.value()
    }

    /// Returns the descriptor corresponding to this type. Borrows, except for object types
    /// created from an internal name.
    #[must_use]
    pub fn get_descriptor(&self) -> Cow<'_, JavaStr> {
        if self.internal {
            let mut descriptor = JavaString::with_capacity(self.end - self.begin + 2);
            self.append_descriptor(&mut descriptor);
            Cow::Owned(descriptor)
        } else if self.sort == TypeSort::Object {
            Cow::Borrowed(self.object_descriptor())
        } else {
            Cow::Borrowed(self.value())
        }
    }

    /// Returns the descriptor corresponding to the given argument and return types.
    #[must_use]
    pub fn get_method_descriptor(return_type: &Type, argument_types: &[Type]) -> JavaString {
        let mut descriptor = JavaString::new();
        descriptor.push('(');
        for argument_type in argument_types {
            argument_type.append_descriptor(&mut descriptor);
        }
        descriptor.push(')');
        return_type.append_descriptor(&mut descriptor);
        descriptor
    }

    /// Appends the descriptor corresponding to this type to the given string.
    pub fn append_descriptor(&self, out: &mut JavaString) {
        if self.internal {
            out.push('L');
            out.push_java_str(self.value());
            out.push(';');
        } else if self.sort == TypeSort::Object {
            out.push_java_str(self.object_descriptor());
        } else {
            out.push_java_str(self.value());
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Methods to get the sort, dimension, size, and opcodes corresponding to a Type or descriptor.
    // ---------------------------------------------------------------------------------------------

    /// Returns the sort of this type.
    #[inline]
    #[must_use]
    pub fn get_sort(&self) -> TypeSort {
        self.sort
    }

    /// Returns the number of dimensions of this array type. This method should only be used for
    /// an array type.
    #[must_use]
    pub fn get_dimensions(&self) -> usize {
        let value = &self.buf.bytes()[self.begin..];
        let mut num_dimensions = 1;
        while value.get(num_dimensions) == Some(&b'[') {
            num_dimensions += 1;
        }
        num_dimensions
    }

    /// Returns the size of values of this type: 2 for `long` and `double`, 0 for `void` and 1
    /// otherwise. This method must not be used for method types.
    ///
    /// # Panics
    ///
    /// On a method type (ASM's `AssertionError`, ADR-06).
    #[must_use]
    #[track_caller]
    pub fn get_size(&self) -> usize {
        match self.sort {
            TypeSort::Void => 0,
            TypeSort::Long | TypeSort::Double => 2,
            TypeSort::Method => panic!("java.lang.AssertionError"),
            _ => 1,
        }
    }

    /// Returns the number of arguments of this method type. Each argument counts for 1, even
    /// `long` and `double` ones. The implicit `this` argument is not counted. This method should
    /// only be used for method types.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor.
    #[must_use]
    #[track_caller]
    pub fn get_argument_count(&self) -> usize {
        Type::get_argument_count_of(&*self.get_descriptor())
    }

    /// Returns the number of arguments in the given method descriptor. Each argument counts for 1,
    /// even `long` and `double` ones. The implicit `this` argument is not counted.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_argument_count_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_argument_count_of(method_descriptor: impl AsRef<JavaStr>) -> usize {
        Type::try_get_argument_count_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the number of arguments in the given method descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its `)`.
    pub fn try_get_argument_count_of(
        method_descriptor: impl AsRef<JavaStr>,
    ) -> Result<usize, InvalidDescriptor> {
        let method_descriptor = method_descriptor.as_ref();
        let d = method_descriptor.as_bytes();
        let past_end = |offset| out_of_bounds(method_descriptor, offset);
        let mut argument_count = 0;
        // Skip the first character, which is always a '('.
        let mut current_offset = char_len(d, 0);
        // Parse the argument types, one at a each loop iteration.
        while *d
            .get(current_offset)
            .ok_or_else(|| past_end(current_offset))?
            != b')'
        {
            current_offset = skip_argument(d, current_offset).map_err(past_end)?;
            argument_count += 1;
        }
        Ok(argument_count)
    }

    /// Returns the size of the arguments and of the return value of methods of this type. This
    /// method should only be used for method types.
    ///
    /// The result packs the size of the arguments (plus one for the implicit `this` argument),
    /// `arguments_size`, and the size of the return value, `return_size`, into a single value
    /// `(arguments_size << 2) | return_size`. `long` and `double` values have size 2, the others
    /// size 1 (and `void` 0).
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor.
    #[must_use]
    #[track_caller]
    pub fn get_arguments_and_return_sizes(&self) -> u32 {
        Type::get_arguments_and_return_sizes_of(&*self.get_descriptor())
    }

    /// Computes the size of the arguments and of the return value of a method, packed as in
    /// [`Type::get_arguments_and_return_sizes`].
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_arguments_and_return_sizes_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_arguments_and_return_sizes_of(method_descriptor: impl AsRef<JavaStr>) -> u32 {
        Type::try_get_arguments_and_return_sizes_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Computes the size of the arguments and of the return value of a method, packed as in
    /// [`Type::get_arguments_and_return_sizes`].
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its return type.
    pub fn try_get_arguments_and_return_sizes_of(
        method_descriptor: impl AsRef<JavaStr>,
    ) -> Result<u32, InvalidDescriptor> {
        let method_descriptor = method_descriptor.as_ref();
        let d = method_descriptor.as_bytes();
        let past_end = |offset| out_of_bounds(method_descriptor, offset);
        let at = |i: usize| d.get(i).copied().ok_or_else(|| past_end(i));
        let mut arguments_size: u32 = 1;
        // Skip the first character, which is always a '('.
        let mut current_offset = char_len(d, 0);
        let mut current_char = at(current_offset)?;
        // Parse the argument types and compute their size, one at a each loop iteration.
        while current_char != b')' {
            if current_char == b'J' || current_char == b'D' {
                current_offset += 1;
                arguments_size += 2;
            } else {
                current_offset = skip_argument(d, current_offset).map_err(past_end)?;
                arguments_size += 1;
            }
            current_char = at(current_offset)?;
        }
        current_char = at(current_offset + 1)?;
        if current_char == b'V' {
            Ok(arguments_size << 2)
        } else {
            let return_size = if current_char == b'J' || current_char == b'D' {
                2
            } else {
                1
            };
            Ok((arguments_size << 2) | return_size)
        }
    }

    /// Returns a JVM instruction opcode adapted to this type. This method must not be used for
    /// method types.
    ///
    /// `opcode` must be one of `ILOAD`, `ISTORE`, `IALOAD`, `IASTORE`, `IADD`, `ISUB`, `IMUL`,
    /// `IDIV`, `IREM`, `INEG`, `ISHL`, `ISHR`, `IUSHR`, `IAND`, `IOR`, `IXOR` and `IRETURN`. The
    /// result is similar, but adapted to this type: for example, if this type is `float` and
    /// `opcode` is `IRETURN`, this method returns `FRETURN`.
    ///
    /// # Panics
    ///
    /// If the opcode is not supported for this type, as ASM throws an
    /// `UnsupportedOperationException` (with its `toString()`, ADR-06); see
    /// [`Type::checked_opcode`].
    #[must_use]
    #[track_caller]
    pub fn get_opcode(&self, opcode: u8) -> u8 {
        self.checked_opcode(opcode)
            .unwrap_or_else(|| panic!("java.lang.UnsupportedOperationException"))
    }

    /// [`Type::get_opcode`], with `None` where ASM throws `UnsupportedOperationException`.
    #[must_use]
    pub fn checked_opcode(&self, opcode: u8) -> Option<u8> {
        use opcodes::*;
        if opcode == IALOAD || opcode == IASTORE {
            match self.sort {
                TypeSort::Boolean | TypeSort::Byte => Some(opcode + (BALOAD - IALOAD)),
                TypeSort::Char => Some(opcode + (CALOAD - IALOAD)),
                TypeSort::Short => Some(opcode + (SALOAD - IALOAD)),
                TypeSort::Int => Some(opcode),
                TypeSort::Float => Some(opcode + (FALOAD - IALOAD)),
                TypeSort::Long => Some(opcode + (LALOAD - IALOAD)),
                TypeSort::Double => Some(opcode + (DALOAD - IALOAD)),
                TypeSort::Array | TypeSort::Object => Some(opcode + (AALOAD - IALOAD)),
                TypeSort::Method | TypeSort::Void => None,
            }
        } else {
            match self.sort {
                TypeSort::Void => (opcode == IRETURN).then_some(RETURN),
                TypeSort::Boolean
                | TypeSort::Byte
                | TypeSort::Char
                | TypeSort::Short
                | TypeSort::Int => Some(opcode),
                TypeSort::Float => opcode.checked_add(FRETURN - IRETURN),
                TypeSort::Long => opcode.checked_add(LRETURN - IRETURN),
                TypeSort::Double => opcode.checked_add(DRETURN - IRETURN),
                TypeSort::Array | TypeSort::Object => {
                    if opcode != ILOAD && opcode != ISTORE && opcode != IRETURN {
                        None
                    } else {
                        Some(opcode + (ARETURN - IRETURN))
                    }
                }
                TypeSort::Method => None,
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Private helpers
    // ---------------------------------------------------------------------------------------------

    /// The value: an internal name for object types, a descriptor otherwise.
    #[inline]
    fn value(&self) -> &JavaStr {
        &self.buf.as_java_str()[self.begin..self.end]
    }

    /// The descriptor around the value of an object type created from a descriptor: one
    /// character before and after it (normally `L` and `;`).
    fn object_descriptor(&self) -> &JavaStr {
        let s = self.buf.as_java_str();
        let last = s[self.end..]
            .chars()
            .next()
            .map_or(0, |c| c.len_semi_utf8());
        &s[self.begin - 1..self.end + last]
    }

    /// This method type's descriptor as a shareable buffer (the whole buffer when it is exactly
    /// the descriptor, which is always the case for method types built from a descriptor).
    fn descriptor_buf(&self) -> TypeBuf {
        if !self.internal && self.begin == 0 && self.end == self.buf.bytes().len() {
            self.buf.clone()
        } else {
            shared(&self.get_descriptor())
        }
    }
}

/// Copies a descriptor into a shared buffer.
fn shared(s: &JavaStr) -> TypeBuf {
    TypeBuf::Shared(Arc::from(s))
}

fn invalid(descriptor: &JavaStr) -> InvalidDescriptor {
    InvalidDescriptor {
        descriptor: descriptor.to_owned(),
        out_of_bounds: None,
    }
}

/// The error of reading the byte at `offset` of `descriptor`, which is past its end (Java's
/// `charAt`, whose index counts UTF-16 units).
fn out_of_bounds(descriptor: &JavaStr, offset: usize) -> InvalidDescriptor {
    let length = descriptor.len();
    let units = descriptor
        .get(0..offset.min(length))
        .map_or(0, |prefix| prefix.encode_utf16().count());
    InvalidDescriptor {
        descriptor: descriptor.to_owned(),
        out_of_bounds: Some(units + offset.saturating_sub(length)),
    }
}

/// The length in bytes of the character at `offset` of a semi-UTF-8 descriptor, 1 past its end:
/// where ASM's descriptor parsing skips one `char`, a malformed descriptor may have a non-ASCII
/// character. This is exact for the characters of the Basic Multilingual Plane; ASM sees a
/// supplementary character as two `char`s, which this skips at once.
fn char_len(d: &[u8], offset: usize) -> usize {
    match d.get(offset) {
        Some(0xf0..) => 4,
        Some(0xe0..) => 3,
        Some(0xc0..) => 2,
        _ => 1,
    }
}

/// Skips one argument type of a method descriptor starting at `offset`, the way ASM does: any
/// `[`s, then one character, then (after an `L`) everything up to the next `;`. Fails with the
/// offset past the end that ASM would read.
fn skip_argument(d: &[u8], mut offset: usize) -> Result<usize, usize> {
    while *d.get(offset).ok_or(offset)? == b'[' {
        offset += 1;
    }
    let c = *d.get(offset).ok_or(offset)?;
    offset += char_len(d, offset);
    if c == b'L' {
        // Skip the argument descriptor content. Like ASM's `Math.max(currentOffset,
        // indexOf(';') + 1)`, a missing ';' leaves the offset where it is.
        if let Some(semi_column) = d[offset..].iter().position(|&b| b == b';') {
            offset += semi_column + 1;
        }
    }
    Ok(offset)
}

fn argument_types(buf: &TypeBuf, base: usize) -> Result<Vec<Type>, InvalidDescriptor> {
    let descriptor = &buf.as_java_str()[base..];
    let d = descriptor.as_bytes();
    let past_end = |offset| out_of_bounds(descriptor, offset);
    // First step: compute the number of argument types in methodDescriptor (which, like ASM,
    // reports a descriptor that ends too early before an invalid argument type).
    let num_argument_types = Type::try_get_argument_count_of(descriptor)?;

    // Second step: create a Type instance for each argument type.
    let mut argument_types = Vec::with_capacity(num_argument_types);
    // Skip the first character, which is always a '('.
    let mut current_offset = char_len(d, 0);
    // Parse and create the argument types, one at each loop iteration.
    while *d
        .get(current_offset)
        .ok_or_else(|| past_end(current_offset))?
        != b')'
    {
        let current_argument_type_offset = current_offset;
        current_offset = skip_argument(d, current_offset).map_err(past_end)?;
        argument_types.push(get_type_internal(
            buf,
            base + current_argument_type_offset,
            base + current_offset,
        )?);
    }
    Ok(argument_types)
}

fn return_type(buf: &TypeBuf) -> Result<Type, InvalidDescriptor> {
    let offset = Type::get_return_type_offset(buf.as_java_str())?;
    get_type_internal(buf, offset, buf.bytes().len())
}

/// The primitive type of the given descriptor character, if it is one.
#[inline]
fn primitive_of(c: u8) -> Option<Type> {
    match c {
        b'V' => Some(Type::VOID),
        b'Z' => Some(Type::BOOLEAN),
        b'C' => Some(Type::CHAR),
        b'B' => Some(Type::BYTE),
        b'S' => Some(Type::SHORT),
        b'I' => Some(Type::INT),
        b'F' => Some(Type::FLOAT),
        b'J' => Some(Type::LONG),
        b'D' => Some(Type::DOUBLE),
        _ => None,
    }
}

/// Returns the type corresponding to the descriptor in `buf[begin..end]`.
fn get_type_internal(buf: &TypeBuf, begin: usize, end: usize) -> Result<Type, InvalidDescriptor> {
    let invalid = || invalid(buf.as_java_str());
    if begin >= buf.bytes().len() {
        return Err(out_of_bounds(buf.as_java_str(), begin));
    }
    let sort = match buf.bytes().get(begin).filter(|_| begin < end) {
        Some(b'V') => return Ok(Type::VOID),
        Some(b'Z') => return Ok(Type::BOOLEAN),
        Some(b'C') => return Ok(Type::CHAR),
        Some(b'B') => return Ok(Type::BYTE),
        Some(b'S') => return Ok(Type::SHORT),
        Some(b'I') => return Ok(Type::INT),
        Some(b'F') => return Ok(Type::FLOAT),
        Some(b'J') => return Ok(Type::LONG),
        Some(b'D') => return Ok(Type::DOUBLE),
        Some(b'[') => TypeSort::Array,
        Some(b'L') => TypeSort::Object,
        Some(b'(') => TypeSort::Method,
        _ => return Err(invalid()),
    };
    let (begin, end) = if sort == TypeSort::Object {
        // The internal name, without the 'L' and the final character (normally ';'). ASM drops
        // one UTF-16 unit; on a malformed, non-ASCII end this drops one whole code point.
        let s = buf.as_java_str();
        let mut value_end = end - 1;
        while !s.is_char_boundary(value_end) {
            value_end -= 1;
        }
        if value_end < begin + 1 {
            return Err(invalid());
        }
        (begin + 1, value_end)
    } else {
        (begin, end)
    };
    Ok(Type {
        sort,
        internal: false,
        buf: buf.clone(),
        begin,
        end,
    })
}

// -------------------------------------------------------------------------------------------------
// Equals, hashCode and toString.
// -------------------------------------------------------------------------------------------------

impl PartialEq for Type {
    fn eq(&self, other: &Type) -> bool {
        self.sort == other.sort && self.value() == other.value()
    }
}

impl Eq for Type {}

impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sort.hash(state);
        self.value().hash(state);
    }
}

impl fmt::Display for Type {
    /// Writes the descriptor of this type.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.get_descriptor(), f)
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Type({:?})", &*self.get_descriptor())
    }
}
