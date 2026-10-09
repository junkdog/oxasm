//! Port of ASM's `ClassReaderTest`.
//!
//! The Java tests read their own class files and the JDK's through the class loader; these use
//! the T0 corpus instead. The `Api` dimension is gone (ADR-09), so every class is readable by
//! every visitor.

use std::cell::{Cell, RefCell};

use oxasm::error::ReadErrorKind;
use oxasm::opcodes::{
    ACC_ABSTRACT, ACC_PUBLIC, ACC_STATIC, ACC_SUPER, GOTO, NOP, RETURN, V_PREVIEW, V1_1,
};
use oxasm::{
    AnnotationVisitor, Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags,
    FieldVisitor, JavaStr, Label, MethodVisitor, ModuleVisitor, ParsingOptions,
    RecordComponentVisitor, TypePath, TypeReference, UnknownAttribute, jstr,
};
use oxasm_testkit::{InvalidClass, PrecompiledClass};

/// An annotation visitor that visits nested annotations and arrays with itself (ASM's
/// `EmptyClassVisitor.annotationVisitor`).
struct EmptyAnnotationVisitor;

impl AnnotationVisitor for EmptyAnnotationVisitor {
    fn visit_annotation<'a>(
        &'a mut self,
        _: Option<&JavaStr>,
        _: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(self))
    }

    fn visit_array<'a>(
        &'a mut self,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(self))
    }
}

/// Visits everything, and does nothing (ASM's `EmptyClassVisitor`).
#[derive(Default)]
struct EmptyClassVisitor {
    annotation_visitor: EmptyAnnotationVisitor,
}

impl Default for EmptyAnnotationVisitor {
    fn default() -> Self {
        EmptyAnnotationVisitor
    }
}

impl ClassVisitor for EmptyClassVisitor {
    fn visit_annotation<'a>(
        &'a mut self,
        _: &JavaStr,
        _: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(&mut self.annotation_visitor))
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _: TypeReference,
        _: Option<&TypePath>,
        _: &JavaStr,
        _: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(&mut self.annotation_visitor))
    }

    fn visit_module<'a>(
        &'a mut self,
        _: &JavaStr,
        _: u32,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        Some(Box::new(EmptyModuleVisitor))
    }

    fn visit_record_component<'a>(
        &'a mut self,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        Some(Box::new(EmptyMemberVisitor))
    }

    fn visit_field<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&oxasm::Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        Some(Box::new(EmptyMemberVisitor))
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(EmptyMemberVisitor))
    }
}

struct EmptyModuleVisitor;

impl ModuleVisitor for EmptyModuleVisitor {}

/// The field, method and record component visitors of `EmptyClassVisitor`: they visit every
/// annotation with an `EmptyAnnotationVisitor`.
struct EmptyMemberVisitor;

macro_rules! empty_annotations {
    ($($method:ident($($arg:ty),* $(,)?)),* $(,)?) => {$(
        fn $method<'a>(&'a mut self, $(_: $arg),*) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(EmptyAnnotationVisitor))
        }
    )*};
}

impl FieldVisitor for EmptyMemberVisitor {
    empty_annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool)
    );
}

impl RecordComponentVisitor for EmptyMemberVisitor {
    empty_annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool)
    );
}

impl MethodVisitor for EmptyMemberVisitor {
    empty_annotations!(
        visit_annotation_default(),
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_parameter_annotation(u8, &JavaStr, bool),
        visit_insn_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_try_catch_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_local_variable_annotation(
            TypeReference,
            Option<&TypePath>,
            &[oxasm::Label],
            &[oxasm::Label],
            &[u16],
            &JavaStr,
            bool,
        )
    );
}

fn reader(class: PrecompiledClass) -> ClassReader<'static> {
    ClassReader::new(class.bytes()).unwrap_or_else(|e| panic!("{class}: {e}"))
}

#[test]
fn test_read_byte() {
    let bytes = PrecompiledClass::Jdk3AllInstructions.bytes();
    let class_reader = reader(PrecompiledClass::Jdk3AllInstructions);

    assert_eq!(class_reader.read_byte(0).unwrap(), bytes[0]);
}

#[test]
fn test_get_item() {
    let class_reader = reader(PrecompiledClass::Jdk3AllInstructions);

    let item = class_reader.get_item(1).unwrap();

    assert!(item >= 10);
    assert!(item < class_reader.header());
}

#[test]
fn test_get_access() {
    assert_eq!(
        reader(PrecompiledClass::Jdk3AllInstructions)
            .get_access()
            .unwrap(),
        ACC_SUPER
    );
    assert_eq!(
        reader(PrecompiledClass::Jdk3AllStructures)
            .get_access()
            .unwrap(),
        ACC_ABSTRACT | ACC_SUPER
    );
}

#[test]
fn test_get_class_name() {
    assert_eq!(
        reader(PrecompiledClass::Jdk3AllInstructions)
            .get_class_name()
            .unwrap(),
        "jdk3/AllInstructions"
    );
}

#[test]
fn test_get_super_name() {
    assert_eq!(
        reader(PrecompiledClass::Jdk3AllInstructions)
            .get_super_name()
            .unwrap(),
        Some(JavaStr::from_str("java/lang/Object"))
    );
    // java/lang/Object has no super class: set super_class to 0 in a copy of a class.
    let mut bytes = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    let header = reader(PrecompiledClass::Jdk3AllInstructions).header();
    bytes[header + 4] = 0;
    bytes[header + 5] = 0;
    assert_eq!(
        ClassReader::new(&bytes).unwrap().get_super_name().unwrap(),
        None
    );
}

#[test]
fn test_get_interfaces() {
    let class_reader = reader(PrecompiledClass::Jdk3AnonymousInnerClass);

    let interfaces = class_reader.get_interfaces().unwrap();

    assert_eq!(interfaces, ["java/lang/Runnable"]);
}

#[test]
fn test_get_interfaces_empty() {
    let class_reader = reader(PrecompiledClass::Jdk3AllInstructions);

    let interfaces = class_reader.get_interfaces().unwrap();

    assert!(interfaces.is_empty());
}

#[test]
fn test_byte_array_constructor() {
    for &class in PrecompiledClass::ALL {
        let class_reader = reader(class);

        assert_ne!(class_reader.get_access().unwrap(), 0, "{class}");
        assert_eq!(
            class_reader.get_class_name().unwrap(),
            class.internal_name().as_str()
        );
        let super_name = class_reader.get_super_name().unwrap();
        if class.internal_name() == "module-info" {
            assert_eq!(super_name, None);
        } else {
            assert!(super_name.unwrap().starts_with("java"), "{class}");
        }
        class_reader.get_interfaces().unwrap();
    }
}

/// `new ClassReader(byteBuffer, 1, classFile.length)`: a reader over part of a buffer is a
/// reader over a sub-slice.
#[test]
fn test_byte_array_constructor_with_offset() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        let mut byte_buffer = vec![0; class_file.len() + 1];
        byte_buffer[1..].copy_from_slice(class_file);

        let class_reader = ClassReader::new(&byte_buffer[1..]).unwrap();

        assert_ne!(class_reader.get_access().unwrap(), 0);
        assert_eq!(
            class_reader.get_class_name().unwrap(),
            class.internal_name().as_str()
        );
        let super_name = class_reader.get_super_name().unwrap();
        if class.internal_name() == "module-info" {
            assert_eq!(super_name, None);
        } else {
            assert!(super_name.unwrap().starts_with("java"));
        }
        class_reader.get_interfaces().unwrap();
        struct VersionVisitor(u32);
        impl ClassVisitor for VersionVisitor {
            fn visit(
                &mut self,
                version: u32,
                _: u32,
                _: &JavaStr,
                _: Option<&JavaStr>,
                _: Option<&JavaStr>,
                _: &[&JavaStr],
            ) {
                self.0 = version;
            }
        }
        let mut visitor = VersionVisitor(0);
        class_reader
            .accept(&mut visitor, ParsingOptions::empty())
            .unwrap();
        assert!((visitor.0 & 0xFFFF) >= (V1_1 & 0xFFFF));
    }
}

#[test]
fn test_byte_array_constructor_invalid_class_header() {
    let error = ClassReader::new(InvalidClass::InvalidClassVersion.bytes()).unwrap_err();
    assert_eq!(error.kind, ReadErrorKind::UnsupportedVersion(i16::MAX));

    let error = ClassReader::new(InvalidClass::InvalidCpInfoTag.bytes()).unwrap_err();
    assert_eq!(error.kind, ReadErrorKind::InvalidConstantPoolTag(255));
}

// testStringConstructor, testStreamConstructor and testStreamConstructor_nullStream are dropped:
// the class loader and InputStream constructors are not ported (a reader reads a byte slice).

/// ASM's `testStreamConstructor_emptyStream`: an empty class file.
#[test]
fn test_stream_constructor_empty_stream() {
    let error = ClassReader::new(&[]).unwrap_err();

    assert_eq!(error.kind, ReadErrorKind::UnexpectedEnd);
}

fn accept_empty_visitor(options: ParsingOptions) {
    for &class in PrecompiledClass::ALL {
        let class_reader = reader(class);
        let mut class_visitor = EmptyClassVisitor::default();

        let result = class_reader.accept(&mut class_visitor, options);

        assert_eq!(result, Ok(()), "{class}");
    }
}

#[test]
fn test_accept_empty_visitor() {
    accept_empty_visitor(ParsingOptions::empty());
}

#[test]
fn test_accept_empty_visitor_skip_debug() {
    accept_empty_visitor(ParsingOptions::SKIP_DEBUG);
}

#[test]
fn test_accept_empty_visitor_expand_frames() {
    accept_empty_visitor(ParsingOptions::EXPAND_FRAMES);
}

#[test]
fn test_accept_empty_visitor_skip_frames() {
    accept_empty_visitor(ParsingOptions::SKIP_FRAMES);
}

#[test]
fn test_accept_empty_visitor_skip_code() {
    accept_empty_visitor(ParsingOptions::SKIP_CODE);
}

#[test]
fn test_accept_empty_visitor_skip_field_method_and_module_content() {
    /// Visits the class, without its modules, record components, fields and methods.
    struct SkipContent;
    impl ClassVisitor for SkipContent {}

    for &class in PrecompiledClass::ALL {
        let class_reader = reader(class);

        let result = class_reader.accept(&mut SkipContent, ParsingOptions::empty());

        assert_eq!(result, Ok(()), "{class}");
    }
}

#[test]
fn test_accept_empty_visitor_invalid_class() {
    for &invalid_class in InvalidClass::ALL {
        if matches!(
            invalid_class,
            InvalidClass::InvalidClassVersion | InvalidClass::InvalidCpInfoTag
        ) {
            continue;
        }
        let class_reader = ClassReader::new(invalid_class.bytes()).unwrap();

        let result =
            class_reader.accept(&mut EmptyClassVisitor::default(), ParsingOptions::empty());

        // ASM throws an ArrayIndexOutOfBoundsException for these three (with an index > 0 in
        // its message), and an IllegalArgumentException for the others.
        let Err(error) = result else {
            panic!("{invalid_class}: accepted");
        };
        let expected = match invalid_class {
            InvalidClass::InvalidConstantPoolIndex => {
                ReadErrorKind::InvalidConstantPoolIndex(65535)
            }
            InvalidClass::InvalidConstantPoolReference => {
                ReadErrorKind::InvalidConstantPoolIndex(15465)
            }
            InvalidClass::InvalidBytecodeOffset => ReadErrorKind::InvalidBytecodeOffset(32773),
            InvalidClass::InvalidCodeLength => ReadErrorKind::InvalidLength(0x7FFF_FFFF),
            InvalidClass::InvalidElementValue => ReadErrorKind::InvalidElementValueTag(b' '),
            InvalidClass::InvalidInsnTypeAnnotationTargetType
            | InvalidClass::InvalidTypeAnnotationTargetType => {
                ReadErrorKind::InvalidTypeAnnotationTarget(255)
            }
            InvalidClass::InvalidOpcode => ReadErrorKind::InvalidOpcode(255),
            InvalidClass::InvalidSourceDebugExtension => ReadErrorKind::InvalidLength(0x7FFF_FFFF),
            InvalidClass::InvalidStackMapFrameType => ReadErrorKind::InvalidFrameType(128),
            InvalidClass::InvalidVerificationTypeInfo => {
                ReadErrorKind::InvalidVerificationType(255)
            }
            InvalidClass::InvalidWideOpcode => ReadErrorKind::InvalidWideOpcode(255),
            InvalidClass::InvalidClassVersion | InvalidClass::InvalidCpInfoTag => unreachable!(),
        };
        assert_eq!(error.kind, expected, "{invalid_class}");
    }
}

#[test]
fn test_accept_default_visitor() {
    /// A class visitor that overrides nothing, and has no delegate.
    struct DefaultVisitor;
    impl ClassVisitor for DefaultVisitor {}

    for &class in PrecompiledClass::ALL {
        let class_reader = reader(class);

        let result = class_reader.accept(&mut DefaultVisitor, ParsingOptions::empty());

        assert_eq!(result, Ok(()), "{class}");
    }
}

#[test]
fn test_accept_default_annotation_field_method_and_module_visitors() {
    /// Returns default (no delegate, no override) visitors for every child.
    struct DefaultChildren;
    struct Default;
    impl AnnotationVisitor for Default {}
    impl ModuleVisitor for Default {}
    impl FieldVisitor for Default {}
    impl MethodVisitor for Default {}
    impl RecordComponentVisitor for Default {
        fn visit_annotation<'a>(
            &'a mut self,
            _: &JavaStr,
            _: bool,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_type_annotation<'a>(
            &'a mut self,
            _: TypeReference,
            _: Option<&TypePath>,
            _: &JavaStr,
            _: bool,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(Default))
        }
    }
    impl ClassVisitor for DefaultChildren {
        fn visit_annotation<'a>(
            &'a mut self,
            _: &JavaStr,
            _: bool,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_type_annotation<'a>(
            &'a mut self,
            _: TypeReference,
            _: Option<&TypePath>,
            _: &JavaStr,
            _: bool,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_module<'a>(
            &'a mut self,
            _: &JavaStr,
            _: u32,
            _: Option<&JavaStr>,
        ) -> Option<Box<dyn ModuleVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_record_component<'a>(
            &'a mut self,
            _: &JavaStr,
            _: &JavaStr,
            _: Option<&JavaStr>,
        ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_field<'a>(
            &'a mut self,
            _: u32,
            _: &JavaStr,
            _: &JavaStr,
            _: Option<&JavaStr>,
            _: Option<&oxasm::Constant<'_>>,
        ) -> Option<Box<dyn FieldVisitor + 'a>> {
            Some(Box::new(Default))
        }
        fn visit_method<'a>(
            &'a mut self,
            _: u32,
            _: &JavaStr,
            _: &JavaStr,
            _: Option<&JavaStr>,
            _: &[&JavaStr],
        ) -> Option<Box<dyn MethodVisitor + 'a>> {
            Some(Box::new(Default))
        }
    }

    for &class in PrecompiledClass::ALL {
        let class_reader = reader(class);

        let result = class_reader.accept(&mut DefaultChildren, ParsingOptions::empty());

        assert_eq!(result, Ok(()), "{class}");
    }
}

#[test]
fn test_accept_parameter_annotation_indices() {
    let class_reader = reader(PrecompiledClass::Jdk5LocalClass);
    let parameter_index = Cell::new(-1);
    struct ReadParameterIndex<'c>(&'c Cell<i32>);
    impl ClassVisitor for ReadParameterIndex<'_> {
        fn visit_method<'a>(
            &'a mut self,
            _: u32,
            _: &JavaStr,
            _: &JavaStr,
            _: Option<&JavaStr>,
            _: &[&JavaStr],
        ) -> Option<Box<dyn MethodVisitor + 'a>> {
            Some(Box::new(ParameterIndex(self.0)))
        }
    }
    struct ParameterIndex<'c>(&'c Cell<i32>);
    impl MethodVisitor for ParameterIndex<'_> {
        fn visit_parameter_annotation<'a>(
            &'a mut self,
            parameter: u8,
            descriptor: &JavaStr,
            _: bool,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            if descriptor == "Ljava/lang/Deprecated;" {
                self.0.set(i32::from(parameter));
            }
            None
        }
    }

    class_reader
        .accept(
            &mut ReadParameterIndex(&parameter_index),
            ParsingOptions::empty(),
        )
        .unwrap();

    assert_eq!(parameter_index.get(), 0);
}

#[test]
fn test_accept_preview_class() {
    let mut class_file = PrecompiledClass::Jdk11AllInstructions.bytes().to_vec();
    // Set the minor version to 65535.
    class_file[4] = 0xFF;
    class_file[5] = 0xFF;
    let class_reader = ClassReader::new(&class_file).unwrap();
    struct ReadVersion(u32);
    impl ClassVisitor for ReadVersion {
        fn visit(
            &mut self,
            version: u32,
            _: u32,
            _: &JavaStr,
            _: Option<&JavaStr>,
            _: Option<&JavaStr>,
            _: &[&JavaStr],
        ) {
            self.0 = version;
        }
    }
    let mut visitor = ReadVersion(0);

    class_reader
        .accept(&mut visitor, ParsingOptions::empty())
        .unwrap();

    assert_eq!(visitor.0 & V_PREVIEW, V_PREVIEW);
}

#[test]
fn test_accept_invalid_custom_attribute() {
    // Base64 "IftdBAAAAAAAAgEAAAD/AAAAAAAAAAAAAAIBAAF/////".
    let input = [
        0x21, 0xfb, 0x5d, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0xff,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x7f,
        0xff, 0xff, 0xff,
    ];
    let class_reader = ClassReader::new(&input).unwrap();

    let result = class_reader.accept(
        &mut EmptyClassVisitor::default(),
        ParsingOptions::EXPAND_FRAMES,
    );

    assert!(result.is_err());
}

/// oxasm: a reader is shareable across threads (success criterion 7).
#[test]
fn test_class_reader_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ClassReader<'static>>();

    let class_reader = reader(PrecompiledClass::Jdk8AllStructures);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                class_reader
                    .accept(&mut EmptyClassVisitor::default(), ParsingOptions::empty())
                    .unwrap();
            });
        }
    });
}

/// Records the member counts hint, and the members visited.
#[derive(Default)]
struct MemberCounts {
    visited: bool,
    hint: Option<(u16, u16)>,
    fields: u16,
    methods: u16,
}

impl ClassVisitor for MemberCounts {
    fn visit(
        &mut self,
        _: u32,
        _: u32,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) {
        self.visited = true;
    }

    fn visit_member_counts(&mut self, fields_count: u16, methods_count: u16) {
        assert!(self.visited && self.hint.is_none(), "one hint, after visit");
        assert_eq!(
            (self.fields, self.methods),
            (0, 0),
            "a hint before the members"
        );
        self.hint = Some((fields_count, methods_count));
    }

    fn visit_field<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&oxasm::Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        self.fields += 1;
        None
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.methods += 1;
        None
    }
}

/// oxasm-specific: a reader visits the number of fields and methods once, after the header and
/// before the members (W12 in oxasm-optimization.md: a writer or a tree sizes its member storage
/// from it).
#[test]
fn test_accept_visits_the_member_counts() {
    for &class in PrecompiledClass::ALL {
        let mut member_counts = MemberCounts::default();

        reader(class)
            .accept(&mut member_counts, ParsingOptions::empty())
            .unwrap();

        let MemberCounts {
            hint,
            fields,
            methods,
            ..
        } = member_counts;
        assert_eq!(hint, Some((fields, methods)), "{class}");
    }
}

/// Records the labels, line numbers and instructions of the methods a reader visits.
#[derive(Default)]
struct LineNumberEvents(RefCell<Vec<String>>);

impl ClassVisitor for LineNumberEvents {
    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(&*self))
    }
}

impl MethodVisitor for &LineNumberEvents {
    fn visit_label(&mut self, _: Label) {
        self.0.borrow_mut().push("label".into());
    }

    fn visit_line_number(&mut self, line: u16, _: Label) {
        self.0.borrow_mut().push(format!("line {line}"));
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.0.borrow_mut().push(format!("insn {opcode}"));
    }
}

/// oxasm-specific: the line numbers of an offset are visited after its label, in the order of
/// the LineNumberTable, whatever the order of the offsets in it; one at the end of the code is
/// not visited, as in ASM (R3 in oxasm-optimization.md: the reader sorts the entries by offset).
#[test]
fn test_accept_visits_the_line_numbers_in_table_order() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        let [label0, label1, label2, end] = [(); 4].map(|()| Label::new());
        method_visitor.visit_code();
        method_visitor.visit_label(label0);
        method_visitor.visit_insn(NOP);
        method_visitor.visit_label(label1);
        method_visitor.visit_line_number(30, label1);
        method_visitor.visit_insn(NOP);
        method_visitor.visit_label(label2);
        method_visitor.visit_line_number(10, label0);
        method_visitor.visit_line_number(31, label1);
        method_visitor.visit_line_number(20, label2);
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_label(end);
        method_visitor.visit_line_number(99, end);
        method_visitor.visit_line_number(32, label1);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    let class_file = class_writer.to_bytes().unwrap();
    let mut events = LineNumberEvents::default();

    ClassReader::new(&class_file)
        .unwrap()
        .accept(&mut events, ParsingOptions::empty())
        .unwrap();

    assert_eq!(
        events.0.into_inner(),
        [
            "label", "line 10", "insn 0", "label", "line 30", "line 31", "line 32", "insn 0",
            "label", "line 20", "insn 177", "label",
        ]
    );
}

/// oxasm-specific: a code_length of -1 is ASM's `new Label[0]`, a method without instructions.
/// The code's first byte is then read as the exception table, whose first entry fails on
/// `labels[0]`, as in ASM and before R1 in oxasm-optimization.md (the `roundtrip` fuzz corpus
/// found that R1's code reader took -1 for a huge length).
#[test]
fn test_accept_code_length_minus_one() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    let mut class_file = class_writer.to_bytes().unwrap();
    // max_stack 0, max_locals 0, code_length 1, RETURN.
    let code = [0, 0, 0, 0, 0, 0, 0, 1, RETURN];
    let code_offset = class_file
        .windows(code.len())
        .position(|bytes| bytes == code)
        .unwrap();
    class_file[code_offset + 4..code_offset + 8].copy_from_slice(&(-1i32).to_be_bytes());

    let error = ClassReader::new(&class_file)
        .unwrap()
        .accept(
            &mut ClassWriter::new(ClassWriterFlags::empty()),
            ParsingOptions::empty(),
        )
        .unwrap_err();

    assert_eq!(error.kind, ReadErrorKind::InvalidBytecodeOffset(0));
}

/// A class `C` whose static methods `()V` are the given instructions, each a `NOP`, a `RETURN`, or
/// a label (`None`) with a line number on the instruction after it.
fn class_with_methods(methods: &[&[Option<u8>]]) -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    for (i, code) in methods.iter().enumerate() {
        let name = format!("m{i}");
        let mut method_visitor = class_writer
            .visit_method(
                ACC_STATIC,
                JavaStr::from_str(&name),
                jstr!("()V"),
                None,
                &[],
            )
            .unwrap();
        method_visitor.visit_code();
        for (line, insn) in (1..).zip(code.iter()) {
            match *insn {
                Some(opcode) => method_visitor.visit_insn(opcode),
                None => {
                    let label = Label::new();
                    method_visitor.visit_label(label);
                    method_visitor.visit_line_number(line, label);
                }
            }
        }
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    class_writer.to_bytes().unwrap()
}

/// oxasm-specific: a method sees none of the labels of the methods read before it, in its class
/// or in a class read before on the same thread (R2 in oxasm-optimization.md: the reader's label
/// table is kept per thread, and a method only empties the slots the previous one filled).
#[test]
fn test_accept_visits_no_label_of_a_previous_method() {
    let labelled: &[Option<u8>] = &[Some(NOP), Some(NOP), None, Some(NOP), Some(RETURN)];
    let unlabelled: &[Option<u8>] = &[Some(NOP), Some(NOP), Some(NOP), Some(RETURN)];
    let first = class_with_methods(&[labelled, unlabelled]);
    let second = class_with_methods(&[unlabelled]);
    let mut first_events = LineNumberEvents::default();
    let mut second_events = LineNumberEvents::default();

    ClassReader::new(&first)
        .unwrap()
        .accept(&mut first_events, ParsingOptions::empty())
        .unwrap();
    ClassReader::new(&second)
        .unwrap()
        .accept(&mut second_events, ParsingOptions::empty())
        .unwrap();

    assert_eq!(
        first_events.0.into_inner(),
        [
            "insn 0", "insn 0", "label", "line 3", "insn 0", "insn 177", // m0
            "insn 0", "insn 0", "insn 0", "insn 177", // m1
        ]
    );
    assert_eq!(
        second_events.0.into_inner(),
        ["insn 0", "insn 0", "insn 0", "insn 177"]
    );
}

/// oxasm-specific: a jump past the end of a method's code is invalid, even when an earlier
/// method's code was long enough (R2 in oxasm-optimization.md: the reader's label table keeps
/// the slots of the longest method so far).
#[test]
fn test_accept_jump_past_the_code_after_a_longer_method() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("long"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        for _ in 0..10 {
            method_visitor.visit_insn(NOP);
        }
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("short"), jstr!("()V"), None, &[])
            .unwrap();
        let start = Label::new();
        method_visitor.visit_code();
        method_visitor.visit_label(start);
        method_visitor.visit_insn(NOP);
        method_visitor.visit_jump_insn(GOTO, start);
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    let mut class_file = class_writer.to_bytes().unwrap();
    // NOP, GOTO -1, RETURN: make the GOTO jump to offset 8, past the code's 5 bytes.
    let code = [NOP, GOTO, 0xFF, 0xFF, RETURN];
    let code_offset = class_file
        .windows(code.len())
        .position(|bytes| bytes == code)
        .unwrap();
    class_file[code_offset + 2..code_offset + 4].copy_from_slice(&7i16.to_be_bytes());

    let error = ClassReader::new(&class_file)
        .unwrap()
        .accept(
            &mut ClassWriter::new(ClassWriterFlags::empty()),
            ParsingOptions::empty(),
        )
        .unwrap_err();

    assert_eq!(error.kind, ReadErrorKind::InvalidBytecodeOffset(8));
}

/// oxasm-specific: a type annotation of the class whose target has bytecode offsets (a
/// LOCAL_VARIABLE one, which a class can't have) finds no labels, as before the first Code
/// attribute of any class, even after a class with code was read on the same thread (ASM's
/// `null` label array: a `NullPointerException`). R2 in oxasm-optimization.md keeps the label
/// table per thread; the `read_noop` fuzz corpus found that its first version handed the previous
/// class's table over as it was.
#[test]
fn test_accept_class_type_annotation_with_bytecode_offsets() {
    let with_code = class_with_methods(&[&[Some(NOP), None, Some(RETURN)]]);
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    let desc = class_writer.new_utf8(jstr!("LA;"));
    // One annotation: a LOCAL_VARIABLE target with one range (start_pc 0, length 0, index 0),
    // an empty type path, the type and no element value pairs.
    let mut content = vec![
        0,
        1,
        TypeReference::LOCAL_VARIABLE,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    content.extend(desc.to_be_bytes());
    content.extend([0, 0]);
    class_writer.visit_attribute(&Attribute::Unknown(UnknownAttribute {
        name: "RuntimeInvisibleTypeAnnotations".into(),
        content,
    }));
    class_writer.visit_end();
    let annotated = class_writer.to_bytes().unwrap();
    let read = |class_file: &[u8]| {
        ClassReader::new(class_file).unwrap().accept(
            &mut ClassWriter::new(ClassWriterFlags::empty()),
            ParsingOptions::empty(),
        )
    };

    read(&with_code).unwrap();
    let error = read(&annotated).unwrap_err();

    assert_eq!(error.kind, ReadErrorKind::InvalidBytecodeOffset(0));
}
