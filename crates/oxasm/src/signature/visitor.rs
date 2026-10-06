//! [`SignatureVisitor`]: ASM's `SignatureVisitor` (ADR-14).

use crate::jstr::JavaStr;

/// Wildcard for an "extends" type argument.
pub const EXTENDS: char = '+';
/// Wildcard for a "super" type argument.
pub const SUPER: char = '-';
/// Wildcard for a normal type argument.
pub const INSTANCEOF: char = '=';

/// A visitor to visit a generic signature. The methods of this trait must be called in one of
/// the three following orders (the last one is the only valid order for a `SignatureVisitor`
/// that is returned by a method of this trait):
///
/// - *ClassSignature* = ( `visit_formal_type_parameter` `visit_class_bound`?
///   `visit_interface_bound`* )* ( `visit_superclass` `visit_interface`* )
/// - *MethodSignature* = ( `visit_formal_type_parameter` `visit_class_bound`?
///   `visit_interface_bound`* )* ( `visit_parameter_type`* `visit_return_type`
///   `visit_exception_type`* )
/// - *TypeSignature* = `visit_base_type` | `visit_type_variable` | `visit_array_type` | (
///   `visit_class_type` `visit_type_argument`* ( `visit_inner_class_type`
///   `visit_type_argument`* )* `visit_end` )
///
/// The methods that start a nested type return the visitor for it: by default `self` (as
/// ASM's `SignatureVisitor` and `SignatureWriter` do), or a child the implementation keeps (for
/// example in an `Option<Box<Self>>` field), since a signature reader finishes with a child
/// before calling its parent again. Every method has a default implementation that does nothing
/// (or returns `self`), so a visitor only implements what it needs.
pub trait SignatureVisitor: AsDynSignatureVisitor {
    /// Visits a formal type parameter.
    fn visit_formal_type_parameter(&mut self, _name: &JavaStr) {}

    /// Visits the class bound of the last visited formal type parameter, and returns a visitor
    /// to visit the signature of the class bound.
    fn visit_class_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits an interface bound of the last visited formal type parameter, and returns a visitor
    /// to visit the signature of the interface bound.
    fn visit_interface_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits the type of the super class, and returns a visitor to visit its signature.
    fn visit_superclass(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits the type of an interface implemented by the class, and returns a visitor to visit
    /// its signature.
    fn visit_interface(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits the type of a method parameter, and returns a visitor to visit its signature.
    fn visit_parameter_type(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits the return type of the method, and returns a visitor to visit its signature.
    fn visit_return_type(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits the type of a method exception, and returns a visitor to visit its signature.
    fn visit_exception_type(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Visits a signature corresponding to a primitive type: `descriptor` is the descriptor of
    /// the primitive type, or 'V' for `void`.
    fn visit_base_type(&mut self, _descriptor: char) {}

    /// Visits a signature corresponding to a type variable.
    fn visit_type_variable(&mut self, _name: &JavaStr) {}

    /// Visits a signature corresponding to an array type, and returns a visitor to visit the
    /// signature of the array element type.
    fn visit_array_type(&mut self) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Starts the visit of a signature corresponding to a class or interface type: `name` is the
    /// internal name of the class or interface.
    fn visit_class_type(&mut self, _name: &JavaStr) {}

    /// Visits an inner class: `name` is the local name of the inner class in its enclosing
    /// class.
    fn visit_inner_class_type(&mut self, _name: &JavaStr) {}

    /// Visits an unbounded type argument (`*`) of the last visited class or inner class type.
    fn visit_type_argument(&mut self) {}

    /// Visits a type argument of the last visited class or inner class type (Java's
    /// `visitTypeArgument(char)`), and returns a visitor to visit the signature of the type
    /// argument. `wildcard` is [`EXTENDS`], [`SUPER`] or [`INSTANCEOF`].
    fn visit_type_argument_wildcard(&mut self, _wildcard: char) -> &mut dyn SignatureVisitor {
        self.as_dyn()
    }

    /// Ends the visit of a signature corresponding to a class or interface type.
    fn visit_end(&mut self) {}
}

/// The `&mut self` → `&mut dyn SignatureVisitor` conversion behind the "return `self`" defaults
/// of [`SignatureVisitor`]. Implemented for every sized `SignatureVisitor`; never implement it by
/// hand.
pub trait AsDynSignatureVisitor {
    /// This visitor as a trait object.
    fn as_dyn(&mut self) -> &mut dyn SignatureVisitor;
}

impl<T: SignatureVisitor> AsDynSignatureVisitor for T {
    fn as_dyn(&mut self) -> &mut dyn SignatureVisitor {
        self
    }
}
