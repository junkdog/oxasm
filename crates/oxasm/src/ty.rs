//! A Java field or method type ([`Type`]): ASM's `Type`, which makes it easier to manipulate type
//! and method descriptors (ADR-15).
//!
//! A `Type<'a>` is a window into a descriptor buffer, as in Java. The buffer is either borrowed
//! for `'a` (the caller's descriptor, or a `'static` one such as the primitive types' in `const`s)
//! or an `Arc<JavaStr>` shared by every `Type` derived from it. Parsing a descriptor borrows it
//! instead of copying it, and the types derived from a type ([`Type::get_argument_types`],
//! [`Type::get_element_type`], ...) share its buffer. Cloning is cheap.
//!
//! A `Type` that must outlive its descriptor is made owned with [`Type::into_owned`], which copies
//! the descriptor once: `Type<'static>` is the type to store.
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
///
/// The lifetime is the descriptor buffer's: see the [module documentation](self).
#[derive(Clone)]
pub struct Type<'a> {
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
    buf: TypeBuf<'a>,
    /// The beginning index, inclusive, of the value of this type in `buf`.
    begin: usize,
    /// The end index, exclusive, of the value of this type in `buf`.
    end: usize,
}

#[derive(Clone)]
enum TypeBuf<'a> {
    /// A `'static` buffer, which [`Type::into_owned`] keeps.
    Static(&'static JavaStr),
    Borrowed(&'a JavaStr),
    Shared(Arc<JavaStr>),
}

impl TypeBuf<'_> {
    #[inline]
    fn as_java_str(&self) -> &JavaStr {
        match self {
            TypeBuf::Static(s) | TypeBuf::Borrowed(s) => s,
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

const fn primitive(sort: TypeSort) -> Type<'static> {
    let index = sort as usize;
    Type {
        sort,
        internal: false,
        buf: TypeBuf::Static(PRIMITIVE_DESCRIPTORS),
        begin: index,
        end: index + 1,
    }
}

impl Type<'static> {
    /// The `void` type.
    pub const VOID: Type<'static> = primitive(TypeSort::Void);
    /// The `boolean` type.
    pub const BOOLEAN: Type<'static> = primitive(TypeSort::Boolean);
    /// The `char` type.
    pub const CHAR: Type<'static> = primitive(TypeSort::Char);
    /// The `byte` type.
    pub const BYTE: Type<'static> = primitive(TypeSort::Byte);
    /// The `short` type.
    pub const SHORT: Type<'static> = primitive(TypeSort::Short);
    /// The `int` type.
    pub const INT: Type<'static> = primitive(TypeSort::Int);
    /// The `float` type.
    pub const FLOAT: Type<'static> = primitive(TypeSort::Float);
    /// The `long` type.
    pub const LONG: Type<'static> = primitive(TypeSort::Long);
    /// The `double` type.
    pub const DOUBLE: Type<'static> = primitive(TypeSort::Double);

    /// [`Type::get_object_type`] of a `'static` internal name, usable in a `const` (no
    /// allocation): `Type::static_object_type(jstr!("java/lang/Object"))`.
    #[must_use]
    pub const fn static_object_type(internal_name: &'static JavaStr) -> Type<'static> {
        object_type(internal_name)
    }

    /// Returns the method type corresponding to the given argument and return types (Java's
    /// `getMethodType(Type, Type...)`).
    #[must_use]
    pub fn get_method_type_of(
        return_type: &Type<'_>,
        argument_types: &[Type<'_>],
    ) -> Type<'static> {
        // The descriptor is written once, in place (ASM builds a `String`, which `getType` wraps).
        let end = method_descriptor_len(return_type, argument_types);
        let mut bytes = Arc::<[u8]>::new_uninit_slice(end);
        let mut rest = &mut Arc::get_mut(&mut bytes).expect("a new Arc is unique")[..];
        let mut put = |part: &[u8]| {
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(part.len());
            head.write_copy_of_slice(part);
            rest = tail;
        };
        put(b"(");
        for argument_type in argument_types {
            argument_type
                .descriptor_parts()
                .into_iter()
                .for_each(&mut put);
        }
        put(b")");
        return_type
            .descriptor_parts()
            .into_iter()
            .for_each(&mut put);
        assert!(rest.is_empty());
        // SAFETY: the parts, whose lengths sum to the slice's, initialized every byte.
        let bytes = unsafe { bytes.assume_init() };
        // SAFETY: `JavaStr` is `repr(transparent)` over `[u8]`, so `Arc<[u8]>` and `Arc<JavaStr>`
        // have the same layout and metadata; the bytes are canonical semi-UTF-8, a concatenation
        // of ASCII and of whole descriptors (or internal names) of other types.
        let descriptor = unsafe { Arc::from_raw(Arc::into_raw(bytes) as *const JavaStr) };
        Type {
            sort: TypeSort::Method,
            internal: false,
            buf: TypeBuf::Shared(descriptor),
            begin: 0,
            end,
        }
    }
}

impl<'a> Type<'a> {
    // ---------------------------------------------------------------------------------------------
    // Methods to get Type(s) from a descriptor, other types, etc.
    // ---------------------------------------------------------------------------------------------

    /// Returns the type corresponding to the given field or method type descriptor. The type
    /// borrows the descriptor.
    ///
    /// # Panics
    ///
    /// If the descriptor does not start with a type character (see [`Type::try_get_type`]).
    #[must_use]
    #[track_caller]
    pub fn get_type<S: AsRef<JavaStr> + ?Sized>(type_descriptor: &'a S) -> Type<'a> {
        Type::try_get_type(type_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the given field or method type descriptor. The type
    /// borrows the descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor does not start with a type character.
    pub fn try_get_type<S: AsRef<JavaStr> + ?Sized>(
        type_descriptor: &'a S,
    ) -> Result<Type<'a>, InvalidDescriptor> {
        let type_descriptor = type_descriptor.as_ref();
        // The common cases, inline: an object type whose descriptor ends with an ASCII character
        // (normally its `;`), and array and method types, whose value is the whole descriptor.
        let (sort, begin, end) = match type_descriptor.as_bytes() {
            [b'L', .., last] if last.is_ascii() => (TypeSort::Object, 1, type_descriptor.len() - 1),
            [b'[', ..] => (TypeSort::Array, 0, type_descriptor.len()),
            [b'(', ..] => (TypeSort::Method, 0, type_descriptor.len()),
            _ => {
                let buf = TypeBuf::Borrowed(type_descriptor);
                return get_type_internal(&buf, 0, type_descriptor.len());
            }
        };
        Ok(Type {
            sort,
            internal: false,
            buf: TypeBuf::Borrowed(type_descriptor),
            begin,
            end,
        })
    }

    /// This type, owning its descriptor: a borrowed buffer is copied (only the part this type
    /// uses), a shared one is kept.
    #[must_use]
    pub fn into_owned(self) -> Type<'static> {
        let Type {
            sort,
            internal,
            buf,
            begin,
            end,
        } = self;
        match buf {
            TypeBuf::Static(buf) => Type {
                sort,
                internal,
                buf: TypeBuf::Static(buf),
                begin,
                end,
            },
            TypeBuf::Shared(buf) => Type {
                sort,
                internal,
                buf: TypeBuf::Shared(buf),
                begin,
                end,
            },
            TypeBuf::Borrowed(s) => {
                // An object type created from a descriptor keeps the characters around its value.
                let (from, to) = if sort == TypeSort::Object && !internal {
                    (begin - 1, end + next_char_len(s, end))
                } else {
                    (begin, end)
                };
                Type {
                    sort,
                    internal,
                    buf: TypeBuf::Shared(Arc::from(&s[from..to])),
                    begin: begin - from,
                    end: end - from,
                }
            }
        }
    }

    /// Returns the type of the elements of this array type. This method should only be used for
    /// an array type. The returned type shares this type's buffer.
    ///
    /// # Panics
    ///
    /// If this type is a malformed array type such as `[` (see [`Type::try_get_element_type`]).
    #[must_use]
    #[track_caller]
    pub fn get_element_type(&self) -> Type<'a> {
        self.try_get_element_type().unwrap_or_else(|e| e.throw())
    }

    /// Returns the type of the elements of this array type. This method should only be used for
    /// an array type. The returned type shares this type's buffer.
    ///
    /// # Errors
    ///
    /// If the element type descriptor is missing or invalid.
    pub fn try_get_element_type(&self) -> Result<Type<'a>, InvalidDescriptor> {
        let num_dimensions = self.get_dimensions();
        get_type_internal(&self.buf, self.begin + num_dimensions, self.end)
    }

    /// Returns the type corresponding to the given internal name (see
    /// [`Type::get_internal_name`]). A name starting with `[` gives an array type. The type
    /// borrows the name.
    #[must_use]
    pub fn get_object_type<S: AsRef<JavaStr> + ?Sized>(internal_name: &'a S) -> Type<'a> {
        object_type(internal_name.as_ref())
    }

    /// Returns the type corresponding to the given method descriptor. Equivalent to
    /// `Type::get_type(method_descriptor)` for valid descriptors, but does not look at the
    /// descriptor at all. The type borrows the descriptor.
    #[must_use]
    pub fn get_method_type<S: AsRef<JavaStr> + ?Sized>(method_descriptor: &'a S) -> Type<'a> {
        let method_descriptor = method_descriptor.as_ref();
        Type {
            sort: TypeSort::Method,
            internal: false,
            buf: TypeBuf::Borrowed(method_descriptor),
            begin: 0,
            end: method_descriptor.len(),
        }
    }

    /// Returns the argument types of methods of this type. This method should only be used for
    /// method types. The returned types share this type's buffer.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_argument_types_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_argument_types(&self) -> Vec<Type<'a>> {
        let descriptor = self.descriptor_buf();
        argument_types(&descriptor, 0).unwrap_or_else(|e| e.throw())
    }

    /// Returns the types corresponding to the argument types of the given method descriptor.
    /// The types borrow the descriptor.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_argument_types_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_argument_types_of<S: AsRef<JavaStr> + ?Sized>(
        method_descriptor: &'a S,
    ) -> Vec<Type<'a>> {
        Type::try_get_argument_types_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the types corresponding to the argument types of the given method descriptor.
    /// The types borrow the descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its `)`, or an argument type character is invalid.
    pub fn try_get_argument_types_of<S: AsRef<JavaStr> + ?Sized>(
        method_descriptor: &'a S,
    ) -> Result<Vec<Type<'a>>, InvalidDescriptor> {
        argument_types(&TypeBuf::Borrowed(method_descriptor.as_ref()), 0)
    }

    /// Calls `f` with each argument type of the given method descriptor, in order, without
    /// collecting them. The errors are those of [`Type::try_get_argument_types_of`] followed by
    /// `f`'s: a malformed descriptor is reported before any error of `f`.
    pub(crate) fn try_for_each_argument_type_of<E: From<InvalidDescriptor>>(
        method_descriptor: &'a JavaStr,
        f: impl FnMut(Type<'a>) -> Result<(), E>,
    ) -> Result<(), E> {
        for_each_argument_type(&TypeBuf::Borrowed(method_descriptor), 0, f)
    }

    /// Returns the return type of methods of this type. This method should only be used for
    /// method types. The returned type shares this type's buffer.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_return_type_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_return_type(&self) -> Type<'a> {
        let descriptor = self.descriptor_buf();
        return_type(&descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the return type of the given method descriptor. The
    /// type borrows the descriptor.
    ///
    /// # Panics
    ///
    /// On a malformed method descriptor (see [`Type::try_get_return_type_of`]).
    #[must_use]
    #[track_caller]
    pub fn get_return_type_of<S: AsRef<JavaStr> + ?Sized>(method_descriptor: &'a S) -> Type<'a> {
        Type::try_get_return_type_of(method_descriptor).unwrap_or_else(|e| e.throw())
    }

    /// Returns the type corresponding to the return type of the given method descriptor. The
    /// type borrows the descriptor.
    ///
    /// # Errors
    ///
    /// If the descriptor ends before its return type, or the return type is invalid.
    pub fn try_get_return_type_of<S: AsRef<JavaStr> + ?Sized>(
        method_descriptor: &'a S,
    ) -> Result<Type<'a>, InvalidDescriptor> {
        return_type(&TypeBuf::Borrowed(method_descriptor.as_ref()))
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
    pub fn get_method_descriptor(
        return_type: &Type<'_>,
        argument_types: &[Type<'_>],
    ) -> JavaString {
        let mut descriptor =
            JavaString::with_capacity(method_descriptor_len(return_type, argument_types));
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

    /// The length in bytes of this type's descriptor.
    fn descriptor_len(&self) -> usize {
        let [a, b, c] = self.descriptor_parts();
        a.len() + b.len() + c.len()
    }

    /// This type's descriptor, in up to three parts (`L`, the internal name and `;` for an
    /// object type created from an internal name).
    fn descriptor_parts(&self) -> [&[u8]; 3] {
        if self.internal {
            [b"L", self.value().as_bytes(), b";"]
        } else if self.sort == TypeSort::Object {
            [self.object_descriptor().as_bytes(), b"", b""]
        } else {
            [self.value().as_bytes(), b"", b""]
        }
    }

    /// The descriptor around the value of an object type created from a descriptor: one
    /// character before and after it (normally `L` and `;`).
    fn object_descriptor(&self) -> &JavaStr {
        let s = self.buf.as_java_str();
        &s[self.begin - 1..self.end + next_char_len(s, self.end)]
    }

    /// This method type's descriptor as a buffer of its own: a borrowed buffer is narrowed to the
    /// descriptor, a shared one is kept when it is exactly the descriptor (always the case for
    /// method types built from a descriptor).
    fn descriptor_buf(&self) -> TypeBuf<'a> {
        if self.internal {
            return TypeBuf::Shared(Arc::from(&*self.get_descriptor()));
        }
        match &self.buf {
            TypeBuf::Static(s) => TypeBuf::Static(&s[self.begin..self.end]),
            TypeBuf::Borrowed(s) => TypeBuf::Borrowed(&s[self.begin..self.end]),
            TypeBuf::Shared(s) if self.begin == 0 && self.end == s.len() => self.buf.clone(),
            TypeBuf::Shared(s) => TypeBuf::Shared(Arc::from(&s[self.begin..self.end])),
        }
    }
}

/// The length in bytes of the descriptor of a method of the given return and argument types.
fn method_descriptor_len(return_type: &Type<'_>, argument_types: &[Type<'_>]) -> usize {
    let mut length = 2 + return_type.descriptor_len();
    for argument_type in argument_types {
        length += argument_type.descriptor_len();
    }
    length
}

/// The length of the character at `offset` of `s`, 0 at its end.
fn next_char_len(s: &JavaStr, offset: usize) -> usize {
    match s.as_bytes().get(offset) {
        None => 0,
        // The common case: a descriptor's `;`.
        Some(&b) if b.is_ascii() => 1,
        Some(_) => s[offset..].chars().next().map_or(0, |c| c.len_semi_utf8()),
    }
}

/// The object or array type of an internal name.
const fn object_type(internal_name: &JavaStr) -> Type<'_> {
    let bytes = internal_name.as_bytes();
    let is_array = !bytes.is_empty() && bytes[0] == b'[';
    Type {
        sort: if is_array {
            TypeSort::Array
        } else {
            TypeSort::Object
        },
        internal: !is_array,
        buf: TypeBuf::Borrowed(internal_name),
        begin: 0,
        end: bytes.len(),
    }
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

fn argument_types<'a>(buf: &TypeBuf<'a>, base: usize) -> Result<Vec<Type<'a>>, InvalidDescriptor> {
    let mut argument_types = Vec::new();
    for_each_argument_type(buf, base, |argument_type| {
        argument_types.push(argument_type);
        Ok::<_, InvalidDescriptor>(())
    })?;
    Ok(argument_types)
}

/// Checks the argument types of the method descriptor at `base` of `buf`, as
/// [`argument_types`] does (not generic, so that [`for_each_argument_type`] can call it).
fn check_argument_types(buf: &TypeBuf<'_>, base: usize) -> Result<(), InvalidDescriptor> {
    for_each_argument_type(buf, base, |_| Ok(()))
}

/// Calls `f` with each argument type of the method descriptor at `base` of `buf`, in order: ASM's
/// `for (Type argumentType : getArgumentTypes(descriptor))`, without collecting the types.
///
/// The errors are ASM's, which creates all the argument types before the loop uses the first: a
/// malformed descriptor is reported before an error of `f`.
fn for_each_argument_type<'a, E: From<InvalidDescriptor>>(
    buf: &TypeBuf<'a>,
    base: usize,
    mut f: impl FnMut(Type<'a>) -> Result<(), E>,
) -> Result<(), E> {
    let descriptor = &buf.as_java_str()[base..];
    let d = descriptor.as_bytes();
    let past_end = |offset| out_of_bounds(descriptor, offset);
    // ASM counts the argument types first, then creates them: a descriptor that ends too early
    // is reported before an invalid argument type. This does both in one pass, and finishes the
    // count only when an argument type is invalid.
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
        match get_type_internal(
            buf,
            base + current_argument_type_offset,
            base + current_offset,
        ) {
            Ok(argument_type) => {
                if let Err(error) = f(argument_type) {
                    // The rest of the descriptor must be valid for this error to be the first.
                    check_argument_types(buf, base)?;
                    return Err(error);
                }
            }
            Err(invalid) => {
                Type::try_get_argument_count_of(descriptor)?;
                return Err(invalid.into());
            }
        }
    }
    Ok(())
}

fn return_type<'a>(buf: &TypeBuf<'a>) -> Result<Type<'a>, InvalidDescriptor> {
    let offset = Type::get_return_type_offset(buf.as_java_str())?;
    get_type_internal(buf, offset, buf.bytes().len())
}

/// Returns the type corresponding to the descriptor in `buf[begin..end]`.
fn get_type_internal<'a>(
    buf: &TypeBuf<'a>,
    begin: usize,
    end: usize,
) -> Result<Type<'a>, InvalidDescriptor> {
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

impl<'b> PartialEq<Type<'b>> for Type<'_> {
    fn eq(&self, other: &Type<'b>) -> bool {
        self.sort == other.sort && self.value() == other.value()
    }
}

impl Eq for Type<'_> {}

impl Hash for Type<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.sort.hash(state);
        self.value().hash(state);
    }
}

impl fmt::Display for Type<'_> {
    /// Writes the descriptor of this type.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.get_descriptor(), f)
    }
}

impl fmt::Debug for Type<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Type({:?})", &*self.get_descriptor())
    }
}
