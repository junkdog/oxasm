//! Port of ASM's `AnalyzerWithBasicVerifierTest`: unit tests for `Analyzer`, when used with a
//! `BasicVerifier`.
//!
//! - `testConstructor` keeps its non-throwing half: there is no subclassing, so the
//!   `IllegalStateException` of `new BasicVerifier() {}` cannot be written (ADR-09).
//! - The `Api` axis of `testAnalyze_basicVerifier` is dropped (ADR-09): it loops over
//!   `PrecompiledClass::ALL`.

mod common;

use common::MethodNodeBuilder;
use oxasm::opcodes::*;
use oxasm::{ClassReader, Label, ParsingOptions, jstr};
use oxasm_analysis::{Analyzer, AnalyzerError, BasicVerifier};
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, MethodNode};

const CLASS_NAME: &str = "C";

/// ASM's `newAnalyzer().analyze(CLASS_NAME, methodNode)`, without the frames.
fn analyze(method_node: &MethodNode) -> Result<(), AnalyzerError> {
    new_analyzer()
        .analyze(jstr!(CLASS_NAME), method_node)
        .map(|_| ())
}

/// The message of the error of `analyze`, which must fail.
#[track_caller]
fn error_message(analyze: Result<(), AnalyzerError>) -> String {
    analyze.unwrap_err().message.to_string()
}

#[test]
fn test_constructor() {
    let _ = BasicVerifier::new();
    // ASM: assertThrows(IllegalStateException.class, () -> new BasicVerifier() {}) - dropped,
    // there is no subclassing (ADR-09).
}

#[test]
fn test_analyze_invalid_aload() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .istore(1)
        .aload(1)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Expected an object reference, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_astore() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .astore(1)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Expected an object reference or a return address, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_iload_due_to_last_instruction_of_exception_handler() {
    let start_try_label = Label::new();
    let end_try_label = Label::new();
    let catch_label = Label::new();
    let method_node = MethodNodeBuilder::new()
        .trycatch(start_try_label, end_try_label, catch_label)
        .iconst_0()
        .istore(1)
        .label(start_try_label)
        .aconst_null()
        .astore(1)
        .label(end_try_label)
        .vreturn()
        .label(catch_label)
        .iload(1)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Error at instruction 8: Expected I, but found ."),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_istore() {
    let method_node = MethodNodeBuilder::new()
        .aconst_null()
        .istore(1)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(message.contains(" Expected I, but found R"), "{message}");
}

#[test]
fn test_analyze_invalid_checkcast() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .type_insn(CHECKCAST, "java/lang/String")
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Expected an object reference, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_arraylength() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .insn(ARRAYLENGTH)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Expected an array reference, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_athrow() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .insn(ATHROW)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Expected an object reference, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_ineg() {
    let method_node = MethodNodeBuilder::new()
        .insn(FCONST_0)
        .insn(INEG)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(message.contains("Expected I, but found F"), "{message}");
}

#[test]
fn test_analyze_invalid_iadd() {
    let method_node = MethodNodeBuilder::new()
        .insn(FCONST_0)
        .insn(ICONST_0)
        .insn(IADD)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("First argument: expected I, but found F"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_iastore() {
    let method_node = MethodNodeBuilder::new()
        .insn(ICONST_1)
        .int_insn(NEWARRAY, T_INT)
        .insn(FCONST_0)
        .insn(ICONST_0)
        .insn(IASTORE)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Second argument: expected I, but found F"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_fastore() {
    let method_node = MethodNodeBuilder::new()
        .insn(ICONST_1)
        .int_insn(NEWARRAY, T_FLOAT)
        .insn(ICONST_0)
        .insn(ICONST_0)
        .insn(FASTORE)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("Third argument: expected F, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_lastore() {
    let method_node = MethodNodeBuilder::new()
        .insn(ICONST_1)
        .insn(ICONST_0)
        .insn(LCONST_0)
        .insn(LASTORE)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert!(
        message.contains("First argument: expected a R array reference, but found I"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_multianewarray() {
    let method_node = MethodNodeBuilder::new()
        .insn(FCONST_1)
        .insn(ICONST_2)
        .multi_a_new_array_insn("[[I", 2)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = error_message(analyze);
    assert_eq!("Error at instruction 2: Expected I, but found F", message);
}

#[test]
fn test_analyze_valid_method_with_exception_handlers() {
    let label0 = Label::new();
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let label4 = Label::new();
    let label5 = Label::new();
    let label6 = Label::new();
    let method_node = MethodNodeBuilder::with_descriptor("(Ljava/lang/Object;)V", 3, 3)
        .trycatch_type(label0, label1, label2, Some("java/lang/Exception"))
        .trycatch_type(label1, label3, label4, None)
        .trycatch_type(label5, label6, label4, None)
        .trycatch_type(label6, label2, label2, Some("java/lang/Exception"))
        .aload(0)
        .ifnonnull(label0)
        .aconst_null()
        .athrow()
        .label(label0)
        .aload(0)
        .astore(2)
        .label(label1)
        .nop()
        .aload(2)
        .pop()
        .label(label3)
        .vreturn()
        .label(label4)
        .astore(1)
        .label(label5)
        .aload(2)
        .pop()
        .label(label6)
        .aload(1)
        .athrow()
        .label(label2)
        .astore(1)
        .go(label3)
        .build();

    let analyze = analyze(&method_node);

    analyze.unwrap();
}

/// Tests that the precompiled classes can be successfully analyzed with a BasicVerifier.
#[test]
fn test_analyze_basic_verifier() {
    for &class_parameter in PrecompiledClass::ALL {
        let mut class_node = ClassNode::new();
        ClassReader::new(class_parameter.bytes())
            .unwrap()
            .accept(&mut class_node, ParsingOptions::empty())
            .unwrap();
        let mut analyzer = new_analyzer();

        for method_node in &class_node.methods {
            if let Err(e) = analyzer.analyze(&class_node.name, method_node) {
                panic!(
                    "{class_parameter} {}{}: {e}",
                    method_node.name, method_node.desc
                );
            }
        }
    }
}

fn new_analyzer() -> Analyzer<BasicVerifier> {
    Analyzer::new(BasicVerifier::new())
}
