//! A port of `asm-test`'s `ClassFile` (ASM 9.10.1): an *independent* class file parser that prints
//! a canonical structural dump of a class, for `assert_eq!(ClassFile::new(a), ClassFile::new(b))`
//! style comparisons (rung 3 of the oracle ladder, oxasm-verification.md).
//!
//! Its independence from oxasm is the point, so this module reimplements everything it needs
//! (Java's modified UTF-8 decoding, `Float`/`Double.toString`, UTF-16 string ordering). The dump
//! must equal the Java `new ClassFile(bytes).toString()` byte for byte, in the semi-UTF-8
//! encoding of the oracle: constant pool references are printed with their content, bytecode
//! offsets are replaced with instruction indices, and attributes (and type annotations) are
//! sorted, so that two class files that differ only by the order of their constant pool or
//! attributes have the same dump.
//!
//! Errors mirror what the Java class throws: a `ClassFormatException` with the same message (or
//! `null`, for a premature end of file), and in a few corner cases another Java exception (see
//! [`ClassFormatError::java_exception`]). `ClassFile.newInstance()` is not ported; the oracle's
//! `verify` command replaces it.
//!
//! **Deliberate differences**, on inputs no compiler produces: where the Java code would overflow
//! its stack (attributes nested more than [`MAX_NESTING`] levels deep, chains or cycles of method
//! handle constants), this reports a `java.lang.StackOverflowError`; annotation element values are
//! parsed iteratively, so they never overflow.

#![allow(clippy::cast_possible_truncation)]

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;

use crate::java_fmt;

/// How deep attribute lists may nest (`Code` and `Record` attributes contain attribute lists)
/// before this reports a `java.lang.StackOverflowError`.
pub const MAX_NESTING: u32 = 256;

const CLASS_FORMAT_EXCEPTION: &str = "org.objectweb.asm.test.ClassFormatException";
const STACK_OVERFLOW_ERROR: &str = "java.lang.StackOverflowError";
const NEGATIVE_ARRAY_SIZE_EXCEPTION: &str = "java.lang.NegativeArraySizeException";
const CLASS_CAST_EXCEPTION: &str = "java.lang.ClassCastException";
const CLASS_FILE: &str = "org.objectweb.asm.test.ClassFile";

/// What `ClassFile` throws on a malformed or unsupported class file.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ClassFormatError {
    exception: &'static str,
    message: Option<Vec<u16>>,
}

impl ClassFormatError {
    fn class_format(message: Vec<u16>) -> ClassFormatError {
        ClassFormatError {
            exception: CLASS_FORMAT_EXCEPTION,
            message: Some(message),
        }
    }

    fn class_format_str(message: &str) -> ClassFormatError {
        ClassFormatError::class_format(message.encode_utf16().collect())
    }

    /// A premature end of the class file: Java's `EOFException`, rethrown as a
    /// `ClassFormatException` with its (`null`) message.
    fn eof() -> ClassFormatError {
        ClassFormatError {
            exception: CLASS_FORMAT_EXCEPTION,
            message: None,
        }
    }

    fn stack_overflow() -> ClassFormatError {
        ClassFormatError {
            exception: STACK_OVERFLOW_ERROR,
            message: None,
        }
    }

    /// The fully qualified name of the Java exception: almost always
    /// `org.objectweb.asm.test.ClassFormatException`.
    pub fn java_exception(&self) -> &'static str {
        self.exception
    }

    /// The exception message (lossy for unpaired surrogates), or `None` for Java's `null`.
    pub fn message(&self) -> Option<String> {
        self.message.as_deref().map(String::from_utf16_lossy)
    }

    /// `<exception class>: <message>` in semi-UTF-8, as the oracle reports it and as the `.err`
    /// goldens hold it.
    pub fn describe(&self) -> Vec<u8> {
        let mut text: Vec<u16> = self.exception.encode_utf16().collect();
        text.extend(": ".encode_utf16());
        match &self.message {
            Some(message) => text.extend_from_slice(message),
            None => text.extend("null".encode_utf16()),
        }
        semi_utf8(&text)
    }
}

impl fmt::Display for ClassFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.describe()))
    }
}

impl fmt::Debug for ClassFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for ClassFormatError {}

type Result<T> = std::result::Result<T, ClassFormatError>;

/// The dumps of a class file (`ClassFile.computeNameAndDumps`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Dumps {
    class_name: Vec<u16>,
    dump: Vec<u16>,
    constant_pool_dump: Vec<u16>,
}

/// A class file and its canonical dump (ASM's `org.objectweb.asm.test.ClassFile`).
///
/// Two `ClassFile`s are equal if their dumps are (or if both are invalid in the same way).
/// `Debug` and `Display` print the dump, so `assert_eq!` failures show a readable diff.
pub struct ClassFile<'a> {
    class_bytes: &'a [u8],
    dumps: OnceCell<Result<Dumps>>,
}

impl<'a> ClassFile<'a> {
    /// A class file to dump (parsed lazily, on the first query).
    pub fn new(class_bytes: &'a [u8]) -> ClassFile<'a> {
        ClassFile {
            class_bytes,
            dumps: OnceCell::new(),
        }
    }

    fn dumps(&self) -> &Result<Dumps> {
        self.dumps
            .get_or_init(|| compute_name_and_dumps(self.class_bytes))
    }

    /// The dump of this class (Java's `toString()`), in semi-UTF-8.
    pub fn dump(&self) -> Result<Vec<u8>> {
        self.dumps()
            .as_ref()
            .map(|d| semi_utf8(&d.dump))
            .map_err(Clone::clone)
    }

    /// The dump as text, with unpaired surrogates replaced (convenient for assertions).
    pub fn dump_lossy(&self) -> Result<String> {
        self.dumps()
            .as_ref()
            .map(|d| String::from_utf16_lossy(&d.dump))
            .map_err(Clone::clone)
    }

    /// The dump of the constant pool of this class (`getConstantPoolDump()`), in semi-UTF-8.
    pub fn constant_pool_dump(&self) -> Result<Vec<u8>> {
        self.dumps()
            .as_ref()
            .map(|d| semi_utf8(&d.constant_pool_dump))
            .map_err(Clone::clone)
    }

    /// The binary name of this class (`jdk3.AllInstructions`), lossy for unpaired surrogates.
    pub fn class_name(&self) -> Result<String> {
        self.dumps()
            .as_ref()
            .map(|d| String::from_utf16_lossy(&d.class_name))
            .map_err(Clone::clone)
    }

    /// Java's `hashCode()`: the `String.hashCode` of the dump.
    pub fn hash_code(&self) -> Result<i32> {
        self.dumps().as_ref().map_err(Clone::clone).map(|d| {
            d.dump
                .iter()
                .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
        })
    }
}

impl PartialEq for ClassFile<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self.dumps(), other.dumps()) {
            (Ok(a), Ok(b)) => a.dump == b.dump,
            (Err(a), Err(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for ClassFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dumps() {
            Ok(d) => f.write_str(&String::from_utf16_lossy(&d.dump)),
            Err(e) => write!(f, "<{e}>"),
        }
    }
}

impl fmt::Debug for ClassFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        fmt::Display::fmt(self, f)
    }
}

/// `new ClassFile(classBytes).toString()`, in semi-UTF-8.
pub fn dump(class_bytes: &[u8]) -> Result<Vec<u8>> {
    ClassFile::new(class_bytes).dump()
}

/// Encodes UTF-16 text as UTF-8, except that unpaired surrogates become 3-byte sequences (the
/// oracle's `semiUtf8`, the encoding of oxasm's `JavaStr`).
pub fn semi_utf8(text: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut i = 0;
    while i < text.len() {
        let c = u32::from(text[i]);
        if c < 0x80 {
            out.push(c as u8);
        } else if c < 0x800 {
            out.push(0xC0 | (c >> 6) as u8);
            out.push(0x80 | (c & 0x3F) as u8);
        } else if (0xD800..0xDC00).contains(&c)
            && i + 1 < text.len()
            && (0xDC00..0xE000).contains(&text[i + 1])
        {
            i += 1;
            let code_point = 0x10000 + ((c - 0xD800) << 10) + (u32::from(text[i]) - 0xDC00);
            out.push(0xF0 | (code_point >> 18) as u8);
            out.push(0x80 | ((code_point >> 12) & 0x3F) as u8);
            out.push(0x80 | ((code_point >> 6) & 0x3F) as u8);
            out.push(0x80 | (code_point & 0x3F) as u8);
        } else {
            out.push(0xE0 | (c >> 12) as u8);
            out.push(0x80 | ((c >> 6) & 0x3F) as u8);
            out.push(0x80 | (c & 0x3F) as u8);
        }
        i += 1;
    }
    out
}

// -------------------------------------------------------------------------------------------------
// Parser: Java's DataInputStream over the class bytes
// -------------------------------------------------------------------------------------------------

struct Parser<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(ClassFormatError::eof)?;
        let bytes = &self.data[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    fn u1(&mut self) -> Result<i32> {
        Ok(i32::from(self.take(1)?[0]))
    }

    fn s1(&mut self) -> Result<i32> {
        Ok(i32::from(self.take(1)?[0] as i8))
    }

    fn u2(&mut self) -> Result<i32> {
        let b = self.take(2)?;
        Ok(i32::from(u16::from_be_bytes([b[0], b[1]])))
    }

    fn s2(&mut self) -> Result<i32> {
        let b = self.take(2)?;
        Ok(i32::from(i16::from_be_bytes([b[0], b[1]])))
    }

    fn u4(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn s8(&mut self) -> Result<i64> {
        let b = self.take(8)?;
        Ok(i64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// `DataInputStream.readUTF`: Java's modified UTF-8, decoded into UTF-16 units.
    fn utf8(&mut self) -> Result<Vec<u16>> {
        let length = self.u2()? as usize;
        let bytes = self.take(length)?;
        let malformed = |message: String| ClassFormatError::class_format_str(&message);
        let mut chars = Vec::with_capacity(length);
        let mut count = 0;
        while count < length {
            let c = u32::from(bytes[count]);
            match c >> 4 {
                0..=7 => {
                    count += 1;
                    chars.push(c as u16);
                }
                12 | 13 => {
                    count += 2;
                    if count > length {
                        return Err(malformed(
                            "malformed input: partial character at end".to_owned(),
                        ));
                    }
                    let char2 = u32::from(bytes[count - 1]);
                    if char2 & 0xC0 != 0x80 {
                        return Err(malformed(format!("malformed input around byte {count}")));
                    }
                    chars.push((((c & 0x1F) << 6) | (char2 & 0x3F)) as u16);
                }
                14 => {
                    count += 3;
                    if count > length {
                        return Err(malformed(
                            "malformed input: partial character at end".to_owned(),
                        ));
                    }
                    let char2 = u32::from(bytes[count - 2]);
                    let char3 = u32::from(bytes[count - 1]);
                    if char2 & 0xC0 != 0x80 || char3 & 0xC0 != 0x80 {
                        return Err(malformed(format!(
                            "malformed input around byte {}",
                            count - 1
                        )));
                    }
                    chars
                        .push((((c & 0x0F) << 12) | ((char2 & 0x3F) << 6) | (char3 & 0x3F)) as u16);
                }
                _ => return Err(malformed(format!("malformed input around byte {count}"))),
            }
        }
        Ok(chars)
    }

    fn bytes(&mut self, length: i32) -> Result<&'a [u8]> {
        let available = self.data.len() - self.position;
        if i64::from(length) > available as i64 {
            return Err(ClassFormatError::class_format_str(&format!(
                "Invalid length: {length}"
            )));
        }
        if length < 0 {
            return Err(ClassFormatError {
                exception: NEGATIVE_ARRAY_SIZE_EXCEPTION,
                message: Some(length.to_string().encode_utf16().collect()),
            });
        }
        self.take(length as usize)
    }
}

// -------------------------------------------------------------------------------------------------
// Constant pool entries (the CpInfo subclasses)
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CpKind {
    Class,
    FieldRef,
    MethodRef,
    InterfaceMethodRef,
    String,
    Integer,
    Float,
    Long,
    Double,
    NameAndType,
    Utf8,
    MethodHandle,
    MethodType,
    Dynamic,
    InvokeDynamic,
    Module,
    Package,
}

impl CpKind {
    fn simple_name(self) -> &'static str {
        match self {
            CpKind::Class => "ConstantClassInfo",
            CpKind::FieldRef => "ConstantFieldRefInfo",
            CpKind::MethodRef => "ConstantMethodRefInfo",
            CpKind::InterfaceMethodRef => "ConstantInterfaceMethodRefInfo",
            CpKind::String => "ConstantStringInfo",
            CpKind::Integer => "ConstantIntegerInfo",
            CpKind::Float => "ConstantFloatInfo",
            CpKind::Long => "ConstantLongInfo",
            CpKind::Double => "ConstantDoubleInfo",
            CpKind::NameAndType => "ConstantNameAndTypeInfo",
            CpKind::Utf8 => "ConstantUtf8Info",
            CpKind::MethodHandle => "ConstantMethodHandleInfo",
            CpKind::MethodType => "ConstantMethodTypeInfo",
            CpKind::Dynamic => "ConstantDynamicInfo",
            CpKind::InvokeDynamic => "ConstantInvokeDynamicInfo",
            CpKind::Module => "ConstantModuleInfo",
            CpKind::Package => "ConstantPackageInfo",
        }
    }
}

/// A constant pool entry: its kind, plus either its (eagerly computed) dump for the "terminal"
/// entries that Java constructs with a dump string, or its two operands.
#[derive(Clone, Debug)]
struct CpInfo {
    kind: CpKind,
    terminal_dump: Option<Vec<u16>>,
    operand1: i32,
    operand2: i32,
}

impl CpInfo {
    fn terminal(kind: CpKind, dump: Vec<u16>) -> CpInfo {
        CpInfo {
            kind,
            terminal_dump: Some(dump),
            operand1: 0,
            operand2: 0,
        }
    }

    fn reference(kind: CpKind, operand1: i32, operand2: i32) -> CpInfo {
        CpInfo {
            kind,
            terminal_dump: None,
            operand1,
            operand2,
        }
    }

    fn size(&self) -> i32 {
        match self.kind {
            CpKind::Long | CpKind::Double => 2,
            _ => 1,
        }
    }
}

fn parse_cp_info(parser: &mut Parser<'_>) -> Result<CpInfo> {
    let tag = parser.u1()?;
    let ascii = |s: String| -> Vec<u16> { s.encode_utf16().collect() };
    Ok(match tag {
        7 => CpInfo::reference(CpKind::Class, parser.u2()?, 0),
        9 | 10 | 11 | 12 | 17 | 18 => {
            let kind = match tag {
                9 => CpKind::FieldRef,
                10 => CpKind::MethodRef,
                11 => CpKind::InterfaceMethodRef,
                12 => CpKind::NameAndType,
                17 => CpKind::Dynamic,
                _ => CpKind::InvokeDynamic,
            };
            let operand1 = parser.u2()?;
            CpInfo::reference(kind, operand1, parser.u2()?)
        }
        8 => CpInfo::reference(CpKind::String, parser.u2()?, 0),
        3 => CpInfo::terminal(CpKind::Integer, ascii(parser.u4()?.to_string())),
        4 => CpInfo::terminal(
            CpKind::Float,
            ascii(java_fmt::float_to_string(f32::from_bits(
                parser.u4()? as u32
            ))),
        ),
        5 => CpInfo::terminal(CpKind::Long, ascii(parser.s8()?.to_string())),
        6 => CpInfo::terminal(
            CpKind::Double,
            ascii(java_fmt::double_to_string(f64::from_bits(
                parser.s8()? as u64
            ))),
        ),
        1 => CpInfo::terminal(CpKind::Utf8, parser.utf8()?),
        15 => {
            let reference_kind = parser.u1()?;
            CpInfo::reference(CpKind::MethodHandle, reference_kind, parser.u2()?)
        }
        16 => CpInfo::reference(CpKind::MethodType, parser.u2()?, 0),
        19 => CpInfo::reference(CpKind::Module, parser.u2()?, 0),
        20 => CpInfo::reference(CpKind::Package, parser.u2()?, 0),
        _ => {
            return Err(ClassFormatError::class_format_str(&format!(
                "Invalid constant pool item tag {tag}"
            )));
        }
    })
}

// -------------------------------------------------------------------------------------------------
// Builders: the dump tree (AbstractBuilder, Builder and SortedBuilder)
// -------------------------------------------------------------------------------------------------

/// A child of a `Builder` (an element of its `children` list).
#[derive(Clone, Debug)]
enum Item {
    Str(&'static str),
    Text(Vec<u16>),
    Int(i64),
    /// A constant pool entry, printed with its `toString()`.
    Cp(usize),
    /// An `InstructionIndex`, resolved through the context of the builder `context`.
    Insn {
        bytecode_offset: i32,
        context: usize,
    },
    /// A `SortedBuilder`.
    Sorted(usize),
}

#[derive(Debug, Default)]
struct Node {
    parent: Option<usize>,
    /// Whether this is a `SortedBuilder` (whose children are the builders in `sorted`).
    is_sorted: bool,
    name: Vec<u16>,
    items: Vec<Item>,
    sorted: Vec<usize>,
    /// The bytecode offset → instruction index part of the builder's context.
    insn_indices: HashMap<i32, i32>,
}

/// The cp key part of the builders' context is the same in every builder chain (only the two
/// roots hold cp entries, and both hold all of them), so it is a single map here.
struct Dumper<'a> {
    parser: Parser<'a>,
    nodes: Vec<Node>,
    cp: Vec<CpInfo>,
    cp_index: HashMap<i32, usize>,
    nesting: u32,
}

const CLASS_BUILDER: usize = 0;
const CONSTANT_POOL_BUILDER: usize = 1;
const CP_INFO_KEY: u32 = 0xF000_0000;

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl<'a> Dumper<'a> {
    fn new(class_bytes: &'a [u8]) -> Dumper<'a> {
        Dumper {
            parser: Parser {
                data: class_bytes,
                position: 0,
            },
            nodes: vec![
                Node {
                    name: utf16("ClassFile"),
                    ..Node::default()
                },
                Node {
                    name: utf16("ConstantPool"),
                    ..Node::default()
                },
            ],
            cp: Vec::new(),
            cp_index: HashMap::new(),
            nesting: 0,
        }
    }

    // Builder operations -------------------------------------------------------------------------

    fn add(&mut self, builder: usize, name: &'static str, value: Item) {
        let items = &mut self.nodes[builder].items;
        items.push(Item::Str(name));
        items.push(value);
        items.push(Item::Str("\n"));
    }

    /// `add(name, value)` with a computed name.
    fn add_text(&mut self, builder: usize, name: String, value: Item) {
        let items = &mut self.nodes[builder].items;
        items.push(Item::Text(utf16(&name)));
        items.push(value);
        items.push(Item::Str("\n"));
    }

    /// `addInsnIndex(name, bytecodeOffset)` with a computed name.
    fn add_text_insn_index(&mut self, builder: usize, name: String, bytecode_offset: i32) {
        let item = Item::Insn {
            bytecode_offset,
            context: builder,
        };
        self.add_text(builder, name, item);
    }

    fn add_int(&mut self, builder: usize, name: &'static str, value: i32) -> i32 {
        self.add(builder, name, Item::Int(i64::from(value)));
        value
    }

    fn add_insn_index(&mut self, builder: usize, name: &'static str, bytecode_offset: i32) -> i32 {
        let item = Item::Insn {
            bytecode_offset,
            context: builder,
        };
        self.add(builder, name, item);
        bytecode_offset
    }

    fn add_cp_info(&mut self, builder: usize, name: &'static str, cp_index: i32) -> Result<()> {
        let value = if cp_index == 0 {
            Item::Int(0)
        } else {
            Item::Cp(self.get_cp_info(cp_index)?)
        };
        self.add(builder, name, value);
        Ok(())
    }

    fn add_insn(&mut self, builder: usize, insn_index: i32, opcode: i32, arguments: Vec<Item>) {
        let items = &mut self.nodes[builder].items;
        items.push(Item::Int(i64::from(insn_index)));
        items.push(Item::Str(": "));
        items.push(Item::Int(i64::from(opcode)));
        for argument in arguments {
            items.push(Item::Str(" "));
            items.push(argument);
        }
        items.push(Item::Str("\n"));
    }

    fn add_sorted_builder(&mut self, builder: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(builder),
            is_sorted: true,
            ..Node::default()
        });
        self.nodes[builder].items.push(Item::Sorted(id));
        id
    }

    fn add_builder(&mut self, sorted_builder: usize, name: Vec<u16>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(sorted_builder),
            name,
            ..Node::default()
        });
        self.nodes[sorted_builder].sorted.push(id);
        id
    }

    fn put_insn_index(&mut self, builder: usize, bytecode_offset: i32, insn_index: i32) {
        self.nodes[builder]
            .insn_indices
            .insert(bytecode_offset, insn_index);
    }

    /// `getCpInfo(cpIndex)`: the entry at this index, of any type.
    fn get_cp_info(&self, cp_index: i32) -> Result<usize> {
        self.cp_index.get(&cp_index).copied().ok_or_else(|| {
            ClassFormatError::class_format_str(&format!("Invalid constant pool index: {cp_index}"))
        })
    }

    /// `getCpInfo(cpIndex, cpInfoType)`.
    fn get_typed_cp_info(&self, cp_index: i32, kind: CpKind) -> Result<usize> {
        let entry = self.get_cp_info(cp_index)?;
        let actual = self.cp[entry].kind;
        if actual != kind {
            return Err(ClassFormatError::class_format_str(&format!(
                "Invalid constant pool type: {CLASS_FILE}${} should be {CLASS_FILE}${}",
                actual.simple_name(),
                kind.simple_name()
            )));
        }
        Ok(entry)
    }

    /// `getInsnIndex(bytecodeOffset)`, from the context of `builder`.
    fn get_insn_index(&self, builder: usize, bytecode_offset: i32) -> Result<i32> {
        let mut node = Some(builder);
        while let Some(id) = node {
            if let Some(insn_index) = self.nodes[id].insn_indices.get(&bytecode_offset) {
                return Ok(*insn_index);
            }
            node = self.nodes[id].parent;
        }
        // The builders' context also maps CP_INFO_KEY | cpIndex keys to constant pool entries,
        // which Java would then fail to cast to an Integer.
        let key = bytecode_offset as u32;
        if key & CP_INFO_KEY == CP_INFO_KEY
            && let Some(entry) = self.cp_index.get(&((key & !CP_INFO_KEY) as i32))
            && key & !CP_INFO_KEY <= 0xFFFF
        {
            let class = format!("{CLASS_FILE}${}", self.cp[*entry].kind.simple_name());
            return Err(ClassFormatError {
                exception: CLASS_CAST_EXCEPTION,
                message: Some(utf16(&format!(
                    "class {class} cannot be cast to class java.lang.Integer ({class} is in \
                     unnamed module of loader 'app'; java.lang.Integer is in module java.base \
                     of loader 'bootstrap')"
                ))),
            });
        }
        Err(ClassFormatError::class_format_str(&format!(
            "Invalid bytecode offset: {bytecode_offset}"
        )))
    }

    // CpInfo.dump() and toString() ---------------------------------------------------------------

    fn cp_dump(&self, entry: usize, depth: u32) -> Result<Vec<u16>> {
        if depth > MAX_NESTING {
            return Err(ClassFormatError::stack_overflow());
        }
        let info = &self.cp[entry];
        if let Some(dump) = &info.terminal_dump {
            return Ok(dump.clone());
        }
        let utf8 = |index: i32| -> Result<Vec<u16>> {
            self.cp_dump(self.get_typed_cp_info(index, CpKind::Utf8)?, depth + 1)
        };
        Ok(match info.kind {
            CpKind::Class
            | CpKind::String
            | CpKind::MethodType
            | CpKind::Module
            | CpKind::Package => utf8(info.operand1)?,
            CpKind::FieldRef | CpKind::MethodRef | CpKind::InterfaceMethodRef => {
                let class = self.get_typed_cp_info(info.operand1, CpKind::Class)?;
                let mut dump = self.cp_dump(class, depth + 1)?;
                dump.push(u16::from(b'.'));
                let name_and_type = self.get_typed_cp_info(info.operand2, CpKind::NameAndType)?;
                dump.extend(self.cp_dump(name_and_type, depth + 1)?);
                dump
            }
            CpKind::NameAndType => {
                let mut dump = utf8(info.operand1)?;
                dump.extend(utf8(info.operand2)?);
                dump
            }
            CpKind::MethodHandle => {
                let mut dump = utf16(&format!("{}.", info.operand1));
                let reference = self.get_cp_info(info.operand2)?;
                dump.extend(self.cp_to_string(reference, depth + 1)?);
                dump
            }
            CpKind::Dynamic | CpKind::InvokeDynamic => {
                let mut dump = utf16(&format!("{}.", info.operand1));
                let name_and_type = self.get_typed_cp_info(info.operand2, CpKind::NameAndType)?;
                dump.extend(self.cp_dump(name_and_type, depth + 1)?);
                dump
            }
            CpKind::Integer | CpKind::Float | CpKind::Long | CpKind::Double | CpKind::Utf8 => {
                unreachable!("terminal entries have a dump")
            }
        })
    }

    fn cp_to_string(&self, entry: usize, depth: u32) -> Result<Vec<u16>> {
        let info = &self.cp[entry];
        if let Some(dump) = &info.terminal_dump {
            return Ok(dump.clone());
        }
        let mut text = utf16(info.kind.simple_name());
        text.push(u16::from(b' '));
        text.extend(self.cp_dump(entry, depth)?);
        Ok(text)
    }

    // Output --------------------------------------------------------------------------------------

    fn item_to_string(&self, item: &Item, out: &mut Vec<u16>) -> Result<()> {
        match item {
            Item::Str(s) => out.extend(s.encode_utf16()),
            Item::Text(text) => out.extend_from_slice(text),
            Item::Int(value) => out.extend(value.to_string().encode_utf16()),
            Item::Cp(entry) => out.extend(self.cp_to_string(*entry, 0)?),
            Item::Insn {
                bytecode_offset,
                context,
            } => {
                let insn_index = self.get_insn_index(*context, *bytecode_offset)?;
                out.extend(format!("<{insn_index}>").encode_utf16());
            }
            Item::Sorted(id) => self.build(*id, out)?,
        }
        Ok(())
    }

    fn build(&self, id: usize, out: &mut Vec<u16>) -> Result<()> {
        let node = &self.nodes[id];
        if node.is_sorted {
            // Collections.sort: stable, by name, in UTF-16 order (which Vec<u16> ordering is).
            let mut children = node.sorted.clone();
            children.sort_by(|a, b| self.nodes[*a].name.cmp(&self.nodes[*b].name));
            for child in children {
                self.build(child, out)?;
            }
        } else {
            for item in &node.items {
                self.item_to_string(item, out)?;
            }
        }
        Ok(())
    }

    /// `Builder.sortByContent()`.
    fn sort_by_content(&mut self, builder: usize) -> Result<()> {
        let mut name = Vec::new();
        for item in &self.nodes[builder].items {
            match item {
                // Instruction index might not be known at this point, use bytecodeOffset instead.
                Item::Insn {
                    bytecode_offset, ..
                } => name.extend(bytecode_offset.to_string().encode_utf16()),
                // Unreachable: type annotation builders contain no sorted builder.
                Item::Sorted(_) => {}
                _ => self.item_to_string(item, &mut name)?,
            }
        }
        self.nodes[builder].name = name;
        Ok(())
    }

    // The class file structure -------------------------------------------------------------------

    /// `dumpClassFile`; returns the entry of the class's `ConstantClassInfo`.
    fn dump_class_file(&mut self) -> Result<usize> {
        let builder = CLASS_BUILDER;
        let magic = self.parser.u4()?;
        self.add_int(builder, "magic: ", magic);
        let minor_version = self.parser.u2()?;
        self.add_int(builder, "minor_version: ", minor_version);
        let major_version = self.parser.u2()?;
        if major_version > /* V15= */ 59 {
            return Err(ClassFormatError::class_format_str(
                "Unsupported class version",
            ));
        }
        self.add_int(builder, "major_version: ", major_version);
        let constant_pool_count = self.parser.u2()?;
        let mut cp_index = 1;
        while cp_index < constant_pool_count {
            let cp_info = parse_cp_info(&mut self.parser)?;
            let size = cp_info.size();
            self.cp.push(cp_info);
            self.cp_index.insert(cp_index, self.cp.len() - 1);
            self.add_cp_info(CONSTANT_POOL_BUILDER, "constant_pool: ", cp_index)?;
            cp_index += size;
        }
        let access_flags = self.parser.u2()?;
        self.add_int(builder, "access_flags: ", access_flags);
        let this_class = self.parser.u2()?;
        self.add_cp_info(builder, "this_class: ", this_class)?;
        let super_class = self.parser.u2()?;
        self.add_cp_info(builder, "super_class: ", super_class)?;
        let interface_count = self.parser.u2()?;
        self.add_int(builder, "interfaces_count: ", interface_count);
        for _ in 0..interface_count {
            let interface = self.parser.u2()?;
            self.add_cp_info(builder, "interface: ", interface)?;
        }
        let field_count = self.parser.u2()?;
        self.add_int(builder, "fields_count: ", field_count);
        for _ in 0..field_count {
            self.dump_field_info(builder)?;
        }
        let method_count = self.parser.u2()?;
        self.add_int(builder, "methods_count: ", method_count);
        for _ in 0..method_count {
            // method_info has the same top level structure as field_info.
            self.dump_field_info(builder)?;
        }
        self.dump_attribute_list(builder)?;
        self.get_typed_cp_info(this_class, CpKind::Class)
    }

    fn dump_attribute_list(&mut self, builder: usize) -> Result<()> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return Err(ClassFormatError::stack_overflow());
        }
        let attribute_count = self.parser.u2()?;
        self.add_int(builder, "attributes_count: ", attribute_count);
        let sorted_builder = self.add_sorted_builder(builder);
        for _ in 0..attribute_count {
            self.dump_attribute_info(sorted_builder)?;
        }
        self.nesting -= 1;
        Ok(())
    }

    fn dump_field_info(&mut self, builder: usize) -> Result<()> {
        let access_flags = self.parser.u2()?;
        self.add_int(builder, "access_flags: ", access_flags);
        let name_index = self.parser.u2()?;
        self.add_cp_info(builder, "name_index: ", name_index)?;
        let descriptor_index = self.parser.u2()?;
        self.add_cp_info(builder, "descriptor_index: ", descriptor_index)?;
        self.dump_attribute_list(builder)
    }

    fn dump_attribute_info(&mut self, sorted_builder: usize) -> Result<()> {
        let name_index = self.parser.u2()?;
        let attribute_name = self.cp_to_string(self.get_cp_info(name_index)?, 0)?;
        let attribute_length = self.parser.u4()?;
        let builder = self.add_builder(sorted_builder, attribute_name.clone());
        self.add(
            builder,
            "attribute_name_index: ",
            Item::Text(attribute_name.clone()),
        );
        let name = String::from_utf16(&attribute_name).unwrap_or_default();
        match name.as_str() {
            "ConstantValue" => self.dump_cp_info_field(builder, "constantvalue_index: "),
            "Code" => self.dump_code_attribute(builder),
            "StackMapTable" => self.dump_stack_map_table_attribute(builder),
            "Exceptions" => {
                self.dump_cp_info_list(builder, "number_of_exceptions: ", "exception_index: ")
            }
            "InnerClasses" => self.dump_inner_classes_attribute(builder),
            "EnclosingMethod" => {
                self.dump_cp_info_field(builder, "class_index: ")?;
                self.dump_cp_info_field(builder, "method_index: ")
            }
            "Synthetic" | "Deprecated" | "CodeComment" | "Comment" => Ok(()),
            "Signature" => self.dump_cp_info_field(builder, "signature_index: "),
            "SourceFile" => self.dump_cp_info_field(builder, "sourcefile_index: "),
            "SourceDebugExtension" => self.dump_source_debug_attribute(attribute_length, builder),
            "LineNumberTable" => self.dump_line_number_table_attribute(builder),
            "LocalVariableTable" => self.dump_local_variable_table_attribute(
                builder,
                "local_variable_table_length: ",
                "descriptor_index: ",
            ),
            "LocalVariableTypeTable" => self.dump_local_variable_table_attribute(
                builder,
                "local_variable_type_table_length: ",
                "signature_index: ",
            ),
            "RuntimeVisibleAnnotations" | "RuntimeInvisibleAnnotations" => {
                self.dump_annotations_attribute(builder)
            }
            "RuntimeVisibleParameterAnnotations" | "RuntimeInvisibleParameterAnnotations" => {
                self.dump_parameter_annotations_attribute(builder)
            }
            "RuntimeVisibleTypeAnnotations" | "RuntimeInvisibleTypeAnnotations" => {
                self.dump_type_annotations_attribute(builder)
            }
            "AnnotationDefault" => self.dump_element_values(builder, Task::ElementValue),
            "BootstrapMethods" => self.dump_bootstrap_methods_attribute(builder),
            "MethodParameters" => self.dump_method_parameters_attribute(builder),
            "Module" => self.dump_module_attribute(builder),
            "ModulePackages" => self.dump_cp_info_list(builder, "package_count: ", "package: "),
            "ModuleMainClass" => self.dump_cp_info_field(builder, "main_class: "),
            "NestHost" => self.dump_cp_info_field(builder, "host_class: "),
            "NestMembers" => self.dump_cp_info_list(builder, "number_of_classes: ", "class: "),
            "PermittedSubclasses" => {
                self.dump_cp_info_list(builder, "permitted_subclasses_count: ", "class: ")
            }
            "Record" => self.dump_record_attribute(builder),
            "StackMap" => self.dump_stack_map_attribute(builder),
            _ => {
                // Not a standard attribute nor one the of empty non-standard attributes used for
                // tests.
                let mut message = utf16("Unknown attribute ");
                message.extend(attribute_name);
                Err(ClassFormatError::class_format(message))
            }
        }
    }

    fn dump_cp_info_field(&mut self, builder: usize, name: &'static str) -> Result<()> {
        let cp_index = self.parser.u2()?;
        self.add_cp_info(builder, name, cp_index)
    }

    fn dump_cp_info_list(
        &mut self,
        builder: usize,
        count_name: &'static str,
        name: &'static str,
    ) -> Result<()> {
        let count = self.parser.u2()?;
        self.add_int(builder, count_name, count);
        for _ in 0..count {
            self.dump_cp_info_field(builder, name)?;
        }
        Ok(())
    }

    fn dump_code_attribute(&mut self, builder: usize) -> Result<()> {
        let max_stack = self.parser.u2()?;
        self.add_int(builder, "max_stack: ", max_stack);
        let max_locals = self.parser.u2()?;
        self.add_int(builder, "max_locals: ", max_locals);
        let code_length = self.parser.u4()?;
        self.dump_instructions(code_length, builder)?;
        let exception_count = self.parser.u2()?;
        self.add_int(builder, "exception_table_length: ", exception_count);
        for _ in 0..exception_count {
            for name in ["start_pc: ", "end_pc: ", "handler_pc: "] {
                let pc = self.parser.u2()?;
                self.add_insn_index(builder, name, pc);
            }
            self.dump_cp_info_field(builder, "catch_type: ")?;
        }
        self.dump_attribute_list(builder)
    }

    fn cp_operand(&mut self, cp_index: i32) -> Result<Item> {
        Ok(Item::Cp(self.get_cp_info(cp_index)?))
    }

    #[allow(clippy::too_many_lines)] // one arm per opcode family, as in the Java switch
    fn dump_instructions(&mut self, code_length: i32, builder: usize) -> Result<()> {
        let int = |value: i32| Item::Int(i64::from(value));
        let mut bytecode_offset: i32 = 0; // Number of bytes parsed so far.
        let mut insn_index: i32 = 0; // Number of instructions parsed so far.
        while bytecode_offset < code_length {
            self.put_insn_index(builder, bytecode_offset, insn_index);
            let opcode = self.parser.u1()?;
            let start_offset = bytecode_offset;
            bytecode_offset += 1;
            let jump = |offset: i32| Item::Insn {
                bytecode_offset: start_offset.wrapping_add(offset),
                context: builder,
            };
            match opcode {
                0x00..=0x0F
                | 0x2E..=0x35
                | 0x4F..=0x83
                | 0x85..=0x98
                | 0xAC..=0xB1
                | 0xBE
                | 0xBF
                | 0xC2
                | 0xC3 => self.add_insn(builder, insn_index, opcode, vec![]),
                // aload, astore, bipush, dload, dstore, fload, fstore, iload, istore, lload,
                // lstore, newarray, ret.
                0x19 | 0x3A | 0x10 | 0x18 | 0x39 | 0x17 | 0x38 | 0x15 | 0x36 | 0x16 | 0x37
                | 0xBC | 0xA9 => {
                    let operand = self.parser.u1()?;
                    self.add_insn(builder, insn_index, opcode, vec![int(operand)]);
                    bytecode_offset += 1;
                }
                // <x>load_<n> and <x>store_<n>.
                0x1A..=0x2D | 0x3B..=0x4E => {
                    let (base, first) = match opcode {
                        0x1A..=0x1D => (0x15, 0x1A),
                        0x1E..=0x21 => (0x16, 0x1E),
                        0x22..=0x25 => (0x17, 0x22),
                        0x26..=0x29 => (0x18, 0x26),
                        0x2A..=0x2D => (0x19, 0x2A),
                        0x3B..=0x3E => (0x36, 0x3B),
                        0x3F..=0x42 => (0x37, 0x3F),
                        0x43..=0x46 => (0x38, 0x43),
                        0x47..=0x4A => (0x39, 0x47),
                        _ => (0x3A, 0x4B),
                    };
                    self.add_insn(builder, insn_index, base, vec![int(opcode - first)]);
                }
                // anewarray, checkcast, getfield, getstatic, instanceof, invokespecial,
                // invokestatic, invokevirtual, new, putfield, putstatic.
                0xBD | 0xC0 | 0xB4 | 0xB2 | 0xC1 | 0xB7 | 0xB8 | 0xB6 | 0xBB | 0xB5 | 0xB3 => {
                    let cp_index = self.parser.u2()?;
                    let operand = self.cp_operand(cp_index)?;
                    self.add_insn(builder, insn_index, opcode, vec![operand]);
                    bytecode_offset += 2;
                }
                // goto, if<cond>, if_<x>cmp<cond>, ifnull, ifnonnull, jsr.
                0xA7 | 0x99..=0xA6 | 0xC6 | 0xC7 | 0xA8 => {
                    let offset = self.parser.s2()?;
                    self.add_insn(builder, insn_index, opcode, vec![jump(offset)]);
                    bytecode_offset += 2;
                }
                // goto_w, jsr_w.
                0xC8 | 0xC9 => {
                    let offset = self.parser.u4()?;
                    let base = if opcode == 0xC8 { 0xA7 } else { 0xA8 };
                    self.add_insn(builder, insn_index, base, vec![jump(offset)]);
                    bytecode_offset += 4;
                }
                0x84 => {
                    // iinc
                    let index = self.parser.u1()?;
                    let increment = self.parser.s1()?;
                    self.add_insn(
                        builder,
                        insn_index,
                        opcode,
                        vec![int(index), int(increment)],
                    );
                    bytecode_offset += 2;
                }
                0xBA => {
                    // invokedynamic
                    let cp_index = self.parser.u2()?;
                    let operand = self.cp_operand(cp_index)?;
                    self.add_insn(builder, insn_index, opcode, vec![operand]);
                    self.parser.u2()?;
                    bytecode_offset += 4;
                }
                0xB9 => {
                    // invokeinterface
                    let cp_index = self.parser.u2()?;
                    let operand = self.cp_operand(cp_index)?;
                    let count = self.parser.u1()?;
                    self.add_insn(builder, insn_index, opcode, vec![operand, int(count)]);
                    self.parser.u1()?;
                    bytecode_offset += 4;
                }
                0x12 => {
                    // ldc
                    let cp_index = self.parser.u1()?;
                    let operand = self.cp_operand(cp_index)?;
                    self.add_insn(builder, insn_index, 0x12, vec![operand]);
                    bytecode_offset += 1;
                }
                0x13 | 0x14 => {
                    // ldc_w, ldc2_w
                    let cp_index = self.parser.u2()?;
                    let operand = self.cp_operand(cp_index)?;
                    let base = if opcode == 0x13 { 0x12 } else { 0x14 };
                    self.add_insn(builder, insn_index, base, vec![operand]);
                    bytecode_offset += 2;
                }
                0xAB => {
                    // lookupswitch
                    self.add_insn(builder, insn_index, opcode, vec![]);
                    while bytecode_offset % 4 != 0 {
                        self.parser.u1()?;
                        bytecode_offset += 1;
                    }
                    let default = self.parser.u4()?;
                    self.add_insn_index(builder, "default: ", start_offset.wrapping_add(default));
                    let pair_count = self.parser.u4()?;
                    self.add_int(builder, "npairs: ", pair_count);
                    bytecode_offset = bytecode_offset.wrapping_add(8);
                    for _ in 0..pair_count {
                        let key = self.parser.u4()?;
                        let offset = self.parser.u4()?;
                        self.add_text_insn_index(
                            builder,
                            format!("{key}: "),
                            start_offset.wrapping_add(offset),
                        );
                        bytecode_offset = bytecode_offset.wrapping_add(8);
                    }
                }
                0xC5 => {
                    // multianewarray
                    let cp_index = self.parser.u2()?;
                    let operand = self.cp_operand(cp_index)?;
                    let dimensions = self.parser.u1()?;
                    self.add_insn(builder, insn_index, opcode, vec![operand, int(dimensions)]);
                    bytecode_offset += 3;
                }
                0x11 => {
                    // sipush
                    let operand = self.parser.s2()?;
                    self.add_insn(builder, insn_index, opcode, vec![int(operand)]);
                    bytecode_offset += 2;
                }
                0xAA => {
                    // tableswitch
                    self.add_insn(builder, insn_index, opcode, vec![]);
                    while bytecode_offset % 4 != 0 {
                        self.parser.u1()?;
                        bytecode_offset += 1;
                    }
                    let default = self.parser.u4()?;
                    self.add_insn_index(builder, "default: ", start_offset.wrapping_add(default));
                    let low = self.parser.u4()?;
                    self.add_int(builder, "low: ", low);
                    let high = self.parser.u4()?;
                    self.add_int(builder, "high: ", high);
                    bytecode_offset = bytecode_offset.wrapping_add(12);
                    let mut i = i64::from(low);
                    while i <= i64::from(high) {
                        let offset = self.parser.u4()?;
                        self.add_text_insn_index(
                            builder,
                            format!("{i}: "),
                            start_offset.wrapping_add(offset),
                        );
                        bytecode_offset = bytecode_offset.wrapping_add(4);
                        i += 1;
                    }
                }
                0xC4 => {
                    // wide
                    let opcode = self.parser.u1()?;
                    bytecode_offset += 1;
                    match opcode {
                        0x15 | 0x17 | 0x19 | 0x16 | 0x18 | 0x36 | 0x38 | 0x3A | 0x37 | 0x39
                        | 0xA9 => {
                            let operand = self.parser.u2()?;
                            self.add_insn(builder, insn_index, opcode, vec![int(operand)]);
                            bytecode_offset += 2;
                        }
                        0x84 => {
                            let index = self.parser.u2()?;
                            let increment = self.parser.s2()?;
                            self.add_insn(
                                builder,
                                insn_index,
                                opcode,
                                vec![int(index), int(increment)],
                            );
                            bytecode_offset += 4;
                        }
                        _ => {
                            return Err(ClassFormatError::class_format_str(&format!(
                                "Unknown wide opcode: {opcode}"
                            )));
                        }
                    }
                }
                _ => {
                    return Err(ClassFormatError::class_format_str(&format!(
                        "Unknown opcode: {opcode}"
                    )));
                }
            }
            insn_index += 1;
        }
        self.put_insn_index(builder, bytecode_offset, insn_index);
        Ok(())
    }

    fn dump_stack_map_table_attribute(&mut self, builder: usize) -> Result<()> {
        let entry_count = self.parser.u2()?;
        self.add_int(builder, "number_of_entries: ", entry_count);
        let mut bytecode_offset: i32 = -1;
        for _ in 0..entry_count {
            let frame_type = self.parser.u1()?;
            if frame_type < 64 {
                bytecode_offset += frame_type + 1;
                self.add_insn_index(builder, "SAME ", bytecode_offset);
            } else if frame_type < 128 {
                bytecode_offset += frame_type - 64 + 1;
                self.add_insn_index(builder, "SAME_LOCALS_1_STACK_ITEM ", bytecode_offset);
                self.dump_verification_type_info(builder)?;
            } else if frame_type < 247 {
                return Err(ClassFormatError::class_format_str(&format!(
                    "Unknown frame type {frame_type}"
                )));
            } else {
                let offset_delta = self.parser.u2()?;
                bytecode_offset += offset_delta + 1;
                if frame_type == 247 {
                    self.add_insn_index(builder, "SAME_LOCALS_1_STACK_ITEM ", bytecode_offset);
                    self.dump_verification_type_info(builder)?;
                } else if frame_type < 251 {
                    let name = format!("CHOP_{} ", 251 - frame_type);
                    self.add_text_insn_index(builder, name, bytecode_offset);
                } else if frame_type == 251 {
                    self.add_insn_index(builder, "SAME ", bytecode_offset);
                } else if frame_type < 255 {
                    let name = format!("APPEND_{} ", frame_type - 251);
                    self.add_text_insn_index(builder, name, bytecode_offset);
                    for _ in 0..frame_type - 251 {
                        self.dump_verification_type_info(builder)?;
                    }
                } else {
                    self.add_insn_index(builder, "FULL ", bytecode_offset);
                    self.dump_verification_type_infos(builder)?;
                }
            }
        }
        Ok(())
    }

    /// The `number_of_locals`, locals, `number_of_stack_items` and stack items of a full frame.
    fn dump_verification_type_infos(&mut self, builder: usize) -> Result<()> {
        for count_name in ["number_of_locals: ", "number_of_stack_items: "] {
            let count = self.parser.u2()?;
            self.add_int(builder, count_name, count);
            for _ in 0..count {
                self.dump_verification_type_info(builder)?;
            }
        }
        Ok(())
    }

    fn dump_verification_type_info(&mut self, builder: usize) -> Result<()> {
        let tag = self.parser.u1()?;
        self.add_int(builder, "tag: ", tag);
        if tag > 8 {
            return Err(ClassFormatError::class_format_str(&format!(
                "Unknown verification_type_info tag: {tag}"
            )));
        }
        if tag == 7 {
            self.dump_cp_info_field(builder, "cpool_index: ")?;
        } else if tag == 8 {
            let offset = self.parser.u2()?;
            self.add_insn_index(builder, "offset: ", offset);
        }
        Ok(())
    }

    fn dump_inner_classes_attribute(&mut self, builder: usize) -> Result<()> {
        let class_count = self.parser.u2()?;
        self.add_int(builder, "number_of_classes: ", class_count);
        for _ in 0..class_count {
            self.dump_cp_info_field(builder, "inner_class_info_index: ")?;
            self.dump_cp_info_field(builder, "outer_class_info_index: ")?;
            self.dump_cp_info_field(builder, "inner_name_index: ")?;
            let access_flags = self.parser.u2()?;
            self.add_int(builder, "inner_class_access_flags: ", access_flags);
        }
        Ok(())
    }

    fn dump_source_debug_attribute(&mut self, attribute_length: i32, builder: usize) -> Result<()> {
        let attribute_data = self.parser.bytes(attribute_length)?;
        let mut text = String::new();
        for data in attribute_data {
            text.push_str(&(*data as i8).to_string());
            text.push(',');
        }
        self.add(builder, "debug_extension: ", Item::Text(utf16(&text)));
        Ok(())
    }

    fn dump_line_number_table_attribute(&mut self, builder: usize) -> Result<()> {
        let line_number_count = self.parser.u2()?;
        self.add_int(builder, "line_number_table_length: ", line_number_count);
        for _ in 0..line_number_count {
            let start_pc = self.parser.u2()?;
            self.add_insn_index(builder, "start_pc: ", start_pc);
            let line_number = self.parser.u2()?;
            self.add_int(builder, "line_number: ", line_number);
        }
        Ok(())
    }

    fn dump_local_variable_table_attribute(
        &mut self,
        builder: usize,
        count_name: &'static str,
        descriptor_name: &'static str,
    ) -> Result<()> {
        let local_variable_count = self.parser.u2()?;
        self.add_int(builder, count_name, local_variable_count);
        for _ in 0..local_variable_count {
            let start_pc = self.parser.u2()?;
            self.add_insn_index(builder, "start_pc: ", start_pc);
            let length = self.parser.u2()?;
            self.add_insn_index(builder, "length: ", start_pc + length);
            self.dump_cp_info_field(builder, "name_index: ")?;
            self.dump_cp_info_field(builder, descriptor_name)?;
            let index = self.parser.u2()?;
            self.add_int(builder, "index: ", index);
        }
        Ok(())
    }

    fn dump_annotations_attribute(&mut self, builder: usize) -> Result<()> {
        let annotation_count = self.parser.u2()?;
        self.add_int(builder, "num_annotations: ", annotation_count);
        for _ in 0..annotation_count {
            self.dump_element_values(builder, Task::Annotation)?;
        }
        Ok(())
    }

    fn dump_parameter_annotations_attribute(&mut self, builder: usize) -> Result<()> {
        let parameter_count = self.parser.u1()?;
        self.add_int(builder, "num_parameters: ", parameter_count);
        for _ in 0..parameter_count {
            self.dump_annotations_attribute(builder)?;
        }
        Ok(())
    }

    /// `dumpAnnotation` / `dumpElementValue`, with an explicit stack instead of recursion.
    fn dump_element_values(&mut self, builder: usize, task: Task) -> Result<()> {
        let mut stack = vec![task];
        while let Some(task) = stack.pop() {
            match task {
                Task::Annotation => {
                    self.dump_cp_info_field(builder, "type_index: ")?;
                    let pair_count = self.parser.u2()?;
                    self.add_int(builder, "num_element_value_pairs: ", pair_count);
                    stack.push(Task::ElementValuePairs(pair_count));
                }
                Task::ElementValuePairs(0) | Task::ElementValues(0) => {}
                Task::ElementValuePairs(remaining) => {
                    stack.push(Task::ElementValuePairs(remaining - 1));
                    self.dump_cp_info_field(builder, "element_name_index: ")?;
                    stack.push(Task::ElementValue);
                }
                Task::ElementValues(remaining) => {
                    stack.push(Task::ElementValues(remaining - 1));
                    stack.push(Task::ElementValue);
                }
                Task::ElementValue => {
                    let tag = self.parser.u1()?;
                    match tag as u8 {
                        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' | b'c' => {
                            let cp_index = self.parser.u2()?;
                            let value = if cp_index == 0 {
                                Item::Int(0)
                            } else {
                                Item::Cp(self.get_cp_info(cp_index)?)
                            };
                            let name = format!("{}: ", tag as u8 as char);
                            self.add_text(builder, name, value);
                        }
                        b'e' => {
                            self.dump_cp_info_field(builder, "e: ")?;
                            self.dump_cp_info_field(builder, "const_name_index: ")?;
                        }
                        b'@' => {
                            self.add(builder, "@: ", Item::Str(""));
                            stack.push(Task::Annotation);
                        }
                        b'[' => {
                            let value_count = self.parser.u2()?;
                            self.add_int(builder, "[: ", value_count);
                            stack.push(Task::ElementValues(value_count));
                        }
                        _ => {
                            return Err(ClassFormatError::class_format_str(&format!(
                                "Unknown element_type tag: {tag}"
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn dump_type_annotations_attribute(&mut self, builder: usize) -> Result<()> {
        let annotation_count = self.parser.u2()?;
        self.add_int(builder, "num_annotations: ", annotation_count);
        let sorted_builder = self.add_sorted_builder(builder);
        for _ in 0..annotation_count {
            self.dump_type_annotation(sorted_builder)?;
        }
        Ok(())
    }

    fn dump_type_annotation(&mut self, sorted_builder: usize) -> Result<()> {
        let target_type = self.parser.u1()?;
        let builder = self.add_builder(sorted_builder, utf16(&target_type.to_string()));
        self.add_int(builder, "target_type: ", target_type);
        match target_type {
            // type_parameter_target
            0x00 | 0x01 => self.dump_u1(builder, "type_parameter_index: ")?,
            // supertype_target
            0x10 => self.dump_u2(builder, "supertype_index: ")?,
            // type_parameter_bound_target
            0x11 | 0x12 => {
                self.dump_u1(builder, "type_parameter_index: ")?;
                self.dump_u1(builder, "bound_index: ")?;
            }
            // empty_target
            0x13..=0x15 => {}
            // formal_parameter_target
            0x16 => self.dump_u1(builder, "formal_parameter_index: ")?,
            // throws_target
            0x17 => self.dump_u2(builder, "throws_type_index: ")?,
            // localvar_target
            0x40 | 0x41 => {
                let table_length = self.parser.u2()?;
                self.add_int(builder, "table_length: ", table_length);
                for _ in 0..table_length {
                    let start_pc = self.parser.u2()?;
                    self.add_insn_index(builder, "start_pc: ", start_pc);
                    let length = self.parser.u2()?;
                    self.add_insn_index(builder, "length: ", start_pc + length);
                    self.dump_u2(builder, "index: ")?;
                }
            }
            // catch_target
            0x42 => self.dump_u2(builder, "exception_table_index: ")?,
            // offset_target
            0x43..=0x46 => {
                let offset = self.parser.u2()?;
                self.add_insn_index(builder, "offset: ", offset);
            }
            // type_argument_target
            0x47..=0x4B => {
                let offset = self.parser.u2()?;
                self.add_insn_index(builder, "offset: ", offset);
                self.dump_u1(builder, "type_argument_index: ")?;
            }
            _ => {
                return Err(ClassFormatError::class_format_str(&format!(
                    "Unknown target_type: {target_type}"
                )));
            }
        }
        // dumpTypePath.
        let path_length = self.parser.u1()?;
        self.add_int(builder, "path_length: ", path_length);
        for _ in 0..path_length {
            self.dump_u1(builder, "type_path_kind: ")?;
            self.dump_u1(builder, "type_argument_index: ")?;
        }
        // Sort type annotations based on the full target_info and type_path (excluding the
        // annotation content), instead of only on their target_type.
        self.sort_by_content(builder)?;
        self.dump_element_values(builder, Task::Annotation)
    }

    fn dump_u1(&mut self, builder: usize, name: &'static str) -> Result<()> {
        let value = self.parser.u1()?;
        self.add_int(builder, name, value);
        Ok(())
    }

    fn dump_u2(&mut self, builder: usize, name: &'static str) -> Result<()> {
        let value = self.parser.u2()?;
        self.add_int(builder, name, value);
        Ok(())
    }

    fn dump_bootstrap_methods_attribute(&mut self, builder: usize) -> Result<()> {
        let bootstrap_method_count = self.parser.u2()?;
        self.add_int(builder, "num_bootstrap_methods: ", bootstrap_method_count);
        for _ in 0..bootstrap_method_count {
            self.dump_cp_info_field(builder, "bootstrap_method_ref: ")?;
            self.dump_cp_info_list(builder, "num_bootstrap_arguments: ", "bootstrap_argument: ")?;
        }
        Ok(())
    }

    fn dump_method_parameters_attribute(&mut self, builder: usize) -> Result<()> {
        let parameter_count = self.parser.u1()?;
        self.add_int(builder, "parameters_count: ", parameter_count);
        for _ in 0..parameter_count {
            self.dump_cp_info_field(builder, "name_index: ")?;
            self.dump_u2(builder, "access_flags: ")?;
        }
        Ok(())
    }

    fn dump_module_attribute(&mut self, builder: usize) -> Result<()> {
        self.dump_cp_info_field(builder, "name: ")?;
        self.dump_u2(builder, "access: ")?;
        self.dump_cp_info_field(builder, "version: ")?;
        let require_count = self.parser.u2()?;
        self.add_int(builder, "require_count: ", require_count);
        for _ in 0..require_count {
            self.dump_cp_info_field(builder, "name: ")?;
            self.dump_u2(builder, "access: ")?;
            self.dump_cp_info_field(builder, "version: ")?;
        }
        for (count_name, to_count_name) in [
            ("export_count: ", "export_to_count: "),
            ("open_count: ", "open_to_count: "),
        ] {
            let count = self.parser.u2()?;
            self.add_int(builder, count_name, count);
            for _ in 0..count {
                self.dump_cp_info_field(builder, "name: ")?;
                self.dump_u2(builder, "access: ")?;
                self.dump_cp_info_list(builder, to_count_name, "to: ")?;
            }
        }
        self.dump_cp_info_list(builder, "use_count: ", "use: ")?;
        let provide_count = self.parser.u2()?;
        self.add_int(builder, "provide_count: ", provide_count);
        for _ in 0..provide_count {
            self.dump_cp_info_field(builder, "provide: ")?;
            self.dump_cp_info_list(builder, "provide_with_count: ", "with: ")?;
        }
        Ok(())
    }

    fn dump_record_attribute(&mut self, builder: usize) -> Result<()> {
        let component_count = self.parser.u2()?;
        self.add_int(builder, "number_of_component_records: ", component_count);
        for _ in 0..component_count {
            self.dump_cp_info_field(builder, "record_component_name: ")?;
            self.dump_cp_info_field(builder, "record_component_descriptor: ")?;
            self.dump_attribute_list(builder)?;
        }
        Ok(())
    }

    fn dump_stack_map_attribute(&mut self, builder: usize) -> Result<()> {
        let entry_count = self.parser.u2()?;
        self.add_int(builder, "number_of_entries: ", entry_count);
        for _ in 0..entry_count {
            let offset = self.parser.u2()?;
            self.add_insn_index(builder, "offset: ", offset);
            self.dump_verification_type_infos(builder)?;
        }
        Ok(())
    }
}

/// A pending step of the iterative `dumpAnnotation` / `dumpElementValue`.
#[derive(Clone, Copy, Debug)]
enum Task {
    Annotation,
    ElementValuePairs(i32),
    ElementValue,
    ElementValues(i32),
}

/// `computeNameAndDumps`.
fn compute_name_and_dumps(class_bytes: &[u8]) -> Result<Dumps> {
    let mut dumper = Dumper::new(class_bytes);
    let class_info = dumper.dump_class_file()?;
    let class_name = dumper
        .cp_dump(class_info, 0)?
        .into_iter()
        .map(|c| {
            if c == u16::from(b'/') {
                u16::from(b'.')
            } else {
                c
            }
        })
        .collect();
    let mut dump = Vec::new();
    dumper.build(CLASS_BUILDER, &mut dump)?;
    let mut constant_pool_dump = Vec::new();
    dumper.build(CONSTANT_POOL_BUILDER, &mut constant_pool_dump)?;
    Ok(Dumps {
        class_name,
        dump,
        constant_pool_dump,
    })
}
