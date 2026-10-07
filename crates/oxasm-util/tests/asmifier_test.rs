//! Port of ASM's `ASMifierTest`, plus a hand-driven test (oxasm-specific) that needs no class
//! reader.
//!
//! - `testConstructor`: the "subclasses must use the protected constructor" half is dropped
//!   (there is no subclassing).
//! - `testAsmify_precompiledClass` compiles the ASMifier output with Janino, runs it and compares
//!   the class it generates with the original. There is no Java compiler here: it becomes
//!   `test_asmify_precompiled_class`, which compares the output with ASM's ASMifier output (the
//!   committed oracle goldens, `asmifier/default`), so the generated code is ASM's, which ASM's
//!   own test proves correct. `cargo xtask parity --mode asmify*` does the same on all tiers.
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
use oxasm_testkit::golden::{self, Golden};
use oxasm_util::{Asmifier, Printer, TraceClassVisitor};

const EXPECTED_USAGE: &str = "Prints the ASM code to generate the given class.\n\
    Usage: ASMifier [-nodebug] <fully qualified class name or class file name>";

fn class_file_path(class: PrecompiledClass) -> PathBuf {
    oxasm_testkit::corpus_t0().join(format!("{}.class", class.name().replace('.', "/")))
}

/// The ASMifier output for the given class file, or the reader's error.
fn asmify(class_file: &[u8], options: ParsingOptions) -> Result<Vec<u8>, oxasm::error::ReadError> {
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::with_printer(Asmifier::new(), &mut output);
    ClassReader::new(class_file)?.accept(&mut trace_class_visitor, options)?;
    trace_class_visitor.take_io_error().unwrap();
    Ok(output)
}

#[test]
fn test_constructor() {
    let asmifier = Asmifier::new();
    assert!(asmifier.get_text().is_empty());
    assert_eq!(asmifier.name, "classWriter");
    assert_eq!(asmifier.id, 0);
}

/// Adapted (see the module docs): the ASMifier output of every T0 class (the precompiled classes,
/// the other modules' test resources, and the invalid classes) is equal to ASM's, from the
/// committed oracle goldens; a golden `.err` means that the reader must fail.
#[test]
fn test_asmify_precompiled_class() {
    let dir = golden::goldens_dir().join("asmifier/default");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            path.file_stem().unwrap().to_str().unwrap().to_owned()
        })
        .collect();
    names.sort();
    names.dedup();
    let mut precompiled = 0;
    for name in &names {
        let base = oxasm_testkit::corpus_t0().join(name.replace('.', "/"));
        let class_file = std::fs::read(base.with_extension("class"))
            .or_else(|_| std::fs::read(base.with_extension("clazz")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let actual = asmify(&class_file, ParsingOptions::empty());
        let golden = match std::fs::read(dir.join(format!("{name}.txt"))) {
            Ok(bytes) => Golden::Ok(bytes),
            Err(_) => {
                Golden::Err(std::fs::read_to_string(dir.join(format!("{name}.err"))).unwrap())
            }
        };
        match golden {
            Golden::Ok(expected) => {
                let actual = actual.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(
                    actual == expected,
                    "{name}: ASMifier output differs from the golden"
                );
            }
            Golden::Err(asm_error) => {
                assert!(actual.is_err(), "{name}: ASM fails ({asm_error})");
            }
        }
        if PrecompiledClass::ALL.iter().any(|c| c.name() == name) {
            precompiled += 1;
        }
    }
    assert_eq!(precompiled, PrecompiledClass::ALL.len());
    assert!(names.len() > 60);
}

#[test]
fn test_main_missing_class_name() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Asmifier::main(&[], &mut output, &mut logger).unwrap();
    assert!(output.is_empty());
    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_missing_class_name_with_nodebug() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Asmifier::main(&["-nodebug"], &mut output, &mut logger).unwrap();
    assert!(output.is_empty());
    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_too_many_arguments() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Asmifier::main(
        &["-nodebug", "ASMifierTest", "extraArgument"],
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
    let result = Asmifier::main(&["DoNotExist.class"], &mut output, &mut logger);
    assert!(result.is_err());
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

#[test]
fn test_main_class_not_found() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    let result = Asmifier::main(&["do\\not\\exist"], &mut output, &mut logger);
    assert!(result.is_err());
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

/// Adapted: there is no class path to load a class by name from.
#[test]
fn test_main_class_name() {
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    let error = Asmifier::main(
        &["org.objectweb.asm.util.ASMifierTest"],
        &mut output,
        &mut logger,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Class not found");
    assert!(output.is_empty());
    assert!(logger.is_empty());
}

/// Adapted from `testMain_className_withNodebug`: on a class file.
#[test]
fn test_main_class_name_with_nodebug() {
    let path = class_file_path(PrecompiledClass::Jdk3AllStructures);
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Asmifier::main(
        &["-nodebug", path.to_str().unwrap()],
        &mut output,
        &mut logger,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("public class AllStructuresDump implements Opcodes"));
    assert!(!output.contains("\nmethodVisitor.visitLineNumber("));
    assert!(!output.contains("\nmethodVisitor.visitLocalVariable("));
    assert!(logger.is_empty());
}

#[test]
fn test_main_class_file() {
    let path = class_file_path(PrecompiledClass::Jdk3AllStructures);
    let (mut output, mut logger) = (Vec::new(), Vec::new());
    Asmifier::main(&[path.to_str().unwrap()], &mut output, &mut logger).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("public class AllStructuresDump implements Opcodes"));
    assert!(output.contains("\nmethodVisitor.visitLineNumber("));
    assert!(output.contains("\nmethodVisitor.visitLocalVariable("));
    assert!(logger.is_empty());
}

// -------------------------------------------------------------------------------------------------
// Hand-driven test (oxasm-specific)
// -------------------------------------------------------------------------------------------------

/// An attribute with an ASMifier representation (ASM's `ASMifierSupport`).
#[derive(Clone, Debug)]
struct Comment;

impl CustomAttribute for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn write(&self, _cx: &mut oxasm::writer::AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn asmify(
        &self,
        variable_name: &str,
        _label_name: &mut dyn FnMut(Label) -> String,
    ) -> Option<String> {
        Some(format!(
            "Attribute {variable_name} = new org.objectweb.asm.util.Comment();"
        ))
    }
}

/// A code attribute that names labels in its ASMifier representation: one already declared, and
/// one that is not.
#[derive(Clone, Debug)]
struct CodeComment(Label, Label);

impl CustomAttribute for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn labels(&self) -> Vec<Label> {
        vec![self.0, self.1]
    }
    fn write(&self, _cx: &mut oxasm::writer::AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn asmify(
        &self,
        variable_name: &str,
        label_name: &mut dyn FnMut(Label) -> String,
    ) -> Option<String> {
        Some(format!(
            "Attribute {variable_name} = new CodeComment({}, {});\n",
            label_name(self.0),
            label_name(self.1)
        ))
    }
}

/// Drives a TraceClassVisitor with an Asmifier through every kind of event. The expected output,
/// `data/asmifier_hand_driven.txt`, is ASM 9.10.1's ASMifier output for the same events (driven
/// by `data/AsmifierHand.java`, see its header).
#[test]
#[allow(clippy::too_many_lines)]
fn test_hand_driven_class() {
    let mut output = Vec::new();
    let mut cv = TraceClassVisitor::with_printer(Asmifier::new(), &mut output);
    cv.visit(
        V1_8,
        ACC_PUBLIC | ACC_SUPER | ACC_DEPRECATED | ACC_ABSTRACT,
        jstr!("pkg/sub/C-\u{e9}(1)"),
        Some(jstr!(
            "<T:Ljava/lang/Object;>Ljava/lang/Object;Ljava/lang/Comparable<TT;>;"
        )),
        Some(jstr!("java/lang/Object")),
        &[jstr!("java/lang/Comparable"), jstr!("java/io/Serializable")],
    );
    cv.visit_source(Some(jstr!("C.java")), Some(jstr!("dbg\n")));
    {
        let mut mv = cv
            .visit_module(jstr!("m"), ACC_OPEN | ACC_SYNTHETIC, Some(jstr!("1.0")))
            .unwrap();
        mv.visit_main_class(jstr!("pkg/Main"));
        mv.visit_package(jstr!("pkg"));
        mv.visit_require(
            jstr!("java.base"),
            ACC_TRANSITIVE | ACC_STATIC_PHASE | ACC_MANDATED,
            None,
        );
        mv.visit_require(jstr!("other"), 0, Some(jstr!("2")));
        mv.visit_export(jstr!("pkg"), 0, &[]);
        mv.visit_export(jstr!("pkg/a"), ACC_SYNTHETIC, &[jstr!("x"), jstr!("y")]);
        mv.visit_open(jstr!("pkg/b"), 0, &[jstr!("z")]);
        mv.visit_use(jstr!("pkg/S"));
        mv.visit_provide(jstr!("pkg/S"), &[jstr!("pkg/I1"), jstr!("pkg/I2")]);
        mv.visit_end();
    }
    cv.visit_nest_host(jstr!("pkg/H"));
    cv.visit_outer_class(jstr!("pkg/O"), Some(jstr!("m")), Some(jstr!("()V")));
    {
        let mut av = cv.visit_annotation(jstr!("LA;"), true).unwrap();
        av.visit(
            Some(jstr!("s")),
            &AnnotationValue::from("x\"\n\u{1}\u{20ac}"),
        );
        av.visit(Some(jstr!("b")), &AnnotationValue::Byte(-3));
        av.visit(Some(jstr!("z")), &AnnotationValue::Boolean(true));
        av.visit(Some(jstr!("zf")), &AnnotationValue::Boolean(false));
        av.visit(Some(jstr!("sh")), &AnnotationValue::Short(-300));
        av.visit(Some(jstr!("c")), &AnnotationValue::Char(0xe9));
        av.visit(Some(jstr!("i")), &AnnotationValue::Int(i32::MIN));
        av.visit(Some(jstr!("j")), &AnnotationValue::Long(i64::MAX));
        av.visit(Some(jstr!("f")), &AnnotationValue::Float(1.0e10));
        av.visit(
            Some(jstr!("d")),
            &AnnotationValue::Double(f64::NEG_INFINITY),
        );
        av.visit(
            Some(jstr!("t")),
            &AnnotationValue::Type(Type::get_type("[[Ljava/lang/String;")),
        );
        av.visit(
            Some(jstr!("ab")),
            &AnnotationValue::ByteArray(Cow::Owned(vec![1, -1])),
        );
        av.visit(
            Some(jstr!("az")),
            &AnnotationValue::BooleanArray(Cow::Owned(vec![true, false])),
        );
        av.visit(
            Some(jstr!("as")),
            &AnnotationValue::ShortArray(Cow::Owned(vec![2, -2])),
        );
        av.visit(
            Some(jstr!("ac")),
            &AnnotationValue::CharArray(Cow::Owned(vec![97, 0xffff])),
        );
        av.visit(
            Some(jstr!("ai")),
            &AnnotationValue::IntArray(Cow::Owned(vec![])),
        );
        av.visit(
            Some(jstr!("aj")),
            &AnnotationValue::LongArray(Cow::Owned(vec![-1, 5])),
        );
        av.visit(
            Some(jstr!("af")),
            &AnnotationValue::FloatArray(Cow::Owned(vec![0.1, f32::NAN])),
        );
        av.visit(
            Some(jstr!("ad")),
            &AnnotationValue::DoubleArray(Cow::Owned(vec![0.1, f64::from_bits(1)])),
        );
        av.visit_enum(Some(jstr!("e")), jstr!("LE;"), jstr!("V"));
        {
            let mut arr = av.visit_array(Some(jstr!("arr"))).unwrap();
            arr.visit(None, &AnnotationValue::Long(1));
            {
                let mut nested = arr.visit_annotation(None, jstr!("LB;")).unwrap();
                nested.visit_array(Some(jstr!("deep"))).unwrap().visit_end();
                nested.visit_end();
            }
            arr.visit_end();
        }
        av.visit_end();
    }
    cv.visit_type_annotation(
        TypeReference::new_super_type_reference(-1),
        TypePath::from_string("[.*1;").unwrap().as_ref(),
        jstr!("LTA;"),
        false,
    )
    .unwrap()
    .visit_end();
    cv.visit_attribute(&Attribute::Custom(Box::new(Comment)));
    cv.visit_attribute(&Attribute::Unknown(oxasm::UnknownAttribute {
        name: "Raw\u{e9}".into(),
        content: vec![1, 2],
    }));
    cv.visit_nest_member(jstr!("pkg/N"));
    cv.visit_permitted_subclass(jstr!("pkg/P"));
    cv.visit_inner_class(
        jstr!("pkg/C$1"),
        None,
        None,
        ACC_PRIVATE | ACC_STATIC | ACC_ENUM | ACC_ANNOTATION | ACC_INTERFACE | ACC_ABSTRACT,
    );
    cv.visit_inner_class(jstr!("pkg/C$I"), Some(jstr!("pkg/C")), Some(jstr!("I")), 0);
    {
        let mut rv = cv
            .visit_record_component(jstr!("rc"), jstr!("I"), Some(jstr!("TT;")))
            .unwrap();
        rv.visit_annotation(jstr!("LR;"), false)
            .unwrap()
            .visit_end();
        rv.visit_type_annotation(
            TypeReference::new_type_reference(TypeReference::FIELD),
            None,
            jstr!("LRT;"),
            true,
        )
        .unwrap()
        .visit_end();
        rv.visit_attribute(&Attribute::Custom(Box::new(Comment)));
        rv.visit_end();
    }
    {
        let mut fv = cv
            .visit_field(
                ACC_PROTECTED
                    | ACC_STATIC
                    | ACC_FINAL
                    | ACC_VOLATILE
                    | ACC_TRANSIENT
                    | ACC_ENUM
                    | ACC_SYNTHETIC
                    | ACC_DEPRECATED,
                jstr!("F"),
                jstr!("F"),
                None,
                Some(&Constant::Float(f32::NAN)),
            )
            .unwrap();
        fv.visit_annotation(jstr!("LFA;"), true)
            .unwrap()
            .visit_end();
        fv.visit_type_annotation(
            TypeReference::new_type_reference(TypeReference::FIELD),
            TypePath::from_string("0;").unwrap().as_ref(),
            jstr!("LFT;"),
            false,
        )
        .unwrap()
        .visit_end();
        fv.visit_attribute(&Attribute::Custom(Box::new(Comment)));
        fv.visit_end();
    }
    cv.visit_field(
        0,
        jstr!("J"),
        jstr!("J"),
        Some(jstr!("TT;")),
        Some(&Constant::Long(-7)),
    )
    .unwrap()
    .visit_end();
    cv.visit_field(
        ACC_PUBLIC,
        jstr!("S"),
        jstr!("Ljava/lang/String;"),
        None,
        Some(&Constant::from("\\\t")),
    )
    .unwrap()
    .visit_end();
    cv.visit_field(
        ACC_PRIVATE,
        jstr!("D"),
        jstr!("D"),
        None,
        Some(&Constant::Double(1.0e-300)),
    )
    .unwrap()
    .visit_end();
    cv.visit_field(
        ACC_PRIVATE,
        jstr!("I"),
        jstr!("I"),
        None,
        Some(&Constant::Integer(42)),
    )
    .unwrap()
    .visit_end();
    {
        let mut mv = cv
            .visit_method(
                ACC_PUBLIC
                    | ACC_SYNCHRONIZED
                    | ACC_BRIDGE
                    | ACC_VARARGS
                    | ACC_NATIVE
                    | ACC_STRICT
                    | ACC_SYNTHETIC,
                jstr!("m"),
                jstr!("(I)V"),
                Some(jstr!("(TT;)V")),
                &[jstr!("java/io/IOException"), jstr!("java/lang/Error")],
            )
            .unwrap();
        mv.visit_parameter(None, ACC_FINAL);
        mv.visit_parameter(Some(jstr!("p\u{20ac}")), ACC_MANDATED | ACC_SYNTHETIC);
        {
            let mut dv = mv.visit_annotation_default().unwrap();
            dv.visit(None, &AnnotationValue::Int(1));
            dv.visit_end();
        }
        mv.visit_annotation(jstr!("LM;"), true).unwrap().visit_end();
        mv.visit_type_annotation(
            TypeReference::new_type_reference(TypeReference::METHOD_RETURN),
            None,
            jstr!("LMT;"),
            true,
        )
        .unwrap()
        .visit_end();
        mv.visit_annotable_parameter_count(1, false);
        mv.visit_parameter_annotation(0, jstr!("LP;"), false)
            .unwrap()
            .visit_end();
        mv.visit_attribute(&Attribute::Custom(Box::new(Comment)));
        mv.visit_code();
        let (l0, l1, l2, l3, l4) = (
            Label::new(),
            Label::new(),
            Label::new(),
            Label::new(),
            Label::new(),
        );
        mv.visit_try_catch_block(l0, l1, l2, None);
        mv.visit_try_catch_block(l0, l1, l3, Some(jstr!("java/lang/Exception")));
        mv.visit_try_catch_annotation(
            TypeReference::new_try_catch_reference(1),
            None,
            jstr!("LTC;"),
            true,
        )
        .unwrap()
        .visit_end();
        mv.visit_label(l0);
        mv.visit_line_number(7, l0);
        mv.visit_insn(NOP);
        mv.visit_int_insn(NEWARRAY, T_INT);
        mv.visit_int_insn(BIPUSH, -5);
        mv.visit_int_insn(SIPUSH, 1000);
        mv.visit_var_insn(ILOAD, 1);
        mv.visit_var_insn(ASTORE, 300);
        mv.visit_type_insn(NEW, jstr!("java/lang/Object"));
        mv.visit_field_insn(GETSTATIC, jstr!("pkg/C"), jstr!("F"), jstr!("F"));
        mv.visit_method_insn(
            INVOKEINTERFACE,
            jstr!("java/lang/Comparable"),
            jstr!("compareTo"),
            jstr!("(Ljava/lang/Object;)I"),
            true,
        );
        mv.visit_method_insn(
            INVOKESTATIC,
            jstr!("pkg/C"),
            jstr!("s"),
            jstr!("()V"),
            false,
        );
        mv.visit_ldc_insn(&Constant::Integer(-1));
        mv.visit_ldc_insn(&Constant::Float(-0.0));
        mv.visit_ldc_insn(&Constant::Long(1 << 40));
        mv.visit_ldc_insn(&Constant::Double(-0.0));
        mv.visit_ldc_insn(&Constant::from("ldc\u{0}"));
        mv.visit_ldc_insn(&Constant::Type(Type::get_type("Ljava/lang/String;")));
        mv.visit_ldc_insn(&Constant::Type(Type::get_method_type("(I)V")));
        mv.visit_ldc_insn(&Constant::Handle(Handle::new(
            HandleKind::GetStatic,
            "o",
            "f",
            "I",
            false,
        )));
        mv.visit_ldc_insn(&Constant::from(ConstantDynamic::new(
            "c",
            "I",
            Handle::new(HandleKind::InvokeStatic, "B", "c", "()I", true),
            vec![],
        )));
        mv.visit_iinc_insn(3, -1);
        mv.visit_table_switch_insn(-1, 0, l2, &[l1, l4]);
        mv.visit_lookup_switch_insn(l1, &[5, -6], &[l2, l3]);
        mv.visit_jump_insn(IFNULL, l3);
        mv.visit_invoke_dynamic_insn(
            jstr!("run"),
            jstr!("()Ljava/lang/Runnable;"),
            &Handle::new(HandleKind::InvokeStatic, "B", "bsm", "()V", true),
            &[
                Constant::Type(Type::get_method_type("()V")),
                Constant::Type(Type::get_type("LX;")),
                Constant::Handle(Handle::new(
                    HandleKind::NewInvokeSpecial,
                    "X",
                    "<init>",
                    "()V",
                    false,
                )),
                Constant::from(ConstantDynamic::new(
                    "c",
                    "I",
                    Handle::new(HandleKind::InvokeStatic, "B", "c", "()I", false),
                    vec![
                        Constant::Integer(1),
                        Constant::from("s").into_owned(),
                        Constant::from(ConstantDynamic::new(
                            "d",
                            "J",
                            Handle::new(HandleKind::InvokeStatic, "B", "d", "()J", false),
                            vec![Constant::Long(2)],
                        )),
                    ],
                )),
            ],
        );
        mv.visit_invoke_dynamic_insn(
            jstr!("none"),
            jstr!("()V"),
            &Handle::new(HandleKind::InvokeStatic, "B", "bsm", "()V", false),
            &[],
        );
        mv.visit_multi_a_new_array_insn(jstr!("[[I"), 2);
        mv.visit_insn_annotation(
            TypeReference::new_type_reference(TypeReference::NEW),
            TypePath::from_string("[").unwrap().as_ref(),
            jstr!("LI;"),
            false,
        )
        .unwrap()
        .visit_end();
        mv.visit_label(l1);
        mv.visit_frame(
            FrameKind::Full,
            &[
                FrameValue::from("pkg/C"),
                FrameValue::Integer,
                FrameValue::Top,
            ],
            &[FrameValue::Uninitialized(l0), FrameValue::Uninitialized(l4)],
        );
        mv.visit_frame(
            FrameKind::New,
            &[
                FrameValue::Float,
                FrameValue::Double,
                FrameValue::Long,
                FrameValue::Null,
                FrameValue::UninitializedThis,
            ],
            &[],
        );
        mv.visit_frame(FrameKind::Append, &[FrameValue::from("[I")], &[]);
        mv.visit_frame(FrameKind::Chop(2), &[], &[]);
        mv.visit_frame(FrameKind::Same, &[], &[]);
        mv.visit_frame(FrameKind::Same1, &[], &[FrameValue::from("[I")]);
        mv.visit_insn(RETURN);
        mv.visit_label(l2);
        mv.visit_label(l3);
        mv.visit_label(l4);
        mv.visit_local_variable(jstr!("x"), jstr!("I"), None, l0, l2, 1);
        mv.visit_local_variable(jstr!("y"), jstr!("TT;"), Some(jstr!("TT;")), l1, l3, 2);
        mv.visit_local_variable_annotation(
            TypeReference::new_type_reference(TypeReference::LOCAL_VARIABLE),
            TypePath::from_string("*").unwrap().as_ref(),
            &[l0, l1],
            &[l2, l3],
            &[1, 2],
            jstr!("LLV;"),
            true,
        )
        .unwrap()
        .visit_end();
        mv.visit_attribute(&Attribute::Custom(Box::new(CodeComment(l2, Label::new()))));
        mv.visit_maxs(1, 2);
        mv.visit_end();
    }
    cv.visit_method(ACC_ABSTRACT, jstr!("a"), jstr!("()V"), None, &[])
        .unwrap()
        .visit_end();
    cv.visit_end();
    cv.take_io_error().unwrap();
    drop(cv);
    assert_eq!(
        String::from_utf8(output).unwrap(),
        include_str!("data/asmifier_hand_driven.txt")
    );
}

/// sift-like usage: an Asmifier can print a single method, whose variable is `methodVisitor`.
#[test]
fn test_with_names() {
    let mut asmifier = Asmifier::with_names("mv", 0);
    asmifier.visit_insn(ICONST_0);
    asmifier.visit_var_insn(ISTORE, 1);
    let text: Vec<&JavaStr> = asmifier
        .get_text()
        .iter()
        .filter_map(|n| n.as_str())
        .collect();
    assert_eq!(
        text,
        ["mv.visitInsn(ICONST_0);\n", "mv.visitVarInsn(ISTORE, 1);\n"]
    );
}

/// oxasm-specific: events that only malformed class files produce, and on which ASM's ASMifier
/// throws, are printed without panicking.
#[test]
fn test_malformed_events_do_not_panic() {
    let mut asmifier = Asmifier::new();
    asmifier.visit_int_insn(NEWARRAY, 99);
    asmifier.visit_int_insn(NEWARRAY, -1);
    asmifier.visit_insn(254);
    asmifier.visit_frame(FrameKind::Insert, &[], &[]);
    asmifier.visit_frame(FrameKind::Same1, &[], &[]);
    asmifier.visit_line_number(1, Label::new());
    let mut output = Vec::new();
    asmifier.print(&mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "classWriter.visitIntInsn(NEWARRAY, 99);\n\
         classWriter.visitIntInsn(NEWARRAY, -1);\n\
         classWriter.visitInsn(254);\n\
         classWriter.visitFrame(256, 0, new Object[] {}, 0, new Object[] {});\n\
         classWriter.visitFrame(Opcodes.F_SAME1, 0, null, 1, new Object[] {});\n\
         classWriter.visitLineNumber(1, null);\n"
    );
}
