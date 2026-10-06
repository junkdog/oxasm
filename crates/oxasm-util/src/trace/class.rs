//! [`TraceClassVisitor`]: ASM's `TraceClassVisitor`.

use std::io;

use oxasm::constant::Constant;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FieldVisitor, JavaStr, MethodVisitor,
    ModuleVisitor, RecordComponentVisitor, TypePath, TypeReference,
};

use crate::printer::Printer;
use crate::textifier::Textifier;
use crate::trace::{
    TraceAnnotationVisitor, TraceFieldVisitor, TraceMethodVisitor, TraceModuleVisitor,
    TraceRecordComponentVisitor,
};

/// A class visitor that does nothing: the downstream visitor type of a [`TraceClassVisitor`]
/// that has none.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDelegate;

impl ClassVisitor for NoDelegate {}

/// A [`ClassVisitor`] that prints the classes it visits with a [`Printer`], and forwards the
/// events to an optional downstream visitor `V`.
///
/// The text is printed to the optional writer `W` at the end of the class (in `visit_end`). As
/// Java's `PrintWriter` does, the visitor does not fail on a write error: it keeps the first one,
/// which [`take_io_error`](Self::take_io_error) returns.
///
/// ```no_run
/// use oxasm::{ClassReader, ParsingOptions};
/// use oxasm_util::TraceClassVisitor;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let class_file = std::fs::read("Foo.class")?;
/// let mut output = Vec::new();
/// let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
/// ClassReader::new(&class_file)?.accept(&mut trace_class_visitor, ParsingOptions::empty())?;
/// trace_class_visitor.take_io_error()?;
/// # Ok(())
/// # }
/// ```
pub struct TraceClassVisitor<V, P, W> {
    cv: Option<V>,
    /// The printer to convert the visited class into text.
    pub p: P,
    print_writer: Option<W>,
    io_error: Option<io::Error>,
}

impl<W: io::Write> TraceClassVisitor<NoDelegate, Textifier, W> {
    /// Constructs a new visitor that prints the visited class with a [`Textifier`] to
    /// `print_writer`.
    pub fn new(print_writer: W) -> Self {
        TraceClassVisitor::with_printer(Textifier::new(), print_writer)
    }
}

impl<P: Printer, W: io::Write> TraceClassVisitor<NoDelegate, P, W> {
    /// Constructs a new visitor that prints the visited class with `printer` to `print_writer`.
    pub fn with_printer(printer: P, print_writer: W) -> Self {
        TraceClassVisitor::with_delegate(None, printer, Some(print_writer))
    }
}

impl<V: ClassVisitor, P: Printer, W: io::Write> TraceClassVisitor<V, P, W> {
    /// Constructs a new visitor that prints the visited class with `printer`, to `print_writer`
    /// if any, and forwards the events to `class_visitor`, if any.
    pub fn with_delegate(class_visitor: Option<V>, printer: P, print_writer: Option<W>) -> Self {
        TraceClassVisitor {
            cv: class_visitor,
            p: printer,
            print_writer,
            io_error: None,
        }
    }

    /// The downstream visitor, if any.
    pub fn class_visitor(&mut self) -> Option<&mut V> {
        self.cv.as_mut()
    }

    /// Takes the first error that occurred while printing, if any.
    ///
    /// # Errors
    ///
    /// The error.
    pub fn take_io_error(&mut self) -> io::Result<()> {
        self.io_error.take().map_or(Ok(()), Err)
    }

    /// The parts of this visitor: the downstream visitor, the printer and the writer.
    pub fn into_parts(self) -> (Option<V>, P, Option<W>) {
        (self.cv, self.p, self.print_writer)
    }
}

impl<V, P, W> std::fmt::Debug for TraceClassVisitor<V, P, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceClassVisitor").finish_non_exhaustive()
    }
}

impl<V: ClassVisitor, P: Printer, W: io::Write> ClassVisitor for TraceClassVisitor<V, P, W> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        match &mut self.cv {
            Some(cv) => Some(cv),
            None => None,
        }
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.p
            .visit(version, access, name, signature, super_name, interfaces);
        if let Some(cv) = &mut self.cv {
            cv.visit(version, access, name, signature, super_name, interfaces);
        }
    }

    fn visit_source(&mut self, file: Option<&JavaStr>, debug: Option<&JavaStr>) {
        self.p.visit_source(file, debug);
        if let Some(cv) = &mut self.cv {
            cv.visit_source(file, debug);
        }
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        flags: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        let module_printer = self.p.visit_module(name, flags, version);
        let mv = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_module(name, flags, version));
        Some(Box::new(TraceModuleVisitor::with_delegate(
            mv,
            module_printer,
        )))
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.p.visit_nest_host(nest_host);
        if let Some(cv) = &mut self.cv {
            cv.visit_nest_host(nest_host);
        }
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        self.p.visit_outer_class(owner, name, descriptor);
        if let Some(cv) = &mut self.cv {
            cv.visit_outer_class(owner, name, descriptor);
        }
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let annotation_printer = self.p.visit_class_annotation(descriptor, visible);
        let av = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_annotation(descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let annotation_printer = self
            .p
            .visit_class_type_annotation(type_ref, type_path, descriptor, visible);
        let av = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_type_annotation(type_ref, type_path, descriptor, visible));
        Some(Box::new(TraceAnnotationVisitor::with_delegate(
            av,
            annotation_printer,
        )))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.p.visit_class_attribute(attribute);
        if let Some(cv) = &mut self.cv {
            cv.visit_attribute(attribute);
        }
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        self.p.visit_nest_member(nest_member);
        if let Some(cv) = &mut self.cv {
            cv.visit_nest_member(nest_member);
        }
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        self.p.visit_permitted_subclass(permitted_subclass);
        if let Some(cv) = &mut self.cv {
            cv.visit_permitted_subclass(permitted_subclass);
        }
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        self.p
            .visit_inner_class(name, outer_name, inner_name, access);
        if let Some(cv) = &mut self.cv {
            cv.visit_inner_class(name, outer_name, inner_name, access);
        }
    }

    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        let record_component_printer = self.p.visit_record_component(name, descriptor, signature);
        let rcv = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_record_component(name, descriptor, signature));
        Some(Box::new(TraceRecordComponentVisitor::with_delegate(
            rcv,
            record_component_printer,
        )))
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        let field_printer = self
            .p
            .visit_field(access, name, descriptor, signature, value);
        let fv = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_field(access, name, descriptor, signature, value));
        Some(Box::new(TraceFieldVisitor::with_delegate(
            fv,
            field_printer,
        )))
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let method_printer = self
            .p
            .visit_method(access, name, descriptor, signature, exceptions);
        let mv = self
            .cv
            .as_mut()
            .and_then(|cv| cv.visit_method(access, name, descriptor, signature, exceptions));
        Some(Box::new(TraceMethodVisitor::with_delegate(
            mv,
            method_printer,
        )))
    }

    fn visit_end(&mut self) {
        self.p.visit_class_end();
        if let Some(print_writer) = &mut self.print_writer {
            let result = self
                .p
                .print(print_writer)
                .and_then(|()| print_writer.flush());
            if let Err(e) = result
                && self.io_error.is_none()
            {
                self.io_error = Some(e);
            }
        }
        if let Some(cv) = &mut self.cv {
            cv.visit_end();
        }
    }
}
