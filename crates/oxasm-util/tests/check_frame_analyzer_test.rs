//! Port of ASM's `CheckFrameAnalyzerTest`.
//!
//! - `testAnalyze_illegalFrameType` uses ASM's internal `F_INSERT` frame type (256), the only
//!   frame type outside of the valid ones that a `FrameKind` can hold, instead of 123456.
//! - `testAnalyze_illegalStackMapFrameValue` is dropped: its frame value (a `java.lang.Object`)
//!   cannot be a `FrameValue`.
//! - The tests over the precompiled classes compute their frames with the class hierarchy that
//!   ASM's tests get from class loading (T0 and the JDK, from the oracle).

mod common;

use oxasm::constant::FrameValue;
use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, FrameKind, Label, ParsingOptions};
use oxasm_analysis::BasicVerifier;
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, MethodNode};
use oxasm_util::check::{CheckFrameAnalyzer, check_frame_analyzer};

use common::{MethodNodeBuilder, class_loader_hierarchy};

const CLASS_NAME: &str = "C";

fn new_analyzer() -> CheckFrameAnalyzer<BasicVerifier> {
    check_frame_analyzer(BasicVerifier::new())
}

/// The message of the analysis error of `method_node`.
#[track_caller]
fn analyze_error(method_node: &MethodNode) -> String {
    match new_analyzer().analyze(oxasm::jstr!(CLASS_NAME), method_node) {
        Ok(_) => panic!("expected an AnalyzerException"),
        Err(e) => e.message.to_string(),
    }
}

#[track_caller]
fn assert_contains(message: &str, expected: &str) {
    assert!(
        message.contains(expected),
        "{message:?} does not contain {expected:?}"
    );
}

#[test]
fn test_analyze_valid_bytecode() {
    let method_node = MethodNodeBuilder::with_descriptor("(Ljava/lang/Object;)V", 1, 2)
        .aload(0)
        .astore(1)
        .iconst_0()
        .istore(0)
        .vreturn()
        .build();

    let mut analyzer = new_analyzer();
    let result = analyzer.analyze(oxasm::jstr!(CLASS_NAME), &method_node);

    assert!(result.is_ok(), "{:?}", result.err());
}

#[test]
fn test_analyze_invalid_jsr() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .jsr(label0)
        .label(label0)
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 0: JSR instructions are unsupported",
    );
}

#[test]
fn test_analyze_invalid_ret() {
    let method_node = MethodNodeBuilder::new().ret(0).vreturn().build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 0: RET instructions are unsupported",
    );
}

#[test]
fn test_analyze_missing_frame_at_jump_target() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .ifne(label0)
        .label(label0)
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Expected stack map frame at instruction 2",
    );
}

#[test]
fn test_analyze_missing_frame_after_goto() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .nop()
        .go(label0)
        .nop()
        .label(label0)
        .frame(FrameKind::Same, &[], &[])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Expected stack map frame at instruction 2",
    );
}

/// Adapted: with `F_INSERT` (256) instead of 123456, which a `FrameKind` cannot hold.
#[test]
fn test_analyze_illegal_frame_type() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .nop()
        .go(label0)
        .frame(FrameKind::Insert, &[], &[])
        .nop()
        .label(label0)
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(&message, "Error at instruction 2: Illegal frame type 256");
}

#[test]
fn test_analyze_invalid_append_frame() {
    let method_node = MethodNodeBuilder::with_maxs(0, 1)
        .nop()
        .frame(FrameKind::Append, &[FrameValue::Integer], &[])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Cannot append more locals than maxLocals",
    );
}

#[test]
fn test_analyze_invalid_chop_frame() {
    let method_node = MethodNodeBuilder::with_maxs(0, 1)
        .nop()
        .frame(FrameKind::Chop(2), &[], &[])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Cannot chop more locals than defined",
    );
}

// testAnalyze_illegalStackMapFrameValue: dropped, a `java.lang.Object` frame value cannot be a
// `FrameValue`.

#[test]
fn test_analyze_illegal_label_node_stack_map_frame_value() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::with_maxs(0, 2)
        .nop()
        .frame(FrameKind::Append, &[FrameValue::Uninitialized(label0)], &[])
        .label(label0)
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: LabelNode does not designate a NEW instruction",
    );
}

#[test]
fn test_analyze_frame_at_jump_target_has_incompatible_stack_height() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .ifne(label0)
        .iconst_0()
        .label(label0)
        .frame(FrameKind::Same1, &[], &[FrameValue::Integer])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Stack map frame incompatible with frame at instruction 3 \
         (incompatible stack heights)",
    );
}

#[test]
fn test_analyze_frame_at_jump_target_has_incompatible_local_values() {
    let label0 = Label::new();
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .ifne(label0)
        .iconst_0()
        .label(label0)
        .frame(FrameKind::New, &[FrameValue::Integer], &[])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 1: Stack map frame incompatible with frame at instruction 3 \
         (incompatible types at local 0: R and I)",
    );
}

#[test]
fn test_analyze_frame_at_jump_target_has_incompatible_stack_values() {
    let label0 = Label::new();
    let c = || FrameValue::Object(std::borrow::Cow::Borrowed(oxasm::jstr!("C")));
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .iconst_0()
        .ifne(label0)
        .iconst_0()
        .iconst_0()
        .label(label0)
        .frame(FrameKind::New, &[c()], &[c()])
        .vreturn()
        .build();

    let message = analyze_error(&method_node);

    assert_contains(
        &message,
        "Error at instruction 2: Stack map frame incompatible with frame at instruction 5 \
         (incompatible types at stack item 0: I and R)",
    );
}

fn has_jsr_or_ret_instructions(class_parameter: PrecompiledClass) -> bool {
    class_parameter == PrecompiledClass::Jdk3AllInstructions
        || class_parameter == PrecompiledClass::Jdk3LargeMethod
}

/// The class, with its frames recomputed by a `ClassWriter`, as a class node.
fn compute_frames(class_parameter: PrecompiledClass) -> ClassNode {
    let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
    let mut class_writer =
        ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES).with_hierarchy(class_loader_hierarchy());
    class_reader
        .accept(&mut class_writer, ParsingOptions::empty())
        .unwrap();
    let class_file = class_writer.to_bytes().unwrap();
    let mut class_node = ClassNode::new();
    ClassReader::new(&class_file)
        .unwrap()
        .accept(&mut class_node, ParsingOptions::empty())
        .unwrap();
    class_node
}

/// The index of the last real instruction of a method, or `None`.
fn last_jvm_insn_index(method: &MethodNode) -> Option<usize> {
    (0..method.instructions.len())
        .rev()
        .find(|&i| method.instructions.get(i).unwrap().opcode().is_some())
}

/// Tests that the precompiled classes can be successfully analyzed from their existing stack
/// map frames with a `BasicVerifier`.
#[test]
fn test_analyze_basic_verifier() {
    for &class_parameter in PrecompiledClass::ALL {
        if has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let class_node = compute_frames(class_parameter);
        let mut analyzer = new_analyzer();

        for method_node in &class_node.methods {
            let method_name = method_node.resolve(method_node.name);
            let frames = analyzer
                .analyze(class_node.resolve(class_node.name), method_node)
                .unwrap_or_else(|e| panic!("{class_parameter} {method_name}: {e}"));
            let last = last_jvm_insn_index(method_node).unwrap_or(0);
            for (j, frame) in frames.iter().enumerate().take(last) {
                assert!(
                    frame.is_some(),
                    "{class_parameter} {method_name}: no frame at {j}"
                );
            }
        }
    }
}

/// Tests that the precompiled classes can be successfully analyzed from their existing stack
/// map frames with a `BasicVerifier`, even if the method node's max locals and max stack size
/// are not set.
#[test]
fn test_analyze_and_compute_maxs_basic_verifier() {
    for &class_parameter in PrecompiledClass::ALL {
        if has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let mut class_node = compute_frames(class_parameter);
        let method_maxs: Vec<(u16, u16)> = class_node
            .methods
            .iter_mut()
            .map(|method_node| {
                let maxs = (method_node.max_stack, method_node.max_locals);
                method_node.max_stack = 0;
                method_node.max_locals = 0;
                maxs
            })
            .collect();
        let mut analyzer = new_analyzer();

        let name = class_node.resolve(class_node.name).to_owned();
        for (method_node, &(max_stack, max_locals)) in
            class_node.methods.iter_mut().zip(&method_maxs)
        {
            let method_name = method_node.resolve(method_node.name).to_owned();
            analyzer
                .analyze_and_compute_maxs(&name, method_node)
                .unwrap_or_else(|e| panic!("{class_parameter} {method_name}: {e}"));
            assert!(method_node.max_locals >= max_locals, "{class_parameter}");
            assert!(method_node.max_stack >= max_stack, "{class_parameter}");
        }
    }
}
