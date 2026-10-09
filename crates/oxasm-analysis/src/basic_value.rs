//! [`BasicValue`]: ASM's `BasicValue`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use oxasm::{JavaStr, JavaString, Type, TypeSort, jstr};
use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::value::Value;

/// A [`Value`] that is represented with its type in a seven types type system, which
/// distinguishes the uninitialized, int, float, long, double, reference and return address types
/// (ASM's `BasicValue`).
///
/// ASM's static instances (`BasicValue.INT_VALUE`, ...) are the associated constants. Values are
/// equal if their types are; but, as in ASM, the [uninitialized](Self::UNINITIALIZED_VALUE),
/// [return address](Self::RETURNADDRESS_VALUE) and [reference](Self::REFERENCE_VALUE) constants
/// print as `.`, `A` and `R`, where other values (including equal ones) print as their type's
/// descriptor.
///
/// A value is one pointer, which copies without a reference count. The values of
/// [`BasicValue::new`] are interned for the whole process: the first value of a type allocates
/// it, and it is never freed (so memory grows with the number of distinct types that values are
/// made of, as a class path's names do in an interner).
#[derive(Clone, Copy)]
pub struct BasicValue(&'static Entry);

/// What a [`BasicValue`] points to: one of ASM's static instances, or the value of a type made
/// with [`BasicValue::new`], which is unique to that type's descriptor.
///
/// Equal types can have different descriptors: an object type parsed from a malformed descriptor
/// keeps the character after its internal name, which need not be a `;` (ASM's `getDescriptor()`
/// reads it from the descriptor's buffer). Such types get values of their own, which print their
/// own descriptor, and equal those of the equal types.
struct Entry {
    kind: Kind,
    /// The type, `None` for an uninitialized value.
    ty: Option<Type<'static>>,
    /// The descriptor of the type (empty for none).
    descriptor: &'static JavaStr,
    /// The identity of the type: the same for equal types, 0 for none.
    type_id: u64,
    /// The value's size, in 32 bits words.
    size: u8,
}

/// ASM's static instances, and the other values (`Of`). Their types are equal when the
/// instance's type is (`Of(Type::INT)` equals `Int`), but they print differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Uninitialized,
    Int,
    Float,
    Long,
    Double,
    Reference,
    ReturnAddress,
    Of,
}

/// The type of `REFERENCE_VALUE`.
const OBJECT: Type<'static> = Type::static_object_type(jstr!("java/lang/Object"));

/// The type ids of the types of ASM's static instances. Those of other types start after them.
const INT_ID: u64 = 1;
const FLOAT_ID: u64 = 2;
const LONG_ID: u64 = 3;
const DOUBLE_ID: u64 = 4;
const OBJECT_ID: u64 = 5;
const VOID_ID: u64 = 6;
/// The next type id.
static NEXT_TYPE_ID: AtomicU64 = AtomicU64::new(VOID_ID + 1);

const fn entry(
    kind: Kind,
    ty: Option<Type<'static>>,
    descriptor: &'static JavaStr,
    type_id: u64,
    size: u8,
) -> Entry {
    Entry {
        kind,
        ty,
        descriptor,
        type_id,
        size,
    }
}

/// The descriptor of [`OBJECT`].
const OBJECT_DESCRIPTOR: &JavaStr = jstr!("Ljava/lang/Object;");

static UNINITIALIZED: Entry = entry(Kind::Uninitialized, None, jstr!(""), 0, 1);
static INT: Entry = entry(Kind::Int, Some(Type::INT), jstr!("I"), INT_ID, 1);
static FLOAT: Entry = entry(Kind::Float, Some(Type::FLOAT), jstr!("F"), FLOAT_ID, 1);
static LONG: Entry = entry(Kind::Long, Some(Type::LONG), jstr!("J"), LONG_ID, 2);
static DOUBLE: Entry = entry(Kind::Double, Some(Type::DOUBLE), jstr!("D"), DOUBLE_ID, 2);
static REFERENCE: Entry = entry(
    Kind::Reference,
    Some(OBJECT),
    OBJECT_DESCRIPTOR,
    OBJECT_ID,
    1,
);
static RETURN_ADDRESS: Entry = entry(
    Kind::ReturnAddress,
    Some(Type::VOID),
    jstr!("V"),
    VOID_ID,
    1,
);
/// The `new` values of the types of the static instances.
static OF_INT: Entry = entry(Kind::Of, Some(Type::INT), jstr!("I"), INT_ID, 1);
static OF_FLOAT: Entry = entry(Kind::Of, Some(Type::FLOAT), jstr!("F"), FLOAT_ID, 1);
static OF_LONG: Entry = entry(Kind::Of, Some(Type::LONG), jstr!("J"), LONG_ID, 2);
static OF_DOUBLE: Entry = entry(Kind::Of, Some(Type::DOUBLE), jstr!("D"), DOUBLE_ID, 2);
static OF_OBJECT: Entry = entry(Kind::Of, Some(OBJECT), OBJECT_DESCRIPTOR, OBJECT_ID, 1);
static OF_VOID: Entry = entry(Kind::Of, Some(Type::VOID), jstr!("V"), VOID_ID, 1);

impl BasicValue {
    /// An uninitialized value.
    pub const UNINITIALIZED_VALUE: BasicValue = BasicValue(&UNINITIALIZED);
    /// A byte, boolean, char, short, or int value.
    pub const INT_VALUE: BasicValue = BasicValue(&INT);
    /// A float value.
    pub const FLOAT_VALUE: BasicValue = BasicValue(&FLOAT);
    /// A long value.
    pub const LONG_VALUE: BasicValue = BasicValue(&LONG);
    /// A double value.
    pub const DOUBLE_VALUE: BasicValue = BasicValue(&DOUBLE);
    /// An object or array reference value.
    pub const REFERENCE_VALUE: BasicValue = BasicValue(&REFERENCE);
    /// A return address value (produced by a `JSR` instruction).
    pub const RETURNADDRESS_VALUE: BasicValue = BasicValue(&RETURN_ADDRESS);

    /// Constructs a new value of the given type, `None` for an uninitialized value.
    pub fn new(ty: Option<Type<'_>>) -> BasicValue {
        match ty {
            None => BasicValue::UNINITIALIZED_VALUE,
            Some(ty) => BasicValue(interned(&ty)),
        }
    }

    /// The type of this value, or `None` for uninitialized values (ASM's `getType()`).
    pub fn get_type(&self) -> Option<&Type<'static>> {
        self.0.ty.as_ref()
    }

    /// Whether this value corresponds to an object or array reference (ASM's `isReference()`).
    pub fn is_reference(&self) -> bool {
        self.get_type()
            .is_some_and(|ty| matches!(ty.get_sort(), TypeSort::Object | TypeSort::Array))
    }

    /// ASM's `toString()`, with the unpaired surrogates a type name may have.
    pub fn to_java_string(&self) -> JavaString {
        match self.0.kind {
            Kind::Uninitialized => JavaString::from("."),
            Kind::ReturnAddress => JavaString::from("A"),
            Kind::Reference => JavaString::from("R"),
            _ => self
                .get_type()
                .expect("an initialized value")
                .get_descriptor()
                .into_owned(),
        }
    }
}

impl Value for BasicValue {
    #[inline]
    fn get_size(&self) -> usize {
        usize::from(self.0.size)
    }
}

impl PartialEq for BasicValue {
    #[inline]
    fn eq(&self, other: &BasicValue) -> bool {
        std::ptr::eq(self.0, other.0) || self.0.type_id == other.0.type_id
    }
}

impl Eq for BasicValue {}

impl Hash for BasicValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get_type().hash(state);
    }
}

impl fmt::Debug for BasicValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tuple = f.debug_tuple("BasicValue");
        match (self.0.kind, &self.0.ty) {
            (Kind::Of, Some(ty)) => tuple.field(&format_args!("Of({ty:?})")),
            (kind, _) => tuple.field(&kind),
        };
        tuple.finish()
    }
}

impl fmt::Display for BasicValue {
    /// ASM's `toString()` (see [`BasicValue::to_java_string`]).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_java_string(), f)
    }
}

// -------------------------------------------------------------------------------------------------
// The interned values.
// -------------------------------------------------------------------------------------------------

/// The number of shards of [`INTERNED`] (a power of two).
const SHARDS: usize = 16;

/// The values of [`BasicValue::new`] of every type but those of the static instances, by the
/// hash of their type: a list, for types of equal hashes and equal types of different
/// descriptors. Each thread looks in its own [`CACHE`] first.
static INTERNED: [Mutex<FxHashMap<u64, Vec<&'static Entry>>>; SHARDS] =
    [const { Mutex::new(FxHashMap::with_hasher(FxBuildHasher)) }; SHARDS];

/// A thread's [`INTERNED`] values.
#[derive(Default)]
struct Cache {
    /// The values this thread has used, by the hash of their type (the last one used, for types of
    /// equal hashes).
    entries: FxHashMap<u64, &'static Entry>,
    /// A buffer for the descriptor of an object type.
    descriptor: JavaString,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// The value of [`BasicValue::new`] of `ty`.
fn interned(ty: &Type<'_>) -> &'static Entry {
    match ty.get_sort() {
        TypeSort::Int => return &OF_INT,
        TypeSort::Float => return &OF_FLOAT,
        TypeSort::Long => return &OF_LONG,
        TypeSort::Double => return &OF_DOUBLE,
        TypeSort::Void => return &OF_VOID,
        _ => {}
    }
    let hash = FxBuildHasher.hash_one(ty);
    CACHE
        .try_with(|cache| {
            let Cache {
                entries,
                descriptor,
            } = &mut *cache.borrow_mut();
            // Equal types other than object types have equal descriptors (their value).
            let descriptor = (ty.get_sort() == TypeSort::Object).then(|| {
                descriptor.clear();
                ty.append_descriptor(descriptor);
                &**descriptor
            });
            if let Some(&entry) = entries.get(&hash)
                && entry.ty.as_ref() == Some(ty)
                && descriptor.is_none_or(|descriptor| entry.descriptor == descriptor)
            {
                return entry;
            }
            let entry = interned_shared(ty, hash, descriptor);
            entries.insert(hash, entry);
            entry
        })
        // The cache is gone while the thread's thread locals are dropped.
        .unwrap_or_else(|_| {
            let descriptor = ty.get_descriptor();
            interned_shared(ty, hash, Some(&descriptor))
        })
}

/// The value of `ty` in [`INTERNED`], made if needed. `descriptor` is that of `ty`, if it is
/// known.
#[cold]
fn interned_shared(ty: &Type<'_>, hash: u64, descriptor: Option<&JavaStr>) -> &'static Entry {
    let descriptor = descriptor.map_or_else(|| ty.get_descriptor(), Cow::Borrowed);
    let mut shard = INTERNED[hash as usize % SHARDS]
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut type_id = None;
    if *ty == OBJECT {
        if *descriptor == *OBJECT_DESCRIPTOR {
            return &OF_OBJECT;
        }
        type_id = Some(OBJECT_ID);
    }
    let entries = shard.entry(hash).or_default();
    for &entry in &*entries {
        if entry.ty.as_ref() == Some(ty) {
            if entry.descriptor == &*descriptor {
                return entry;
            }
            type_id = Some(entry.type_id);
        }
    }
    let descriptor: &'static JavaStr = Box::leak(descriptor.into_owned().into_boxed_java_str());
    // The type gets a copy of its descriptor only, not of a buffer it shares. Parsing the
    // descriptor gives the same type, unless the type is malformed in a way that parsing does
    // not reproduce: a copy of the type then.
    let parsed = Type::get_type(descriptor);
    let ty = if parsed == *ty && *parsed.get_descriptor() == *descriptor {
        parsed
    } else {
        ty.clone().into_owned()
    };
    let size = match ty.get_sort() {
        TypeSort::Long | TypeSort::Double => 2,
        _ => 1,
    };
    let entry: &'static Entry = Box::leak(Box::new(Entry {
        kind: Kind::Of,
        ty: Some(ty),
        descriptor,
        type_id: type_id.unwrap_or_else(|| NEXT_TYPE_ID.fetch_add(1, Ordering::Relaxed)),
        size,
    }));
    entries.push(entry);
    entry
}
