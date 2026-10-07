//! [`SimpleRemapper`]: ASM's `SimpleRemapper`.

use std::borrow::Cow;

use oxasm::{Constant, Handle, JavaStr, JavaString};
use rustc_hash::FxHashMap;

use crate::remapper::Remapper;

/// A [`Remapper`] using a map to define its mapping.
#[derive(Clone, Debug, Default)]
pub struct SimpleRemapper {
    mapping: FxHashMap<JavaString, JavaString>,
}

impl SimpleRemapper {
    /// Constructs a new remapper with the given mapping.
    ///
    /// `mapping` is a map specifying a remapping as follows:
    ///
    /// - for method names, the key is the owner, name and descriptor of the method (in the form
    ///   `<owner>.<name><descriptor>`), and the value is the new method name.
    /// - for invokedynamic method names, the key is the name and descriptor of the method (in the
    ///   form `.<name><descriptor>`), and the value is the new method name.
    /// - for field names, the key is the owner and name of the field or attribute (in the form
    ///   `<owner>.<name>`), and the value is the new field name.
    /// - for attribute names, the key is the annotation descriptor and the name of the attribute
    ///   (in the form `<descriptor>.<name>`), and the value is the new attribute name.
    /// - for internal names, the key is the old internal name, and the value is the new internal
    ///   name.
    pub fn new(mapping: FxHashMap<JavaString, JavaString>) -> SimpleRemapper {
        SimpleRemapper { mapping }
    }

    /// Constructs a new remapper with a mapping with a single key (ASM's
    /// `SimpleRemapper(oldName, newName)`).
    ///
    /// - `old_name`: the key corresponding to a method, field or internal name (see
    ///   [`SimpleRemapper::new`] for the format of these keys).
    /// - `new_name`: the new method, field or internal name.
    pub fn with_single_mapping(
        old_name: impl Into<JavaString>,
        new_name: impl Into<JavaString>,
    ) -> SimpleRemapper {
        let mut mapping = FxHashMap::default();
        mapping.insert(old_name.into(), new_name.into());
        SimpleRemapper { mapping }
    }

    /// The mapping of this remapper.
    pub fn mapping(&self) -> &FxHashMap<JavaString, JavaString> {
        &self.mapping
    }

    /// The mapped value of a key built from the given parts, or `name` if there is none.
    fn map_key<'a>(&'a self, parts: &[&JavaStr], name: &'a JavaStr) -> Cow<'a, JavaStr> {
        let mut key = JavaString::with_capacity(parts.iter().map(|part| part.len()).sum());
        for part in parts {
            key.push_java_str(part);
        }
        match self.map(&key) {
            Some(remapped_name) => Cow::Owned(remapped_name.into_owned()),
            None => Cow::Borrowed(name),
        }
    }
}

impl<K: Into<JavaString>, V: Into<JavaString>> FromIterator<(K, V)> for SimpleRemapper {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> SimpleRemapper {
        SimpleRemapper::new(
            iter.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

const DOT: &JavaStr = oxasm::jstr!(".");

impl Remapper for SimpleRemapper {
    fn map_method_name<'a>(
        &'a self,
        owner: &'a JavaStr,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        self.map_key(&[owner, DOT, name, descriptor], name)
    }

    fn map_basic_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
        _bootstrap_method_handle: &'a Handle,
        _bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        self.map_key(&[DOT, name, descriptor], name)
    }

    fn map_annotation_attribute_name<'a>(
        &'a self,
        descriptor: &'a JavaStr,
        name: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        self.map_key(&[descriptor, DOT, name], name)
    }

    fn map_field_name<'a>(
        &'a self,
        owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        self.map_key(&[owner, DOT, name], name)
    }

    fn map<'a>(&'a self, key: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        self.mapping
            .get(key)
            .map(|value| Cow::Borrowed(value.as_java_str()))
    }
}
