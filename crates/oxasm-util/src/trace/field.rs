//! [`TraceFieldVisitor`]: ASM's `TraceFieldVisitor`.

use oxasm::{AnnotationVisitor, Attribute, FieldVisitor, JavaStr, TypePath, TypeReference};

use crate::printer::Printer;
use crate::trace::TraceAnnotationVisitor;

/// A [`FieldVisitor`] that prints the fields it visits with a [`Printer`].
pub struct TraceFieldVisitor<'a, P> {
    fv: Option<Box<dyn FieldVisitor + 'a>>,
    /// The printer to convert the visited field into text.
    pub p: P,
}

impl<P: Printer> TraceFieldVisitor<'_, P> {
    /// Constructs a new visitor, without a downstream visitor.
    pub fn new(printer: P) -> Self {
        TraceFieldVisitor {
            fv: None,
            p: printer,
        }
    }
}

impl<'a, P: Printer> TraceFieldVisitor<'a, P> {
    /// Constructs a new visitor that also forwards the events to `field_visitor`.
    pub fn with_delegate(field_visitor: Option<Box<dyn FieldVisitor + 'a>>, printer: P) -> Self {
        TraceFieldVisitor {
            fv: field_visitor,
            p: printer,
        }
    }
}

impl<P> std::fmt::Debug for TraceFieldVisitor<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceFieldVisitor").finish_non_exhaustive()
    }
}

impl<P: Printer> FieldVisitor for TraceFieldVisitor<'_, P> {
    fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
        match &mut self.fv {
            Some(fv) => Some(&mut **fv),
            None => None,
        }
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self.p.visit_field_annotation(descriptor, visible);
        let av = self
            .fv
            .as_mut()
            .and_then(|fv| fv.visit_annotation(descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .p
            .visit_field_type_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .fv
            .as_mut()
            .and_then(|fv| fv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.p.visit_field_attribute(attribute);
        if let Some(fv) = &mut self.fv {
            fv.visit_attribute(attribute);
        }
    }

    fn visit_end(&mut self) {
        self.p.visit_field_end();
        if let Some(fv) = &mut self.fv {
            fv.visit_end();
        }
    }
}
