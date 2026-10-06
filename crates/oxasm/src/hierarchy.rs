//! [`ClassHierarchy`]: the super class and interface facts that frame computation
//! (`COMPUTE_FRAMES`) and `SimpleVerifier` need, replacing ASM's class loading (ADR-07).
//!
//! ASM's `ClassWriter.getCommonSuperClass` and `SimpleVerifier.isAssignableFrom` load classes
//! through the running JVM. oxasm asks a `ClassHierarchy` instead: [`MapHierarchy`] (explicit
//! facts, e.g. from a classpath index), [`ObjectHierarchy`] (answers `java/lang/Object` for any
//! two unrelated classes), [`ChainedHierarchy`] (one, then another), or a user implementation.
//!
//! The model knows each class's super class and whether it is an interface, not which interfaces
//! a class implements. So `a` is assignable from `b` iff `a == b`, `a` is `java/lang/Object`, or
//! `a` is on `b`'s super class chain. This is the same model as the parity oracle's `Hierarchy`
//! class (`oracle/Oracle.java`), which both sides must answer identically.

use rustc_hash::FxHashMap;

use crate::jstr::{JavaStr, JavaString};

/// The internal name of `java.lang.Object`.
pub const OBJECT: &JavaStr = crate::jstr!("java/lang/Object");

/// The longest super class chain followed before giving up: it bounds cyclic hierarchies.
const MAX_DEPTH: usize = 1 << 16;

/// Why a [`ClassHierarchy`] could not answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HierarchyError {
    /// The class is unknown (ASM's `TypeNotPresentException` from class loading).
    #[error("Type {0} not present")]
    TypeNotPresent(JavaString),
    /// The super class chain of a class ends (or cycles) without reaching a common super class.
    #[error("broken super class chain from {0}")]
    BrokenSuperClassChain(JavaString),
}

/// The class hierarchy facts needed to merge reference types: super classes and interfaces.
///
/// Implement [`ClassHierarchy::super_class`] and [`ClassHierarchy::is_interface`]; the other
/// methods derive ASM's algorithms from them.
pub trait ClassHierarchy {
    /// The internal name of the super class of `name`, or `None` for `java/lang/Object` (and for
    /// any other root).
    ///
    /// # Errors
    ///
    /// [`HierarchyError::TypeNotPresent`] if `name` is unknown.
    fn super_class(&self, name: &JavaStr) -> Result<Option<JavaString>, HierarchyError>;

    /// Whether `name` is an interface.
    ///
    /// # Errors
    ///
    /// [`HierarchyError::TypeNotPresent`] if `name` is unknown.
    fn is_interface(&self, name: &JavaStr) -> Result<bool, HierarchyError>;

    /// Whether a value of class `b` can be assigned to a variable of class `a`: `a == b`, `a` is
    /// `java/lang/Object`, or `a` is on `b`'s super class chain.
    ///
    /// # Errors
    ///
    /// If `a` or `b`, or a class on `b`'s super class chain, is unknown.
    fn is_assignable_from(&self, a: &JavaStr, b: &JavaStr) -> Result<bool, HierarchyError> {
        // Both must be known, even when the answer is obvious (as with class loading).
        self.super_class(a)?;
        self.super_class(b)?;
        if a == b || a == OBJECT {
            return Ok(true);
        }
        let mut class = Some(b.to_owned());
        let mut depth = 0;
        while let Some(current) = class {
            if depth >= MAX_DEPTH {
                break;
            }
            if *current == *a {
                return Ok(true);
            }
            class = self.super_class(&current)?;
            depth += 1;
        }
        Ok(false)
    }

    /// The internal name of the common super class of `a` and `b`: ASM's
    /// `ClassWriter.getCommonSuperClass`, over [`ClassHierarchy::is_assignable_from`].
    ///
    /// # Errors
    ///
    /// If a class is unknown, or `a`'s super class chain ends before a common super class.
    fn common_super_class(&self, a: &JavaStr, b: &JavaStr) -> Result<JavaString, HierarchyError> {
        if self.is_assignable_from(a, b)? {
            return Ok(a.to_owned());
        }
        if self.is_assignable_from(b, a)? {
            return Ok(b.to_owned());
        }
        if self.is_interface(a)? || self.is_interface(b)? {
            return Ok(OBJECT.to_owned());
        }
        let mut class = a.to_owned();
        let mut depth = 0;
        loop {
            depth += 1;
            class = match self.super_class(&class)? {
                Some(super_class) if depth <= MAX_DEPTH => super_class,
                _ => return Err(HierarchyError::BrokenSuperClassChain(a.to_owned())),
            };
            if self.is_assignable_from(&class, b)? {
                return Ok(class);
            }
        }
    }
}

impl<T: ClassHierarchy + ?Sized> ClassHierarchy for &T {
    fn super_class(&self, name: &JavaStr) -> Result<Option<JavaString>, HierarchyError> {
        (**self).super_class(name)
    }
    fn is_interface(&self, name: &JavaStr) -> Result<bool, HierarchyError> {
        (**self).is_interface(name)
    }
    fn is_assignable_from(&self, a: &JavaStr, b: &JavaStr) -> Result<bool, HierarchyError> {
        (**self).is_assignable_from(a, b)
    }
    fn common_super_class(&self, a: &JavaStr, b: &JavaStr) -> Result<JavaString, HierarchyError> {
        (**self).common_super_class(a, b)
    }
}

/// A class hierarchy given as explicit facts: `(name, super class, is interface)`.
#[derive(Clone, Debug, Default)]
pub struct MapHierarchy {
    entries: FxHashMap<JavaString, (Option<JavaString>, bool)>,
}

/// The error of [`MapHierarchy::from_tsv`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("malformed hierarchy line {line}: {reason}")]
pub struct MalformedHierarchy {
    /// The 1-based line number.
    pub line: usize,
    /// What is wrong with it.
    pub reason: String,
}

impl MapHierarchy {
    /// An empty hierarchy.
    #[must_use]
    pub fn new() -> MapHierarchy {
        MapHierarchy::default()
    }

    /// Adds the facts of one class. The first facts given for a name win (later ones are
    /// ignored, as in the oracle).
    pub fn insert(
        &mut self,
        name: impl Into<JavaString>,
        super_class: Option<JavaString>,
        is_interface: bool,
    ) {
        self.entries
            .entry(name.into())
            .or_insert((super_class, is_interface));
    }

    /// The number of classes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no class is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Parses the oracle's `hierarchy.tsv`: one `name \t super \t is_interface` line per class,
    /// in semi-UTF-8, where an empty `super` means none and `is_interface` is `1` or `0`. Empty
    /// lines are skipped.
    ///
    /// # Errors
    ///
    /// If a line does not have three fields, or is not semi-UTF-8.
    pub fn from_tsv(tsv: &[u8]) -> Result<MapHierarchy, MalformedHierarchy> {
        let mut hierarchy = MapHierarchy::new();
        for (index, line) in tsv.split(|&b| b == b'\n').enumerate() {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let malformed = |reason: &str| MalformedHierarchy {
                line: index + 1,
                reason: reason.into(),
            };
            let fields: Vec<&[u8]> = line.split(|&b| b == b'\t').collect();
            let [name, super_name, is_interface] = fields[..] else {
                return Err(malformed("expected 3 tab-separated fields"));
            };
            let text =
                |bytes| JavaStr::from_semi_utf8(bytes).map_err(|e| malformed(&e.to_string()));
            let super_name = text(super_name)?;
            hierarchy.insert(
                text(name)?,
                (!super_name.is_empty()).then(|| super_name.to_owned()),
                is_interface == b"1",
            );
        }
        Ok(hierarchy)
    }

    fn entry(&self, name: &JavaStr) -> Result<&(Option<JavaString>, bool), HierarchyError> {
        self.entries
            .get(name)
            .ok_or_else(|| HierarchyError::TypeNotPresent(name.to_owned()))
    }
}

impl ClassHierarchy for MapHierarchy {
    fn super_class(&self, name: &JavaStr) -> Result<Option<JavaString>, HierarchyError> {
        Ok(self.entry(name)?.0.clone())
    }

    fn is_interface(&self, name: &JavaStr) -> Result<bool, HierarchyError> {
        Ok(self.entry(name)?.1)
    }
}

/// A hierarchy that knows nothing, and so answers `java/lang/Object` as the common super class of
/// any two distinct classes. This is the pragmatic choice of many ASM-based tools; the frames it
/// produces verify as long as no merged value is used as anything but an `Object`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ObjectHierarchy;

impl ClassHierarchy for ObjectHierarchy {
    fn super_class(&self, name: &JavaStr) -> Result<Option<JavaString>, HierarchyError> {
        Ok((name != OBJECT).then(|| OBJECT.to_owned()))
    }

    fn is_interface(&self, _name: &JavaStr) -> Result<bool, HierarchyError> {
        Ok(false)
    }
}

/// Asks `first`, then `second` for the classes `first` does not know.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChainedHierarchy<A, B>(pub A, pub B);

impl<A: ClassHierarchy, B: ClassHierarchy> ClassHierarchy for ChainedHierarchy<A, B> {
    fn super_class(&self, name: &JavaStr) -> Result<Option<JavaString>, HierarchyError> {
        match self.0.super_class(name) {
            Err(HierarchyError::TypeNotPresent(_)) => self.1.super_class(name),
            answer => answer,
        }
    }

    fn is_interface(&self, name: &JavaStr) -> Result<bool, HierarchyError> {
        match self.0.is_interface(name) {
            Err(HierarchyError::TypeNotPresent(_)) => self.1.is_interface(name),
            answer => answer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jstr;

    fn sample() -> MapHierarchy {
        MapHierarchy::from_tsv(
            b"java/lang/Object\t\t0\n\
              java/lang/Number\tjava/lang/Object\t0\n\
              java/lang/Integer\tjava/lang/Number\t0\n\
              java/lang/Long\tjava/lang/Number\t0\n\
              java/lang/String\tjava/lang/Object\t0\n\
              java/lang/Runnable\tjava/lang/Object\t1\n\
              \n",
        )
        .unwrap()
    }

    #[test]
    fn common_super_class_follows_the_oracle_model() {
        let h = sample();
        let csc = |a, b| h.common_super_class(jstr!(a), jstr!(b)).unwrap();
        assert_eq!(
            csc("java/lang/Integer", "java/lang/Long"),
            "java/lang/Number"
        );
        assert_eq!(
            csc("java/lang/Number", "java/lang/Integer"),
            "java/lang/Number"
        );
        assert_eq!(
            csc("java/lang/Integer", "java/lang/Number"),
            "java/lang/Number"
        );
        assert_eq!(
            csc("java/lang/Integer", "java/lang/String"),
            "java/lang/Object"
        );
        assert_eq!(
            csc("java/lang/Integer", "java/lang/Runnable"),
            "java/lang/Object"
        );
        assert_eq!(
            csc("java/lang/Object", "java/lang/Runnable"),
            "java/lang/Object"
        );
    }

    #[test]
    fn unknown_types_are_reported() {
        let h = sample();
        let error = h
            .common_super_class(jstr!("a/B"), jstr!("java/lang/Object"))
            .unwrap_err();
        assert_eq!(
            error,
            HierarchyError::TypeNotPresent(JavaString::from("a/B"))
        );
        assert_eq!(error.to_string(), "Type a/B not present");
    }

    #[test]
    fn cycles_are_bounded() {
        let mut h = MapHierarchy::new();
        h.insert("a/A", Some("a/B".into()), false);
        h.insert("a/B", Some("a/A".into()), false);
        h.insert("a/C", Some("java/lang/Object".into()), false);
        h.insert("java/lang/Object", None, false);
        assert!(!h.is_assignable_from(jstr!("a/C"), jstr!("a/A")).unwrap());
        assert_eq!(
            h.common_super_class(jstr!("a/A"), jstr!("a/C")),
            Err(HierarchyError::BrokenSuperClassChain(JavaString::from(
                "a/A"
            )))
        );
    }

    #[test]
    fn object_and_chained_hierarchies() {
        assert_eq!(
            ObjectHierarchy
                .common_super_class(jstr!("a/A"), jstr!("b/B"))
                .unwrap(),
            "java/lang/Object"
        );
        let chained = ChainedHierarchy(sample(), ObjectHierarchy);
        assert_eq!(
            chained
                .common_super_class(jstr!("java/lang/Integer"), jstr!("java/lang/Long"))
                .unwrap(),
            "java/lang/Number"
        );
        assert_eq!(
            chained
                .common_super_class(jstr!("a/A"), jstr!("java/lang/Integer"))
                .unwrap(),
            "java/lang/Object"
        );
    }

    #[test]
    fn malformed_tsv_is_rejected() {
        let error = MapHierarchy::from_tsv(b"a/A\tjava/lang/Object\n").unwrap_err();
        assert_eq!(error.line, 1);
    }
}
