//! [`TraceSignatureVisitor`]: ASM's `TraceSignatureVisitor`, a [`SignatureVisitor`] that builds
//! the Java generic type declaration corresponding to the signature it visits.

use oxasm::signature::{EXTENDS, SUPER, SignatureVisitor};
use oxasm::{JavaStr, JavaString, opcodes};

const COMMA_SEPARATOR: &str = ", ";
const EXTENDS_SEPARATOR: &str = " extends ";
const IMPLEMENTS_SEPARATOR: &str = " implements ";

/// The Java names of the base types, by descriptor character.
fn base_type(descriptor: char) -> Option<&'static str> {
    Some(match descriptor {
        'Z' => "boolean",
        'B' => "byte",
        'C' => "char",
        'S' => "short",
        'I' => "int",
        'J' => "long",
        'F' => "float",
        'D' => "double",
        'V' => "void",
        _ => return None,
    })
}

/// The things to do at the end of a type, innermost last (ASM's `END_*` tasks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    /// Append `[]`.
    ArrayType,
    /// Nothing to append: the class type has no type arguments.
    EmptyTypeArguments,
    /// Append `>`.
    NonEmptyTypeArguments,
}

/// A [`SignatureVisitor`] that builds the Java generic type declaration corresponding to the
/// signature it visits.
///
/// ASM's version writes the return type and the exception types with fresh visitors sharing
/// a `StringBuilder`; here they are child visitors owned by this one (ADR-14), whose declarations
/// [`get_return_type`](Self::get_return_type) and [`get_exceptions`](Self::get_exceptions)
/// return.
///
/// Event sequences that ASM's version rejects with an exception (other than an invalid base type,
/// which panics as ASM throws) are tolerated, so that a malformed signature from a class file
/// cannot make a printer panic.
#[derive(Debug)]
pub struct TraceSignatureVisitor {
    /// Whether the visited signature is a class signature of a Java interface.
    is_interface: bool,
    /// The Java generic type declaration corresponding to the visited signature.
    declaration: JavaString,
    /// The visitor of the return type, if the signature is a method signature.
    return_type: Option<Box<TraceSignatureVisitor>>,
    /// The visitors of the exception types, if any.
    exceptions: Vec<TraceSignatureVisitor>,
    /// Whether `visit_formal_type_parameter` has been called since the last visit of a
    /// superclass, interface, parameter or return type.
    formal_type_parameter_visited: bool,
    /// Whether `visit_interface_bound` has been called since the last visit of a formal type
    /// parameter.
    interface_bound_visited: bool,
    /// Whether `visit_parameter_type` has been called since the last visit of a return type.
    parameter_type_visited: bool,
    /// Whether `visit_interface` has been called since the last visit of a superclass.
    interface_visited: bool,
    /// The stack of tasks to complete at the end of each type.
    pending_tasks: Vec<Task>,
    /// The separator to append before the next visited class or inner class type.
    separator: &'static str,
}

impl TraceSignatureVisitor {
    /// Constructs a new visitor for a class, method or field with the given access flags.
    #[must_use]
    pub fn new(access_flags: u32) -> TraceSignatureVisitor {
        TraceSignatureVisitor {
            is_interface: access_flags & opcodes::ACC_INTERFACE != 0,
            declaration: JavaString::new(),
            return_type: None,
            exceptions: Vec::new(),
            formal_type_parameter_visited: false,
            interface_bound_visited: false,
            parameter_type_visited: false,
            interface_visited: false,
            pending_tasks: Vec::new(),
            separator: "",
        }
    }

    /// The Java generic type declaration corresponding to the visited signature.
    #[must_use]
    pub fn get_declaration(&self) -> &JavaStr {
        &self.declaration
    }

    /// The Java generic method return type declaration corresponding to the visited signature,
    /// or `None` if the signature is not a method signature.
    #[must_use]
    pub fn get_return_type(&self) -> Option<&JavaStr> {
        self.return_type.as_ref().map(|v| v.get_declaration())
    }

    /// The Java generic exception types declaration corresponding to the visited signature, or
    /// `None` if the signature has no exception types.
    #[must_use]
    pub fn get_exceptions(&self) -> Option<JavaString> {
        let (first, rest) = self.exceptions.split_first()?;
        let mut exceptions = first.declaration.clone();
        for exception in rest {
            exceptions.push_str(COMMA_SEPARATOR);
            exceptions.push_java_str(&exception.declaration);
        }
        Some(exceptions)
    }

    fn end_formals(&mut self) {
        if self.formal_type_parameter_visited {
            self.declaration.push('>');
            self.formal_type_parameter_visited = false;
        }
    }

    fn end_type_arguments(&mut self) {
        if self.pending_tasks.pop() == Some(Task::NonEmptyTypeArguments) {
            self.declaration.push('>');
        }
    }

    fn end_type(&mut self) {
        while self.pending_tasks.last() == Some(&Task::ArrayType) {
            self.declaration.push_str("[]");
            self.pending_tasks.pop();
        }
    }

    /// Starts the type arguments, or appends a separator if there are already some.
    fn start_type_argument(&mut self) {
        match self.pending_tasks.last_mut() {
            Some(task @ Task::EmptyTypeArguments) => {
                *task = Task::NonEmptyTypeArguments;
                self.declaration.push('<');
            }
            _ => self.declaration.push_str(COMMA_SEPARATOR),
        }
    }
}

impl SignatureVisitor for TraceSignatureVisitor {
    fn visit_formal_type_parameter(&mut self, name: &JavaStr) {
        self.declaration
            .push_str(if self.formal_type_parameter_visited {
                COMMA_SEPARATOR
            } else {
                "<"
            });
        self.declaration.push_java_str(name);
        self.formal_type_parameter_visited = true;
        self.interface_bound_visited = false;
    }

    fn visit_class_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.separator = EXTENDS_SEPARATOR;
        self
    }

    fn visit_interface_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.separator = if self.interface_bound_visited {
            COMMA_SEPARATOR
        } else {
            EXTENDS_SEPARATOR
        };
        self.interface_bound_visited = true;
        self
    }

    fn visit_superclass(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        self.separator = EXTENDS_SEPARATOR;
        self
    }

    fn visit_interface(&mut self) -> &mut dyn SignatureVisitor {
        if self.interface_visited {
            self.separator = COMMA_SEPARATOR;
        } else {
            self.separator = if self.is_interface {
                EXTENDS_SEPARATOR
            } else {
                IMPLEMENTS_SEPARATOR
            };
            self.interface_visited = true;
        }
        self
    }

    fn visit_parameter_type(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        if self.parameter_type_visited {
            self.declaration.push_str(COMMA_SEPARATOR);
        } else {
            self.declaration.push('(');
            self.parameter_type_visited = true;
        }
        self
    }

    fn visit_return_type(&mut self) -> &mut dyn SignatureVisitor {
        self.end_formals();
        if self.parameter_type_visited {
            self.parameter_type_visited = false;
        } else {
            self.declaration.push('(');
        }
        self.declaration.push(')');
        &mut **self
            .return_type
            .insert(Box::new(TraceSignatureVisitor::new(0)))
    }

    fn visit_exception_type(&mut self) -> &mut dyn SignatureVisitor {
        self.exceptions.push(TraceSignatureVisitor::new(0));
        self.exceptions.last_mut().expect("just pushed")
    }

    /// # Panics
    ///
    /// If `descriptor` is not a base type descriptor (ASM throws `IllegalArgumentException`).
    fn visit_base_type(&mut self, descriptor: char) {
        let base_type = base_type(descriptor)
            .unwrap_or_else(|| panic!("invalid base type descriptor: {descriptor:?}"));
        self.declaration.push_str(base_type);
        self.end_type();
    }

    fn visit_type_variable(&mut self, name: &JavaStr) {
        self.declaration.push_str(self.separator);
        self.declaration.push_java_str(name);
        self.separator = "";
        self.end_type();
    }

    fn visit_array_type(&mut self) -> &mut dyn SignatureVisitor {
        self.pending_tasks.push(Task::ArrayType);
        self
    }

    fn visit_class_type(&mut self, name: &JavaStr) {
        if name == "java/lang/Object" {
            // 'Map<java.lang.Object,java.util.List>' or 'abstract public V get(Object key);'
            // should have Object 'but java.lang.String extends java.lang.Object' is unnecessary.
            let need_object_class = self.pending_tasks.last() == Some(&Task::NonEmptyTypeArguments)
                || self.parameter_type_visited;
            if need_object_class {
                self.declaration.push_str(self.separator);
                self.declaration.push_java_str(&name.replace('/', '.'));
            }
        } else {
            self.declaration.push_str(self.separator);
            self.declaration.push_java_str(&name.replace('/', '.'));
        }
        self.separator = "";
        self.pending_tasks.push(Task::EmptyTypeArguments);
    }

    fn visit_inner_class_type(&mut self, name: &JavaStr) {
        self.end_type_arguments();
        self.declaration.push('.');
        self.declaration.push_str(self.separator);
        self.declaration.push_java_str(&name.replace('/', '.'));
        self.pending_tasks.push(Task::EmptyTypeArguments);
        self.separator = "";
    }

    fn visit_type_argument(&mut self) {
        self.start_type_argument();
        self.declaration.push('?');
    }

    fn visit_type_argument_wildcard(&mut self, tag: char) -> &mut dyn SignatureVisitor {
        self.start_type_argument();
        if tag == EXTENDS {
            self.declaration.push_str("? extends ");
        } else if tag == SUPER {
            self.declaration.push_str("? super ");
        }
        self
    }

    fn visit_end(&mut self) {
        self.end_type_arguments();
        self.end_type();
    }
}
