//! [`TraceAnnotationVisitor`]: ASM's `TraceAnnotationVisitor`.

use oxasm::constant::AnnotationValue;
use oxasm::{AnnotationVisitor, JavaStr};

use crate::printer::Printer;

/// An [`AnnotationVisitor`] that prints the annotations it visits with a [`Printer`].
pub struct TraceAnnotationVisitor<'a, P> {
    av: Option<Box<dyn AnnotationVisitor + 'a>>,
    /// The printer to convert the visited annotation into text.
    pub printer: P,
}

impl<P: Printer> TraceAnnotationVisitor<'_, P> {
    /// Constructs a new visitor, without a downstream visitor.
    pub fn new(printer: P) -> Self {
        TraceAnnotationVisitor { av: None, printer }
    }
}

impl<'a, P: Printer> TraceAnnotationVisitor<'a, P> {
    /// Constructs a new visitor that also forwards the events to `annotation_visitor`.
    pub fn with_delegate(
        annotation_visitor: Option<Box<dyn AnnotationVisitor + 'a>>,
        printer: P,
    ) -> Self {
        TraceAnnotationVisitor {
            av: annotation_visitor,
            printer,
        }
    }
}

impl<P> std::fmt::Debug for TraceAnnotationVisitor<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceAnnotationVisitor")
            .finish_non_exhaustive()
    }
}

impl<P: Printer> AnnotationVisitor for TraceAnnotationVisitor<'_, P> {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        match &mut self.av {
            Some(av) => Some(&mut **av),
            None => None,
        }
    }

    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        self.printer.visit_annotation_value(name, value);
        if let Some(av) = &mut self.av {
            av.visit(name, value);
        }
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        self.printer.visit_enum(name, descriptor, value);
        if let Some(av) = &mut self.av {
            av.visit_enum(name, descriptor, value);
        }
    }

    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self.printer.visit_annotation(name, descriptor);
        let av = self
            .av
            .as_mut()
            .and_then(|av| av.visit_annotation(name, descriptor));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_array<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let array_printer = self.printer.visit_array(name);
        let av = self.av.as_mut().and_then(|av| av.visit_array(name));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            array_printer,
        )))
    }

    fn visit_end(&mut self) {
        self.printer.visit_annotation_end();
        if let Some(av) = &mut self.av {
            av.visit_end();
        }
    }
}
