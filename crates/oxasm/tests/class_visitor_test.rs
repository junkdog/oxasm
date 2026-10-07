//! Port of ASM's `ClassVisitorTest` (which also tests the field, method, module and annotation
//! visitors through class adapters).
//!
//! The `Api` dimension is gone (ADR-09): every class can be visited by every adapter, and
//! `testConstructor_invalidApi` is dropped. Java's anonymous `ClassVisitor` subclasses are small
//! structs that delegate (see the `visitor` module docs). ASM's `ClassFile` equality is the
//! equality of the `ClassFile` dumps (`oxasm_testkit::ClassFile`).

use std::ptr;

use oxasm::error::ReadError;
use oxasm::opcodes::{
    ACC_ABSTRACT, ACC_DEPRECATED, ACC_PUBLIC, ACC_SYNTHETIC, ALOAD, INVOKESPECIAL, RETURN, V1_4,
    V1_5, V1_7,
};
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{
    AnnotationVisitor, AttributeCodec, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags,
    Constant, CustomAttribute, FieldVisitor, JavaStr, Label, MethodVisitor, ModuleVisitor,
    ParsingOptions, Type, TypePath, TypeReference, jstr,
};
use oxasm_testkit::{ClassFile, PrecompiledClass};

/// Asserts that two class files have the same `ClassFile` dump (ASM's `assertEquals(new
/// ClassFile(a), new ClassFile(b))`).
#[track_caller]
fn assert_same_class(expected: &[u8], actual: &[u8]) {
    let dump = |bytes: &[u8]| {
        ClassFile::new(bytes)
            .dump_lossy()
            .expect("a dumpable class")
    };
    assert_eq!(dump(expected), dump(actual));
}

/// `classReader.accept(classVisitor, attributes(), 0)`.
fn accept(class_reader: &ClassReader<'_>, class_visitor: &mut dyn ClassVisitor) {
    class_reader
        .accept_with_codecs(class_visitor, &attributes(), ParsingOptions::empty())
        .expect("a valid class");
}

/// ASM's `attributes()`: the `Comment` and `CodeComment` test attribute prototypes.
fn attributes() -> [&'static dyn AttributeCodec; 2] {
    [&Comment, &CodeComment]
}

/// ASM's `Comment` test attribute: a class attribute with no content.
#[derive(Clone, Debug)]
struct Comment;

impl CustomAttribute for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
}

impl AttributeCodec for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(Comment))
    }
}

/// ASM's `CodeComment` test attribute: a Code attribute with no content.
#[derive(Clone, Debug)]
struct CodeComment;

impl CustomAttribute for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
}

impl AttributeCodec for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(CodeComment))
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

/// A class visitor with no delegate.
struct NoDelegate;

impl ClassVisitor for NoDelegate {}

/// A class visitor that delegates to another.
struct Delegating<'a>(&'a mut dyn ClassVisitor);

impl ClassVisitor for Delegating<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }
}

#[test]
fn test_constructor_valid_api() {
    // A visitor without a delegate accepts every event, and is not interested in any content.
    let mut visitor = NoDelegate;
    assert!(visitor.delegate().is_none());
    visitor.visit(V1_7, ACC_PUBLIC, jstr!("C"), None, None, &[]);
    assert!(visitor.visit_annotation(jstr!("LA;"), true).is_none());
    assert!(
        visitor
            .visit_method(0, jstr!("m"), jstr!("()V"), None, &[])
            .is_none()
    );
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

/// Tests that classes are unchanged when transformed with a ClassReader -> class adapter ->
/// ClassWriter chain, where "class adapter" is a ClassVisitor which returns FieldVisitor,
/// MethodVisitor, ModuleVisitor and AnnotationVisitor instances.
#[test]
fn test_read_and_write_empty_visitor() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        let class_reader = ClassReader::new(class_file).expect("a valid class");
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        accept(&class_reader, &mut ClassAdapter(&mut class_writer));
        let output = class_writer.to_bytes().expect("a written class");
        assert_same_class(class_file, &output);
    }
}

/// Writes `class_file` with an adapter built by `adapter`, without and with the copy pool
/// option, and returns the two classes.
fn write_with_and_without_copy_pool(
    class_file: &[u8],
    flags: ClassWriterFlags,
    mut adapter: impl FnMut(&mut ClassWriter<'static>, &ClassReader<'_>),
) -> (Vec<u8>, Vec<u8>) {
    let class_reader = ClassReader::new(class_file).expect("a valid class");
    let mut class_writer = ClassWriter::new(flags);
    let mut class_writer_with_copy_pool =
        ClassWriter::from_reader(&class_reader, flags).expect("a valid constant pool");
    adapter(&mut class_writer, &class_reader);
    adapter(&mut class_writer_with_copy_pool, &class_reader);
    (
        class_writer.to_bytes().expect("a written class"),
        class_writer_with_copy_pool
            .to_bytes()
            .expect("a written class"),
    )
}

/// Tests that a ClassReader -> class adapter -> ClassWriter chain gives the same result with or
/// without the copy pool option, and a class adapter that changes the method exceptions.
#[test]
fn test_read_and_write_copy_pool_change_method_exceptions() {
    for &class in PrecompiledClass::ALL {
        let (output, output_with_copy_pool) =
            write_with_and_without_copy_pool(class.bytes(), ClassWriterFlags::empty(), |w, r| {
                accept(r, &mut ChangeExceptionAdapter(w));
            });
        assert_same_class(&output, &output_with_copy_pool);
    }
}

/// Tests that a ClassReader -> class adapter -> ClassWriter chain gives the same result with or
/// without the copy pool option, and a class adapter that changes the deprecated method flags.
#[test]
fn test_read_and_write_copy_pool_change_method_deprecated_flag() {
    for &class in PrecompiledClass::ALL {
        let (output, output_with_copy_pool) =
            write_with_and_without_copy_pool(class.bytes(), ClassWriterFlags::empty(), |w, r| {
                accept(r, &mut ChangeAccessAdapter(w, ACC_DEPRECATED));
            });
        assert_same_class(&output, &output_with_copy_pool);
    }
}

/// Tests that a ClassReader -> class adapter -> ClassWriter chain gives the same result with or
/// without the copy pool option, and a class adapter that changes the synthetic method flags.
#[test]
fn test_read_and_write_copy_pool_change_method_synthetic_flag() {
    for &class in PrecompiledClass::ALL {
        let (output, output_with_copy_pool) =
            write_with_and_without_copy_pool(class.bytes(), ClassWriterFlags::empty(), |w, r| {
                accept(r, &mut ChangeAccessAdapter(w, ACC_SYNTHETIC));
            });
        assert_same_class(&output, &output_with_copy_pool);
    }
}

/// Tests that a ClassReader -> class adapter -> ClassWriter chain gives the same result with or
/// without the copy pool option, and a class adapter that changes the class version (and
/// optionally the method synthetic flags / attributes as well).
#[test]
fn test_read_and_write_copy_pool_change_class_version_and_method_synthetic_flag() {
    for (upgrade_version, change_synthetic) in
        [(true, true), (true, false), (false, true), (false, false)]
    {
        let mut source_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        source_class_writer.visit(
            if upgrade_version { V1_4 } else { V1_5 },
            ACC_ABSTRACT,
            jstr!("C"),
            None,
            Some(jstr!("java/lang/Object")),
            &[],
        );
        source_class_writer
            .visit_method(
                ACC_ABSTRACT | ACC_SYNTHETIC,
                jstr!("m"),
                jstr!("()V"),
                None,
                &[],
            )
            .expect("a method writer")
            .visit_end();
        source_class_writer.visit_end();
        let source = source_class_writer.to_bytes().expect("a written class");
        let version = if upgrade_version { V1_5 } else { V1_4 };
        let access = if change_synthetic { ACC_SYNTHETIC } else { 0 };
        let (output, output_with_copy_pool) =
            write_with_and_without_copy_pool(&source, ClassWriterFlags::empty(), |w, r| {
                r.accept(
                    &mut ChangeVersionAdapter(ChangeAccessAdapter(w, access), version),
                    ParsingOptions::empty(),
                )
                .expect("a valid class");
            });
        assert_same_class(&output, &output_with_copy_pool);
    }
}

/// Tests that a ClassReader -> class adapter -> ClassWriter chain gives the same result with or
/// without the copy pool option, and a class adapter that changes the method descriptors.
#[test]
fn test_read_and_write_copy_pool_change_method_descriptor() {
    let mut source_class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
    source_class_writer.visit(
        V1_7,
        ACC_ABSTRACT,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = source_class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .expect("a method writer");
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ALOAD, 0);
        method_visitor.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    source_class_writer.visit_end();
    let source = source_class_writer.to_bytes().expect("a written class");
    let (output, output_with_copy_pool) =
        write_with_and_without_copy_pool(&source, ClassWriterFlags::COMPUTE_MAXS, |w, r| {
            r.accept(&mut AddParameterAdapter(w), ParsingOptions::empty())
                .expect("a valid class");
        });
    assert_same_class(&output, &output_with_copy_pool);
}

/// Test that classes with only visible or only invisible annotations can be read correctly.
#[test]
fn test_read_and_write_remove_annotations() {
    for visibility_value in [true, false] {
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        ClassReader::new(PrecompiledClass::Jdk8AllStructures.bytes())
            .expect("a valid class")
            .accept(
                &mut RemoveAnnotationAdapter(&mut class_writer, visibility_value),
                ParsingOptions::empty(),
            )
            .expect("a valid class");
        let class_file = class_writer.to_bytes().expect("a written class");
        let mut new_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        ClassReader::new(&class_file)
            .expect("a valid class")
            .accept(
                &mut RemoveAnnotationAdapter(&mut new_class_writer, visibility_value),
                ParsingOptions::empty(),
            )
            .expect("a valid class");
        assert_same_class(
            &class_file,
            &new_class_writer.to_bytes().expect("a written class"),
        );
    }
}

/// Tests that optional module data (ModulePackage, ModuleMainClass, etc) can be removed with a
/// ClassReader -> class adapter -> ClassWriter chain.
#[test]
fn test_read_and_write_remove_optional_module_data() {
    struct RemoveModuleDataAdapter<'a>(&'a mut ClassWriter<'static>);

    impl ClassVisitor for RemoveModuleDataAdapter<'_> {
        fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
            Some(&mut *self.0)
        }

        fn visit_module<'a>(
            &'a mut self,
            name: &JavaStr,
            access: u32,
            version: Option<&JavaStr>,
        ) -> Option<Box<dyn ModuleVisitor + 'a>> {
            let module_visitor = self.0.visit_module(name, access, version)?;
            Some(Box::new(RemoveModuleData(module_visitor)))
        }
    }

    struct RemoveModuleData<'a>(Box<dyn ModuleVisitor + 'a>);

    impl ModuleVisitor for RemoveModuleData<'_> {
        fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
            Some(&mut *self.0)
        }
        fn visit_main_class(&mut self, _main_class: &JavaStr) {
            // Remove main class.
        }
        fn visit_package(&mut self, _packaze: &JavaStr) {
            // Remove package.
        }
        fn visit_require(&mut self, module: &JavaStr, access: u32, _version: Option<&JavaStr>) {
            self.0.visit_require(module, access, None);
        }
        fn visit_export(&mut self, packaze: &JavaStr, access: u32, _modules: &[&JavaStr]) {
            self.0.visit_export(packaze, access, &[]);
        }
        fn visit_open(&mut self, packaze: &JavaStr, access: u32, _modules: &[&JavaStr]) {
            self.0.visit_open(packaze, access, &[]);
        }
    }

    let class_reader =
        ClassReader::new(PrecompiledClass::Jdk9Module.bytes()).expect("a valid class");
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_reader
        .accept(
            &mut RemoveModuleDataAdapter(&mut class_writer),
            ParsingOptions::empty(),
        )
        .expect("a valid class");
    let class_dump = ClassFile::new(&class_writer.to_bytes().expect("a written class"))
        .dump_lossy()
        .expect("a dumpable class");
    assert!(!class_dump.contains("ModulePackage"));
    assert!(!class_dump.contains("ModuleMainClass"));
}

// -------------------------------------------------------------------------------------------------
// Adapters
// -------------------------------------------------------------------------------------------------

/// Forwards the given annotation visitor factory methods to the delegate, wrapping the returned
/// visitor in an `AnnotationAdapter`.
macro_rules! adapt_annotations {
    ($($method:ident($($arg:ident: $ty:ty),* $(,)?)),* $(,)?) => {$(
        fn $method<'b>(&'b mut self, $($arg: $ty),*) -> Option<Box<dyn AnnotationVisitor + 'b>> {
            let annotation_visitor = self.0.$method($($arg),*)?;
            Some(Box::new(AnnotationAdapter(annotation_visitor)))
        }
    )*};
}

struct AnnotationAdapter<'a>(Box<dyn AnnotationVisitor + 'a>);

impl AnnotationVisitor for AnnotationAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut *self.0)
    }

    adapt_annotations!(
        visit_annotation(name: Option<&JavaStr>, descriptor: &JavaStr),
        visit_array(name: Option<&JavaStr>),
    );
}

struct ClassAdapter<'w>(&'w mut ClassWriter<'static>);

impl ClassVisitor for ClassAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    adapt_annotations!(
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
    );

    fn visit_field<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'b>> {
        let field_visitor = self
            .0
            .visit_field(access, name, descriptor, signature, value)?;
        Some(Box::new(FieldAdapter(field_visitor)))
    }

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
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(MethodAdapter(method_visitor)))
    }

    fn visit_module<'b>(
        &'b mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'b>> {
        let module_visitor = self.0.visit_module(name, access, version)?;
        Some(Box::new(ModuleAdapter(module_visitor)))
    }
}

struct ModuleAdapter<'a>(Box<dyn ModuleVisitor + 'a>);

impl ModuleVisitor for ModuleAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        Some(&mut *self.0)
    }
}

struct FieldAdapter<'a>(Box<dyn FieldVisitor + 'a>);

impl FieldVisitor for FieldAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
        Some(&mut *self.0)
    }

    adapt_annotations!(
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
    );
}

struct MethodAdapter<'a>(Box<dyn MethodVisitor + 'a>);

impl MethodVisitor for MethodAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }

    adapt_annotations!(
        visit_annotation_default(),
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_parameter_annotation(parameter: u8, descriptor: &JavaStr, visible: bool),
        visit_insn_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_try_catch_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_local_variable_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            start: &[Label],
            end: &[Label],
            index: &[u16],
            descriptor: &JavaStr,
            visible: bool,
        ),
    );
}

/// Replaces the first exception of each method with `java/lang/Throwable`. It returns the
/// writer's method visitor itself, so the method copy optimization applies when possible.
struct ChangeExceptionAdapter<'w>(&'w mut ClassWriter<'static>);

impl ClassVisitor for ChangeExceptionAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'b>> {
        let mut exceptions = exceptions.to_vec();
        if let Some(first) = exceptions.first_mut() {
            *first = jstr!("java/lang/Throwable");
        }
        self.0
            .visit_method(access, name, descriptor, signature, &exceptions)
    }
}

/// Changes the class version.
struct ChangeVersionAdapter<V>(V, u32);

impl<V: ClassVisitor> ClassVisitor for ChangeVersionAdapter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }

    fn visit(
        &mut self,
        _version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.0
            .visit(self.1, access, name, signature, super_name, interfaces);
    }
}

/// Toggles the given method access flags. It returns the writer's method visitor itself, so the
/// method copy optimization applies when possible.
struct ChangeAccessAdapter<'w>(&'w mut ClassWriter<'static>, u32);

impl ClassVisitor for ChangeAccessAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'b>> {
        self.0
            .visit_method(access ^ self.1, name, descriptor, signature, exceptions)
    }
}

/// Removes either all visible or all invisible \[type\] annotations.
struct RemoveAnnotationAdapter<'w>(&'w mut ClassWriter<'static>, bool);

/// Returns `None` for the annotations of the removed visibility, and forwards the others.
macro_rules! remove_annotations {
    ($visible:ident; $($method:ident($($arg:ident: $ty:ty),* $(,)?)),* $(,)?) => {$(
        fn $method<'b>(&'b mut self, $($arg: $ty),*) -> Option<Box<dyn AnnotationVisitor + 'b>> {
            if $visible == self.1 {
                return None;
            }
            self.0.$method($($arg),*)
        }
    )*};
}

impl ClassVisitor for RemoveAnnotationAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    remove_annotations!(
        visible;
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
    );

    fn visit_field<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'b>> {
        let field_visitor = self
            .0
            .visit_field(access, name, descriptor, signature, value)?;
        Some(Box::new(RemoveFieldAnnotations(field_visitor, self.1)))
    }

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
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(RemoveMethodAnnotations(method_visitor, self.1)))
    }
}

struct RemoveFieldAnnotations<'a>(Box<dyn FieldVisitor + 'a>, bool);

impl FieldVisitor for RemoveFieldAnnotations<'_> {
    fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
        Some(&mut *self.0)
    }

    remove_annotations!(
        visible;
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
    );
}

struct RemoveMethodAnnotations<'a>(Box<dyn MethodVisitor + 'a>, bool);

impl MethodVisitor for RemoveMethodAnnotations<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }

    remove_annotations!(
        visible;
        visit_annotation(descriptor: &JavaStr, visible: bool),
        visit_type_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_parameter_annotation(parameter: u8, descriptor: &JavaStr, visible: bool),
        visit_insn_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_try_catch_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            descriptor: &JavaStr,
            visible: bool,
        ),
        visit_local_variable_annotation(
            type_ref: TypeReference,
            type_path: Option<&TypePath>,
            start: &[Label],
            end: &[Label],
            index: &[u16],
            descriptor: &JavaStr,
            visible: bool,
        ),
    );
}

/// Adds an `int` parameter to the declared method descriptors.
struct AddParameterAdapter<'w>(&'w mut ClassWriter<'static>);

impl ClassVisitor for AddParameterAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method<'b>(
        &'b mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'b>> {
        let mut argument_types = Type::get_argument_types_of(descriptor);
        argument_types.push(Type::INT);
        let return_type = Type::get_return_type_of(descriptor);
        let descriptor = Type::get_method_descriptor(&return_type, &argument_types);
        self.0
            .visit_method(access, name, &descriptor, signature, exceptions)
    }
}
