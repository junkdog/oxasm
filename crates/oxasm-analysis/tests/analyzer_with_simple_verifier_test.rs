//! Port of ASM's `AnalyzerWithSimpleVerifierTest`: unit tests for `Analyzer`, when used with a
//! `SimpleVerifier`.
//!
//! ASM's `SimpleVerifier` loads the JDK classes it needs; this one asks a `ClassHierarchy`
//! (ADR-07). The tests use a `MapHierarchy` of the JDK facts they need (`JDK_HIERARCHY`, the
//! entries of the parity oracle's T0 `hierarchy.tsv` for these classes). The hierarchy model
//! knows super classes and `is_interface`, not implemented interfaces; no assertion below
//! depends on implemented interfaces (`SimpleVerifier` relaxes the subtyping check against an
//! interface type).
//!
//! - `testAnalyze_classNotFound`: the message is the hierarchy's `Type D not present`, the
//!   oracle's `TypeNotPresentException(name)`, where ASM's class loading gives
//!   `Type java.lang.ClassNotFoundException: D not present`.
//! - `buildClassWithMethod(methodNode).newInstance()`, which checks that the JVM verifier
//!   accepts the method, is split in two: the tests write the class (`build_class_with_method`),
//!   and an ignored twin, `<test>_new_instance`, has the JVM verify it (`cargo test --
//!   --ignored`, which needs the oracle).
//! - The `Api` axis of `testAnalyze_simpleVerifier` is dropped (ADR-09): it loops over
//!   `PrecompiledClass::ALL`.

mod common;

use std::sync::Arc;

use common::{MethodNodeBuilder, assert_new_instance, build_class_with_method};
use oxasm::opcodes::*;
use oxasm::{ClassReader, Label, MapHierarchy, ParsingOptions, Type, jstr};
use oxasm_analysis::{Analyzer, AnalyzerError, SimpleVerifier};
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, MethodNode};

const CLASS_NAME: &str = "C";

/// The JDK classes the tests below (and the methods of the precompiled classes) need, as the
/// parity oracle's `hierarchy.tsv` gives them: `name \t super class \t is interface`, where
/// interfaces have `java/lang/Object` as super class. The precompiled classes, which ASM's test
/// class loader also knows, are included where they are needed.
const JDK_HIERARCHY: &str = "\
java/lang/Object\t\t0
java/lang/Enum\tjava/lang/Object\t0
java/lang/Exception\tjava/lang/Throwable\t0
java/lang/Float\tjava/lang/Number\t0
java/lang/IllegalArgumentException\tjava/lang/RuntimeException\t0
java/lang/IllegalStateException\tjava/lang/RuntimeException\t0
java/lang/Integer\tjava/lang/Number\t0
java/lang/Iterable\tjava/lang/Object\t1
java/lang/Number\tjava/lang/Object\t0
java/lang/Runnable\tjava/lang/Object\t1
java/lang/RuntimeException\tjava/lang/Exception\t0
java/lang/String\tjava/lang/Object\t0
java/lang/System\tjava/lang/Object\t0
java/lang/Throwable\tjava/lang/Object\t0
java/io/PrintWriter\tjava/io/Writer\t0
java/io/Serializable\tjava/lang/Object\t1
java/io/Writer\tjava/lang/Object\t0
java/nio/file/Path\tjava/lang/Object\t1
java/sql/SQLException\tjava/lang/Exception\t0
java/util/AbstractCollection\tjava/lang/Object\t0
java/util/AbstractList\tjava/util/AbstractCollection\t0
java/util/AbstractMap\tjava/lang/Object\t0
java/util/ArrayList\tjava/util/AbstractList\t0
java/util/HashMap\tjava/util/AbstractMap\t0
java/util/List\tjava/lang/Object\t1
java/util/Map\tjava/lang/Object\t1
jdk3/AllStructures$1\tjava/lang/Object\t0
jdk8/AllStructures$1\tjava/lang/Object\t0
";

/// What ASM's class loader knows about the classes of these tests.
fn jdk_hierarchy() -> MapHierarchy {
    MapHierarchy::from_tsv(JDK_HIERARCHY.as_bytes()).unwrap()
}

/// ASM's `newAnalyzer().analyze(CLASS_NAME, methodNode)`, without the frames.
fn analyze(method_node: &MethodNode) -> Result<(), AnalyzerError> {
    new_analyzer()
        .analyze(jstr!(CLASS_NAME), method_node)
        .map(|_| ())
}

fn test_analyze_different_dimensions_method_node() -> MethodNode {
    let otherwise = Label::new();
    let finish = Label::new();
    MethodNodeBuilder::with_descriptor("()[Ljava/io/Serializable;", 3, 1)
        .insn(ICONST_0)
        .ifne(otherwise)
        .iconst_0()
        .iconst_0()
        .multi_a_new_array_insn("[[I", 2)
        .go(finish)
        .label(otherwise)
        .iconst_0()
        .iconst_0()
        .iconst_0()
        .multi_a_new_array_insn("[[[Ljava/lang/System;", 3)
        .label(finish)
        .areturn()
        .build()
}

#[test]
fn test_analyze_different_dimensions() {
    let method_node = test_analyze_different_dimensions_method_node();

    let analyze = analyze(&method_node);

    analyze.unwrap();
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_different_dimensions_new_instance() {
    assert_new_instance(&test_analyze_different_dimensions_method_node());
}

fn test_analyze_array_of_current_class_method_node() -> MethodNode {
    let label0 = Label::new();
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    MethodNodeBuilder::new()
        .iconst_0()
        .type_insn(ANEWARRAY, CLASS_NAME)
        .astore(1)
        .iconst_0()
        .istore(2)
        .label(label0)
        .iload(2)
        .ifne(label1)
        .aload(1)
        .iload(2)
        .insn(AALOAD)
        .pop()
        .go(label0)
        .label(label1)
        .label(label2)
        .iload(2)
        .ifne(label3)
        .aload(1)
        .iload(2)
        .insn(AALOAD)
        .pop()
        .go(label2)
        .label(label3)
        .vreturn()
        .build()
}

#[test]
fn test_analyze_array_of_current_class() {
    let method_node = test_analyze_array_of_current_class_method_node();

    let analyze = analyze(&method_node);

    analyze.unwrap();
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_array_of_current_class_new_instance() {
    assert_new_instance(&test_analyze_array_of_current_class_method_node());
}

#[test]
fn test_analyze_primitive_array_return_type() {
    let method_node = MethodNodeBuilder::with_descriptor("()[I", 1, 1)
        .iconst_0()
        .type_insn(ANEWARRAY, CLASS_NAME)
        .areturn()
        .build();

    let analyze = analyze(&method_node);

    let message = analyze.unwrap_err().message.to_string();
    assert!(
        message.contains("Incompatible return type: expected [I, but found [LC;"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_invokevirtual() {
    let method_node = MethodNodeBuilder::new()
        .insn(ACONST_NULL)
        .type_insn(CHECKCAST, "java/lang/Object")
        .method_insn(INVOKEVIRTUAL, "java/util/ArrayList", "size", "()I", false)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = analyze.unwrap_err().message.to_string();
    assert!(
        message
            .contains("Method owner: expected Ljava/util/ArrayList;, but found Ljava/lang/Object;"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_invokeinterface() {
    let method_node = MethodNodeBuilder::new()
        .insn(ACONST_NULL)
        .type_insn(CHECKCAST, "java/util/List")
        .insn(FCONST_0)
        .method_insn(
            INVOKEINTERFACE,
            "java/util/List",
            "get",
            "(I)Ljava/lang/Object;",
            true,
        )
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    let message = analyze.unwrap_err().message.to_string();
    assert_eq!(
        "Error at instruction 3: Argument 1: expected I, but found F",
        message
    );
}

#[test]
fn test_analyze_class_not_found() {
    let loop_label = Label::new();
    let method_node = MethodNodeBuilder::new()
        .aload(0)
        .astore(1)
        .label(loop_label)
        .aconst_null()
        .type_insn(CHECKCAST, "D")
        .astore(1)
        .go(loop_label)
        .build();

    let analyze = analyze(&method_node);

    let message = analyze.unwrap_err().message.to_string();
    // ASM: "Type java.lang.ClassNotFoundException: D not present", the toString() of the
    // ClassNotFoundException of class loading. A ClassHierarchy reports the class name only, as
    // the parity oracle's TypeNotPresentException(name) does.
    assert!(message.contains("Type D not present"), "{message}");
}

fn test_analyze_merge_stack_frames_method_node() -> MethodNode {
    let loop_label = Label::new();
    MethodNodeBuilder::with_maxs(1, 4)
        .aload(0)
        .astore(1)
        .aconst_null()
        .type_insn(CHECKCAST, "java/lang/Number")
        .astore(2)
        .aload(0)
        .astore(3)
        .label(loop_label)
        .aconst_null()
        .type_insn(CHECKCAST, "java/lang/Number")
        .astore(1)
        .aload(0)
        .astore(2)
        .aconst_null()
        .type_insn(CHECKCAST, "java/lang/Integer")
        .astore(3)
        .go(loop_label)
        .build()
}

#[test]
fn test_analyze_merge_stack_frames() {
    let method_node = test_analyze_merge_stack_frames_method_node();

    let analyze = analyze(&method_node);

    analyze.unwrap();
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_merge_stack_frames_new_instance() {
    assert_new_instance(&test_analyze_merge_stack_frames_method_node());
}

#[test]
fn test_analyze_merge_stack_frames_with_exception_handlers() {
    let start_try0_label = Label::new();
    let end_try0_label = Label::new();
    let catch0_label = Label::new();
    let start_try1_label = Label::new();
    let end_try1_label = Label::new();
    let catch1_label = Label::new();
    let start_try2_label = Label::new();
    let end_try2_label = Label::new();
    let catch2_label = Label::new();
    let label0 = Label::new();
    let label_return = Label::new();
    let method_node = MethodNodeBuilder::with_maxs(2, 6)
        .trycatch_type(
            start_try0_label,
            end_try0_label,
            catch0_label,
            Some("java/lang/Throwable"),
        )
        .trycatch_type(
            start_try1_label,
            end_try1_label,
            catch1_label,
            Some("java/lang/Throwable"),
        )
        .trycatch(start_try2_label, end_try2_label, catch2_label)
        .iconst_0()
        .istore(2)
        .type_insn(NEW, "java/lang/String")
        .astore(3)
        .type_insn(NEW, "java/nio/file/Path")
        .astore(1)
        .label(start_try2_label)
        .type_insn(NEW, "java/io/PrintWriter")
        .astore(2)
        .label(start_try0_label)
        .label(end_try0_label)
        .aload(2)
        .method_insn(INVOKEVIRTUAL, "java/io/PrintWriter", "close", "()V", false)
        .go(end_try2_label)
        .label(catch0_label)
        .astore(3)
        .label(start_try1_label)
        .aload(2)
        .method_insn(INVOKEVIRTUAL, "java/io/PrintWriter", "close", "()V", false)
        .label(end_try1_label)
        .go(label0)
        .label(catch1_label)
        .astore(4)
        .aload(3)
        .aload(4)
        .method_insn(
            INVOKEVIRTUAL,
            "java/lang/Throwable",
            "addSuppressed",
            "(Ljava/lang/Throwable;)V",
            false,
        )
        .label(label0)
        .aload(3)
        .athrow()
        .label(end_try2_label)
        .go(label_return)
        .label(catch2_label)
        .astore(5)
        .aload(5)
        .athrow()
        .label(label_return)
        .vreturn()
        .build();

    let analyze = analyze(&method_node);

    analyze.unwrap();
}

/// Tests that the precompiled classes can be successfully analyzed with a SimpleVerifier.
#[test]
fn test_analyze_simple_verifier() {
    for &class_parameter in PrecompiledClass::ALL {
        let mut class_node = ClassNode::with_interner(Arc::default());
        ClassReader::new(class_parameter.bytes())
            .unwrap()
            .accept(&mut class_node, ParsingOptions::empty())
            .unwrap();
        // ASM: assumeFalse(classNode.methods.isEmpty()).
        if class_node.methods.is_empty() {
            continue;
        }
        // ASM's SimpleVerifier(Type, Type, boolean) constructor, which has no interfaces.
        let mut analyzer = Analyzer::new(SimpleVerifier::for_class(
            Type::get_object_type(class_node.resolve(class_node.name)),
            class_node
                .super_name
                .map(|s| Type::get_object_type(class_node.resolve(s))),
            Vec::new(),
            class_node.access & ACC_INTERFACE != 0,
            jdk_hierarchy(),
        ));

        for method_node in &class_node.methods {
            if let Err(e) = analyzer.analyze(class_node.resolve(class_node.name), method_node) {
                panic!(
                    "{class_parameter} {}{}: {e}",
                    method_node.resolve(method_node.name),
                    method_node.resolve(method_node.desc)
                );
            }
        }
    }
}

/// Checks that the merge of an ArrayList and an SQLException can be returned as an Iterable. The
/// merged type is recomputed by SimpleVerifier as Object (because of limitations of the merging
/// algorithm, due to multiple interface inheritance), but the subtyping check is relaxed if the
/// super type is an interface type.
fn test_is_assignable_from_interface_method_node() -> MethodNode {
    let else_label = Label::new();
    let end_if_label = Label::new();
    MethodNodeBuilder::with_descriptor(
        "(Ljava/util/ArrayList;Ljava/sql/SQLException;)Ljava/lang/Iterable;",
        1,
        3,
    )
    .aload(1)
    .ifnonnull(else_label)
    .aload(1)
    .go(end_if_label)
    .label(else_label)
    .aload(2)
    .label(end_if_label)
    .areturn()
    .build()
}

#[test]
fn test_is_assignable_from_interface() {
    let method_node = test_is_assignable_from_interface_method_node();

    let analyze = analyze(&method_node);

    analyze.unwrap();
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_is_assignable_from_interface_new_instance() {
    assert_new_instance(&test_is_assignable_from_interface_method_node());
}

fn new_analyzer() -> Analyzer<SimpleVerifier<MapHierarchy>> {
    Analyzer::new(SimpleVerifier::for_class(
        Type::get_type("LC;"),
        Some(Type::get_type("Ljava/lang/Number;")),
        Vec::new(),
        false,
        jdk_hierarchy(),
    ))
}
