//! [`CheckFieldAdapter`] and [`CheckRecordComponentAdapter`]: ASM's `CheckFieldAdapter` and
//! `CheckRecordComponentAdapter`, which check the same things.

use oxasm::opcodes::V1_5;
use oxasm::{
    AnnotationVisitor, Attribute, FieldVisitor, JavaStr, RecordComponentVisitor, TypePath,
    TypeReference,
};

use crate::check::CheckAnnotationAdapter;
use crate::check::support::{check_descriptor, check_type_ref, check_type_ref_sort, illegal_state};

/// The checks of the annotations of a field or record component.
fn check_annotation(descriptor: &JavaStr) {
    // Annotations can only appear in V1_5 or more classes.
    check_descriptor(V1_5, descriptor, false);
}

/// The checks of the type annotations of a field or record component.
fn check_type_annotation(type_ref: TypeReference, descriptor: &JavaStr) {
    check_type_ref_sort(type_ref, &[TypeReference::FIELD]);
    check_type_ref(type_ref);
    check_descriptor(V1_5, descriptor, false);
}

fn check_visit_end_not_called(visit_end_called: bool) {
    if visit_end_called {
        illegal_state("Cannot call a visit method after visitEnd has been called");
    }
}

/// A [`FieldVisitor`] that checks that its methods are properly used, and forwards them to an
/// optional downstream visitor.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckFieldAdapter<'a> {
    fv: Option<Box<dyn FieldVisitor + 'a>>,
    /// Whether `visit_end` has been called.
    visit_end_called: bool,
}

impl<'a> CheckFieldAdapter<'a> {
    /// Constructs a new adapter that forwards the events to `field_visitor`.
    pub fn new(field_visitor: Option<Box<dyn FieldVisitor + 'a>>) -> Self {
        CheckFieldAdapter {
            fv: field_visitor,
            visit_end_called: false,
        }
    }
}

impl std::fmt::Debug for CheckFieldAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckFieldAdapter")
            .field("visit_end_called", &self.visit_end_called)
            .finish_non_exhaustive()
    }
}

impl FieldVisitor for CheckFieldAdapter<'_> {
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
        check_visit_end_not_called(self.visit_end_called);
        check_annotation(descriptor);
        let av = self
            .fv
            .as_mut()
            .and_then(|fv| fv.visit_annotation(descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        check_visit_end_not_called(self.visit_end_called);
        check_type_annotation(type_ref, descriptor);
        let av = self
            .fv
            .as_mut()
            .and_then(|fv| fv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        check_visit_end_not_called(self.visit_end_called);
        if let Some(fv) = &mut self.fv {
            fv.visit_attribute(attribute);
        }
    }

    fn visit_end(&mut self) {
        check_visit_end_not_called(self.visit_end_called);
        self.visit_end_called = true;
        if let Some(fv) = &mut self.fv {
            fv.visit_end();
        }
    }
}

/// A [`RecordComponentVisitor`] that checks that its methods are properly used, and forwards
/// them to an optional downstream visitor.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckRecordComponentAdapter<'a> {
    rcv: Option<Box<dyn RecordComponentVisitor + 'a>>,
    /// Whether `visit_end` has been called.
    visit_end_called: bool,
}

impl<'a> CheckRecordComponentAdapter<'a> {
    /// Constructs a new adapter that forwards the events to `record_component_visitor`.
    pub fn new(record_component_visitor: Option<Box<dyn RecordComponentVisitor + 'a>>) -> Self {
        CheckRecordComponentAdapter {
            rcv: record_component_visitor,
            visit_end_called: false,
        }
    }
}

impl std::fmt::Debug for CheckRecordComponentAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckRecordComponentAdapter")
            .field("visit_end_called", &self.visit_end_called)
            .finish_non_exhaustive()
    }
}

impl RecordComponentVisitor for CheckRecordComponentAdapter<'_> {
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
        check_visit_end_not_called(self.visit_end_called);
        check_annotation(descriptor);
        let av = self
            .rcv
            .as_mut()
            .and_then(|rcv| rcv.visit_annotation(descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        check_visit_end_not_called(self.visit_end_called);
        check_type_annotation(type_ref, descriptor);
        let av = self
            .rcv
            .as_mut()
            .and_then(|rcv| rcv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        CheckAnnotationAdapter::boxed(av, true)
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        check_visit_end_not_called(self.visit_end_called);
        if let Some(rcv) = &mut self.rcv {
            rcv.visit_attribute(attribute);
        }
    }

    fn visit_end(&mut self) {
        check_visit_end_not_called(self.visit_end_called);
        self.visit_end_called = true;
        if let Some(rcv) = &mut self.rcv {
            rcv.visit_end();
        }
    }
}
