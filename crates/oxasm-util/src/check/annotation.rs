//! [`CheckAnnotationAdapter`]: ASM's `CheckAnnotationAdapter`.

use oxasm::constant::AnnotationValue;
use oxasm::opcodes::V1_5;
use oxasm::{AnnotationVisitor, JavaStr, TypeSort};

use crate::check::support::{check_descriptor, illegal_argument, illegal_state};

/// An [`AnnotationVisitor`] that checks that its methods are properly used, and forwards them to
/// an optional downstream visitor.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckAnnotationAdapter<'a> {
    av: Option<Box<dyn AnnotationVisitor + 'a>>,
    /// Whether the values of the visited annotation are named. The visitors of annotation
    /// defaults and arrays use unnamed values.
    use_named_value: bool,
    /// Whether `visit_end` has been called.
    visit_end_called: bool,
}

impl<'a> CheckAnnotationAdapter<'a> {
    /// Constructs a new adapter, for an annotation with named values, that forwards the events to
    /// `annotation_visitor`.
    pub fn new(annotation_visitor: Option<Box<dyn AnnotationVisitor + 'a>>) -> Self {
        CheckAnnotationAdapter::with_named_values(annotation_visitor, true)
    }

    /// Constructs a new adapter whose values are named or not (unnamed for annotation defaults
    /// and arrays).
    pub(crate) fn with_named_values(
        annotation_visitor: Option<Box<dyn AnnotationVisitor + 'a>>,
        use_named_values: bool,
    ) -> Self {
        CheckAnnotationAdapter {
            av: annotation_visitor,
            use_named_value: use_named_values,
            visit_end_called: false,
        }
    }

    /// A boxed adapter for a child annotation visitor, as the check adapters return.
    pub(crate) fn boxed(
        annotation_visitor: Option<Box<dyn AnnotationVisitor + 'a>>,
        use_named_values: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(CheckAnnotationAdapter::with_named_values(
            annotation_visitor,
            use_named_values,
        )))
    }

    fn check_name(&self, name: Option<&JavaStr>) {
        if self.use_named_value && name.is_none() {
            illegal_argument("Annotation value name must not be null");
        }
    }

    fn check_visit_end_not_called(&self) {
        if self.visit_end_called {
            illegal_state("Cannot call a visit method after visitEnd has been called");
        }
    }
}

impl std::fmt::Debug for CheckAnnotationAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckAnnotationAdapter")
            .field("use_named_value", &self.use_named_value)
            .field("visit_end_called", &self.visit_end_called)
            .finish_non_exhaustive()
    }
}

impl AnnotationVisitor for CheckAnnotationAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        match &mut self.av {
            Some(av) => Some(&mut **av),
            None => None,
        }
    }

    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        self.check_visit_end_not_called();
        self.check_name(name);
        // The other values of ASM's check (a primitive, a string, a type or a primitive array)
        // are the only ones an AnnotationValue can hold.
        if let AnnotationValue::Type(ty) = value
            && ty.get_sort() == TypeSort::Method
        {
            illegal_argument("Invalid annotation value");
        }
        if let Some(av) = &mut self.av {
            av.visit(name, value);
        }
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        self.check_visit_end_not_called();
        self.check_name(name);
        // Annotations can only appear in V1_5 or more classes.
        check_descriptor(V1_5, descriptor, false);
        if let Some(av) = &mut self.av {
            av.visit_enum(name, descriptor, value);
        }
    }

    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        self.check_name(name);
        // Annotations can only appear in V1_5 or more classes.
        check_descriptor(V1_5, descriptor, false);
        let av = self
            .av
            .as_mut()
            .and_then(|av| av.visit_annotation(name, descriptor));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_array<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.check_visit_end_not_called();
        self.check_name(name);
        let av = self.av.as_mut().and_then(|av| av.visit_array(name));
        CheckAnnotationAdapter::boxed(av, false)
    }

    fn visit_end(&mut self) {
        self.check_visit_end_not_called();
        self.visit_end_called = true;
        if let Some(av) = &mut self.av {
            av.visit_end();
        }
    }
}
