//! Port of ASM's `TextifierTest`, plus hand-driven tests (oxasm-specific) that need no class
//! reader.
//!
//! - `testConstructor`: the "subclasses must use the protected constructor" half is dropped
//!   (there is no subclassing).
//! - `testMain_className*`: Rust has no class path, so a class *name* is a "Class not found"
//!   error; the content checks run on a class *file* instead.

use std::borrow::Cow;
use std::path::PathBuf;

use oxasm::constant::{AnnotationValue, Constant, FrameValue};
use oxasm::opcodes::*;
use oxasm::{
    Attribute, ClassReader, ClassVisitor, ConstantDynamic, CustomAttribute, FrameKind, Handle,
    HandleKind, JavaStr, Label, ParsingOptions, Type, TypePath, TypeReference, jstr,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_util::{Printer, Textifier, TraceClassVisitor, TraceMethodVisitor};

const EXPECTED_USAGE: &str = "Prints a disassembled view of the given class.\n\
    Usage: Textifier [-nodebug] <fully qualified class name or class file name>";

fn class_file_path(class: PrecompiledClass) -> PathBuf {
    oxasm_testkit::corpus_t0().join(format!("{}.class", class.name().replace('.', "/")))
}

fn parsing_options(options: ReaderOptions) -> ParsingOptions {
    match options {
        ReaderOptions::Default => ParsingOptions::empty(),
        ReaderOptions::SkipCode => ParsingOptions::SKIP_CODE,
        ReaderOptions::SkipDebug => ParsingOptions::SKIP_DEBUG,
        ReaderOptions::SkipFrames => ParsingOptions::SKIP_FRAMES,
        ReaderOptions::ExpandFrames => ParsingOptions::EXPAND_FRAMES,
    }
}

/// The Textifier output for the given class file, or the reader's error.
fn textify(class_file: &[u8], options: ParsingOptions) -> Result<Vec<u8>, oxasm::error::ReadError> {
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    ClassReader::new(class_file)?.accept(&mut trace_class_visitor, options)?;
    trace_class_visitor.take_io_error().unwrap();
    Ok(output)
}

#[test]
fn test_constructor() {
    let textifier = Textifier::new();
    assert!(textifier.get_text().is_empty());
}

/// Tests that the text produced with a Textifier is equal to ASM's own expected text
/// (`asm-util/src/test/resources/*.txt`, copied to `corpus/t0/upstream/textifier`).
#[test]
fn test_textify_precompiled_class() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        if class_file.len() >= 32768 {
            continue;
        }
        let output = textify(class_file, ParsingOptions::empty()).unwrap();
        let expected_text = std::fs::read_to_string(
            oxasm_testkit::corpus_t0().join(format!("upstream/textifier/{}.txt", class.name())),
        )
        .unwrap()
        .replace('\r', "");
        assert_eq!(String::from_utf8_lossy(&output), expected_text, "{class}");
    }
}

/// oxasm-specific: the Textifier output of every T0 class (the precompiled classes, the other
/// modules' test resources, and the invalid classes), with each of the five reader options, is
/// equal to the committed oracle goldens; a golden `.err` means that the reader must fail.
#[test]
fn test_textify_goldens() {
    let mut checked = 0;
    for options in ReaderOptions::ALL {
        let dir = golden::goldens_dir().join(match options {
            ReaderOptions::Default => "textifier/default",
            ReaderOptions::SkipCode => "textifier/skip_code",
            ReaderOptions::SkipDebug => "textifier/skip_debug",
            ReaderOptions::SkipFrames => "textifier/skip_frames",
            ReaderOptions::ExpandFrames => "textifier/expand_frames",
        });
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                path.file_stem().unwrap().to_str().unwrap().to_owned()
            })
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            let base = oxasm_testkit::corpus_t0().join(name.replace('.', "/"));
            let class_file = std::fs::read(base.with_extension("class"))
                .or_else(|_| std::fs::read(base.with_extension("clazz")))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let actual = textify(&class_file, parsing_options(options));
            match golden::textifier(&name, options) {
                Golden::Ok(expected) => {
                    let actual = actual.unwrap_or_else(|e| panic!("{options:?} {name}: {e}"));
                    assert!(
                        actual == expected,
                        "{options:?} {name}: Textifier output differs from the golden"
                    );
                }
                Golden::Err(asm_error) => {
                    assert!(
                        actual.is_err(),
                        "{options:?} {name}: ASM fails ({asm_error})"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 300);
}

#[test]
fn test_main_missing_class_name() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Textifier::main(&[], &mut output, &mut logger).unwrap();
    assert!(output.is_empty());
    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_missing_class_name_with_nodebug() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Textifier::main(&["-nodebug"], &mut output, &mut logger).unwrap();
    assert!(output.is_empty());
    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_too_many_arguments() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Textifier::main(
        &["-nodebug", "TextifierTest", "extraArgument"],
        &mut output,
        &mut logger,
    )
    .unwrap();
    assert!(output.is_empty());
    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_class_file_not_found() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    let result = Textifier::main(&["DoNotExist.class"], &mut output, &mut logger);
    assert!(result.is_err());
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

#[test]
fn test_main_class_not_found() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    let result = Textifier::main(&["do\\not\\exist"], &mut output, &mut logger);
    assert!(result.is_err());
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

/// Adapted: there is no class path to load a class by name from.
#[test]
fn test_main_class_name() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    let error = Textifier::main(
        &["org.objectweb.asm.util.TextifierTest"],
        &mut output,
        &mut logger,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Class not found");
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

/// Adapted from `testMain_className_withNoebug`: on a class file.
#[test]
fn test_main_class_name_with_nodebug() {
    let path = class_file_path(PrecompiledClass::Jdk3AllStructures);
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Textifier::main(
        &["-nodebug", path.to_str().unwrap()],
        &mut output,
        &mut logger,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\nabstract class jdk3/AllStructures"));
    assert!(!output.contains("\n    LINENUMBER"));
    assert!(!output.contains("\n    LOCALVARIABLE"));
    assert!(logger.is_empty());
}

#[test]
fn test_main_class_file() {
    let path = class_file_path(PrecompiledClass::Jdk3AllStructures);
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Textifier::main(&[path.to_str().unwrap()], &mut output, &mut logger).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\nabstract class jdk3/AllStructures"));
    assert!(output.contains("\n    LINENUMBER"));
    assert!(output.contains("\n    LOCALVARIABLE"));
    assert!(logger.is_empty());
}

// -------------------------------------------------------------------------------------------------
// Hand-driven tests (oxasm-specific)
// -------------------------------------------------------------------------------------------------

/// A class attribute with a Textifier representation (ASM's `TextifierSupport`).
#[derive(Clone, Debug)]
struct Comment;

impl CustomAttribute for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn write(&self, _cx: &mut oxasm::writer::AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn textify(&self, _label_name: &mut dyn FnMut(Label) -> String) -> Option<String> {
        Some(" : comment\n".to_owned())
    }
}

/// A code attribute that names a label in its Textifier representation.
#[derive(Clone, Debug)]
struct CodeComment(Label);

impl CustomAttribute for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn labels(&self) -> Vec<Label> {
        vec![self.0]
    }
    fn write(&self, _cx: &mut oxasm::writer::AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn textify(&self, label_name: &mut dyn FnMut(Label) -> String) -> Option<String> {
        Some(format!(" -> {}\n", label_name(self.0)))
    }
}

/// Drives a TraceClassVisitor through every kind of event, as `data/Hand.java` does with ASM's,
/// whose output `data/textifier_hand_driven.txt` is.
#[test]
fn test_hand_driven_class() {
    let mut output = Vec::new();
    let mut cv = TraceClassVisitor::new(&mut output);
    cv.visit(
        V1_8,
        ACC_PUBLIC | ACC_SUPER,
        jstr!("pkg/C\u{e9}"),
        Some(jstr!(
            "<T:Ljava/lang/Object;>Ljava/lang/Object;Ljava/lang/Comparable<TT;>;"
        )),
        Some(jstr!("java/lang/Object")),
        &[jstr!("java/lang/Comparable")],
    );
    cv.visit_source(Some(jstr!("C.java")), None);
    {
        let mut av = cv.visit_annotation(jstr!("LA;"), true).unwrap();
        av.visit(
            Some(jstr!("s")),
            &AnnotationValue::from("x\"\n\u{1}\u{20ac}"),
        );
        av.visit(Some(jstr!("c")), &AnnotationValue::Char(u16::from(b'c')));
        av.visit(Some(jstr!("f")), &AnnotationValue::Float(1.0e10));
        av.visit(
            Some(jstr!("d")),
            &AnnotationValue::DoubleArray(Cow::Owned(vec![0.1, f64::from_bits(1)])),
        );
        av.visit(
            Some(jstr!("t")),
            &AnnotationValue::Type(Type::get_type("[[Ljava/lang/String;")),
        );
        av.visit_enum(Some(jstr!("e")), jstr!("LE;"), jstr!("V"));
        {
            let mut arr = av.visit_array(Some(jstr!("arr"))).unwrap();
            arr.visit(None, &AnnotationValue::Long(1));
            arr.visit_annotation(None, jstr!("LB;"))
                .unwrap()
                .visit_end();
            arr.visit_end();
        }
        av.visit_end();
    }
    cv.visit_attribute(&Attribute::Custom(Box::new(Comment)));
    cv.visit_inner_class(jstr!("pkg/C$1"), None, None, 0);
    cv.visit_field(
        ACC_STATIC | ACC_FINAL,
        jstr!("F"),
        jstr!("F"),
        None,
        Some(&Constant::Float(f32::NAN)),
    )
    .unwrap()
    .visit_end();
    {
        let mut mv = cv
            .visit_method(
                ACC_PUBLIC,
                jstr!("m"),
                jstr!("(I)V"),
                None,
                &[jstr!("java/io/IOException")],
            )
            .unwrap();
        mv.visit_parameter(None, ACC_FINAL);
        mv.visit_code();
        let (l0, l1, l2) = (Label::new(), Label::new(), Label::new());
        mv.visit_try_catch_block(l0, l1, l2, None);
        mv.visit_label(l0);
        mv.visit_line_number(7, l0);
        mv.visit_int_insn(NEWARRAY, T_INT);
        mv.visit_int_insn(BIPUSH, -5);
        mv.visit_ldc_insn(&Constant::Double(-0.0));
        mv.visit_ldc_insn(&Constant::Type(Type::get_type("Ljava/lang/String;")));
        mv.visit_ldc_insn(&Constant::Handle(Handle::new(
            HandleKind::GetStatic,
            "o",
            "f",
            "I",
            false,
        )));
        mv.visit_iinc_insn(3, -1);
        mv.visit_table_switch_insn(-1, 0, l2, &[l1, l2]);
        mv.visit_lookup_switch_insn(l1, &[5], &[l2]);
        mv.visit_invoke_dynamic_insn(
            jstr!("run"),
            jstr!("()Ljava/lang/Runnable;"),
            &Handle::new(HandleKind::InvokeStatic, "B", "bsm", "()V", true),
            &[
                Constant::Type(Type::get_method_type("()V")),
                Constant::Type(Type::get_type("LX;")),
                Constant::from(ConstantDynamic::new(
                    "c",
                    "I",
                    Handle::new(HandleKind::InvokeStatic, "B", "c", "()I", false),
                    vec![Constant::Integer(1), Constant::from("s").into_owned()],
                )),
            ],
        );
        mv.visit_label(l1);
        mv.visit_frame(
            FrameKind::Full,
            &[FrameValue::from("pkg/C"), FrameValue::Integer],
            &[FrameValue::Uninitialized(l0)],
        );
        mv.visit_frame(FrameKind::Chop(1), &[], &[]);
        mv.visit_frame(FrameKind::Same1, &[], &[FrameValue::from("[I")]);
        mv.visit_insn(RETURN);
        mv.visit_label(l2);
        mv.visit_type_annotation(
            TypeReference::new_exception_reference(0),
            TypePath::from_string("[*0;").unwrap().as_ref(),
            jstr!("LT;"),
            false,
        )
        .unwrap()
        .visit_end();
        mv.visit_local_variable(jstr!("x"), jstr!("I"), None, l0, l2, 1);
        mv.visit_attribute(&Attribute::Custom(Box::new(CodeComment(l2))));
        mv.visit_maxs(1, 2);
        mv.visit_end();
    }
    cv.visit_end();
    cv.take_io_error().unwrap();
    drop(cv);
    assert_eq!(
        String::from_utf8(output).unwrap(),
        include_str!("data/textifier_hand_driven.txt")
    );
}

/// sift's usage: a method's Textifier text has one string per instruction, and labels are named
/// per method, by first appearance.
#[test]
fn test_trace_method_visitor_text() {
    let mut textifier = Textifier::new();
    let mut mv = TraceMethodVisitor::new(&mut textifier);
    let (l0, l1) = (Label::new(), Label::new());
    use oxasm::MethodVisitor;
    mv.visit_jump_insn(GOTO, l1);
    mv.visit_label(l0);
    mv.visit_label(l1);
    mv.visit_method_insn(INVOKEINTERFACE, jstr!("I"), jstr!("m"), jstr!("()V"), true);
    drop(mv);
    let text: Vec<&JavaStr> = textifier
        .get_text()
        .iter()
        .filter_map(|n| n.as_str())
        .collect();
    assert_eq!(
        text,
        [
            "    GOTO L0\n",
            "   L1\n",
            "   L0\n",
            "    INVOKEINTERFACE I.m ()V (itf)\n"
        ]
    );
}

/// oxasm-specific: events that only malformed class files produce, and on which ASM's Textifier
/// throws, are printed without panicking.
#[test]
fn test_malformed_events_do_not_panic() {
    let mut textifier = Textifier::new();
    textifier.visit(V1_8, ACC_PUBLIC, jstr!("C"), Some(jstr!("<T:")), None, &[]);
    let mut mv = TraceMethodVisitor::new(textifier.visit_method(
        0,
        jstr!("m"),
        jstr!("()V"),
        Some(jstr!("(")),
        &[],
    ));
    use oxasm::MethodVisitor;
    mv.visit_int_insn(NEWARRAY, 99);
    mv.visit_int_insn(NEWARRAY, -1);
    mv.visit_insn(0xff);
    mv.visit_frame(FrameKind::Same1, &[], &[]);
    mv.visit_ldc_insn(&Constant::Type(Type::get_method_type("()V")));
    let mut av = mv.visit_annotation_default().unwrap();
    av.visit(None, &AnnotationValue::Type(Type::get_method_type("()V")));
    av.visit(None, &AnnotationValue::Type(Type::get_object_type("[")));
    av.visit_end();
    drop(av);
    drop(mv);
    let text = oxasm_util::printer::text_to_java_string(textifier.get_text());
    assert!(text.contains("NEWARRAY 99"));
    assert!(text.contains("()V.class"));
}
