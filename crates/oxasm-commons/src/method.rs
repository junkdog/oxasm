//! [`Method`]: ASM's `Method`.

use std::fmt;

use oxasm::{JavaStr, JavaString, Type};

/// A named method descriptor.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Method {
    /// The method name.
    name: JavaString,
    /// The method descriptor.
    descriptor: JavaString,
}

/// The descriptors of the primitive Java types (plus `void`), by name.
fn primitive_type_descriptor(name: &[u16]) -> Option<char> {
    const DESCRIPTORS: [(&str, char); 9] = [
        ("void", 'V'),
        ("byte", 'B'),
        ("char", 'C'),
        ("double", 'D'),
        ("float", 'F'),
        ("int", 'I'),
        ("long", 'J'),
        ("short", 'S'),
        ("boolean", 'Z'),
    ];
    DESCRIPTORS
        .iter()
        .find(|(n, _)| n.encode_utf16().eq(name.iter().copied()))
        .map(|&(_, d)| d)
}

/// Java's `String.indexOf(char, fromIndex)`, on UTF-16 units.
fn index_of(s: &[u16], c: char, from_index: isize) -> isize {
    let from = from_index.max(0).unsigned_abs();
    if from >= s.len() {
        return -1;
    }
    s[from..]
        .iter()
        .position(|&u| u == c as u16)
        .map_or(-1, |i| (from + i).cast_signed())
}

/// Java's `String.trim()`: removes the leading and trailing characters `<= ' '`.
fn trim(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&u| u > u16::from(b' '));
    match start {
        None => &[],
        Some(start) => {
            let end = s
                .iter()
                .rposition(|&u| u > u16::from(b' '))
                .unwrap_or(start);
            &s[start..=end]
        }
    }
}

impl Method {
    /// Constructs a new [`Method`].
    ///
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor.
    pub fn new(name: impl Into<JavaString>, descriptor: impl Into<JavaString>) -> Method {
        Method {
            name: name.into(),
            descriptor: descriptor.into(),
        }
    }

    /// Constructs a new [`Method`].
    ///
    /// - `name`: the method's name.
    /// - `return_type`: the method's return type.
    /// - `argument_types`: the method's argument types.
    pub fn from_types(
        name: impl Into<JavaString>,
        return_type: &Type,
        argument_types: &[Type],
    ) -> Method {
        Method::new(
            name,
            Type::get_method_descriptor(return_type, argument_types),
        )
    }

    /// Returns a [`Method`] corresponding to the given Java method declaration, such as
    /// `"void foo(int)"`. Class names without a package are taken to be in `java.lang`; see
    /// [`Method::get_method_with_default_package`].
    ///
    /// - `method`: a Java method declaration, without argument names, of the form `returnType
    ///   name (argumentType1, ... argumentTypeN)`, where the types are in plain Java (e.g.
    ///   `int`, `float`, `java.util.List`, ...). Classes of the `java.lang` package can be
    ///   specified by their unqualified name; all other classes names must be fully qualified.
    ///
    /// # Panics
    ///
    /// If `method` could not get parsed (ASM's `IllegalArgumentException`).
    #[track_caller]
    pub fn get_method(method: &str) -> Method {
        Method::get_method_with_default_package(method, false)
    }

    /// Returns a [`Method`] corresponding to the given Java method declaration.
    ///
    /// - `method`: a Java method declaration, without argument names, of the form `returnType
    ///   name (argumentType1, ... argumentTypeN)`, where the types are in plain Java (e.g.
    ///   `int`, `float`, `java.util.List`, ...). Classes of the `java.lang` package may be
    ///   specified by their unqualified name, depending on the `default_package` argument; all
    ///   other classes names must be fully qualified.
    /// - `default_package`: `true` if unqualified class names belong to the default package, or
    ///   `false` if they correspond to `java.lang` classes. For instance `"Object"` means
    ///   `"Object"` if this option is `true`, or `"java.lang.Object"` otherwise.
    ///
    /// # Panics
    ///
    /// If `method` could not get parsed (ASM's `IllegalArgumentException`).
    #[track_caller]
    pub fn get_method_with_default_package(method: &str, default_package: bool) -> Method {
        // Java's indices are UTF-16 ones: parse the UTF-16 units, as ASM does.
        let method: Vec<u16> = method.encode_utf16().collect();
        let space_index = index_of(&method, ' ', 0);
        let mut current_argument_start_index = index_of(&method, '(', space_index) + 1;
        let end_index = index_of(&method, ')', current_argument_start_index);
        if space_index == -1 || current_argument_start_index == 0 || end_index == -1 {
            // ASM throws an IllegalArgumentException without a message.
            panic!("java.lang.IllegalArgumentException");
        }
        let (space_index, end_index) = (space_index.unsigned_abs(), end_index.unsigned_abs());
        let return_type = &method[..space_index];
        let method_name =
            trim(&method[space_index + 1..current_argument_start_index.unsigned_abs() - 1]);
        let mut descriptor: Vec<u16> = vec![u16::from(b'(')];
        loop {
            let current_argument_end_index = index_of(&method, ',', current_argument_start_index);
            let start = current_argument_start_index.unsigned_abs();
            if current_argument_end_index == -1 {
                descriptor.extend(get_descriptor_internal(
                    trim(substring(&method, start, end_index)),
                    default_package,
                ));
                break;
            }
            descriptor.extend(get_descriptor_internal(
                trim(substring(
                    &method,
                    start,
                    current_argument_end_index.unsigned_abs(),
                )),
                default_package,
            ));
            current_argument_start_index = current_argument_end_index + 1;
        }
        descriptor.push(u16::from(b')'));
        descriptor.extend(get_descriptor_internal(return_type, default_package));
        Method::new(
            method_name.iter().copied().collect::<JavaString>(),
            descriptor.into_iter().collect::<JavaString>(),
        )
    }

    /// Returns the name of the method described by this object.
    #[must_use]
    pub fn get_name(&self) -> &JavaStr {
        &self.name
    }

    /// Returns the descriptor of the method described by this object.
    #[must_use]
    pub fn get_descriptor(&self) -> &JavaStr {
        &self.descriptor
    }

    /// Returns the return type of the method described by this object.
    #[must_use]
    #[track_caller]
    pub fn get_return_type(&self) -> Type {
        Type::get_return_type_of(&*self.descriptor)
    }

    /// Returns the argument types of the method described by this object.
    #[must_use]
    #[track_caller]
    pub fn get_argument_types(&self) -> Vec<Type> {
        Type::get_argument_types_of(&*self.descriptor)
    }
}

/// Java's `String.substring(begin, end)`, which throws (here panics, with Java's message) when
/// `begin > end`.
#[track_caller]
fn substring(s: &[u16], begin: usize, end: usize) -> &[u16] {
    if begin > end {
        panic!(
            "java.lang.StringIndexOutOfBoundsException: Range [{begin}, {end}) out of bounds for length {}",
            s.len()
        );
    }
    &s[begin..end]
}

/// Returns the descriptor corresponding to the given type name.
///
/// - `type_`: a Java type name.
/// - `default_package`: `true` if unqualified class names belong to the default package, or
///   `false` if they correspond to `java.lang` classes.
fn get_descriptor_internal(type_: &[u16], default_package: bool) -> Vec<u16> {
    if type_.is_empty() {
        return Vec::new();
    }

    let mut descriptor: Vec<u16> = Vec::new();
    let mut array_brackets_index: isize = 0;
    loop {
        array_brackets_index = index_of_pair(type_, array_brackets_index) + 1;
        if array_brackets_index <= 0 {
            break;
        }
        descriptor.push(u16::from(b'['));
    }

    let element_type_end = type_.len().saturating_sub(descriptor.len() * 2);
    let element_type = &type_[..element_type_end];
    if let Some(primitive) = primitive_type_descriptor(element_type) {
        descriptor.push(primitive as u16);
    } else {
        descriptor.push(u16::from(b'L'));
        if element_type.contains(&u16::from(b'.')) {
            descriptor.extend(element_type.iter().map(|&u| {
                if u == u16::from(b'.') {
                    u16::from(b'/')
                } else {
                    u
                }
            }));
        } else {
            if !default_package {
                descriptor.extend("java/lang/".encode_utf16());
            }
            descriptor.extend_from_slice(element_type);
        }
        descriptor.push(u16::from(b';'));
    }
    descriptor
}

/// Java's `type.indexOf("[]", fromIndex)`.
fn index_of_pair(s: &[u16], from_index: isize) -> isize {
    let from = from_index.max(0).unsigned_abs();
    if from >= s.len() {
        return -1;
    }
    s[from..]
        .windows(2)
        .position(|w| w == [u16::from(b'['), u16::from(b']')])
        .map_or(-1, |i| (from + i).cast_signed())
}

/// `name + descriptor`, as ASM's `toString()`.
impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.name, self.descriptor)
    }
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Method({self})")
    }
}
