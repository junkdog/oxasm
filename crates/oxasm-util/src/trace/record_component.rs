//! [`TraceRecordComponentVisitor`]: ASM's `TraceRecordComponentVisitor`.

use oxasm::{
    AnnotationVisitor, Attribute, JavaStr, RecordComponentVisitor, TypePath, TypeReference,
};

use crate::printer::Printer;
use crate::trace::TraceAnnotationVisitor;

/// A [`RecordComponentVisitor`] that prints the record components it visits with a [`Printer`].
pub struct TraceRecordComponentVisitor<'a, P> {
    rcv: Option<Box<dyn RecordComponentVisitor + 'a>>,
    /// The printer to convert the visited record component into text.
    pub printer: P,
}

impl<P: Printer> TraceRecordComponentVisitor<'_, P> {
    /// Constructs a new visitor, without a downstream visitor.
    pub fn new(printer: P) -> Self {
        TraceRecordComponentVisitor { rcv: None, printer }
    }
}

impl<'a, P: Printer> TraceRecordComponentVisitor<'a, P> {
    /// Constructs a new visitor that also forwards the events to `record_component_visitor`.
    pub fn with_delegate(
        record_component_visitor: Option<Box<dyn RecordComponentVisitor + 'a>>,
        printer: P,
    ) -> Self {
        TraceRecordComponentVisitor {
            rcv: record_component_visitor,
            printer,
        }
    }
}

impl<P> std::fmt::Debug for TraceRecordComponentVisitor<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceRecordComponentVisitor")
            .finish_non_exhaustive()
    }
}

impl<P: Printer> RecordComponentVisitor for TraceRecordComponentVisitor<'_, P> {
    fn delegate(&mut self) -> Option<&mut dyn RecordComponentVisitor> {
        match &mut self.rcv {
            Some(rcv) => Some(&mut **rcv),
            None => None,
        }
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation_printer = self
            .printer
            .visit_record_component_annotation(descriptor, visible);
        let av = self
            .rcv
            .as_mut()
            .and_then(|rcv| rcv.visit_annotation(descriptor, visible));
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
            .printer
            .visit_record_component_type_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .rcv
            .as_mut()
            .and_then(|rcv| rcv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.printer.visit_record_component_attribute(attribute);
        if let Some(rcv) = &mut self.rcv {
            rcv.visit_attribute(attribute);
        }
    }

    fn visit_end(&mut self) {
        self.printer.visit_record_component_end();
        if let Some(rcv) = &mut self.rcv {
            rcv.visit_end();
        }
    }
}
