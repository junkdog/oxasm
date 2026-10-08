//! [`SymbolTable`]: ASM's `SymbolTable` and `Symbol`, the constant pool, `BootstrapMethods` and
//! (ASM specific) type table entries of a class being written.
//!
//! The hash set of entries is ported as is (ADR-08): an array of buckets, each a list of entries
//! linked through `next`, which new entries are pushed in front of, and which a resize walks in
//! ASM's order. Only the hash *function* differs (FxHash rather than Java's `hashCode`s), which
//! is unobservable: the constant pool is written in insertion order, and the one thing the
//! bucket lists decide, which of several *equal* entries a lookup finds (a pool copied from a
//! class reader can hold duplicates), does not depend on the hash function, since equal entries
//! share a bucket under any hash.

use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;

use rustc_hash::{FxHashMap, FxHasher};

use crate::constant::Constant;
use crate::constant_dynamic::ConstantDynamic;
use crate::constants;
use crate::error::{ReadError, ReadErrorKind, WriteError};
use crate::handle::Handle;
use crate::hierarchy::{ClassHierarchy, HierarchyError};
use crate::jstr::{JavaCompactStr, JavaStr};
use crate::label::Label;
use crate::opcodes::HandleKind;
use crate::reader::ClassReader;
use crate::ty::TypeSort;
use crate::writer::byte_vector::ByteVector;

/// The tag value of `CONSTANT_Class_info` JVMS structures.
pub(crate) const CONSTANT_CLASS_TAG: u8 = 7;
/// The tag value of `CONSTANT_Fieldref_info` JVMS structures.
pub(crate) const CONSTANT_FIELDREF_TAG: u8 = 9;
/// The tag value of `CONSTANT_Methodref_info` JVMS structures.
pub(crate) const CONSTANT_METHODREF_TAG: u8 = 10;
/// The tag value of `CONSTANT_InterfaceMethodref_info` JVMS structures.
pub(crate) const CONSTANT_INTERFACE_METHODREF_TAG: u8 = 11;
/// The tag value of `CONSTANT_String_info` JVMS structures.
pub(crate) const CONSTANT_STRING_TAG: u8 = 8;
/// The tag value of `CONSTANT_Integer_info` JVMS structures.
pub(crate) const CONSTANT_INTEGER_TAG: u8 = 3;
/// The tag value of `CONSTANT_Float_info` JVMS structures.
pub(crate) const CONSTANT_FLOAT_TAG: u8 = 4;
/// The tag value of `CONSTANT_Long_info` JVMS structures.
pub(crate) const CONSTANT_LONG_TAG: u8 = 5;
/// The tag value of `CONSTANT_Double_info` JVMS structures.
pub(crate) const CONSTANT_DOUBLE_TAG: u8 = 6;
/// The tag value of `CONSTANT_NameAndType_info` JVMS structures.
pub(crate) const CONSTANT_NAME_AND_TYPE_TAG: u8 = 12;
/// The tag value of `CONSTANT_Utf8_info` JVMS structures.
pub(crate) const CONSTANT_UTF8_TAG: u8 = 1;
/// The tag value of `CONSTANT_MethodHandle_info` JVMS structures.
pub(crate) const CONSTANT_METHOD_HANDLE_TAG: u8 = 15;
/// The tag value of `CONSTANT_MethodType_info` JVMS structures.
pub(crate) const CONSTANT_METHOD_TYPE_TAG: u8 = 16;
/// The tag value of `CONSTANT_Dynamic_info` JVMS structures.
pub(crate) const CONSTANT_DYNAMIC_TAG: u8 = 17;
/// The tag value of `CONSTANT_InvokeDynamic_info` JVMS structures.
pub(crate) const CONSTANT_INVOKE_DYNAMIC_TAG: u8 = 18;
/// The tag value of `CONSTANT_Module_info` JVMS structures.
pub(crate) const CONSTANT_MODULE_TAG: u8 = 19;
/// The tag value of `CONSTANT_Package_info` JVMS structures.
pub(crate) const CONSTANT_PACKAGE_TAG: u8 = 20;
/// The tag value of the `BootstrapMethods` attribute entries.
pub(crate) const BOOTSTRAP_METHOD_TAG: u8 = 64;
/// The tag value of a normal type entry in the (ASM specific) type table of a class.
pub(crate) const TYPE_TAG: u8 = 128;
/// The tag value of an uninitialized type entry in the type table of a class. This type is used
/// for the normal case where the NEW instruction is before the `<init>` constructor call (in
/// bytecode offset order), i.e. when the label of the NEW instruction is resolved when the
/// constructor call is visited. If the NEW instruction is after the constructor call, use the
/// [`FORWARD_UNINITIALIZED_TYPE_TAG`] tag value instead.
pub(crate) const UNINITIALIZED_TYPE_TAG: u8 = 129;
/// The tag value of an uninitialized type entry in the type table of a class. This type is used
/// for the unusual case where the NEW instruction is after the `<init>` constructor call (in
/// bytecode offset order), i.e. when the label of the NEW instruction is not resolved when the
/// constructor call is visited.
pub(crate) const FORWARD_UNINITIALIZED_TYPE_TAG: u8 = 130;
/// The tag value of a merged type entry in the (ASM specific) type table of a class.
pub(crate) const MERGED_TYPE_TAG: u8 = 131;

/// An entry of the constant pool, of the BootstrapMethods attribute, or of the (ASM specific)
/// type table of a class (ASM's `Symbol`).
#[derive(Clone, Debug)]
pub(crate) struct Symbol {
    /// The index of this symbol in the constant pool, in the BootstrapMethods attribute, or in
    /// the (ASM specific) type table of a class (depending on the `tag` value).
    pub(crate) index: i32,
    /// A tag indicating the type of this symbol (one of the `*_TAG` constants of this module).
    pub(crate) tag: u8,
    /// The internal name of the owner class of this symbol. Only used for fieldref, methodref,
    /// interface methodref and method handle symbols.
    pub(crate) owner: Option<JavaCompactStr>,
    /// The name of the class field or method corresponding to this symbol. Only used for
    /// fieldref, methodref, interface methodref, name and type, method handle, dynamic and
    /// invokedynamic symbols.
    pub(crate) name: Option<JavaCompactStr>,
    /// The string value of this symbol: a field or method descriptor for fieldref, methodref,
    /// interface methodref, name and type, method handle, method type, dynamic and invokedynamic
    /// symbols, an arbitrary string for utf8 and string symbols, an internal class name for
    /// class, type, uninitialized type and forward uninitialized type symbols, `None` for the
    /// others.
    pub(crate) value: Option<JavaCompactStr>,
    /// The numeric value of this symbol: the value of integer, float, long and double symbols
    /// (raw bits), the `reference_kind` of method handle symbols (shifted left by 8 bits for an
    /// interface owner and a kind of at least `H_INVOKEVIRTUAL`), the bootstrap method index of
    /// invokedynamic and dynamic symbols, the offset of a bootstrap method in the
    /// BootstrapMethods `bootstrap_methods` array for bootstrap method symbols, the bytecode
    /// offset of the NEW instruction of uninitialized type symbols, the index in the label table
    /// of the NEW instruction of forward uninitialized type symbols, the type table indices of
    /// the two source types of merged type symbols, and 0 otherwise.
    pub(crate) data: i64,
    /// Additional information about this symbol, which is ignored when comparing symbols: the
    /// index in the InnerClasses `classes` array (plus one) of a class symbol, the type table
    /// index of the merged type of a merged type symbol, 0 otherwise.
    pub(crate) info: i32,
}

impl Symbol {
    /// The value of this symbol, which its tag requires to be present.
    pub(crate) fn value(&self) -> &JavaStr {
        self.value.as_deref().unwrap_or_default()
    }
}

/// The number of buckets of a new, empty [`SymbolTable`]'s hash set (ASM's 256).
const INITIAL_CAPACITY: usize = 256;

/// A reference to an entry of a [`SymbolTable`]'s arena, or `None`: the entry's index plus one,
/// so that a link takes 4 bytes and an array of empty buckets is allocated zeroed.
type Link = Option<NonZeroU32>;

/// The link to the entry at the given index in the arena.
#[allow(clippy::cast_possible_truncation)] // the arena never holds u32::MAX entries
fn link(index: usize) -> Link {
    NonZeroU32::new(index as u32 + 1)
}

/// The index in the arena of the entry that a link refers to.
fn linked(link: NonZeroU32) -> u32 {
    link.get() - 1
}

/// An entry of the hash set of a [`SymbolTable`]: a symbol, its hash code, and the next entry of
/// its bucket.
#[derive(Clone, Debug)]
struct Entry {
    symbol: Symbol,
    /// The hash code of this entry (non negative, as in ASM).
    hash_code: i32,
    /// The next entry in the same bucket, in [`SymbolTable::arena`].
    next: Link,
}

/// The constant pool entries, the BootstrapMethods attribute entries and the (ASM specific) type
/// table entries of a class (ASM's `SymbolTable`).
#[derive(Debug)]
pub(crate) struct SymbolTable {
    /// The identity of the class reader from which this table was constructed, if any (ADR-08).
    source_id: Option<u64>,
    /// The major version number of the class to which this symbol table belongs.
    major_version: i32,
    /// The internal name of the class to which this symbol table belongs.
    class_name: Option<JavaCompactStr>,
    /// Every entry of this table, in insertion order (the storage of ASM's `Entry` objects).
    arena: Vec<Entry>,
    /// A hash set of all the entries in this table (constant pool, bootstrap method and type
    /// table entries): each bucket links to its first entry in `arena`, the others being linked
    /// through [`Entry::next`].
    entries: Vec<Link>,
    /// The number of constant pool items in `constant_pool`, plus 1. The first constant pool item
    /// has index 1, and long and double items count for two items.
    constant_pool_count: i32,
    /// The content of the ClassFile's `constant_pool` JVMS structure corresponding to this
    /// table. The ClassFile's `constant_pool_count` field is *not* included.
    constant_pool: ByteVector,
    /// The number of bootstrap methods in `bootstrap_methods`.
    bootstrap_method_count: i32,
    /// The content of the BootstrapMethods attribute `bootstrap_methods` array corresponding to
    /// this table. Note that the first 6 bytes of the BootstrapMethods_attribute, and its
    /// `num_bootstrap_methods` field, are *not* included.
    bootstrap_methods: Option<ByteVector>,
    /// An ASM specific type table used to temporarily store internal names that will not
    /// necessarily be stored in the constant pool, as indices in `arena`. The type symbol at
    /// index `i` has its `index` equal to `i` (and vice versa).
    type_table: Vec<u32>,
    /// The labels corresponding to the "forward uninitialized" types in the type table.
    label_table: Vec<Label>,
    /// The index of each label of `label_table` in this table.
    label_entries: FxHashMap<Label, i32>,
    /// The first error ASM would have thrown while writing this class (ADR-06): it is reported by
    /// `ClassWriter::to_bytes`.
    error: Option<WriteError>,
}

/// The hash code of a symbol of the given tag and components.
fn hash(tag: u8, parts: impl FnOnce(&mut FxHasher)) -> i32 {
    let mut hasher = FxHasher::default();
    tag.hash(&mut hasher);
    parts(&mut hasher);
    #[allow(clippy::cast_possible_truncation)]
    let hash_code = (hasher.finish() as u32 & 0x7FFF_FFFF) as i32;
    hash_code
}

/// The hash code of a symbol with one string component.
fn hash_str(tag: u8, value: &JavaStr) -> i32 {
    hash(tag, |h| value.hash(h))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Java's int arithmetic
impl SymbolTable {
    /// Constructs a new, empty symbol table.
    pub(crate) fn new() -> SymbolTable {
        SymbolTable {
            source_id: None,
            major_version: 0,
            class_name: None,
            arena: Vec::new(),
            entries: vec![None; INITIAL_CAPACITY],
            constant_pool_count: 1,
            constant_pool: ByteVector::new(),
            bootstrap_method_count: 0,
            bootstrap_methods: None,
            type_table: Vec::new(),
            label_table: Vec::new(),
            label_entries: FxHashMap::default(),
            error: None,
        }
    }

    /// Constructs a new symbol table, initialized with the constant pool and bootstrap methods
    /// of the given class reader.
    ///
    /// # Errors
    ///
    /// If the constant pool or the bootstrap methods are malformed (ASM throws from its
    /// constructor then).
    pub(crate) fn from_reader(class_reader: &ClassReader<'_>) -> Result<SymbolTable, ReadError> {
        // Copy the constant pool binary content.
        let constant_pool_count = class_reader.get_item_count();
        let header = class_reader.header() as i64;
        // ASM's getItem(1) fails for a constant pool without entry 1.
        let constant_pool_offset = class_reader.cp_info_offset(1)? - 1;
        let constant_pool_length = header - constant_pool_offset;
        let pool = class_reader.slice_at(constant_pool_offset, constant_pool_length)?;
        let mut table = SymbolTable {
            source_id: Some(class_reader.id()),
            major_version: 0,
            class_name: None,
            arena: Vec::with_capacity(constant_pool_count),
            // Reserve enough space in 'entries' to avoid too many hash set collisions (entries is
            // not dynamically resized by the add_* calls below), and to account for bootstrap
            // method entries.
            entries: vec![None; (constant_pool_count * 2).max(1)],
            constant_pool_count: constant_pool_count as i32,
            constant_pool: ByteVector::from_vec(pool.to_vec()),
            bootstrap_method_count: 0,
            bootstrap_methods: None,
            type_table: Vec::new(),
            label_table: Vec::new(),
            label_entries: FxHashMap::default(),
            error: None,
        };
        let required = |s: Option<&JavaStr>, offset: i64| {
            s.map(JavaCompactStr::new).ok_or_else(|| {
                // ASM's NullPointerException on a null string.
                ReadError::new(offset, ReadErrorKind::InvalidConstantPoolIndex(0))
            })
        };
        // Add the constant pool items in the symbol table entries.
        let mut has_bootstrap_methods = false;
        let mut item_index = 1;
        while item_index < constant_pool_count {
            let item_offset = class_reader.cp_info_offset(item_index)?;
            let item_tag = class_reader.u8_at(item_offset - 1)?;
            let index = item_index as i32;
            match item_tag {
                CONSTANT_FIELDREF_TAG
                | CONSTANT_METHODREF_TAG
                | CONSTANT_INTERFACE_METHODREF_TAG => {
                    let name_and_type_item_offset = class_reader
                        .cp_info_offset(usize::from(class_reader.u16_at(item_offset + 2)?))?;
                    let owner = required(class_reader.read_class_at(item_offset)?, item_offset)?;
                    let name = required(
                        class_reader.utf8_at(name_and_type_item_offset)?,
                        name_and_type_item_offset,
                    )?;
                    let descriptor = required(
                        class_reader.utf8_at(name_and_type_item_offset + 2)?,
                        name_and_type_item_offset + 2,
                    )?;
                    table
                        .add_constant_member_reference_at(index, item_tag, owner, name, descriptor);
                }
                CONSTANT_INTEGER_TAG | CONSTANT_FLOAT_TAG => {
                    let value = class_reader.i32_at(item_offset)?;
                    table.add_constant_integer_or_float_at(index, item_tag, value);
                }
                CONSTANT_NAME_AND_TYPE_TAG => {
                    let name = required(class_reader.utf8_at(item_offset)?, item_offset)?;
                    let descriptor =
                        required(class_reader.utf8_at(item_offset + 2)?, item_offset + 2)?;
                    table.add_constant_name_and_type_at(index, name, descriptor);
                }
                CONSTANT_LONG_TAG | CONSTANT_DOUBLE_TAG => {
                    let value = class_reader.i64_at(item_offset)?;
                    table.add_constant_long_or_double_at(index, item_tag, value);
                }
                CONSTANT_UTF8_TAG => {
                    let value = JavaCompactStr::new(class_reader.utf(item_index)?);
                    table.add_constant_utf8_at(index, value);
                }
                CONSTANT_METHOD_HANDLE_TAG => {
                    let member_ref_item_offset = class_reader
                        .cp_info_offset(usize::from(class_reader.u16_at(item_offset + 1)?))?;
                    let name_and_type_item_offset = class_reader.cp_info_offset(usize::from(
                        class_reader.u16_at(member_ref_item_offset + 2)?,
                    ))?;
                    let reference_kind = i32::from(class_reader.u8_at(item_offset)?);
                    let owner = required(
                        class_reader.read_class_at(member_ref_item_offset)?,
                        member_ref_item_offset,
                    )?;
                    let name = required(
                        class_reader.utf8_at(name_and_type_item_offset)?,
                        name_and_type_item_offset,
                    )?;
                    let descriptor = required(
                        class_reader.utf8_at(name_and_type_item_offset + 2)?,
                        name_and_type_item_offset + 2,
                    )?;
                    let is_interface = class_reader.u8_at(member_ref_item_offset - 1)?
                        == CONSTANT_INTERFACE_METHODREF_TAG;
                    table.add_constant_method_handle_at(
                        index,
                        reference_kind,
                        owner,
                        name,
                        descriptor,
                        is_interface,
                    );
                }
                CONSTANT_DYNAMIC_TAG | CONSTANT_INVOKE_DYNAMIC_TAG => {
                    has_bootstrap_methods = true;
                    let name_and_type_item_offset = class_reader
                        .cp_info_offset(usize::from(class_reader.u16_at(item_offset + 2)?))?;
                    let name = required(
                        class_reader.utf8_at(name_and_type_item_offset)?,
                        name_and_type_item_offset,
                    )?;
                    let descriptor = required(
                        class_reader.utf8_at(name_and_type_item_offset + 2)?,
                        name_and_type_item_offset + 2,
                    )?;
                    let bootstrap_method_index = i32::from(class_reader.u16_at(item_offset)?);
                    table.add_constant_dynamic_or_invoke_dynamic_reference_at(
                        item_tag,
                        index,
                        name,
                        descriptor,
                        bootstrap_method_index,
                    );
                }
                CONSTANT_STRING_TAG
                | CONSTANT_CLASS_TAG
                | CONSTANT_METHOD_TYPE_TAG
                | CONSTANT_MODULE_TAG
                | CONSTANT_PACKAGE_TAG => {
                    let value = required(class_reader.utf8_at(item_offset)?, item_offset)?;
                    table.add_constant_utf8_reference_at(index, item_tag, value);
                }
                _ => {
                    return Err(ReadError::new(
                        item_offset - 1,
                        ReadErrorKind::InvalidConstantPoolTag(item_tag),
                    ));
                }
            }
            item_index += if item_tag == CONSTANT_LONG_TAG || item_tag == CONSTANT_DOUBLE_TAG {
                2
            } else {
                1
            };
        }
        // Copy the BootstrapMethods, if any.
        if has_bootstrap_methods {
            table.copy_bootstrap_methods(class_reader)?;
        }
        Ok(table)
    }

    /// Reads the BootstrapMethods `bootstrap_methods` array binary content and adds its entries
    /// to this table.
    fn copy_bootstrap_methods(&mut self, class_reader: &ClassReader<'_>) -> Result<(), ReadError> {
        // Find attributOffset of the 'bootstrap_methods' array.
        let mut current_attribute_offset = class_reader.get_first_attribute_offset()?;
        let mut i = class_reader.u16_at(current_attribute_offset - 2)?;
        while i > 0 {
            let attribute_name = class_reader.utf8_at(current_attribute_offset)?;
            if attribute_name == Some(constants::BOOTSTRAP_METHODS) {
                self.bootstrap_method_count =
                    i32::from(class_reader.u16_at(current_attribute_offset + 6)?);
                break;
            }
            current_attribute_offset +=
                6 + i64::from(class_reader.i32_at(current_attribute_offset + 2)?);
            i -= 1;
        }
        if self.bootstrap_method_count > 0 {
            // Compute the offset and the length of the BootstrapMethods 'bootstrap_methods'
            // array.
            let bootstrap_methods_offset = current_attribute_offset + 8;
            let bootstrap_methods_length =
                i64::from(class_reader.i32_at(current_attribute_offset + 2)?) - 2;
            let content =
                class_reader.slice_at(bootstrap_methods_offset, bootstrap_methods_length)?;
            self.bootstrap_methods = Some(ByteVector::from_vec(content.to_vec()));
            // Add each bootstrap method in the symbol table entries.
            let mut current_offset = bootstrap_methods_offset;
            for i in 0..self.bootstrap_method_count {
                let offset = current_offset - bootstrap_methods_offset;
                let bootstrap_method_ref = class_reader.u16_at(current_offset)?;
                current_offset += 2;
                let num_bootstrap_arguments = class_reader.u16_at(current_offset)?;
                current_offset += 2;
                // ASM hashes the decoded constants, which fails on malformed ones.
                class_reader.const_at(usize::from(bootstrap_method_ref))?;
                for _ in 0..num_bootstrap_arguments {
                    let bootstrap_argument = class_reader.u16_at(current_offset)?;
                    current_offset += 2;
                    class_reader.const_at(usize::from(bootstrap_argument))?;
                }
                let length = current_offset - bootstrap_methods_offset - offset;
                let hash_code = self.bootstrap_method_hash(offset as usize, length as usize);
                self.add(Entry {
                    symbol: Symbol {
                        index: i,
                        tag: BOOTSTRAP_METHOD_TAG,
                        owner: None,
                        name: None,
                        value: None,
                        data: offset,
                        info: 0,
                    },
                    hash_code,
                    next: None,
                });
            }
        }
        Ok(())
    }

    /// The hash code of the bootstrap method at `[offset, offset + length[` in
    /// `bootstrap_methods`: its bytes, which equal bootstrap methods share (ASM hashes their
    /// decoded constants, which is equivalent for the lookups).
    fn bootstrap_method_hash(&self, offset: usize, length: usize) -> i32 {
        let data = self
            .bootstrap_methods
            .as_ref()
            .map_or(&[][..], ByteVector::data);
        let bytes = data.get(offset..offset + length).unwrap_or_default();
        hash(BOOTSTRAP_METHOD_TAG, |h| bytes.hash(h))
    }

    /// The identity of the class reader from which this table was constructed, if any.
    pub(crate) fn source_id(&self) -> Option<u64> {
        self.source_id
    }

    /// Returns the major version of the class to which this symbol table belongs.
    pub(crate) fn major_version(&self) -> i32 {
        self.major_version
    }

    /// Returns the internal name of the class to which this symbol table belongs.
    pub(crate) fn class_name(&self) -> &JavaStr {
        self.class_name.as_deref().unwrap_or_default()
    }

    /// Sets the major version and the name of the class to which this symbol table belongs.
    /// Also adds the class name to the constant pool, and returns its index.
    pub(crate) fn set_major_version_and_class_name(
        &mut self,
        major_version: i32,
        class_name: &JavaStr,
    ) -> i32 {
        self.major_version = major_version;
        self.class_name = Some(JavaCompactStr::new(class_name));
        self.add_constant_class(class_name).index
    }

    /// Returns the number of items in this symbol table's `constant_pool` array (plus 1).
    pub(crate) fn constant_pool_count(&self) -> i32 {
        self.constant_pool_count
    }

    /// Returns the length in bytes of this symbol table's `constant_pool` array.
    pub(crate) fn constant_pool_length(&self) -> usize {
        self.constant_pool.size()
    }

    /// Puts this symbol table's `constant_pool` array in the given byte vector, preceded by the
    /// `constant_pool_count` value.
    pub(crate) fn put_constant_pool(&self, output: &mut ByteVector) {
        output
            .put_short(self.constant_pool_count)
            .put_byte_array(self.constant_pool.data());
    }

    /// Returns the size in bytes of this symbol table's BootstrapMethods attribute. Also adds the
    /// attribute name in the constant pool.
    pub(crate) fn compute_bootstrap_methods_size(&mut self) -> usize {
        if let Some(bootstrap_methods) = &self.bootstrap_methods {
            let length = bootstrap_methods.size();
            self.add_constant_utf8(constants::BOOTSTRAP_METHODS);
            8 + length
        } else {
            0
        }
    }

    /// Puts this symbol table's BootstrapMethods attribute in the given byte vector. This
    /// includes the 6 attribute header bytes and the `num_bootstrap_methods` value.
    pub(crate) fn put_bootstrap_methods(&mut self, output: &mut ByteVector) {
        if self.bootstrap_methods.is_some() {
            let name_index = self.add_constant_utf8(constants::BOOTSTRAP_METHODS);
            let bootstrap_methods = self.bootstrap_methods.as_ref().expect("checked above");
            output
                .put_short(name_index)
                .put_int(bootstrap_methods.size() as i32 + 2)
                .put_short(self.bootstrap_method_count)
                .put_byte_array(bootstrap_methods.data());
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Errors
    // ---------------------------------------------------------------------------------------------

    /// Records an error that ASM would have thrown, unless an earlier one was recorded (ASM stops
    /// at the first).
    pub(crate) fn fail(&mut self, error: WriteError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    /// The first recorded error, if any.
    pub(crate) fn error(&self) -> Option<&WriteError> {
        self.error.as_ref()
    }

    // ---------------------------------------------------------------------------------------------
    // Generic symbol table entries management.
    // ---------------------------------------------------------------------------------------------

    /// The first entry of the bucket of the given hash code.
    fn get(&self, hash_code: i32) -> Link {
        self.entries[hash_code as usize % self.entries.len()]
    }

    /// Finds the first entry, in the bucket of `hash_code`, that has this hash code and matches,
    /// and returns its index in the arena.
    fn find(&self, hash_code: i32, matches: impl Fn(&Symbol) -> bool) -> Option<u32> {
        let mut entry = self.get(hash_code);
        while let Some(l) = entry {
            let index = linked(l);
            let e = &self.arena[index as usize];
            if e.hash_code == hash_code && matches(&e.symbol) {
                return Some(index);
            }
            entry = e.next;
        }
        None
    }

    /// Puts the given entry in the hash set, which is resized if necessary (ASM's `put`). Does
    /// not check whether the set already contains a similar entry.
    fn put(&mut self, mut entry: Entry) -> u32 {
        if self.arena.len() > (self.entries.len() * 3) / 4 {
            let current_capacity = self.entries.len();
            let new_capacity = current_capacity * 2 + 1;
            let mut new_entries = vec![None; new_capacity];
            for i in (0..current_capacity).rev() {
                let mut current_entry = self.entries[i];
                while let Some(l) = current_entry {
                    let e = &mut self.arena[linked(l) as usize];
                    let new_current_entry_index = e.hash_code as usize % new_capacity;
                    let next_entry = e.next;
                    e.next = new_entries[new_current_entry_index];
                    new_entries[new_current_entry_index] = Some(l);
                    current_entry = next_entry;
                }
            }
            self.entries = new_entries;
        }
        let bucket = entry.hash_code as usize % self.entries.len();
        entry.next = self.entries[bucket];
        let index = self.arena.len();
        self.arena.push(entry);
        self.entries[bucket] = link(index);
        index as u32
    }

    /// Adds the given entry in the hash set, without resizing it (ASM's `add`).
    fn add(&mut self, mut entry: Entry) {
        let bucket = entry.hash_code as usize % self.entries.len();
        entry.next = self.entries[bucket];
        self.entries[bucket] = link(self.arena.len());
        self.arena.push(entry);
    }

    /// The symbol of the given entry.
    pub(crate) fn symbol(&self, entry: u32) -> &Symbol {
        &self.arena[entry as usize].symbol
    }

    /// The symbol of the given entry, mutably (for its `info` field).
    pub(crate) fn symbol_mut(&mut self, entry: u32) -> &mut Symbol {
        &mut self.arena[entry as usize].symbol
    }

    /// A new constant pool entry, at the next index.
    fn new_entry(
        &mut self,
        tag: u8,
        owner: Option<&JavaStr>,
        name: Option<&JavaStr>,
        value: Option<&JavaStr>,
        data: i64,
        hash_code: i32,
    ) -> Entry {
        let index = self.constant_pool_count;
        self.constant_pool_count += 1;
        Entry {
            symbol: Symbol {
                index,
                tag,
                owner: owner.map(JavaCompactStr::new),
                name: name.map(JavaCompactStr::new),
                value: value.map(JavaCompactStr::new),
                data,
                info: 0,
            },
            hash_code,
            next: None,
        }
    }

    /// An entry read from a class reader's constant pool, at the given index.
    fn entry_at(
        index: i32,
        tag: u8,
        owner: Option<JavaCompactStr>,
        name: Option<JavaCompactStr>,
        value: Option<JavaCompactStr>,
        data: i64,
        hash_code: i32,
    ) -> Entry {
        Entry {
            symbol: Symbol {
                index,
                tag,
                owner,
                name,
                value,
                data,
                info: 0,
            },
            hash_code,
            next: None,
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Constant pool entries management.
    // ---------------------------------------------------------------------------------------------

    /// Adds a number or string constant to the constant pool of this symbol table. Does nothing
    /// if the constant pool already contains a similar item.
    pub(crate) fn add_constant(&mut self, value: &Constant<'_>) -> &Symbol {
        let entry = self.add_constant_entry(value);
        self.symbol(entry)
    }

    /// [`Self::add_constant`], returning the entry.
    pub(crate) fn add_constant_entry(&mut self, value: &Constant<'_>) -> u32 {
        match value {
            Constant::Integer(v) => self.add_constant_integer_or_float(CONSTANT_INTEGER_TAG, *v),
            Constant::Float(v) => {
                self.add_constant_integer_or_float(CONSTANT_FLOAT_TAG, v.to_bits() as i32)
            }
            Constant::Long(v) => self.add_constant_long_or_double(CONSTANT_LONG_TAG, *v),
            Constant::Double(v) => {
                self.add_constant_long_or_double(CONSTANT_DOUBLE_TAG, v.to_bits() as i64)
            }
            Constant::String(v) => self.add_constant_utf8_reference(CONSTANT_STRING_TAG, v),
            Constant::Type(t) => match t.get_sort() {
                TypeSort::Object => {
                    self.add_constant_utf8_reference(CONSTANT_CLASS_TAG, t.get_internal_name())
                }
                TypeSort::Method => {
                    self.add_constant_utf8_reference(CONSTANT_METHOD_TYPE_TAG, &t.get_descriptor())
                }
                // A primitive or array type.
                _ => self.add_constant_utf8_reference(CONSTANT_CLASS_TAG, &t.get_descriptor()),
            },
            Constant::Handle(h) => self.add_constant_method_handle_entry(
                i32::from(h.get_tag().tag()),
                h.get_owner(),
                h.get_name(),
                h.get_desc(),
                h.is_interface(),
            ),
            Constant::Dynamic(d) => self.add_constant_dynamic_entry(d),
        }
    }

    /// Adds a `CONSTANT_Class_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_class(&mut self, value: &JavaStr) -> &Symbol {
        let entry = self.add_constant_utf8_reference(CONSTANT_CLASS_TAG, value);
        self.symbol(entry)
    }

    /// [`Self::add_constant_class`], returning the entry.
    pub(crate) fn add_constant_class_entry(&mut self, value: &JavaStr) -> u32 {
        self.add_constant_utf8_reference(CONSTANT_CLASS_TAG, value)
    }

    /// Adds a `CONSTANT_Fieldref_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_fieldref(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) -> &Symbol {
        let entry =
            self.add_constant_member_reference(CONSTANT_FIELDREF_TAG, owner, name, descriptor);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Methodref_info` or `CONSTANT_InterfaceMethodref_info` to the constant
    /// pool of this symbol table. Does nothing if the constant pool already contains a similar
    /// item.
    pub(crate) fn add_constant_methodref(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) -> &Symbol {
        let entry = self.add_constant_methodref_entry(owner, name, descriptor, is_interface);
        self.symbol(entry)
    }

    /// [`Self::add_constant_methodref`], returning the entry.
    fn add_constant_methodref_entry(
        &mut self,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) -> u32 {
        let tag = if is_interface {
            CONSTANT_INTERFACE_METHODREF_TAG
        } else {
            CONSTANT_METHODREF_TAG
        };
        self.add_constant_member_reference(tag, owner, name, descriptor)
    }

    /// Adds a `CONSTANT_Fieldref_info`, `CONSTANT_Methodref_info` or
    /// `CONSTANT_InterfaceMethodref_info` to the constant pool of this symbol table. Does nothing
    /// if the constant pool already contains a similar item.
    fn add_constant_member_reference(
        &mut self,
        tag: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) -> u32 {
        let hash_code = Self::member_reference_hash(tag, owner, name, descriptor);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == tag
                && s.owner.as_deref() == Some(owner)
                && s.name.as_deref() == Some(name)
                && s.value.as_deref() == Some(descriptor)
        }) {
            return entry;
        }
        let class_index = self.add_constant_class(owner).index;
        let name_and_type_index = self.add_constant_name_and_type(name, descriptor);
        self.constant_pool
            .put122(i32::from(tag), class_index, name_and_type_index);
        let entry = self.new_entry(tag, Some(owner), Some(name), Some(descriptor), 0, hash_code);
        self.put(entry)
    }

    /// The hash code of a member reference symbol.
    fn member_reference_hash(
        tag: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) -> i32 {
        hash(tag, |h| {
            owner.hash(h);
            name.hash(h);
            descriptor.hash(h);
        })
    }

    /// Adds a new `CONSTANT_Fieldref_info`, `CONSTANT_Methodref_info` or
    /// `CONSTANT_InterfaceMethodref_info` to the constant pool of this symbol table.
    fn add_constant_member_reference_at(
        &mut self,
        index: i32,
        tag: u8,
        owner: JavaCompactStr,
        name: JavaCompactStr,
        descriptor: JavaCompactStr,
    ) {
        let hash_code = Self::member_reference_hash(tag, &owner, &name, &descriptor);
        self.add(Self::entry_at(
            index,
            tag,
            Some(owner),
            Some(name),
            Some(descriptor),
            0,
            hash_code,
        ));
    }

    #[allow(dead_code)] // as in ASM, unused by the writer itself
    /// Adds a `CONSTANT_String_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_string(&mut self, value: &JavaStr) -> &Symbol {
        let entry = self.add_constant_utf8_reference(CONSTANT_STRING_TAG, value);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Integer_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_integer(&mut self, value: i32) -> &Symbol {
        let entry = self.add_constant_integer_or_float(CONSTANT_INTEGER_TAG, value);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Float_info` to the constant pool of this symbol table. Does nothing if the
    /// constant pool already contains a similar item.
    pub(crate) fn add_constant_float(&mut self, value: f32) -> &Symbol {
        let entry = self.add_constant_integer_or_float(CONSTANT_FLOAT_TAG, value.to_bits() as i32);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Integer_info` or `CONSTANT_Float_info` to the constant pool of this
    /// symbol table. Does nothing if the constant pool already contains a similar item.
    fn add_constant_integer_or_float(&mut self, tag: u8, value: i32) -> u32 {
        let hash_code = hash(tag, |h| value.hash(h));
        if let Some(entry) = self.find(hash_code, |s| s.tag == tag && s.data == i64::from(value)) {
            return entry;
        }
        self.constant_pool.put_byte(i32::from(tag)).put_int(value);
        let entry = self.new_entry(tag, None, None, None, i64::from(value), hash_code);
        self.put(entry)
    }

    /// Adds a new `CONSTANT_Integer_info` or `CONSTANT_Float_info` to the constant pool of this
    /// symbol table.
    fn add_constant_integer_or_float_at(&mut self, index: i32, tag: u8, value: i32) {
        let hash_code = hash(tag, |h| value.hash(h));
        self.add(Self::entry_at(
            index,
            tag,
            None,
            None,
            None,
            i64::from(value),
            hash_code,
        ));
    }

    /// Adds a `CONSTANT_Long_info` to the constant pool of this symbol table. Does nothing if the
    /// constant pool already contains a similar item.
    pub(crate) fn add_constant_long(&mut self, value: i64) -> &Symbol {
        let entry = self.add_constant_long_or_double(CONSTANT_LONG_TAG, value);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Double_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_double(&mut self, value: f64) -> &Symbol {
        let entry = self.add_constant_long_or_double(CONSTANT_DOUBLE_TAG, value.to_bits() as i64);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Long_info` or `CONSTANT_Double_info` to the constant pool of this symbol
    /// table. Does nothing if the constant pool already contains a similar item.
    fn add_constant_long_or_double(&mut self, tag: u8, value: i64) -> u32 {
        let hash_code = hash(tag, |h| value.hash(h));
        if let Some(entry) = self.find(hash_code, |s| s.tag == tag && s.data == value) {
            return entry;
        }
        let index = self.constant_pool_count;
        self.constant_pool.put_byte(i32::from(tag)).put_long(value);
        self.constant_pool_count += 2;
        self.put(Self::entry_at(
            index, tag, None, None, None, value, hash_code,
        ))
    }

    /// Adds a new `CONSTANT_Long_info` or `CONSTANT_Double_info` to the constant pool of this
    /// symbol table.
    fn add_constant_long_or_double_at(&mut self, index: i32, tag: u8, value: i64) {
        let hash_code = hash(tag, |h| value.hash(h));
        self.add(Self::entry_at(
            index, tag, None, None, None, value, hash_code,
        ));
    }

    /// Adds a `CONSTANT_NameAndType_info` to the constant pool of this symbol table. Does nothing
    /// if the constant pool already contains a similar item. Returns its index.
    pub(crate) fn add_constant_name_and_type(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) -> i32 {
        let tag = CONSTANT_NAME_AND_TYPE_TAG;
        let hash_code = Self::name_and_type_hash(name, descriptor);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == tag
                && s.name.as_deref() == Some(name)
                && s.value.as_deref() == Some(descriptor)
        }) {
            return self.symbol(entry).index;
        }
        let name_index = self.add_constant_utf8(name);
        let descriptor_index = self.add_constant_utf8(descriptor);
        self.constant_pool
            .put122(i32::from(tag), name_index, descriptor_index);
        let entry = self.new_entry(tag, None, Some(name), Some(descriptor), 0, hash_code);
        let entry = self.put(entry);
        self.symbol(entry).index
    }

    /// The hash code of a name and type symbol.
    fn name_and_type_hash(name: &JavaStr, descriptor: &JavaStr) -> i32 {
        hash(CONSTANT_NAME_AND_TYPE_TAG, |h| {
            name.hash(h);
            descriptor.hash(h);
        })
    }

    /// Adds a new `CONSTANT_NameAndType_info` to the constant pool of this symbol table.
    fn add_constant_name_and_type_at(
        &mut self,
        index: i32,
        name: JavaCompactStr,
        descriptor: JavaCompactStr,
    ) {
        let hash_code = Self::name_and_type_hash(&name, &descriptor);
        self.add(Self::entry_at(
            index,
            CONSTANT_NAME_AND_TYPE_TAG,
            None,
            Some(name),
            Some(descriptor),
            0,
            hash_code,
        ));
    }

    /// Adds a `CONSTANT_Utf8_info` to the constant pool of this symbol table. Does nothing if the
    /// constant pool already contains a similar item. Returns its index.
    pub(crate) fn add_constant_utf8(&mut self, value: &JavaStr) -> i32 {
        let hash_code = hash_str(CONSTANT_UTF8_TAG, value);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == CONSTANT_UTF8_TAG && s.value.as_deref() == Some(value)
        }) {
            return self.symbol(entry).index;
        }
        let position = self.constant_pool.size();
        self.constant_pool.put_byte(i32::from(CONSTANT_UTF8_TAG));
        if let Err(error) = self.constant_pool.put_utf8(value) {
            // ASM throws before adding the entry.
            self.constant_pool.truncate(position);
            self.fail(error);
            return 0;
        }
        let entry = self.new_entry(CONSTANT_UTF8_TAG, None, None, Some(value), 0, hash_code);
        let entry = self.put(entry);
        self.symbol(entry).index
    }

    /// Adds a new `CONSTANT_Utf8_info` to the constant pool of this symbol table.
    fn add_constant_utf8_at(&mut self, index: i32, value: JavaCompactStr) {
        let hash_code = hash_str(CONSTANT_UTF8_TAG, &value);
        self.add(Self::entry_at(
            index,
            CONSTANT_UTF8_TAG,
            None,
            None,
            Some(value),
            0,
            hash_code,
        ));
    }

    /// Adds a `CONSTANT_MethodHandle_info` to the constant pool of this symbol table. Does
    /// nothing if the constant pool already contains a similar item.
    pub(crate) fn add_constant_method_handle(
        &mut self,
        reference_kind: i32,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) -> &Symbol {
        let entry = self.add_constant_method_handle_entry(
            reference_kind,
            owner,
            name,
            descriptor,
            is_interface,
        );
        self.symbol(entry)
    }

    /// [`Self::add_constant_method_handle`], returning the entry.
    fn add_constant_method_handle_entry(
        &mut self,
        reference_kind: i32,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) -> u32 {
        let tag = CONSTANT_METHOD_HANDLE_TAG;
        let data = Self::get_constant_method_handle_symbol_data(reference_kind, is_interface);
        // Note that we don't need to include isInterface in the hash computation, because it is
        // redundant with owner (we can't have the same owner with different isInterface values).
        let hash_code = Self::method_handle_hash(owner, name, descriptor, data);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == tag
                && s.data == i64::from(data)
                && s.owner.as_deref() == Some(owner)
                && s.name.as_deref() == Some(name)
                && s.value.as_deref() == Some(descriptor)
        }) {
            return entry;
        }
        let member_index = if reference_kind <= i32::from(HandleKind::PutStatic.tag()) {
            self.add_constant_fieldref(owner, name, descriptor).index
        } else {
            self.add_constant_methodref(owner, name, descriptor, is_interface)
                .index
        };
        self.constant_pool
            .put112(i32::from(tag), reference_kind, member_index);
        let entry = self.new_entry(
            tag,
            Some(owner),
            Some(name),
            Some(descriptor),
            i64::from(data),
            hash_code,
        );
        self.put(entry)
    }

    /// The hash code of a method handle symbol.
    fn method_handle_hash(owner: &JavaStr, name: &JavaStr, descriptor: &JavaStr, data: i32) -> i32 {
        hash(CONSTANT_METHOD_HANDLE_TAG, |h| {
            owner.hash(h);
            name.hash(h);
            descriptor.hash(h);
            data.hash(h);
        })
    }

    /// Adds a new `CONSTANT_MethodHandle_info` to the constant pool of this symbol table.
    fn add_constant_method_handle_at(
        &mut self,
        index: i32,
        reference_kind: i32,
        owner: JavaCompactStr,
        name: JavaCompactStr,
        descriptor: JavaCompactStr,
        is_interface: bool,
    ) {
        let data = Self::get_constant_method_handle_symbol_data(reference_kind, is_interface);
        let hash_code = Self::method_handle_hash(&owner, &name, &descriptor, data);
        self.add(Self::entry_at(
            index,
            CONSTANT_METHOD_HANDLE_TAG,
            Some(owner),
            Some(name),
            Some(descriptor),
            i64::from(data),
            hash_code,
        ));
    }

    /// Returns the `data` field for a `CONSTANT_MethodHandle_info` symbol.
    fn get_constant_method_handle_symbol_data(reference_kind: i32, is_interface: bool) -> i32 {
        if reference_kind > i32::from(HandleKind::PutStatic.tag()) && is_interface {
            return reference_kind << 8;
        }
        reference_kind
    }

    /// Adds a `CONSTANT_MethodType_info` to the constant pool of this symbol table. Does nothing
    /// if the constant pool already contains a similar item.
    pub(crate) fn add_constant_method_type(&mut self, method_descriptor: &JavaStr) -> &Symbol {
        let entry = self.add_constant_utf8_reference(CONSTANT_METHOD_TYPE_TAG, method_descriptor);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Dynamic_info` to the constant pool of this symbol table. Also adds the
    /// related bootstrap method to the BootstrapMethods of this symbol table. Does nothing if the
    /// constant pool already contains a similar item.
    pub(crate) fn add_constant_dynamic(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) -> &Symbol {
        let bootstrap_method =
            self.add_bootstrap_method(bootstrap_method_handle, bootstrap_method_arguments);
        let entry = self.add_constant_dynamic_or_invoke_dynamic_reference(
            CONSTANT_DYNAMIC_TAG,
            name,
            descriptor,
            bootstrap_method,
        );
        self.symbol(entry)
    }

    /// [`Self::add_constant_dynamic`], returning the entry.
    fn add_constant_dynamic_entry(&mut self, constant_dynamic: &ConstantDynamic) -> u32 {
        let bootstrap_method = self.add_bootstrap_method(
            constant_dynamic.get_bootstrap_method(),
            constant_dynamic.get_bootstrap_method_arguments(),
        );
        self.add_constant_dynamic_or_invoke_dynamic_reference(
            CONSTANT_DYNAMIC_TAG,
            constant_dynamic.get_name(),
            constant_dynamic.get_descriptor(),
            bootstrap_method,
        )
    }

    /// Adds a `CONSTANT_InvokeDynamic_info` to the constant pool of this symbol table. Also adds
    /// the related bootstrap method to the BootstrapMethods of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_invoke_dynamic(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) -> &Symbol {
        let bootstrap_method =
            self.add_bootstrap_method(bootstrap_method_handle, bootstrap_method_arguments);
        let entry = self.add_constant_dynamic_or_invoke_dynamic_reference(
            CONSTANT_INVOKE_DYNAMIC_TAG,
            name,
            descriptor,
            bootstrap_method,
        );
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Dynamic` or a `CONSTANT_InvokeDynamic_info` to the constant pool of this
    /// symbol table. Does nothing if the constant pool already contains a similar item.
    fn add_constant_dynamic_or_invoke_dynamic_reference(
        &mut self,
        tag: u8,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_index: i32,
    ) -> u32 {
        let hash_code = Self::dynamic_hash(tag, name, descriptor, bootstrap_method_index);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == tag
                && s.data == i64::from(bootstrap_method_index)
                && s.name.as_deref() == Some(name)
                && s.value.as_deref() == Some(descriptor)
        }) {
            return entry;
        }
        let name_and_type_index = self.add_constant_name_and_type(name, descriptor);
        self.constant_pool
            .put122(i32::from(tag), bootstrap_method_index, name_and_type_index);
        let entry = self.new_entry(
            tag,
            None,
            Some(name),
            Some(descriptor),
            i64::from(bootstrap_method_index),
            hash_code,
        );
        self.put(entry)
    }

    /// The hash code of a dynamic or invokedynamic symbol.
    fn dynamic_hash(
        tag: u8,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_index: i32,
    ) -> i32 {
        hash(tag, |h| {
            name.hash(h);
            descriptor.hash(h);
            bootstrap_method_index.hash(h);
        })
    }

    /// Adds a new `CONSTANT_Dynamic_info` or `CONSTANT_InvokeDynamic_info` to the constant pool
    /// of this symbol table.
    fn add_constant_dynamic_or_invoke_dynamic_reference_at(
        &mut self,
        tag: u8,
        index: i32,
        name: JavaCompactStr,
        descriptor: JavaCompactStr,
        bootstrap_method_index: i32,
    ) {
        let hash_code = Self::dynamic_hash(tag, &name, &descriptor, bootstrap_method_index);
        self.add(Self::entry_at(
            index,
            tag,
            None,
            Some(name),
            Some(descriptor),
            i64::from(bootstrap_method_index),
            hash_code,
        ));
    }

    /// Adds a `CONSTANT_Module_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_module(&mut self, module_name: &JavaStr) -> &Symbol {
        let entry = self.add_constant_utf8_reference(CONSTANT_MODULE_TAG, module_name);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Package_info` to the constant pool of this symbol table. Does nothing if
    /// the constant pool already contains a similar item.
    pub(crate) fn add_constant_package(&mut self, package_name: &JavaStr) -> &Symbol {
        let entry = self.add_constant_utf8_reference(CONSTANT_PACKAGE_TAG, package_name);
        self.symbol(entry)
    }

    /// Adds a `CONSTANT_Class_info`, `CONSTANT_String_info`, `CONSTANT_MethodType_info`,
    /// `CONSTANT_Module_info` or `CONSTANT_Package_info` to the constant pool of this symbol
    /// table. Does nothing if the constant pool already contains a similar item.
    fn add_constant_utf8_reference(&mut self, tag: u8, value: &JavaStr) -> u32 {
        let hash_code = hash_str(tag, value);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == tag && s.value.as_deref() == Some(value)
        }) {
            return entry;
        }
        let utf8_index = self.add_constant_utf8(value);
        self.constant_pool.put12(i32::from(tag), utf8_index);
        let entry = self.new_entry(tag, None, None, Some(value), 0, hash_code);
        self.put(entry)
    }

    /// Adds a new `CONSTANT_Class_info`, `CONSTANT_String_info`, `CONSTANT_MethodType_info`,
    /// `CONSTANT_Module_info` or `CONSTANT_Package_info` to the constant pool of this symbol
    /// table.
    fn add_constant_utf8_reference_at(&mut self, index: i32, tag: u8, value: JavaCompactStr) {
        let hash_code = hash_str(tag, &value);
        self.add(Self::entry_at(
            index,
            tag,
            None,
            None,
            Some(value),
            0,
            hash_code,
        ));
    }

    // ---------------------------------------------------------------------------------------------
    // Bootstrap method entries management.
    // ---------------------------------------------------------------------------------------------

    /// Adds a bootstrap method to the BootstrapMethods attribute of this symbol table. Does
    /// nothing if the BootstrapMethods already contains a similar bootstrap method. Returns its
    /// index.
    pub(crate) fn add_bootstrap_method(
        &mut self,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) -> i32 {
        if self.bootstrap_methods.is_none() {
            self.bootstrap_methods = Some(ByteVector::new());
        }
        // The bootstrap method arguments can be Constant_Dynamic values, which reference other
        // bootstrap methods. We must therefore add the bootstrap method arguments to the constant
        // pool and BootstrapMethods attribute first, so that the BootstrapMethods attribute is
        // not modified while adding the given bootstrap method to it, in the rest of this method.
        let bootstrap_method_argument_indexes: Vec<i32> = bootstrap_method_arguments
            .iter()
            .map(|argument| self.add_constant(argument).index)
            .collect();
        let handle_index = self
            .add_constant_method_handle(
                i32::from(bootstrap_method_handle.get_tag().tag()),
                bootstrap_method_handle.get_owner(),
                bootstrap_method_handle.get_name(),
                bootstrap_method_handle.get_desc(),
                bootstrap_method_handle.is_interface(),
            )
            .index;
        // Write the bootstrap method in the BootstrapMethods table. This is necessary to be able
        // to compare it with existing ones, and will be reverted below if there is already a
        // similar bootstrap method.
        let bootstrap_methods = self.bootstrap_methods.as_mut().expect("created above");
        let bootstrap_method_offset = bootstrap_methods.size();
        bootstrap_methods.put_short(handle_index);
        bootstrap_methods.put_short(bootstrap_method_argument_indexes.len() as i32);
        for index in bootstrap_method_argument_indexes {
            bootstrap_methods.put_short(index);
        }
        // Compute the length and the hash code of the bootstrap method.
        let bootstrap_method_length = bootstrap_methods.size() - bootstrap_method_offset;
        let hash_code =
            self.bootstrap_method_hash(bootstrap_method_offset, bootstrap_method_length);
        // Add the bootstrap method to the symbol table or revert the above changes.
        self.add_bootstrap_method_at(bootstrap_method_offset, bootstrap_method_length, hash_code)
    }

    /// Adds a bootstrap method to the BootstrapMethods attribute of this symbol table. Does
    /// nothing if the BootstrapMethods already contains a similar bootstrap method (more
    /// precisely, reverts the content of `bootstrap_methods` to remove the last, duplicate
    /// bootstrap method).
    fn add_bootstrap_method_at(&mut self, offset: usize, length: usize, hash_code: i32) -> i32 {
        let data = self
            .bootstrap_methods
            .as_ref()
            .expect("created by the caller")
            .data();
        let same = |other_offset: usize| {
            data.get(other_offset..other_offset + length) == Some(&data[offset..offset + length])
        };
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == BOOTSTRAP_METHOD_TAG && same(s.data as usize)
        }) {
            // Revert to old position.
            self.bootstrap_methods
                .as_mut()
                .expect("created by the caller")
                .truncate(offset);
            return self.symbol(entry).index;
        }
        let index = self.bootstrap_method_count;
        self.bootstrap_method_count += 1;
        let entry = self.put(Self::entry_at(
            index,
            BOOTSTRAP_METHOD_TAG,
            None,
            None,
            None,
            offset as i64,
            hash_code,
        ));
        self.symbol(entry).index
    }

    // ---------------------------------------------------------------------------------------------
    // Type table entries management.
    // ---------------------------------------------------------------------------------------------

    /// Returns the type table element whose index is given.
    pub(crate) fn get_type(&self, type_index: i32) -> &Symbol {
        self.symbol(self.type_table[type_index as usize])
    }

    /// Returns the type table element whose index is given, if there is one.
    pub(crate) fn try_get_type(&self, type_index: i32) -> Option<&Symbol> {
        let entry = *self.type_table.get(usize::try_from(type_index).ok()?)?;
        Some(self.symbol(entry))
    }

    /// Returns the label corresponding to the "forward uninitialized" type table element whose
    /// index is given.
    pub(crate) fn get_forward_uninitialized_label(&self, type_index: i32) -> Label {
        self.label_table[self.get_type(type_index).data as usize]
    }

    /// Adds a type in the type table of this symbol table. Does nothing if the type table
    /// already contains a similar type. Returns its index.
    pub(crate) fn add_type(&mut self, value: &JavaStr) -> i32 {
        let hash_code = hash_str(TYPE_TAG, value);
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == TYPE_TAG && s.value.as_deref() == Some(value)
        }) {
            return self.symbol(entry).index;
        }
        let index = self.type_table.len() as i32;
        self.add_type_internal(Entry {
            symbol: Symbol {
                index,
                tag: TYPE_TAG,
                owner: None,
                name: None,
                value: Some(JavaCompactStr::new(value)),
                data: 0,
                info: 0,
            },
            hash_code,
            next: None,
        })
    }

    /// Adds an uninitialized type in the type table of this symbol table. Does nothing if the
    /// type table already contains a similar type. Returns its index.
    pub(crate) fn add_uninitialized_type(&mut self, value: &JavaStr, bytecode_offset: i32) -> i32 {
        let hash_code = hash(UNINITIALIZED_TYPE_TAG, |h| {
            value.hash(h);
            bytecode_offset.hash(h);
        });
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == UNINITIALIZED_TYPE_TAG
                && s.data == i64::from(bytecode_offset)
                && s.value.as_deref() == Some(value)
        }) {
            return self.symbol(entry).index;
        }
        let index = self.type_table.len() as i32;
        self.add_type_internal(Entry {
            symbol: Symbol {
                index,
                tag: UNINITIALIZED_TYPE_TAG,
                owner: None,
                name: None,
                value: Some(JavaCompactStr::new(value)),
                data: i64::from(bytecode_offset),
                info: 0,
            },
            hash_code,
            next: None,
        })
    }

    /// Adds a "forward uninitialized" type in the type table of this symbol table. Does nothing
    /// if the type table already contains a similar type. Returns its index.
    pub(crate) fn add_forward_uninitialized_type(&mut self, value: &JavaStr, label: Label) -> i32 {
        let label_index = self.get_or_add_label_entry(label);
        let hash_code = hash(FORWARD_UNINITIALIZED_TYPE_TAG, |h| {
            value.hash(h);
            label_index.hash(h);
        });
        if let Some(entry) = self.find(hash_code, |s| {
            s.tag == FORWARD_UNINITIALIZED_TYPE_TAG
                && s.data == i64::from(label_index)
                && s.value.as_deref() == Some(value)
        }) {
            return self.symbol(entry).index;
        }
        let index = self.type_table.len() as i32;
        self.add_type_internal(Entry {
            symbol: Symbol {
                index,
                tag: FORWARD_UNINITIALIZED_TYPE_TAG,
                owner: None,
                name: None,
                value: Some(JavaCompactStr::new(value)),
                data: i64::from(label_index),
                info: 0,
            },
            hash_code,
            next: None,
        })
    }

    /// Adds a merged type in the type table of this symbol table. Does nothing if the type table
    /// already contains a similar type. Returns the index of the common super type of the two
    /// given types, in the type table (ASM's `addMergedType`).
    ///
    /// The common super class comes from the given class hierarchy (ASM's
    /// `ClassWriter.getCommonSuperClass`).
    ///
    /// # Errors
    ///
    /// If there is no hierarchy, or if it cannot answer.
    pub(crate) fn add_merged_type(
        &mut self,
        hierarchy: Option<&dyn ClassHierarchy>,
        type_table_index1: i32,
        type_table_index2: i32,
    ) -> Result<i32, WriteError> {
        let data = if type_table_index1 < type_table_index2 {
            i64::from(type_table_index1) | (i64::from(type_table_index2) << 32)
        } else {
            i64::from(type_table_index2) | (i64::from(type_table_index1) << 32)
        };
        let hash_code = hash(MERGED_TYPE_TAG, |h| {
            type_table_index1.wrapping_add(type_table_index2).hash(h);
        });
        if let Some(entry) = self.find(hash_code, |s| s.tag == MERGED_TYPE_TAG && s.data == data) {
            return Ok(self.symbol(entry).info);
        }
        let assertion_error = || WriteError::InvalidArgument("java.lang.AssertionError".to_owned());
        let type1 = self
            .try_get_type(type_table_index1)
            .ok_or_else(assertion_error)?
            .value()
            .to_owned();
        let type2 = self
            .try_get_type(type_table_index2)
            .ok_or_else(assertion_error)?
            .value()
            .to_owned();
        let common_super_class = match hierarchy {
            Some(hierarchy) => hierarchy.common_super_class(&type1, &type2)?,
            None => return Err(HierarchyError::NoHierarchy(type1, type2).into()),
        };
        let common_super_type_index = self.add_type(&common_super_class);
        let index = self.type_table.len() as i32;
        self.put(Entry {
            symbol: Symbol {
                index,
                tag: MERGED_TYPE_TAG,
                owner: None,
                name: None,
                value: None,
                data,
                info: common_super_type_index,
            },
            hash_code,
            next: None,
        });
        Ok(common_super_type_index)
    }

    /// Adds the given type entry to the type table. Its index must be the current size of the
    /// type table. Returns this index.
    fn add_type_internal(&mut self, entry: Entry) -> i32 {
        let index = entry.symbol.index;
        let entry = self.put(entry);
        self.type_table.push(entry);
        index
    }

    /// Returns the index in the label table of the given label, adding it if necessary.
    fn get_or_add_label_entry(&mut self, label: Label) -> i32 {
        if let Some(&index) = self.label_entries.get(&label) {
            return index;
        }
        let index = self.label_table.len() as i32;
        self.label_table.push(label);
        self.label_entries.insert(label, index);
        index
    }
}
