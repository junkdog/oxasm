//! [`CheckSignatureAdapter`]: ASM's `CheckSignatureAdapter`.

use std::marker::PhantomData;
use std::ptr::NonNull;

use oxasm::JavaStr;
use oxasm::signature::SignatureVisitor;

use crate::check::support::{illegal_argument, illegal_state_without_message};

/// The kind of signature a [`CheckSignatureAdapter`] checks (ASM's `CLASS_SIGNATURE`,
/// `METHOD_SIGNATURE` and `TYPE_SIGNATURE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignatureKind {
    /// A class signature.
    Class,
    /// A method signature.
    Method,
    /// A type signature (field signatures, and the types inside class and method signatures).
    Type,
}

/// The states of the automaton that checks the order of the method calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Empty,
    Formal,
    Bound,
    Super,
    Param,
    Return,
    SimpleType,
    ClassType,
    End,
}

/// A [`SignatureVisitor`] that checks that its methods are properly used, and forwards them to
/// an optional downstream visitor.
///
/// Like ASM's, the visitors it returns for nested types are new adapters, which check the
/// nested types: they are owned by this adapter (ADR-14), each one replacing the previous one.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckSignatureAdapter<'a> {
    /// The type of the visited signature.
    kind: SignatureKind,
    /// The current state of the automaton used to check the order of method calls.
    state: State,
    /// Whether the visited signature can be 'V'.
    can_be_void: bool,
    /// The visitor to which this adapter delegates calls, if any: the `&'a mut` given to the
    /// constructor, or, for a nested adapter, the visitor its parent's delegate returned for
    /// the nested type (see [`Self::child`] for why this is a pointer).
    sv: Option<NonNull<dyn SignatureVisitor + 'a>>,
    /// The adapter of the last nested type, which [`Self::child`] returns.
    child: Option<Box<CheckSignatureAdapter<'a>>>,
    _delegate: PhantomData<&'a mut dyn SignatureVisitor>,
}

impl<'a> CheckSignatureAdapter<'a> {
    /// Constructs a new adapter for a signature of the given kind, that forwards the events to
    /// `signature_visitor`.
    pub fn new(
        kind: SignatureKind,
        signature_visitor: Option<&'a mut dyn SignatureVisitor>,
    ) -> CheckSignatureAdapter<'a> {
        CheckSignatureAdapter {
            kind,
            state: State::Empty,
            can_be_void: false,
            sv: signature_visitor.map(NonNull::from),
            child: None,
            _delegate: PhantomData,
        }
    }

    /// The delegate, if any.
    fn delegate(&mut self) -> Option<&mut (dyn SignatureVisitor + 'a)> {
        // SAFETY: `sv` is valid while `self` is in use; see `child`.
        self.sv.map(|mut sv| unsafe { sv.as_mut() })
    }

    /// Creates a new adapter for a nested type, which forwards to the visitor that `nested`
    /// returns from this adapter's delegate, and returns it.
    ///
    /// The nested visitor borrows this adapter's delegate, and the nested adapter that holds it
    /// is owned by this adapter: a self-reference, which is why `sv` is a pointer. This is
    /// sound because the nested adapter is only reachable through the `&mut` returned here,
    /// which borrows `self` mutably:
    ///
    /// - while it is in use, neither this adapter nor its delegate can be used, so the nested
    ///   visitor is not invalidated;
    /// - once this adapter is used again, the nested adapter cannot be reached any more: it is
    ///   never dereferenced again, only replaced by the next one (dropping it does not read its
    ///   pointer), so it does not matter that using the delegate again invalidates its pointer.
    fn child(
        &mut self,
        can_be_void: bool,
        nested: impl FnOnce(&mut dyn SignatureVisitor) -> &mut dyn SignatureVisitor,
    ) -> &mut dyn SignatureVisitor {
        let sv = self.delegate().map(|sv| NonNull::from(nested(sv)));
        // SAFETY: only the trait object's lifetime bound changes; see above for why the
        // pointer is not used beyond the actual lifetime of the nested visitor.
        let sv = sv.map(|sv| unsafe {
            std::mem::transmute::<
                NonNull<dyn SignatureVisitor + '_>,
                NonNull<dyn SignatureVisitor + 'a>,
            >(sv)
        });
        let child = CheckSignatureAdapter {
            kind: SignatureKind::Type,
            state: State::Empty,
            can_be_void,
            sv,
            child: None,
            _delegate: PhantomData,
        };
        &mut **self.child.insert(Box::new(child))
    }

    fn check_state(&self, valid: bool) {
        if !valid {
            illegal_state_without_message();
        }
    }
}

impl std::fmt::Debug for CheckSignatureAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckSignatureAdapter")
            .field("kind", &self.kind)
            .field("state", &self.state)
            .field("can_be_void", &self.can_be_void)
            .finish_non_exhaustive()
    }
}

/// `checkClassName`.
fn check_class_name(name: &JavaStr, message: &str) {
    if name.is_empty() {
        illegal_argument(format_args!(
            "Invalid {message} (must not be null or empty)"
        ));
    }
    if name
        .encode_utf16()
        .any(|c| b".;[<>:".iter().any(|&b| c == u16::from(b)))
    {
        illegal_argument(format_args!(
            "Invalid {message} (must not contain . ; [ < > or :): {name}"
        ));
    }
}

/// `checkIdentifier`.
fn check_identifier(name: &JavaStr, message: &str) {
    if name.is_empty() {
        illegal_argument(format_args!(
            "Invalid {message} (must not be null or empty)"
        ));
    }
    if name
        .encode_utf16()
        .any(|c| b".;[/<>:".iter().any(|&b| c == u16::from(b)))
    {
        illegal_argument(format_args!(
            "Invalid {message} (must not contain . ; [ / < > or :): {name}"
        ));
    }
}

impl SignatureVisitor for CheckSignatureAdapter<'_> {
    // Class and method signatures.

    fn visit_formal_type_parameter(&mut self, name: &JavaStr) {
        self.check_state(
            self.kind != SignatureKind::Type
                && matches!(self.state, State::Empty | State::Formal | State::Bound),
        );
        check_identifier(name, "formal type parameter");
        self.state = State::Formal;
        if let Some(sv) = self.delegate() {
            sv.visit_formal_type_parameter(name);
        }
    }

    fn visit_class_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(self.kind != SignatureKind::Type && self.state == State::Formal);
        self.state = State::Bound;
        self.child(false, |sv| sv.visit_class_bound())
    }

    fn visit_interface_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(
            self.kind != SignatureKind::Type && matches!(self.state, State::Formal | State::Bound),
        );
        self.child(false, |sv| sv.visit_interface_bound())
    }

    // Class signatures.

    fn visit_superclass(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(
            self.kind == SignatureKind::Class
                && matches!(self.state, State::Empty | State::Formal | State::Bound),
        );
        self.state = State::Super;
        self.child(false, |sv| sv.visit_superclass())
    }

    fn visit_interface(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(self.kind == SignatureKind::Class && self.state == State::Super);
        self.child(false, |sv| sv.visit_interface())
    }

    // Method signatures.

    fn visit_parameter_type(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(
            self.kind == SignatureKind::Method
                && matches!(
                    self.state,
                    State::Empty | State::Formal | State::Bound | State::Param
                ),
        );
        self.state = State::Param;
        self.child(false, |sv| sv.visit_parameter_type())
    }

    fn visit_return_type(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(
            self.kind == SignatureKind::Method
                && matches!(
                    self.state,
                    State::Empty | State::Formal | State::Bound | State::Param
                ),
        );
        self.state = State::Return;
        self.child(true, |sv| sv.visit_return_type())
    }

    fn visit_exception_type(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(self.kind == SignatureKind::Method && self.state == State::Return);
        self.child(false, |sv| sv.visit_exception_type())
    }

    // Type signatures.

    fn visit_base_type(&mut self, descriptor: char) {
        self.check_state(self.kind == SignatureKind::Type && self.state == State::Empty);
        if descriptor == 'V' {
            if !self.can_be_void {
                illegal_argument("Base type descriptor can't be V");
            }
        } else if !"ZCBSIFJD".contains(descriptor) {
            illegal_argument("Base type descriptor must be one of ZCBSIFJD");
        }
        self.state = State::SimpleType;
        if let Some(sv) = self.delegate() {
            sv.visit_base_type(descriptor);
        }
    }

    fn visit_type_variable(&mut self, name: &JavaStr) {
        self.check_state(self.kind == SignatureKind::Type && self.state == State::Empty);
        check_identifier(name, "type variable");
        self.state = State::SimpleType;
        if let Some(sv) = self.delegate() {
            sv.visit_type_variable(name);
        }
    }

    fn visit_array_type(&mut self) -> &mut dyn SignatureVisitor {
        self.check_state(self.kind == SignatureKind::Type && self.state == State::Empty);
        self.state = State::SimpleType;
        self.child(false, |sv| sv.visit_array_type())
    }

    fn visit_class_type(&mut self, name: &JavaStr) {
        self.check_state(self.kind == SignatureKind::Type && self.state == State::Empty);
        check_class_name(name, "class name");
        self.state = State::ClassType;
        if let Some(sv) = self.delegate() {
            sv.visit_class_type(name);
        }
    }

    fn visit_inner_class_type(&mut self, name: &JavaStr) {
        self.check_state(self.state == State::ClassType);
        check_identifier(name, "inner class name");
        if let Some(sv) = self.delegate() {
            sv.visit_inner_class_type(name);
        }
    }

    fn visit_type_argument(&mut self) {
        self.check_state(self.state == State::ClassType);
        if let Some(sv) = self.delegate() {
            sv.visit_type_argument();
        }
    }

    fn visit_type_argument_wildcard(&mut self, wildcard: char) -> &mut dyn SignatureVisitor {
        self.check_state(self.state == State::ClassType);
        if !"+-=".contains(wildcard) {
            illegal_argument("Wildcard must be one of +-=");
        }
        self.child(false, |sv| sv.visit_type_argument_wildcard(wildcard))
    }

    fn visit_end(&mut self) {
        self.check_state(self.state == State::ClassType);
        self.state = State::End;
        if let Some(sv) = self.delegate() {
            sv.visit_end();
        }
    }
}
