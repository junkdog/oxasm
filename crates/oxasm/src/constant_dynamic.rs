//! [`ConstantDynamic`]: a constant whose value is computed at runtime, with a bootstrap method.

use std::fmt;

use crate::constant::Constant;
use crate::handle::Handle;
use crate::jstr::{JavaArcStr, JavaStr};

/// A constant whose value is computed at runtime, with a bootstrap method (a
/// `CONSTANT_Dynamic_info`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConstantDynamic {
    /// The constant name (can be arbitrary).
    name: JavaArcStr,
    /// The constant type (must be a field descriptor).
    descriptor: JavaArcStr,
    /// The bootstrap method to use to compute the constant value at runtime.
    bootstrap_method: Handle,
    /// The arguments to pass to the bootstrap method, in order to compute the constant value at
    /// runtime.
    bootstrap_method_arguments: Vec<Constant<'static>>,
}

impl ConstantDynamic {
    /// Constructs a new `ConstantDynamic`.
    ///
    /// - `name`: the constant name (can be arbitrary).
    /// - `descriptor`: the constant type (must be a field descriptor).
    /// - `bootstrap_method`: the bootstrap method to use to compute the constant value at runtime.
    /// - `bootstrap_method_arguments`: the arguments to pass to the bootstrap method, in order to
    ///   compute the constant value at runtime.
    #[must_use]
    pub fn new(
        name: impl Into<JavaArcStr>,
        descriptor: impl Into<JavaArcStr>,
        bootstrap_method: Handle,
        bootstrap_method_arguments: Vec<Constant<'static>>,
    ) -> ConstantDynamic {
        ConstantDynamic {
            name: name.into(),
            descriptor: descriptor.into(),
            bootstrap_method,
            bootstrap_method_arguments,
        }
    }

    /// Returns the name of this constant.
    #[inline]
    #[must_use]
    pub fn get_name(&self) -> &JavaStr {
        &self.name
    }

    /// Returns the type of this constant, as a field descriptor.
    #[inline]
    #[must_use]
    pub fn get_descriptor(&self) -> &JavaStr {
        &self.descriptor
    }

    /// Returns the bootstrap method used to compute the value of this constant.
    #[inline]
    #[must_use]
    pub fn get_bootstrap_method(&self) -> &Handle {
        &self.bootstrap_method
    }

    /// Returns the number of arguments passed to the bootstrap method, in order to compute the
    /// value of this constant.
    #[inline]
    #[must_use]
    pub fn get_bootstrap_method_argument_count(&self) -> usize {
        self.bootstrap_method_arguments.len()
    }

    /// Returns an argument passed to the bootstrap method, in order to compute the value of this
    /// constant.
    ///
    /// # Panics
    ///
    /// If `index` is out of bounds.
    #[inline]
    #[must_use]
    #[track_caller]
    pub fn get_bootstrap_method_argument(&self, index: usize) -> &Constant<'static> {
        &self.bootstrap_method_arguments[index]
    }

    /// Returns the arguments passed to the bootstrap method, in order to compute the value of this
    /// constant.
    #[inline]
    #[must_use]
    pub fn get_bootstrap_method_arguments(&self) -> &[Constant<'static>] {
        &self.bootstrap_method_arguments
    }

    /// Returns the size of this constant: 2 for `long` and `double`, 1 otherwise.
    #[must_use]
    pub fn get_size(&self) -> usize {
        match self.descriptor.as_bytes().first() {
            Some(b'J' | b'D') => 2,
            _ => 1,
        }
    }
}

/// `name " : " descriptor " " bootstrap_method " [" arguments "]"`, as ASM's `toString`.
impl fmt::Display for ConstantDynamic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} : {} {} [",
            self.name, self.descriptor, self.bootstrap_method
        )?;
        for (i, argument) in self.bootstrap_method_arguments.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{argument}")?;
        }
        f.write_str("]")
    }
}
