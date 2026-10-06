//! [`SignatureWriter`]: ASM's `SignatureWriter`.

use std::fmt;

use crate::jstr::{JavaStr, JavaString};
use crate::signature::{INSTANCEOF, SignatureVisitor};

/// A [`SignatureVisitor`] that generates signature literals, as defined in the Java Virtual
/// Machine Specification (JVMS).
///
/// See [JVMS 4.7.9.1](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7.9.1).
#[derive(Clone, Debug)]
pub struct SignatureWriter {
    /// The visited signature.
    signature: JavaString,
    /// Whether the visited signature contains formal type parameters.
    has_formals: bool,
    /// Whether the visited signature contains method parameter types.
    has_parameters: bool,
    /// The stack used to keep track of class types that have arguments: one element per class
    /// type being visited, `true` when it has type arguments. The bottom of the stack is a
    /// sentinel element always equal to `true`.
    ///
    /// Class type arguments must be surrounded with '<' and '>' and, because class types can be
    /// nested (type arguments can themselves be class types), and `SignatureWriter` returns
    /// itself in each `visit_*` method, a stack is needed to properly balance these angle
    /// brackets. A new element is pushed on this stack for each new visited type, and popped when
    /// the visit of this type ends (either in `visit_end`, or because `visit_inner_class_type` is
    /// called).
    ///
    /// ASM packs this stack in the bits of an `int`, and continues in a nested `SignatureWriter`
    /// sharing the same output when it is full; an unbounded stack produces the same output.
    argument_stack: Vec<bool>,
}

impl Default for SignatureWriter {
    fn default() -> Self {
        SignatureWriter::new()
    }
}

impl SignatureWriter {
    /// Constructs a new `SignatureWriter`.
    #[must_use]
    pub fn new() -> SignatureWriter {
        SignatureWriter {
            signature: JavaString::new(),
            has_formals: false,
            has_parameters: false,
            argument_stack: vec![true],
        }
    }

    /// Returns the signature that was built by this signature writer.
    #[must_use]
    pub fn as_java_str(&self) -> &JavaStr {
        &self.signature
    }

    /// Returns the signature that was built by this signature writer.
    #[must_use]
    pub fn into_signature(self) -> JavaString {
        self.signature
    }

    // ---------------------------------------------------------------------------------------------
    // Utility methods
    // ---------------------------------------------------------------------------------------------

    /// Ends the formal type parameters section of the signature.
    fn end_formals(&mut self) {
        if self.has_formals {
            self.has_formals = false;
            self.signature.push('>');
        }
    }

    /// Ends the type arguments of a class or inner class type.
    fn end_arguments(&mut self) {
        // If the top of the stack is 'true', this means that some type arguments have been
        // visited for the type whose visit is now ending. We therefore need to append a '>', and
        // to pop one element from the stack.
        if self.argument_stack.pop() == Some(true) {
            self.signature.push('>');
        }
    }

    /// Marks the currently visited type as having type arguments, opening them if this is the
    /// first one.
    fn begin_argument(&mut self) {
        // If the top of the stack is 'false', this means we are visiting the first type argument
        // of the currently visited type. We therefore need to append a '<', and to replace the
        // top stack element with 'true' (meaning that the current type does have type
        // arguments).
        match self.argument_stack.last_mut() {
            Some(top) if !*top => {
                *top = true;
                self.signature.push('<');
            }
            Some(_) => {}
            // ASM's emptied int stack: `0 & 1 == 0`, then `0 | 1` leaves the sentinel.
            None => {
                self.argument_stack.push(true);
                self.signature.push('<');
            }
        }
    }
}

impl SignatureVisitor for SignatureWriter {
    fn visit_formal_type_parameter(&mut self, name: &JavaStr) {
        if !self.has_formals {
            self.has_formals = true;
            self.signature.push('<');
        }
        self.signature.push_java_str(name);
        self.signature.push(':');
    }

    fn visit_class_bound(&mut self) -> &mut dyn SignatureVisitor {
        self
    }

    fn visit_interface_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.signature.push(':');
        self
    }

    fn visit_superclass(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        self
    }

    fn visit_interface(&mut self) -> &mut dyn SignatureVisitor {
        self
    }

    fn visit_parameter_type(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        if !self.has_parameters {
            self.has_parameters = true;
            self.signature.push('(');
        }
        self
    }

    fn visit_return_type(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        if !self.has_parameters {
            self.signature.push('(');
        }
        self.signature.push(')');
        self
    }

    fn visit_exception_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature.push('^');
        self
    }

    fn visit_base_type(&mut self, descriptor: char) {
        self.signature.push(descriptor);
    }

    fn visit_type_variable(&mut self, name: &JavaStr) {
        self.signature.push('T');
        self.signature.push_java_str(name);
        self.signature.push(';');
    }

    fn visit_array_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature.push('[');
        self
    }

    fn visit_class_type(&mut self, name: &JavaStr) {
        self.signature.push('L');
        self.signature.push_java_str(name);
        // Pushes 'false' on the stack, meaning that this type does not have type arguments (as
        // far as we can tell at this point).
        self.argument_stack.push(false);
    }

    fn visit_inner_class_type(&mut self, name: &JavaStr) {
        self.end_arguments();
        self.signature.push('.');
        self.signature.push_java_str(name);
        // Pushes 'false' on the stack, meaning that this type does not have type arguments (as
        // far as we can tell at this point).
        self.argument_stack.push(false);
    }

    fn visit_type_argument(&mut self) {
        self.begin_argument();
        self.signature.push('*');
    }

    fn visit_type_argument_wildcard(&mut self, wildcard: char) -> &mut dyn SignatureVisitor {
        self.begin_argument();
        if wildcard != INSTANCEOF {
            self.signature.push(wildcard);
        }
        self
    }

    fn visit_end(&mut self) {
        self.end_arguments();
        self.signature.push(';');
    }
}

/// The signature that was built by this signature writer.
impl fmt::Display for SignatureWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.signature, f)
    }
}
