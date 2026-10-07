//! [`SignatureRemapper`]: ASM's `SignatureRemapper`.

use oxasm::signature::SignatureVisitor;
use oxasm::{JavaStr, JavaString};

use crate::remapper::Remapper;

/// A [`SignatureVisitor`] that remaps types with a [`Remapper`], and forwards the remapped
/// events to another signature visitor.
///
/// Like ASM's, it returns itself for the nested types (`visit_class_bound`, ...), and calls the
/// same methods of its delegate, whose results it ignores: the delegate must handle a nested
/// type by itself, as a [`SignatureWriter`](oxasm::signature::SignatureWriter) does.
pub struct SignatureRemapper<'a, R: ?Sized> {
    signature_visitor: &'a mut dyn SignatureVisitor,
    remapper: &'a R,
    class_names: Vec<JavaString>,
}

impl<'a, R: Remapper + ?Sized> SignatureRemapper<'a, R> {
    /// Constructs a new signature remapper.
    ///
    /// - `signature_visitor`: the signature visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited signature.
    pub fn new(signature_visitor: &'a mut dyn SignatureVisitor, remapper: &'a R) -> Self {
        SignatureRemapper {
            signature_visitor,
            remapper,
            class_names: Vec::new(),
        }
    }
}

impl<R: ?Sized> std::fmt::Debug for SignatureRemapper<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignatureRemapper")
            .field("class_names", &self.class_names)
            .finish_non_exhaustive()
    }
}

impl<R: Remapper + ?Sized> SignatureVisitor for SignatureRemapper<'_, R> {
    fn visit_class_type(&mut self, name: &JavaStr) {
        self.class_names.push(name.to_owned());
        self.signature_visitor
            .visit_class_type(&self.remapper.map_type(name));
    }

    /// # Panics
    ///
    /// If no class type is being visited (ASM's `IndexOutOfBoundsException`).
    fn visit_inner_class_type(&mut self, name: &JavaStr) {
        let outer_class_name = self.class_names.pop().expect("an outer class type");
        let mut class_name = outer_class_name.clone();
        class_name.push('$');
        class_name.push_java_str(name);
        let mut remapped_outer = self.remapper.map_type(&outer_class_name).into_owned();
        remapped_outer.push('$');
        let remapped_name = self.remapper.map_type(&class_name);
        let index = if remapped_name.starts_with(&remapped_outer) {
            remapped_outer.len()
        } else {
            remapped_name.rfind('$').map_or(0, |index| index + 1)
        };
        self.signature_visitor
            .visit_inner_class_type(&remapped_name[index..]);
        drop(remapped_name);
        self.class_names.push(class_name);
    }

    fn visit_formal_type_parameter(&mut self, name: &JavaStr) {
        self.signature_visitor.visit_formal_type_parameter(name);
    }

    fn visit_type_variable(&mut self, name: &JavaStr) {
        self.signature_visitor.visit_type_variable(name);
    }

    fn visit_array_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_array_type();
        self
    }

    fn visit_base_type(&mut self, descriptor: char) {
        self.signature_visitor.visit_base_type(descriptor);
    }

    fn visit_class_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_class_bound();
        self
    }

    fn visit_exception_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_exception_type();
        self
    }

    fn visit_interface(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_interface();
        self
    }

    fn visit_interface_bound(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_interface_bound();
        self
    }

    fn visit_parameter_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_parameter_type();
        self
    }

    fn visit_return_type(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_return_type();
        self
    }

    fn visit_superclass(&mut self) -> &mut dyn SignatureVisitor {
        self.signature_visitor.visit_superclass();
        self
    }

    fn visit_type_argument(&mut self) {
        self.signature_visitor.visit_type_argument();
    }

    fn visit_type_argument_wildcard(&mut self, wildcard: char) -> &mut dyn SignatureVisitor {
        self.signature_visitor
            .visit_type_argument_wildcard(wildcard);
        self
    }

    /// # Panics
    ///
    /// If no class type is being visited (ASM's `IndexOutOfBoundsException`).
    fn visit_end(&mut self) {
        self.signature_visitor.visit_end();
        self.class_names.pop().expect("a class type");
    }
}
