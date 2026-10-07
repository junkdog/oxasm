//! Port of ASM's `AnnotationVisitorTest`.
//!
//! The `Api` dimension is gone (ADR-09): `testConstructor_invalidApi` is dropped, and every class
//! can be visited by every adapter.

use std::ptr;

use oxasm::{
    AnnotationVisitor, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, Label,
    MethodVisitor, ParsingOptions, TypePath, TypeReference, jstr,
};
use oxasm_testkit::{ClassFile, PrecompiledClass};

/// An annotation visitor with no delegate.
struct NoDelegate;

impl AnnotationVisitor for NoDelegate {}

/// An annotation visitor that delegates to another.
struct Delegating<'a>(&'a mut dyn AnnotationVisitor);

impl AnnotationVisitor for Delegating<'_> {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut *self.0)
    }
}

#[test]
fn test_constructor_valid_api() {
    // A visitor without a delegate accepts every event, and is not interested in any content.
    let mut visitor = NoDelegate;
    assert!(visitor.delegate().is_none());
    visitor.visit_enum(Some(jstr!("e")), jstr!("LE;"), jstr!("V"));
    assert!(visitor.visit_annotation(None, jstr!("LA;")).is_none());
    assert!(visitor.visit_array(None).is_none());
    visitor.visit_end();
}

// testConstructor_invalidApi: dropped (ADR-09, there are no api versions).

#[test]
fn test_get_delegate() {
    let mut delegate = NoDelegate;
    let delegate_address = ptr::from_mut(&mut delegate).cast::<()>();
    let mut visitor = Delegating(&mut delegate);
    let address = visitor
        .delegate()
        .map(|d| ptr::from_mut(d).cast::<()>())
        .expect("a delegate");
    assert_eq!(address, delegate_address);
}

/// Tests that ClassReader accepts visitor which return `None` annotation visitors, and that
/// returning `None` is equivalent to returning an `EmptyAnnotationVisitor`.
#[test]
fn test_read_and_write_remove_or_delete_annotations() {
    for &class in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class.bytes()).expect("a valid class");
        let mut removed_annotations_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        let mut deleted_annotations_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        class_reader
            .accept(
                &mut AnnotationsAdapter(&mut removed_annotations_class_writer, Mode::Remove),
                ParsingOptions::empty(),
            )
            .expect("a valid class");
        class_reader
            .accept(
                &mut AnnotationsAdapter(&mut deleted_annotations_class_writer, Mode::Delete),
                ParsingOptions::empty(),
            )
            .expect("a valid class");
        let dump = |class_writer: &mut ClassWriter<'_>| {
            let class_file = class_writer.to_bytes().expect("a written class");
            ClassFile::new(&class_file)
                .dump_lossy()
                .expect("a dumpable class")
        };
        assert_eq!(
            dump(&mut removed_annotations_class_writer),
            dump(&mut deleted_annotations_class_writer),
            "{class}"
        );
    }
}

/// An annotation visitor which visits nested annotations and arrays with itself, and ignores
/// everything (ASM's `EmptyAnnotationVisitor`).
struct EmptyAnnotationVisitor;

impl AnnotationVisitor for EmptyAnnotationVisitor {
    fn visit_annotation<'a>(
        &'a mut self,
        _name: Option<&JavaStr>,
        _descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(self))
    }

    fn visit_array<'a>(
        &'a mut self,
        _name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(self))
    }
}

/// How an [`AnnotationsAdapter`] drops annotations.
#[derive(Clone, Copy)]
enum Mode {
    /// Visits them with an `EmptyAnnotationVisitor` (ASM's `RemoveAnnotationsAdapter`).
    Remove,
    /// Returns `None` (ASM's `DeleteAnnotationsAdapter`).
    Delete,
}

impl Mode {
    fn annotation_visitor<'a>(self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        match self {
            Mode::Remove => Some(Box::new(EmptyAnnotationVisitor)),
            Mode::Delete => None,
        }
    }
}

/// Drops the annotations of the given methods.
macro_rules! drop_annotations {
    ($($method:ident($($ty:ty),* $(,)?)),* $(,)?) => {$(
        fn $method<'b>(&'b mut self, $(_: $ty),*) -> Option<Box<dyn AnnotationVisitor + 'b>> {
            self.1.annotation_visitor()
        }
    )*};
}

/// A class adapter which drops all the class and method annotations (ASM's
/// `RemoveAnnotationsAdapter` and `DeleteAnnotationsAdapter`).
struct AnnotationsAdapter<'w>(&'w mut ClassWriter<'static>, Mode);

impl ClassVisitor for AnnotationsAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    drop_annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
    );

    fn visit_method<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'b>> {
        let method_visitor = self
            .0
            .visit_method(access, name, descriptor, signature, exceptions);
        Some(Box::new(MethodAnnotationsAdapter(method_visitor, self.1)))
    }
}

struct MethodAnnotationsAdapter<'a>(Option<Box<dyn MethodVisitor + 'a>>, Mode);

impl MethodVisitor for MethodAnnotationsAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        self.0.as_deref_mut().map(|v| v as &mut dyn MethodVisitor)
    }

    drop_annotations!(
        visit_annotation_default(),
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_parameter_annotation(u8, &JavaStr, bool),
        visit_insn_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_try_catch_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_local_variable_annotation(
            TypeReference,
            Option<&TypePath>,
            &[Label],
            &[Label],
            &[u16],
            &JavaStr,
            bool,
        ),
    );
}
