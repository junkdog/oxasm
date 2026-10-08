//! Interned strings: [`Sym`], a `Copy` id for a string stored in an [`Interner`] (ADR-04's P9
//! amendment).
//!
//! The tree API (`oxasm-tree`) stores its owners, names, descriptors and signatures as `Sym`s:
//! four bytes each, compared and hashed as integers. The strings themselves are in the interner,
//! which the caller owns (usually as an `Arc<Interner>` shared by the nodes of many classes), and
//! [`Interner::resolve`] turns a `Sym` back into its `&JavaStr`.
//!
//! - A `Sym` is only meaningful with the interner that made it. Its `Eq` and `Hash` are those of
//!   the id, so two `Sym`s of one interner are equal exactly when their strings are; `Sym`s of
//!   two interners can't be compared. Resolving a `Sym` with another interner is memory safe, and
//!   returns another string or panics.
//! - `Sym` has no `Ord`: ids follow the order in which threads intern, so code that sorts strings
//!   resolves them and compares the content.
//! - The well-known symbols below (`INIT`, `JAVA_LANG_OBJECT`, ...) have the same id in every
//!   interner, so they can be used as `match` patterns:
//!
//! ```
//! use oxasm::sym::{self, Interner, Sym};
//!
//! let interner = Interner::new();
//! let name: Sym = interner.intern("<init>");
//! assert_eq!(name, sym::INIT);
//! assert!(matches!(name, sym::INIT | sym::CLINIT));
//! assert_eq!(interner.resolve(sym::JAVA_LANG_OBJECT), "java/lang/Object");
//! ```

mod interner;
mod memo;

use std::fmt;
use std::num::NonZeroU32;

pub use interner::{Interner, MAX_LEN};
pub use memo::InternMemo;

use interner::well_known_offsets;

/// A string interned in an [`Interner`]: a `Copy` id, four bytes (and so is `Option<Sym>`).
///
/// The id says where the string's bytes are: its low six bits name one of the interner's 64
/// arenas, and the rest is the string's offset in that arena, in 2-byte units. See the
/// [module docs](self).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sym(NonZeroU32);

impl Sym {
    /// The id of this symbol.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0.get()
    }

    /// The symbol with the given id, or `None` for 0. Any other id is a `Sym`, though only those
    /// an interner returned resolve.
    #[inline]
    #[must_use]
    pub const fn from_u32(id: u32) -> Option<Sym> {
        match NonZeroU32::new(id) {
            Some(id) => Some(Sym(id)),
            None => None,
        }
    }

    /// The well-known symbol whose string starts at the given offset of arena 0.
    const fn well_known(offset: u32) -> Sym {
        Sym(NonZeroU32::new((offset / 2) << interner::ARENA_BITS).expect("offsets start at 2"))
    }
}

/// The empty string, [`EMPTY`], which every interner holds.
impl Default for Sym {
    fn default() -> Self {
        EMPTY
    }
}

impl fmt::Debug for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sym({})", self.0)
    }
}

macro_rules! well_known {
    ($($(#[$doc:meta])* $name:ident = $s:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        enum WellKnown { $($name,)* }

        /// The strings of the well-known symbols, in the order an interner is seeded with them.
        pub(crate) const WELL_KNOWN: &[&str] = &[$($s,)*];

        const WELL_KNOWN_OFFSETS: [u32; WELL_KNOWN.len()] = well_known_offsets(WELL_KNOWN);

        $(
            $(#[$doc])*
            pub const $name: Sym = Sym::well_known(WELL_KNOWN_OFFSETS[WellKnown::$name as usize]);
        )*
    };
}

well_known! {
    /// `""`.
    EMPTY = "",
    /// `"<init>"`, the name of instance initialization methods.
    INIT = "<init>",
    /// `"<clinit>"`, the name of class initialization methods.
    CLINIT = "<clinit>",
    /// `"java/lang/Object"`.
    JAVA_LANG_OBJECT = "java/lang/Object",
    /// `"java/lang/String"`.
    JAVA_LANG_STRING = "java/lang/String",
    /// `"java/lang/Class"`.
    JAVA_LANG_CLASS = "java/lang/Class",
    /// `"java/lang/Throwable"`.
    JAVA_LANG_THROWABLE = "java/lang/Throwable",
    /// `"java/lang/Enum"`.
    JAVA_LANG_ENUM = "java/lang/Enum",
    /// `"java/lang/Record"`.
    JAVA_LANG_RECORD = "java/lang/Record",
    /// `"java/lang/StringBuilder"`.
    JAVA_LANG_STRING_BUILDER = "java/lang/StringBuilder",
    /// `"java/lang/invoke/LambdaMetafactory"`.
    JAVA_LANG_INVOKE_LAMBDA_METAFACTORY = "java/lang/invoke/LambdaMetafactory",
    /// `"Ljava/lang/Object;"`.
    OBJECT_DESCRIPTOR = "Ljava/lang/Object;",
    /// `"Ljava/lang/String;"`.
    STRING_DESCRIPTOR = "Ljava/lang/String;",
    /// `"()V"`.
    VOID_METHOD_DESCRIPTOR = "()V",
    /// `"this"`.
    THIS = "this",
    /// `"append"`.
    APPEND = "append",
    /// `"equals"`.
    EQUALS = "equals",
    /// `"hashCode"`.
    HASH_CODE = "hashCode",
    /// `"metafactory"`.
    METAFACTORY = "metafactory",
    /// `"toString"`.
    TO_STRING = "toString",
    /// `"valueOf"`.
    VALUE_OF = "valueOf",
    /// `"values"`.
    VALUES = "values",
}
