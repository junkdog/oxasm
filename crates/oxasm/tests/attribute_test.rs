//! Port of ASM's `AttributeTest` (the read half; `testStaticWrite` and `testCachedContent` need
//! the class writer, P5), plus oxasm's tests of the read half of the codec API (ADR-10).

use std::cell::RefCell;

use oxasm::error::{ReadError, ReadErrorKind};
use oxasm::reader::AttributeReadContext;
use oxasm::{
    Attribute, AttributeCodec, ClassReader, ClassVisitor, CustomAttribute, JavaStr, JavaString,
    Label, MethodVisitor, ParsingOptions, UnknownAttribute, jstr,
};
use oxasm_testkit::PrecompiledClass;

fn unknown(name: &str) -> Attribute {
    Attribute::Unknown(UnknownAttribute {
        name: JavaString::from(name),
        content: Vec::new(),
    })
}

#[test]
fn test_is_unknown() {
    assert!(unknown("Comment").is_unknown());
}

#[test]
fn test_get_labels() {
    assert_eq!(unknown("Comment").labels(), Vec::<Label>::new());
}

// testStaticWrite and testCachedContent: P5 (they write attributes with a ClassWriter).

/// The `SourceFile` attribute, decoded by a codec instead of the reader (a reader handles the
/// standard attributes itself, so this one is renamed in a copy of a class file).
#[derive(Clone, Debug)]
struct Source {
    file: JavaString,
}

impl CustomAttribute for Source {
    fn name(&self) -> &JavaStr {
        jstr!("Source")
    }
}

struct SourceCodec;

impl AttributeCodec for SourceCodec {
    fn name(&self) -> &JavaStr {
        jstr!("Source")
    }

    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        assert_eq!(cx.length(), 2);
        assert_eq!(cx.code_attribute_offset(), None);
        let file = cx.reader().read_utf8(cx.offset())?.expect("a file name");
        Ok(Box::new(Source {
            file: file.to_owned(),
        }))
    }
}

/// A Code attribute holding a bytecode offset, decoded into a label.
#[derive(Clone, Debug)]
struct Mark(Label);

impl CustomAttribute for Mark {
    fn name(&self) -> &JavaStr {
        jstr!("Mark")
    }

    fn is_code_attribute(&self) -> bool {
        true
    }

    fn labels(&self) -> Vec<Label> {
        vec![self.0]
    }
}

struct MarkCodec;

impl AttributeCodec for MarkCodec {
    fn name(&self) -> &JavaStr {
        jstr!("Mark")
    }

    fn is_code_attribute(&self) -> bool {
        true
    }

    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        assert!(cx.code_attribute_offset().is_some());
        let bytecode_offset = cx.reader().read_unsigned_short(cx.offset())?;
        Ok(Box::new(Mark(cx.read_label(usize::from(bytecode_offset))?)))
    }
}

/// Renames the first constant pool `CONSTANT_Utf8` entry equal to `from` (of the same length).
fn rename_utf8(class_file: &mut [u8], from: &str, to: &str) {
    assert_eq!(from.len(), to.len());
    let mut entry = Vec::new();
    entry.push(1);
    entry.extend_from_slice(&u16::try_from(from.len()).unwrap().to_be_bytes());
    entry.extend_from_slice(from.as_bytes());
    let start = class_file
        .windows(entry.len())
        .position(|w| w == entry)
        .unwrap_or_else(|| panic!("no {from} constant"));
    class_file[start + 3..start + entry.len()].copy_from_slice(to.as_bytes());
}

/// Records the attributes and labels it visits.
#[derive(Default)]
struct Recorder {
    attributes: RefCell<Vec<Attribute>>,
    labels: RefCell<Vec<Label>>,
}

impl ClassVisitor for &Recorder {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.attributes.borrow_mut().push(attribute.clone());
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(*self))
    }
}

impl MethodVisitor for &Recorder {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.attributes.borrow_mut().push(attribute.clone());
    }

    fn visit_label(&mut self, label: Label) {
        self.labels.borrow_mut().push(label);
    }
}

#[test]
fn test_read_with_codec() {
    let mut class_file = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    rename_utf8(&mut class_file, "SourceFile", "Source\0\0\0\0");
    // A 10 characters name, of which the codec only knows the first 6: no codec matches.
    let reader = ClassReader::new(&class_file).unwrap();
    let recorder = Recorder::default();
    reader
        .accept_with_codecs(&mut &recorder, &[&SourceCodec], ParsingOptions::empty())
        .unwrap();
    let attributes = recorder.attributes.take();
    assert_eq!(attributes.len(), 1);
    assert!(attributes[0].is_unknown());
    assert_eq!(attributes[0].name(), "Source\0\0\0\0");

    let mut class_file = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    rename_utf8(&mut class_file, "SourceFile", "SourceFilz");
    struct SourceFilzCodec;
    impl AttributeCodec for SourceFilzCodec {
        fn name(&self) -> &JavaStr {
            jstr!("SourceFilz")
        }
        fn read(
            &self,
            cx: &mut AttributeReadContext<'_, '_>,
        ) -> Result<Box<dyn CustomAttribute>, ReadError> {
            SourceCodec.read(cx)
        }
    }
    let reader = ClassReader::new(&class_file).unwrap();
    let recorder = Recorder::default();
    reader
        .accept_with_codecs(&mut &recorder, &[&SourceFilzCodec], ParsingOptions::empty())
        .unwrap();
    let attributes = recorder.attributes.take();
    assert_eq!(attributes.len(), 1);
    let Attribute::Custom(attribute) = &attributes[0] else {
        panic!("not decoded: {:?}", attributes[0]);
    };
    let source = attribute
        .downcast_ref::<Source>()
        .expect("a Source attribute");
    assert_eq!(source.file, "AllInstructions.java");
}

#[test]
fn test_read_code_attribute_with_codec() {
    // Rename every LineNumberTable to Mark, and make the codec read the first start_pc of the
    // table (the attribute content is: line_number_table_length, then (start_pc, line) pairs).
    let mut class_file = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    rename_utf8(
        &mut class_file,
        "LineNumberTable",
        "Mark\0\0\0\0\0\0\0\0\0\0\0",
    );
    struct MarkAfterLengthCodec;
    impl AttributeCodec for MarkAfterLengthCodec {
        fn name(&self) -> &JavaStr {
            jstr!("Mark\0\0\0\0\0\0\0\0\0\0\0")
        }
        fn is_code_attribute(&self) -> bool {
            true
        }
        fn read(
            &self,
            cx: &mut AttributeReadContext<'_, '_>,
        ) -> Result<Box<dyn CustomAttribute>, ReadError> {
            let start_pc = cx.reader().read_unsigned_short(cx.offset() + 2)?;
            Ok(Box::new(Mark(cx.read_label(usize::from(start_pc))?)))
        }
    }
    let reader = ClassReader::new(&class_file).unwrap();
    let recorder = Recorder::default();
    reader
        .accept_with_codecs(
            &mut &recorder,
            &[&MarkAfterLengthCodec, &MarkCodec],
            ParsingOptions::empty(),
        )
        .unwrap();
    let attributes = recorder.attributes.take();
    let labels = recorder.labels.take();
    let marks: Vec<Label> = attributes
        .iter()
        .filter_map(|a| match a {
            Attribute::Custom(c) => c.downcast_ref::<Mark>().map(|m| m.0),
            Attribute::Unknown(_) => None,
        })
        .collect();
    assert!(!marks.is_empty());
    // Each label a codec read is visited, at its bytecode offset, like any other label.
    for mark in marks {
        assert!(labels.contains(&mark), "{mark:?} not visited");
    }
}

#[test]
fn test_read_label_outside_of_a_code_attribute() {
    let mut class_file = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    rename_utf8(&mut class_file, "SourceFile", "SourceFilz");
    struct LabelCodec;
    impl AttributeCodec for LabelCodec {
        fn name(&self) -> &JavaStr {
            jstr!("SourceFilz")
        }
        fn read(
            &self,
            cx: &mut AttributeReadContext<'_, '_>,
        ) -> Result<Box<dyn CustomAttribute>, ReadError> {
            cx.read_label(0)?;
            unreachable!()
        }
    }
    let reader = ClassReader::new(&class_file).unwrap();

    let result = reader.accept_with_codecs(
        &mut &Recorder::default(),
        &[&LabelCodec],
        ParsingOptions::empty(),
    );

    assert!(matches!(
        result,
        Err(ReadError {
            kind: ReadErrorKind::InvalidAttribute(_),
            ..
        })
    ));
}

/// oxasm-specific: an attribute with a negative `attribute_length` is not passed to its codec
/// (ASM passes the length on to the prototype).
#[test]
fn test_read_negative_length_with_codec() {
    let mut class_file = PrecompiledClass::Jdk3AllInstructions.bytes().to_vec();
    rename_utf8(&mut class_file, "SourceFile", "SourceFilz");
    // The SourceFile attribute is the class's last one: attribute_name_index, attribute_length
    // (2), sourcefile_index. Set its length to -6, which ASM's bounds check lets through.
    let length_offset = class_file.len() - 6;
    assert_eq!(class_file[length_offset..length_offset + 4], [0, 0, 0, 2]);
    class_file[length_offset..length_offset + 4].copy_from_slice(&(-6i32).to_be_bytes());
    struct Panics;
    impl AttributeCodec for Panics {
        fn name(&self) -> &JavaStr {
            jstr!("SourceFilz")
        }
        fn read(
            &self,
            cx: &mut AttributeReadContext<'_, '_>,
        ) -> Result<Box<dyn CustomAttribute>, ReadError> {
            panic!("called with {} bytes", cx.content().len())
        }
    }
    let reader = ClassReader::new(&class_file).unwrap();

    let result = reader.accept_with_codecs(
        &mut &Recorder::default(),
        &[&Panics],
        ParsingOptions::empty(),
    );

    assert_eq!(result.unwrap_err().kind, ReadErrorKind::InvalidLength(-6));
}
