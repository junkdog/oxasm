//! The static checks of ASM's `CheckClassAdapter` and `CheckMethodAdapter`: access flags, names,
//! descriptors, signatures, type references and constants.
//!
//! The strings are checked as Java strings, by UTF-16 code unit, so that the indices in the
//! messages are Java's.

use std::fmt::{self, Display};

use oxasm::constant::Constant;
use oxasm::opcodes::*;
use oxasm::{JavaStr, TypeReference};

use crate::check::java_chars::{JAVA_IDENTIFIER_PART, JAVA_IDENTIFIER_START};

/// The prefix of the messages of most `IllegalArgumentException`s.
pub(crate) const INVALID: &str = "Invalid ";
/// The message suffix for a null or empty name.
const MUST_NOT_BE_NULL_OR_EMPTY: &str = " (must not be null or empty)";
/// The prefix of the messages of invalid descriptors.
const INVALID_DESCRIPTOR: &str = "Invalid descriptor: ";
/// The infix of the messages of invalid signatures.
const ERROR_AT: &str = ": error at index ";

/// The maximum nesting depth of the type arguments of a signature: SignatureReader's (ASM 9.11's)
/// bound, where ASM 9.10.1 recurses without bound (ADR-06).
const MAX_TYPE_ARGUMENT_DEPTH: usize = 256;

// -------------------------------------------------------------------------------------------------
// Failures
// -------------------------------------------------------------------------------------------------

/// Panics as ASM throws an `IllegalArgumentException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn illegal_argument(message: impl Display) -> ! {
    panic!("java.lang.IllegalArgumentException: {message}")
}

/// Panics as ASM throws an `IllegalStateException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn illegal_state(message: impl Display) -> ! {
    panic!("java.lang.IllegalStateException: {message}")
}

/// Panics as ASM throws an `IllegalStateException` without a message.
#[track_caller]
pub(crate) fn illegal_state_without_message() -> ! {
    panic!("java.lang.IllegalStateException")
}

/// Panics as ASM throws an `UnsupportedOperationException`: with the exception's `toString()`.
#[track_caller]
pub(crate) fn unsupported_operation(message: impl Display) -> ! {
    panic!("java.lang.UnsupportedOperationException: {message}")
}

/// The message of an `IllegalArgumentException` that a check catches and wraps in another one.
type CheckResult<T = ()> = Result<T, String>;

// -------------------------------------------------------------------------------------------------
// Java strings
// -------------------------------------------------------------------------------------------------

/// A string as Java sees it: UTF-16 code units, with Java's `String` methods.
pub(crate) struct Utf16 {
    units: Vec<u16>,
}

impl Utf16 {
    pub(crate) fn new(string: &JavaStr) -> Utf16 {
        Utf16 {
            units: string.encode_utf16().collect(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.units.len()
    }

    /// `charAt`, which must be in bounds.
    pub(crate) fn char_at(&self, index: usize) -> u16 {
        self.units[index]
    }

    /// `indexOf(c, from)`.
    pub(crate) fn index_of(&self, c: char, from: usize) -> Option<usize> {
        let c = c as u16;
        self.units
            .get(from..)?
            .iter()
            .position(|&unit| unit == c)
            .map(|position| from + position)
    }

    /// `lastIndexOf(c)`.
    pub(crate) fn last_index_of(&self, c: char) -> Option<usize> {
        let c = c as u16;
        self.units.iter().rposition(|&unit| unit == c)
    }

    /// `codePointAt(index)`, which must be in bounds.
    pub(crate) fn code_point_at(&self, index: usize) -> u32 {
        let high = self.units[index];
        if (0xd800..0xdc00).contains(&high)
            && let Some(&low) = self.units.get(index + 1)
            && (0xdc00..0xe000).contains(&low)
        {
            return 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
        }
        u32::from(high)
    }

    /// `offsetByCodePoints(index, 1)`.
    pub(crate) fn next_code_point(&self, index: usize) -> usize {
        if self.code_point_at(index) >= 0x10000 {
            index + 2
        } else {
            index + 1
        }
    }
}

/// Whether `code_point` is one of the characters of `chars` (Java's `chars.indexOf(codePoint) !=
/// -1`, for ASCII `chars`).
fn is_one_of(code_point: u32, chars: &str) -> bool {
    char::from_u32(code_point).is_some_and(|c| c.is_ascii() && chars.contains(c))
}

/// Whether `code_point` is in one of the sorted, disjoint `ranges`.
pub(crate) fn in_ranges(ranges: &[(u32, u32)], code_point: u32) -> bool {
    ranges
        .binary_search_by(|&(start, end)| {
            if end < code_point {
                std::cmp::Ordering::Less
            } else if start > code_point {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Java's `Character.isJavaIdentifierStart(int)`.
fn is_java_identifier_start(code_point: u32) -> bool {
    in_ranges(JAVA_IDENTIFIER_START, code_point)
}

/// Java's `Character.isJavaIdentifierPart(int)`.
fn is_java_identifier_part(code_point: u32) -> bool {
    in_ranges(JAVA_IDENTIFIER_PART, code_point)
}

/// Whether the class version is Java 5 or more, for which the names are only checked against
/// the characters the JVMS forbids.
fn is_v1_5_or_more(version: u32) -> bool {
    (version & 0xffff) >= V1_5
}

// -------------------------------------------------------------------------------------------------
// Access flags
// -------------------------------------------------------------------------------------------------

/// Checks that the given access flags do not contain invalid flags, nor mutually incompatible
/// ones (`CheckClassAdapter.checkAccess`).
pub(crate) fn check_access(access: u32, possible_access: u32) {
    // Java's message prints the flags as an int.
    let java_access = access.cast_signed();
    if access & !possible_access != 0 {
        illegal_argument(format_args!("Invalid access flags: {java_access}"));
    }
    let public_protected_private = ACC_PUBLIC | ACC_PROTECTED | ACC_PRIVATE;
    if (access & public_protected_private).count_ones() > 1 {
        illegal_argument(format_args!(
            "public, protected and private are mutually exclusive: {java_access}"
        ));
    }
    if (access & (ACC_FINAL | ACC_ABSTRACT)).count_ones() > 1 {
        illegal_argument(format_args!(
            "final and abstract are mutually exclusive: {java_access}"
        ));
    }
}

/// Checks the access flags of a method (`CheckClassAdapter.checkMethodAccess`).
pub(crate) fn check_method_access(version: u32, access: u32, possible_access: u32) {
    check_access(access, possible_access);
    if (version & 0xffff) < V17 && (access & (ACC_STRICT | ACC_ABSTRACT)).count_ones() > 1 {
        illegal_argument(format_args!(
            "strictfp and abstract are mutually exclusive: {}",
            access.cast_signed()
        ));
    }
}

// -------------------------------------------------------------------------------------------------
// Names
// -------------------------------------------------------------------------------------------------

/// Checks that the given name is a fully qualified name, using dots
/// (`CheckClassAdapter.checkFullyQualifiedName`).
pub(crate) fn check_fully_qualified_name(version: u32, name: &JavaStr, source: &str) {
    let utf16 = Utf16::new(name);
    let result = (|| {
        let mut start_index = 0;
        while let Some(dot_index) = utf16.index_of('.', start_index + 1) {
            identifier(version, name, &utf16, start_index, Some(dot_index), None)?;
            start_index = dot_index + 1;
        }
        identifier(version, name, &utf16, start_index, Some(utf16.len()), None)
    })();
    if result.is_err() {
        illegal_argument(format_args!(
            "Invalid {source} (must be a fully qualified name): {name}"
        ));
    }
}

/// Checks that the given string is a valid unqualified name
/// (`CheckMethodAdapter.checkUnqualifiedName`).
pub(crate) fn check_unqualified_name(version: u32, name: &JavaStr, message: &str) {
    check_identifier(version, name, 0, None, message);
}

/// Checks that the given substring is a valid Java identifier
/// (`CheckMethodAdapter.checkIdentifier`). `end_pos` `None` is the end of the string.
pub(crate) fn check_identifier(
    version: u32,
    name: &JavaStr,
    start_pos: usize,
    end_pos: Option<usize>,
    message: &str,
) {
    let utf16 = Utf16::new(name);
    if let Err(message) = identifier(version, name, &utf16, start_pos, end_pos, Some(message)) {
        illegal_argument(message);
    }
}

/// `CheckMethodAdapter.checkIdentifier`, returning the message of its exception.
fn identifier(
    version: u32,
    name: &JavaStr,
    utf16: &Utf16,
    start_pos: usize,
    end_pos: Option<usize>,
    message: Option<&str>,
) -> CheckResult {
    let message = message.unwrap_or("null");
    let max = end_pos.unwrap_or(utf16.len());
    if max <= start_pos {
        return Err(format!("{INVALID}{message}{MUST_NOT_BE_NULL_OR_EMPTY}"));
    }
    if is_v1_5_or_more(version) {
        let mut i = start_pos;
        while i < max {
            if is_one_of(utf16.code_point_at(i), ".;[/") {
                return Err(format!(
                    "{INVALID}{message} (must not contain . ; [ or /): {name}"
                ));
            }
            i = utf16.next_code_point(i);
        }
        return Ok(());
    }
    let mut i = start_pos;
    while i < max {
        let code_point = utf16.code_point_at(i);
        let valid = if i == start_pos {
            is_java_identifier_start(code_point)
        } else {
            is_java_identifier_part(code_point)
        };
        if !valid {
            return Err(format!(
                "{INVALID}{message} (must be a valid Java identifier): {name}"
            ));
        }
        i = utf16.next_code_point(i);
    }
    Ok(())
}

/// Checks that the given string is a valid method name (`CheckMethodAdapter.checkMethodIdentifier`).
pub(crate) fn check_method_identifier(version: u32, name: &JavaStr, message: &str) {
    let utf16 = Utf16::new(name);
    if utf16.len() == 0 {
        illegal_argument(format_args!(
            "{INVALID}{message}{MUST_NOT_BE_NULL_OR_EMPTY}"
        ));
    }
    if is_v1_5_or_more(version) {
        let mut i = 0;
        while i < utf16.len() {
            if is_one_of(utf16.code_point_at(i), ".;[/<>") {
                illegal_argument(format_args!(
                    "{INVALID}{message} (must be a valid unqualified name): {name}"
                ));
            }
            i = utf16.next_code_point(i);
        }
        return;
    }
    let mut i = 0;
    while i < utf16.len() {
        let code_point = utf16.code_point_at(i);
        let valid = if i == 0 {
            is_java_identifier_start(code_point)
        } else {
            is_java_identifier_part(code_point)
        };
        if !valid {
            illegal_argument(format_args!(
                "{INVALID}{message} (must be a '<init>', '<clinit>' or a valid Java identifier): \
                 {name}"
            ));
        }
        i = utf16.next_code_point(i);
    }
}

/// Checks that the given string is a valid internal class name or array type descriptor
/// (`CheckMethodAdapter.checkInternalName`).
pub(crate) fn check_internal_name(version: u32, name: &JavaStr, message: &str) {
    let utf16 = Utf16::new(name);
    if utf16.len() == 0 {
        illegal_argument(format_args!(
            "{INVALID}{message}{MUST_NOT_BE_NULL_OR_EMPTY}"
        ));
    }
    if utf16.char_at(0) == u16::from(b'[') {
        check_descriptor(version, name, false);
    } else if internal_class_name(version, name, &utf16, 0, utf16.len()).is_err() {
        illegal_argument(format_args!(
            "{INVALID}{message} (must be an internal class name): {name}"
        ));
    }
}

/// `CheckMethodAdapter.checkInternalClassName` of `name.substring(start, end)`, returning
/// whether it is valid (the message of its exception is always wrapped in another one).
fn internal_class_name(
    version: u32,
    name: &JavaStr,
    utf16: &Utf16,
    start: usize,
    end: usize,
) -> CheckResult {
    let mut start_index = start;
    loop {
        // Java's 'indexOf' is in the substring, starting at 'startIndex + 1'.
        let slash_index = utf16
            .index_of('/', start_index + 1)
            .filter(|&index| index < end);
        let Some(slash_index) = slash_index else {
            break;
        };
        identifier(version, name, utf16, start_index, Some(slash_index), None)?;
        start_index = slash_index + 1;
    }
    identifier(version, name, utf16, start_index, Some(end), None)
}

// -------------------------------------------------------------------------------------------------
// Descriptors
// -------------------------------------------------------------------------------------------------

/// Checks that the given string is a valid type descriptor (`CheckMethodAdapter.checkDescriptor`).
pub(crate) fn check_descriptor(version: u32, descriptor: &JavaStr, can_be_void: bool) {
    let utf16 = Utf16::new(descriptor);
    let end_pos = descriptor_at(version, descriptor, &utf16, 0, can_be_void);
    if end_pos != utf16.len() {
        illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
    }
}

/// Checks that the type descriptor at `start_pos` is valid, and returns its end.
fn descriptor_at(
    version: u32,
    descriptor: &JavaStr,
    utf16: &Utf16,
    start_pos: usize,
    can_be_void: bool,
) -> usize {
    if start_pos >= utf16.len() {
        illegal_argument("Invalid type descriptor (must not be null or empty)");
    }
    match u8::try_from(utf16.char_at(start_pos)).unwrap_or(0) {
        b'V' => {
            if can_be_void {
                start_pos + 1
            } else {
                illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"))
            }
        }
        b'Z' | b'C' | b'B' | b'S' | b'I' | b'F' | b'J' | b'D' => start_pos + 1,
        b'[' => {
            let mut pos = start_pos + 1;
            while pos < utf16.len() && utf16.char_at(pos) == u16::from(b'[') {
                pos += 1;
            }
            if pos < utf16.len() {
                // ASM recurses here, once (the element type is not an array).
                descriptor_at(version, descriptor, utf16, pos, false)
            } else {
                illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"))
            }
        }
        b'L' => {
            let end_pos = utf16.index_of(';', start_pos);
            let Some(end_pos) = end_pos.filter(|&end_pos| end_pos >= start_pos + 2) else {
                illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
            };
            if internal_class_name(version, descriptor, utf16, start_pos + 1, end_pos).is_err() {
                illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
            }
            end_pos + 1
        }
        _ => illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}")),
    }
}

/// Checks that the given string is a valid method descriptor
/// (`CheckMethodAdapter.checkMethodDescriptor`).
pub(crate) fn check_method_descriptor(version: u32, descriptor: &JavaStr) {
    let utf16 = Utf16::new(descriptor);
    if utf16.len() == 0 {
        illegal_argument("Invalid method descriptor (must not be null or empty)");
    }
    if utf16.char_at(0) != u16::from(b'(') || utf16.len() < 3 {
        illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
    }
    let mut pos = 1;
    if utf16.char_at(pos) != u16::from(b')') {
        loop {
            if utf16.char_at(pos) == u16::from(b'V') {
                illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
            }
            pos = descriptor_at(version, descriptor, &utf16, pos, false);
            if pos >= utf16.len() || utf16.char_at(pos) == u16::from(b')') {
                break;
            }
        }
    }
    pos = descriptor_at(version, descriptor, &utf16, pos + 1, true);
    if pos != utf16.len() {
        illegal_argument(format_args!("{INVALID_DESCRIPTOR}{descriptor}"));
    }
}

// -------------------------------------------------------------------------------------------------
// Signatures
// -------------------------------------------------------------------------------------------------

/// A signature being checked (the `String signature` argument of ASM's static checks).
struct Signature<'s> {
    text: &'s JavaStr,
    utf16: Utf16,
    /// The nesting depth of the type arguments being checked.
    depth: usize,
}

impl Signature<'_> {
    /// `getChar`: the character at `pos`, or 0.
    fn get_char(&self, pos: usize) -> u16 {
        if pos < self.utf16.len() {
            self.utf16.char_at(pos)
        } else {
            0
        }
    }

    fn is(&self, pos: usize, c: char) -> bool {
        self.get_char(pos) == c as u16
    }

    fn check_end(&self, pos: usize) {
        if pos != self.utf16.len() {
            illegal_argument(format_args!("{}{ERROR_AT}{pos}", self.text));
        }
    }

    /// `checkTypeParameters`.
    fn type_parameters(&mut self, start_pos: usize) -> usize {
        let mut pos = self.char('<', start_pos);
        pos = self.type_parameter(pos);
        while !self.is(pos, '>') {
            pos = self.type_parameter(pos);
        }
        pos + 1
    }

    /// `checkTypeParameter`.
    fn type_parameter(&mut self, start_pos: usize) -> usize {
        let mut pos = self.identifier(start_pos);
        pos = self.char(':', pos);
        if self.is(pos, 'L') || self.is(pos, '[') || self.is(pos, 'T') {
            pos = self.reference_type_signature(pos);
        }
        while self.is(pos, ':') {
            pos = self.reference_type_signature(pos + 1);
        }
        pos
    }

    /// `checkReferenceTypeSignature`. ASM's recursion through array types is a loop here.
    fn reference_type_signature(&mut self, start_pos: usize) -> usize {
        let mut pos = start_pos;
        loop {
            if self.is(pos, 'L') {
                return self.class_type_signature(pos);
            } else if self.is(pos, '[') {
                // checkJavaTypeSignature(signature, pos + 1).
                pos += 1;
                if is_one_of(u32::from(self.get_char(pos)), "BCDFIJSZ") {
                    return pos + 1;
                }
            } else {
                return self.type_variable_signature(pos);
            }
        }
    }

    /// `checkClassTypeSignature`.
    fn class_type_signature(&mut self, start_pos: usize) -> usize {
        let mut pos = self.char('L', start_pos);
        pos = self.identifier(pos);
        while self.is(pos, '/') {
            pos = self.identifier(pos + 1);
        }
        if self.is(pos, '<') {
            pos = self.type_arguments(pos);
        }
        while self.is(pos, '.') {
            pos = self.identifier(pos + 1);
            if self.is(pos, '<') {
                pos = self.type_arguments(pos);
            }
        }
        self.char(';', pos)
    }

    /// `checkTypeArguments`, with a nesting depth bound (ADR-06).
    fn type_arguments(&mut self, start_pos: usize) -> usize {
        if self.depth >= MAX_TYPE_ARGUMENT_DEPTH {
            illegal_argument(format_args!(
                "{}: too many nested type arguments at index {start_pos}",
                self.text
            ));
        }
        self.depth += 1;
        let mut pos = self.char('<', start_pos);
        pos = self.type_argument(pos);
        while !self.is(pos, '>') {
            pos = self.type_argument(pos);
        }
        self.depth -= 1;
        pos + 1
    }

    /// `checkTypeArgument`.
    fn type_argument(&mut self, start_pos: usize) -> usize {
        let mut pos = start_pos;
        if self.is(pos, '*') {
            return pos + 1;
        } else if self.is(pos, '+') || self.is(pos, '-') {
            pos += 1;
        }
        self.reference_type_signature(pos)
    }

    /// `checkTypeVariableSignature`.
    fn type_variable_signature(&mut self, start_pos: usize) -> usize {
        let mut pos = self.char('T', start_pos);
        pos = self.identifier(pos);
        self.char(';', pos)
    }

    /// `checkJavaTypeSignature`.
    fn java_type_signature(&mut self, pos: usize) -> usize {
        if is_one_of(u32::from(self.get_char(pos)), "BCDFIJSZ") {
            pos + 1
        } else {
            self.reference_type_signature(pos)
        }
    }

    /// `checkSignatureIdentifier`.
    fn identifier(&self, start_pos: usize) -> usize {
        let mut pos = start_pos;
        while pos < self.utf16.len() && !is_one_of(self.utf16.code_point_at(pos), ".;[/<>:") {
            pos = self.utf16.next_code_point(pos);
        }
        if pos == start_pos {
            illegal_argument(format_args!(
                "{}: identifier expected at index {start_pos}",
                self.text
            ));
        }
        pos
    }

    /// `checkChar`.
    fn char(&self, c: char, pos: usize) -> usize {
        if self.is(pos, c) {
            return pos + 1;
        }
        illegal_argument(format_args!("{}: '{c}' expected at index {pos}", self.text));
    }
}

fn signature(signature: &JavaStr) -> Signature<'_> {
    Signature {
        text: signature,
        utf16: Utf16::new(signature),
        depth: 0,
    }
}

/// Checks a class signature (ASM's `CheckClassAdapter.checkClassSignature`).
///
/// # Panics
///
/// If the signature is invalid, with ASM's `IllegalArgumentException` (see the
/// [module docs](crate::check)).
pub fn check_class_signature(class_signature: &JavaStr) {
    let mut s = signature(class_signature);
    let mut pos = 0;
    if s.is(0, '<') {
        pos = s.type_parameters(pos);
    }
    pos = s.class_type_signature(pos);
    while s.is(pos, 'L') {
        pos = s.class_type_signature(pos);
    }
    s.check_end(pos);
}

/// Checks a method signature (ASM's `CheckClassAdapter.checkMethodSignature`).
///
/// # Panics
///
/// If the signature is invalid, with ASM's `IllegalArgumentException` (see the
/// [module docs](crate::check)).
pub fn check_method_signature(method_signature: &JavaStr) {
    let mut s = signature(method_signature);
    let mut pos = 0;
    if s.is(0, '<') {
        pos = s.type_parameters(pos);
    }
    pos = s.char('(', pos);
    while is_one_of(u32::from(s.get_char(pos)), "ZCBSIFJDL[T") && s.get_char(pos) != 0 {
        pos = s.java_type_signature(pos);
    }
    pos = s.char(')', pos);
    if s.is(pos, 'V') {
        pos += 1;
    } else {
        pos = s.java_type_signature(pos);
    }
    while s.is(pos, '^') {
        pos += 1;
        if s.is(pos, 'L') {
            pos = s.class_type_signature(pos);
        } else {
            pos = s.type_variable_signature(pos);
        }
    }
    s.check_end(pos);
}

/// Checks a field signature (ASM's `CheckClassAdapter.checkFieldSignature`).
///
/// # Panics
///
/// If the signature is invalid, with ASM's `IllegalArgumentException` (see the
/// [module docs](crate::check)).
pub fn check_field_signature(field_signature: &JavaStr) {
    let mut s = signature(field_signature);
    let pos = s.reference_type_signature(0);
    s.check_end(pos);
}

// -------------------------------------------------------------------------------------------------
// Type references, constants
// -------------------------------------------------------------------------------------------------

/// Java's `Integer.toHexString`.
pub(crate) struct Hex(pub(crate) u32);

impl Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

/// Checks that the sort of a type reference is one of `sorts` (the first check of the
/// `visit*TypeAnnotation` methods of the check adapters).
pub(crate) fn check_type_ref_sort(type_ref: TypeReference, sorts: &[u8]) {
    let sort = type_ref.get_sort();
    if !sorts.contains(&sort) {
        illegal_argument(format_args!(
            "Invalid type reference sort 0x{}",
            Hex(u32::from(sort))
        ));
    }
}

/// Checks the reference to a type in a type annotation (`CheckClassAdapter.checkTypeRef`).
pub(crate) fn check_type_ref(type_ref: TypeReference) {
    let type_ref = type_ref.get_value();
    let mask: u32 = match u8::try_from(type_ref >> 24).unwrap_or(u8::MAX) {
        TypeReference::CLASS_TYPE_PARAMETER
        | TypeReference::METHOD_TYPE_PARAMETER
        | TypeReference::METHOD_FORMAL_PARAMETER => 0xffff_0000,
        TypeReference::FIELD
        | TypeReference::METHOD_RETURN
        | TypeReference::METHOD_RECEIVER
        | TypeReference::LOCAL_VARIABLE
        | TypeReference::RESOURCE_VARIABLE
        | TypeReference::INSTANCEOF
        | TypeReference::NEW
        | TypeReference::CONSTRUCTOR_REFERENCE
        | TypeReference::METHOD_REFERENCE => 0xff00_0000,
        TypeReference::CLASS_EXTENDS
        | TypeReference::CLASS_TYPE_PARAMETER_BOUND
        | TypeReference::METHOD_TYPE_PARAMETER_BOUND
        | TypeReference::THROWS
        | TypeReference::EXCEPTION_PARAMETER => 0xffff_ff00,
        TypeReference::CAST
        | TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT
        | TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT
        | TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT
        | TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT => 0xff00_00ff,
        _ => 0,
    };
    if mask == 0 || type_ref & !mask != 0 {
        illegal_argument(format_args!("Invalid type reference 0x{}", Hex(type_ref)));
    }
}

/// Checks that the given value is an `Integer`, `Float`, `Long`, `Double` or `String` constant
/// (`CheckMethodAdapter.checkConstant`).
pub(crate) fn check_constant(value: &Constant<'_>) {
    match value {
        Constant::Integer(_)
        | Constant::Float(_)
        | Constant::Long(_)
        | Constant::Double(_)
        | Constant::String(_) => {}
        Constant::Type(_) | Constant::Handle(_) | Constant::Dynamic(_) => {
            illegal_argument(format_args!("Invalid constant: {value}"));
        }
    }
}
