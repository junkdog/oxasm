//! Port of ASM's `ClassReaderTest`.
//!
//! The Java tests read their own class files and the JDK's through the class loader; these use
//! the T0 corpus instead. The `Api` dimension is gone (ADR-09), so every class is readable by
//! every visitor.

use std::cell::Cell;

use oxasm::error::ReadErrorKind;
use oxasm::opcodes::{ACC_ABSTRACT, ACC_SUPER, V_PREVIEW, V1_1};
use oxasm::{
    AnnotationVisitor, ClassReader, ClassVisitor, FieldVisitor, JavaStr, MethodVisitor,
    ModuleVisitor, ParsingOptions, RecordComponentVisitor, TypePath, TypeReference,
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
