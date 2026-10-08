//! Tests of `JsrInlinerAdapter` (ASM's `JsrInlinerAdapterTest`).
//!
//! ASM's `assertDoesNotThrow(() -> buildClassWithMethod(inlinedMethod).newInstance())`: each
//! test writes the class (`build_class_with_method`), and an ignored twin, `<test>_new_instance`,
//! has the JVM verify it (`cargo test -- --ignored`, which needs the oracle). The labels of each
//! test are `label[i]` (ASM's `labelI`) and `expected[i]` (ASM's `expectedLabelI`).

mod common;

use common::{MethodNodeBuilder, build_class_with_method, to_text};
use std::sync::Arc;

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, Label, MethodVisitor,
    ParsingOptions, jstr,
};
use oxasm_commons::JsrInlinerAdapter;
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::MethodNode;

// Some local variable numbers used in tests.
const LOCAL1: u16 = 1;
const LOCAL2: u16 = 2;
const LOCAL3: u16 = 3;
const LOCAL4: u16 = 4;
const LOCAL5: u16 = 5;

/// Fresh labels (ASM's label fields, new for each test).
fn labels<const N: usize>() -> [Label; N] {
    std::array::from_fn(|_| Label::new())
}

/// ASM's `inputMethod.accept(new JSRInlinerAdapter(inlinedMethod, ACC_PUBLIC, "m", "()V", null,
/// null))`: the inlined method.
fn inline(input_method: &MethodNode) -> MethodNode {
    let mut inlined_method = MethodNode::from_strings(
        Arc::default(),
        ACC_PUBLIC,
        jstr!("m"),
        jstr!("()V"),
        None,
        &[],
    );
    input_method.accept_method_visitor(&mut JsrInlinerAdapter::new(
        Some(Box::new(&mut inlined_method)),
        ACC_PUBLIC,
        jstr!("m"),
        jstr!("()V"),
        None,
        &[],
    ));
    inlined_method
}

/// ASM's `assertDoesNotThrow(() -> classFile.newInstance())`.
#[track_caller]
fn assert_new_instance(class_file: &[u8]) {
    common::assert_new_instance(class_file);
}

#[test]
fn test_constructor() {
    let _ = JsrInlinerAdapter::new(None, ACC_PUBLIC, jstr!("name"), jstr!("()V"), None, &[]);
    // Dropped: ASM's check that a subclass cannot use this constructor (no subclasses in Rust,
    // and no api versions, ADR-09).
}

/// Tests a method which has the most basic `try{}finally` form imaginable. More
/// specifically:
///
/// ```java
/// public void a() {
///   int a = 0;
///   try {
///     a++;
///   } finally {
///     a--;
///   }
/// }
/// ```
fn test_inline_jsr_basic_try_finally_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0]) // Body of try block.
        .iinc(1, 1)
        .go(label[3])
        .label(label[1]) // Exception handler.
        .astore(3)
        .jsr(label[2])
        .aload(3)
        .athrow()
        .label(label[2]) // Subroutine.
        .astore(2)
        .iinc(1, -1)
        .ret(2)
        .label(label[3]) // Non-exceptional exit from try block.
        .jsr(label[2])
        .label(label[4])
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[3], label[4], label[1])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // Try/catch block.
        .iinc(1, 1)
        .go(expected[3])
        .label(expected[1]) // Exception handler.
        .astore(3)
        .aconst_null()
        .go(expected[6])
        .label(expected[2])
        .aload(3)
        .athrow()
        .label(expected[3]) // On non-exceptional exit, try block leads here.
        .aconst_null()
        .go(expected[7])
        .label(expected[4])
        .label(expected[5])
        .vreturn()
        .label(expected[6]) // First instantiation of subroutine.
        .astore(2)
        .iinc(1, -1)
        .go(expected[2])
        .new_label()
        .label(expected[7]) // Second instantiation of subroutine.
        .astore(2)
        .iinc(1, -1)
        .go(expected[4])
        .new_label()
        .trycatch(expected[0], expected[1], expected[1])
        .trycatch(expected[3], expected[5], expected[1])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_basic_try_finally() {
    let (inlined_method, expected_method) = test_inline_jsr_basic_try_finally_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_basic_try_finally_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_basic_try_finally_methods().0,
    ));
}

/// Tests a method which has an if/else in the finally clause. More specifically:
///
/// ```java
/// public void a() {
///   int a = 0;
///   try {
///     a++;
///   } finally {
///     if (a == 0) {
///       a += 2;
///     } else {
///       a += 3;
///     }
///   }
/// }
/// ```
fn test_inline_jsr_if_else_in_finally_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0]) // Body of try block.
        .iinc(1, 1)
        .go(label[5])
        .label(label[1]) // Exception handler.
        .astore(3)
        .jsr(label[2])
        .aload(3)
        .athrow()
        .label(label[2]) // Subroutine.
        .astore(2)
        .iload(1)
        .ifne(label[3])
        .iinc(1, 2)
        .go(label[4])
        .label(label[3]) // Test "a != 0".
        .iinc(1, 3)
        .label(label[4]) // Common exit.
        .ret(2)
        .label(label[5]) // Non-exceptional exit from try block.
        .jsr(label[2])
        .label(label[6]) // Used in the TRYCATCH below.
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[5], label[6], label[1])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // Try/catch block.
        .iinc(1, 1)
        .go(expected[3])
        .label(expected[1]) // Exception handler.
        .astore(3)
        .aconst_null()
        .go(expected[6])
        .label(expected[2])
        .aload(3)
        .athrow()
        .label(expected[3]) // On non-exceptional exit, try block leads here.
        .aconst_null()
        .go(expected[9])
        .label(expected[4])
        .label(expected[5])
        .vreturn()
        .label(expected[6]) // First instantiation of subroutine.
        .astore(2)
        .iload(1)
        .ifne(expected[7])
        .iinc(1, 2)
        .go(expected[8])
        .label(expected[7]) // Test "a != 0".
        .iinc(1, 3)
        .label(expected[8]) // Common exit.
        .go(expected[2])
        .new_label()
        .label(expected[9]) // Second instantiation of subroutine.
        .astore(2)
        .iload(1)
        .ifne(expected[10])
        .iinc(1, 2)
        .go(expected[11])
        .label(expected[10]) // Test "a != 0".
        .iinc(1, 3)
        .label(expected[11]) // Common exit.
        .go(expected[4])
        .new_label()
        .trycatch(expected[0], expected[1], expected[1])
        .trycatch(expected[3], expected[5], expected[1])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_if_else_in_finally() {
    let (inlined_method, expected_method) = test_inline_jsr_if_else_in_finally_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_if_else_in_finally_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_if_else_in_finally_methods().0,
    ));
}

/// Tests a method which has a lookupswitch or tableswitch w/in the finally clause. More
/// specifically:
///
/// ```java
/// public void a() {
///   int a = 0;
///   try {
///     a++;
///   } finally {
///     switch (a) {
///       case 0:
///         a += 2;
///         break;
///       default:
///         a += 3;
///     }
///   }
/// }
/// ```
fn test_inline_jsr_lookup_or_table_switch_in_finally_methods(
    use_table_switch: bool,
) -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0]) // Body of try block.
        .iinc(1, 1)
        .go(label[6])
        .label(label[1]) // Exception handler.
        .astore(3)
        .jsr(label[2])
        .aload(3)
        .athrow()
        .label(label[2]) // Subroutine.
        .astore(2)
        .iload(1)
        .switchto_key(label[4], 0, label[3], use_table_switch)
        .label(label[3]) // First switch case.
        .iinc(1, 2)
        .go(label[5])
        .label(label[4]) // Default switch case.
        .iinc(1, 3)
        .label(label[5]) // Common exit.
        .ret(2)
        .label(label[6]) // Non-exceptional exit from try block.
        .jsr(label[2])
        .label(label[7])
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[6], label[7], label[1])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // Try/catch block.
        .iinc(1, 1)
        .go(expected[3])
        .label(expected[1]) // Exception handler.
        .astore(3)
        .aconst_null()
        .go(expected[6])
        .label(expected[2])
        .aload(3)
        .athrow()
        .label(expected[3]) // On non-exceptional exit, try block leads here.
        .aconst_null()
        .go(expected[10])
        .label(expected[4])
        .label(expected[5])
        .vreturn()
        .label(expected[6]) // First instantiation of subroutine.
        .astore(2)
        .iload(1)
        .switchto_key(expected[8], 0, expected[7], use_table_switch)
        .label(expected[7]) // First switch case.
        .iinc(1, 2)
        .go(expected[9])
        .label(expected[8]) // Default switch case.
        .iinc(1, 3)
        .label(expected[9]) // Common exit.
        .go(expected[2])
        .new_label()
        .label(expected[10]) // Second instantiation of subroutine.
        .astore(2)
        .iload(1)
        .switchto_key(expected[12], 0, expected[11], use_table_switch)
        .label(expected[11]) // First switch case.
        .iinc(1, 2)
        .go(expected[13])
        .label(expected[12]) // Default switch case.
        .iinc(1, 3)
        .label(expected[13]) // Common exit.
        .go(expected[4])
        .new_label()
        .trycatch(expected[0], expected[1], expected[1])
        .trycatch(expected[3], expected[5], expected[1])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_lookup_or_table_switch_in_finally() {
    for use_table_switch in [true, false] {
        let (inlined_method, expected_method) =
            test_inline_jsr_lookup_or_table_switch_in_finally_methods(use_table_switch);
        assert_eq!(to_text(&expected_method), to_text(&inlined_method));
        build_class_with_method(&inlined_method);
    }
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_lookup_or_table_switch_in_finally_new_instance() {
    for use_table_switch in [true, false] {
        assert_new_instance(&build_class_with_method(
            &test_inline_jsr_lookup_or_table_switch_in_finally_methods(use_table_switch).0,
        ));
    }
}

/// Tests a simple nested finally. More specifically:
///
/// ```java
/// public void a1() {
///   int a = 0;
///   try {
///     a += 1;
///   } finally {
///     try {
///       a += 2;
///     } finally {
///       a += 3;
///     }
///   }
/// }
/// ```
fn test_inline_jsr_simple_nested_finally_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(2, 6)
        .iconst_0()
        .istore(1)
        .label(label[0]) // Body of try block.
        .iinc(1, 1)
        .jsr(label[2])
        .go(label[5])
        .label(label[1]) // First exception handler.
        .jsr(label[2])
        .athrow()
        .label(label[2]) // First subroutine.
        .astore(2)
        .iinc(1, 2)
        .jsr(label[4])
        .ret(2)
        .label(label[3]) // Second exception handler.
        .jsr(label[4])
        .athrow()
        .label(label[4]) // Second subroutine.
        .astore(3)
        .iinc(1, 3)
        .ret(3)
        .label(label[5]) // On normal exit, try block jumps here.
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[2], label[3], label[3])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(2, 6)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // Body of try block.
        .iinc(1, 1)
        .aconst_null()
        .go(expected[5])
        .label(expected[1])
        .go(expected[4])
        .label(expected[2]) // First exception handler.
        .aconst_null()
        .go(expected[9])
        .label(expected[3])
        .athrow()
        .label(expected[4]) // On normal exit, try block jumps here.
        .vreturn()
        .label(expected[5]) // First instantiation of first subroutine.
        .astore(2)
        .iinc(1, 2)
        .aconst_null()
        .go(expected[13])
        .label(expected[6])
        .go(expected[1])
        .label(expected[7])
        .aconst_null()
        .go(expected[14])
        .label(expected[8])
        .athrow()
        .new_label()
        .label(expected[9]) // Second instantiation of first subroutine.
        .astore(2)
        .iinc(1, 2)
        .aconst_null()
        .go(expected[15])
        .label(expected[10])
        .go(expected[3])
        .label(expected[11])
        .aconst_null()
        .go(expected[16])
        .label(expected[12])
        .athrow()
        .new_label()
        .label(expected[13]) // First instantiation of second subroutine.
        .astore(3)
        .iinc(1, 3)
        .go(expected[6])
        .new_label()
        .label(expected[14]) // Second instantiation of second subroutine.
        .astore(3)
        .iinc(1, 3)
        .go(expected[8])
        .new_label()
        .label(expected[15]) // Third instantiation of second subroutine.
        .astore(3)
        .iinc(1, 3)
        .go(expected[10])
        .new_label()
        .label(expected[16]) // Fourth instantiation of second subroutine.
        .astore(3)
        .iinc(1, 3)
        .go(expected[12])
        .new_label()
        .trycatch(expected[0], expected[2], expected[2])
        .trycatch(expected[5], expected[7], expected[7])
        .trycatch(expected[9], expected[11], expected[11])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_simple_nested_finally() {
    let (inlined_method, expected_method) = test_inline_jsr_simple_nested_finally_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_simple_nested_finally_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_simple_nested_finally_methods().0,
    ));
}

/// This tests a subroutine which has no ret statement, but ends in a "return" instead.
///
/// We structure this as a try/finally with a break in the finally. Because the while loop is
/// infinite, it's clear from the byte code that the only path which reaches the RETURN instruction
/// is through the subroutine.
///
/// ```java
/// public void a1() {
///   int a = 0;
///   while (true) {
///     try {
///       a += 1;
///     } finally {
///       a += 2;
///       break;
///     }
///   }
/// }
/// ```
fn test_inline_jsr_subroutine_with_no_ret_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0]) // While loop header/try block.
        .iinc(1, 1)
        .jsr(label[2])
        .go(label[3])
        .label(label[1]) // Implicit catch block.
        .astore(2)
        .jsr(label[2])
        .aload(2)
        .athrow()
        .label(label[2]) // Subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(label[4]) // ... note that it does not return!
        .label(label[3]) // End of the loop... goes back to the top!
        .go(label[0])
        .label(label[4])
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // While loop header/try block.
        .iinc(1, 1)
        .aconst_null()
        .go(expected[5])
        .label(expected[1])
        .go(expected[4])
        .label(expected[2]) // Implicit catch block.
        .astore(2)
        .aconst_null()
        .go(expected[7])
        .label(expected[3])
        .aload(2)
        .athrow()
        .label(expected[4]) // End of the loop... goes back to the top!
        .go(expected[0])
        .new_label()
        .label(expected[5]) // First instantiation of subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(expected[6]) // ... note that it does not return!
        .label(expected[6])
        .vreturn()
        .label(expected[7]) // Second instantiation of subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(expected[8]) // ... note that it does not return!
        .label(expected[8])
        .vreturn()
        .trycatch(expected[0], expected[2], expected[2])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_subroutine_with_no_ret() {
    let (inlined_method, expected_method) = test_inline_jsr_subroutine_with_no_ret_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_subroutine_with_no_ret_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_subroutine_with_no_ret_methods().0,
    ));
}

/// This tests a subroutine which has no ret statement, but ends in a "return" instead. The code
/// after the JSR appears to fall through the end of the method, but is in fact unreachable and
/// therefore valid.
///
/// ```java
///   JSR L0
///   GOTO L1
/// L0:
///   ASTORE 0
///   RETURN
/// L1:
///   ACONST_NULL
/// ```
fn test_inline_jsr_subroutine_with_no_ret2_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 1)
        .jsr(label[0])
        .go(label[1])
        .label(label[0])
        .astore(0)
        .vreturn()
        .label(label[1])
        .aconst_null()
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 1)
        .aconst_null()
        .go(expected[2])
        .label(expected[0])
        .go(expected[1])
        .label(expected[1])
        .aconst_null()
        .label(expected[2]) // First instantiation of subroutine.
        .astore(0)
        .vreturn()
        .new_label()
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_subroutine_with_no_ret2() {
    let (inlined_method, expected_method) = test_inline_jsr_subroutine_with_no_ret2_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_subroutine_with_no_ret2_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_subroutine_with_no_ret2_methods().0,
    ));
}

/// This tests a subroutine which has no ret statement, but instead exits implicitly by branching
/// to code which is not part of the subroutine. (Sadly, this is legal)
///
/// We structure this as a try/finally in a loop with a break in the finally. The loop is not
/// trivially infinite, so the RETURN statement is reachable both from the JSR subroutine and from
/// the main entry point.
///
/// ```java
/// public void a1() {
///   int a = 0;
///   while (null == null) {
///     try {
///       a += 1;
///     } finally {
///       a += 2;
///       break;
///     }
///   }
/// }
/// ```
fn test_inline_jsr_implicit_exit_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0]) // While loop header.
        .aconst_null()
        .ifnonnull(label[5])
        .label(label[1]) // Try block.
        .iinc(1, 1)
        .jsr(label[3])
        .go(label[4])
        .label(label[2]) // Implicit catch block.
        .astore(2)
        .jsr(label[3])
        .aload(2)
        .athrow()
        .label(label[3]) // Subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(label[5]) // ... note that it does not return!
        .label(label[4]) // End of the loop... goes back to the top!
        .go(label[1])
        .label(label[5])
        .vreturn()
        .trycatch(label[1], label[2], label[2])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(expected[0]) // While loop header.
        .aconst_null()
        .ifnonnull(expected[6])
        .label(expected[1]) // While loop header/try block.
        .iinc(1, 1)
        .aconst_null()
        .go(expected[7])
        .label(expected[2])
        .go(expected[5])
        .label(expected[3]) // Implicit catch block.
        .astore(2)
        .aconst_null()
        .go(expected[8])
        .label(expected[4])
        .aload(2)
        .athrow()
        .label(expected[5]) // End of the loop... goes back to the top!
        .go(expected[1])
        .label(expected[6]) // Exit, not part of subroutine.
        // Note that the two subroutine instantiations branch here.
        .vreturn()
        .label(expected[7]) // First instantiation of subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(expected[6]) // ... note that it does not return!
        .new_label()
        .label(expected[8]) // Second instantiation of subroutine ...
        .astore(3)
        .iinc(1, 2)
        .go(expected[6]) // ... note that it does not return!
        .new_label()
        .trycatch(expected[1], expected[3], expected[3])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_implicit_exit() {
    let (inlined_method, expected_method) = test_inline_jsr_implicit_exit_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_implicit_exit_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_implicit_exit_methods().0,
    ));
}

/// Tests a nested try/finally with implicit exit from one subroutine to the other subroutine.
/// Equivalent to the following java code:
///
/// ```java
/// void m(boolean b) {
///   try {
///     return;
///   } finally {
///     while (b) {
///       try {
///         return;
///       } finally {
///         // NOTE --- this break avoids the second return above (weird)
///         if (b) {
///           break;
///         }
///       }
///     }
///   }
/// }
/// ```
///
/// This example is from the paper, "Subroutine Inlining and Bytecode Abstraction to Simplify
/// Static and Dynamic Analysis" by Cyrille Artho and Armin Biere.
fn test_inline_jsr_implicit_exit_to_another_subroutine_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 6)
        .iconst_0()
        .istore(1)
        .label(label[0]) // First try.
        .jsr(label[2])
        .vreturn()
        .label(label[1]) // Exception handler for first try.
        .astore(LOCAL2)
        .jsr(label[2])
        .aload(LOCAL2)
        .athrow()
        .label(label[2]) // First finally handler.
        .astore(LOCAL4)
        .go(label[6])
        .label(label[3]) // Body of while loop, also second try.
        .jsr(label[5])
        .vreturn()
        .label(label[4]) // Exception handler for second try.
        .astore(LOCAL3)
        .jsr(label[5])
        .aload(LOCAL3)
        .athrow()
        .label(label[5]) // Second finally handler.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label[7])
        .ret(LOCAL5)
        .label(label[6]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(label[3]) // Falls through.
        .label(label[7]) // Exit from finally block.
        .ret(LOCAL4)
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[3], label[4], label[4])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 6)
        // --- Main Subroutine ---
        .iconst_0()
        .istore(1)
        .label(expected[0]) // First try.
        .aconst_null()
        .go(expected[4])
        .label(expected[1])
        .vreturn()
        .label(expected[2]) // Exception handler for first try.
        .astore(LOCAL2)
        .aconst_null()
        .go(expected[11])
        .label(expected[3])
        .aload(LOCAL2)
        .athrow()
        .new_label()
        // --- First instantiation of first subroutine ---
        .label(expected[4]) // First finally handler.
        .astore(LOCAL4)
        .go(expected[9])
        .label(expected[5]) // Body of while loop, also second try.
        .aconst_null()
        .go(expected[18])
        .label(expected[6])
        .vreturn()
        .label(expected[7]) // Exception handler for second try.
        .astore(LOCAL3)
        .aconst_null()
        .go(expected[19])
        .label(expected[8])
        .aload(LOCAL3)
        .athrow()
        .label(expected[9]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(expected[5]) // Falls through.
        .label(expected[10]) // Exit from finally block.
        .go(expected[1])
        // --- Second instantiation of first subroutine ---
        .label(expected[11]) // First finally handler.
        .astore(LOCAL4)
        .go(expected[16])
        .label(expected[12]) // Body of while loop, also second try.
        .aconst_null()
        .go(expected[20])
        .label(expected[13])
        .vreturn()
        .label(expected[14]) // Exception handler for second try.
        .astore(LOCAL3)
        .aconst_null()
        .go(expected[21])
        .label(expected[15])
        .aload(LOCAL3)
        .athrow()
        .label(expected[16]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(expected[12]) // Falls through.
        .label(expected[17]) // Exit from finally block.
        .go(expected[3])
        // --- Second subroutine's 4 instantiations ---
        .label(expected[18]) // First instantiation.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[10])
        .go(expected[6])
        .new_label()
        .label(expected[19]) // Second instantiation.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[10])
        .go(expected[8])
        .new_label()
        .label(expected[20]) // Third instantiation.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[17])
        .go(expected[13])
        .new_label()
        .label(expected[21]) // Fourth instantiation.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[17])
        .go(expected[15])
        .new_label()
        .trycatch(expected[0], expected[2], expected[2])
        .trycatch(expected[5], expected[7], expected[7])
        .trycatch(expected[12], expected[14], expected[14])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_implicit_exit_to_another_subroutine() {
    let (inlined_method, expected_method) =
        test_inline_jsr_implicit_exit_to_another_subroutine_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_implicit_exit_to_another_subroutine_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_implicit_exit_to_another_subroutine_methods().0,
    ));
}

/// This tests two subroutines, neither of which exit. Instead, they both branch to a common set of
/// code which returns from the method. This code is not reachable except through these
/// subroutines, and since they do not invoke each other, it must be copied into both of them.
///
/// I don't believe this can be represented in Java.
fn test_inline_jsr_common_code_which_must_be_duplicated_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 2)
        .iconst_0()
        .istore(1)
        // Invoke the two subroutines, each twice.
        .jsr(label[0])
        .jsr(label[0])
        .jsr(label[1])
        .jsr(label[1])
        .vreturn()
        .label(label[0]) // Subroutine 1.
        .iinc(1, 1)
        .go(label[2]) // ... note that it does not return!
        .label(label[1]) // Subroutine 2.
        .iinc(1, 2)
        .go(label[2]) // ... note that it does not return!
        .label(label[2]) // Common code to both subroutines: exit method.
        .vreturn()
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 2)
        .iconst_0()
        .istore(1)
        // Invoke the two subroutines, each twice.
        .aconst_null()
        .go(expected[4])
        .label(expected[0])
        .aconst_null()
        .go(expected[6])
        .label(expected[1])
        .aconst_null()
        .go(expected[8])
        .label(expected[2])
        .aconst_null()
        .go(expected[10])
        .label(expected[3])
        .vreturn()
        .new_label()
        .label(expected[4]) // Instantiation 1 of subroutine 1.
        .iinc(1, 1)
        .go(expected[5]) // ... note that it does not return!
        .label(expected[5])
        .vreturn()
        .label(expected[6]) // Instantiation 2 of subroutine 1.
        .iinc(1, 1)
        .go(expected[7]) // ... note that it does not return!
        .label(expected[7])
        .vreturn()
        .label(expected[8]) // Instantiation 1 of subroutine 2.
        .iinc(1, 2)
        .go(expected[9]) // ...note that it does not return!
        .label(expected[9])
        .vreturn()
        .label(expected[10]) // Instantiation 2 of subroutine 2.
        .iinc(1, 2)
        .go(expected[11]) // ... note that it does not return!
        .label(expected[11])
        .vreturn()
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_common_code_which_must_be_duplicated() {
    let (inlined_method, expected_method) =
        test_inline_jsr_common_code_which_must_be_duplicated_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_common_code_which_must_be_duplicated_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_common_code_which_must_be_duplicated_methods().0,
    ));
}

/// This tests a simple subroutine where the control flow jumps back and forth between the
/// subroutine and the caller.
///
/// This would not normally be produced by a java compiler.
fn test_inline_jsr_interleaved_code_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 3)
        .iconst_0()
        .istore(1)
        // Invoke the subroutine, each twice.
        .jsr(label[0])
        .go(label[1])
        .label(label[0]) // Subroutine 1.
        .astore(2)
        .iinc(1, 1)
        .go(label[2])
        .label(label[1]) // Second part of main subroutine.
        .iinc(1, 2)
        .go(label[3])
        .label(label[2]) // Second part of subroutine 1.
        .iinc(1, 4)
        .ret(2)
        .label(label[3]) // Third part of main subroutine.
        .jsr(label[0])
        .vreturn()
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 3)
        // Main routine.
        .iconst_0()
        .istore(1)
        .aconst_null()
        .go(expected[4])
        .label(expected[0])
        .go(expected[1])
        .label(expected[1])
        .iinc(1, 2)
        .go(expected[2])
        .label(expected[2])
        .aconst_null()
        .go(expected[6])
        .label(expected[3])
        .vreturn()
        .label(expected[4]) // Instantiation #1.
        .astore(2)
        .iinc(1, 1)
        .go(expected[5])
        .label(expected[5])
        .iinc(1, 4)
        .go(expected[0])
        .new_label()
        .label(expected[6]) // Instantiation #2.
        .astore(2)
        .iinc(1, 1)
        .go(expected[7])
        .label(expected[7])
        .iinc(1, 4)
        .go(expected[3])
        .new_label()
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_interleaved_code() {
    let (inlined_method, expected_method) = test_inline_jsr_interleaved_code_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_interleaved_code_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_interleaved_code_methods().0,
    ));
}

/// Tests a nested try/finally with implicit exit from one subroutine to the other subroutine, and
/// with a surrounding try/catch thrown in the mix. Equivalent to the following java code:
///
/// ```java
/// void m(int b) {
///   try {
///     try {
///       return;
///     } finally {
///       while (b) {
///         try {
///           return;
///         } finally {
///           // NOTE --- this break avoids the second return above (weird)
///           if (b) {
///             break;
///           }
///         }
///       }
///     }
///   } catch (Exception e) {
///     b += 3;
///     return;
///   }
/// }
/// ```
fn test_inline_jsr_implicit_exit_in_try_catch_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 6)
        .iconst_0()
        .istore(1)
        .label(label[0]) // Outermost try.
        .label(label[1]) // First try.
        .jsr(label[3])
        .vreturn()
        .label(label[2]) // Exception handler for first try.
        .astore(LOCAL2)
        .jsr(label[3])
        .aload(LOCAL2)
        .athrow()
        .label(label[3]) // First finally handler.
        .astore(LOCAL4)
        .go(label[7])
        .label(label[4]) // Body of while loop, also second try.
        .jsr(label[6])
        .vreturn()
        .label(label[5]) // Exception handler for second try.
        .astore(LOCAL3)
        .jsr(label[6])
        .aload(LOCAL3)
        .athrow()
        .label(label[6]) // Second finally handler.
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label[8])
        .ret(LOCAL5)
        .label(label[7]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(label[4]) // Falls through.
        .label(label[8]) // Exit from finally block.
        .ret(LOCAL4)
        .label(label[9]) // Outermost catch.
        .iinc(LOCAL1, 3)
        .vreturn()
        .trycatch(label[1], label[2], label[2])
        .trycatch(label[4], label[5], label[5])
        .trycatch(label[0], label[9], label[9])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 6)
        // --- Main Subroutine ---
        .iconst_0()
        .istore(1)
        .label(expected[0]) // Outermost try / first try.
        .aconst_null()
        .go(expected[5])
        .label(expected[1])
        .vreturn()
        .label(expected[2]) // Exception handler for first try.
        .astore(LOCAL2)
        .aconst_null()
        .go(expected[13])
        .label(expected[3])
        .aload(LOCAL2)
        .athrow()
        .label(expected[4]) // Outermost catch.
        .iinc(LOCAL1, 3)
        .vreturn()
        // --- First instantiation of first subroutine ---
        .label(expected[5]) // First finally handler.
        .astore(LOCAL4)
        .go(expected[10])
        .label(expected[6]) // Body of while loop, also second try.
        .aconst_null()
        .go(expected[21])
        .label(expected[7])
        .vreturn()
        .label(expected[8]) // Exception handler for second try.
        .astore(LOCAL3)
        .aconst_null()
        .go(expected[23])
        .label(expected[9])
        .aload(LOCAL3)
        .athrow()
        .label(expected[10]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(expected[6]) // Falls through.
        .label(expected[11]) // Exit from finally block.
        .go(expected[1])
        .label(expected[12])
        // --- Second instantiation of first subroutine ---
        .label(expected[13]) // First finally handler.
        .astore(LOCAL4)
        .go(expected[18])
        .label(expected[14]) // Body of while loop, also second try.
        .aconst_null()
        .go(expected[25])
        .label(expected[15])
        .vreturn()
        .label(expected[16]) // Exception handler for second try.
        .astore(LOCAL3)
        .aconst_null()
        .go(expected[27])
        .label(expected[17])
        .aload(LOCAL3)
        .athrow()
        .label(expected[18]) // Test for the while loop.
        .iload(LOCAL1)
        .ifne(expected[14]) // Falls through.
        .label(expected[19]) // Exit from finally block.
        .go(expected[3])
        .label(expected[20])
        // --- Second subroutine's 4 instantiations ---
        .label(expected[21])
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[11])
        .go(expected[7])
        .label(expected[22])
        .label(expected[23])
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[11])
        .go(expected[9])
        .label(expected[24])
        .label(expected[25])
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[19])
        .go(expected[15])
        .label(expected[26])
        .label(expected[27])
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(expected[19])
        .go(expected[17])
        .label(expected[28])
        // Main subroutine handlers.
        .trycatch(expected[0], expected[2], expected[2])
        .trycatch(expected[0], expected[4], expected[4])
        // First instance of first subroutine try/catch handlers.
        // Note: reuses handler code from main subroutine.
        .trycatch(expected[6], expected[8], expected[8])
        .trycatch(expected[5], expected[12], expected[4])
        // Second instance of first sub try/catch handlers.
        .trycatch(expected[14], expected[16], expected[16])
        .trycatch(expected[13], expected[20], expected[4])
        // All 4 instances of second subroutine.
        .trycatch(expected[21], expected[22], expected[4])
        .trycatch(expected[23], expected[24], expected[4])
        .trycatch(expected[25], expected[26], expected[4])
        .trycatch(expected[27], expected[28], expected[4])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_implicit_exit_in_try_catch() {
    let (inlined_method, expected_method) = test_inline_jsr_implicit_exit_in_try_catch_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_implicit_exit_in_try_catch_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_implicit_exit_in_try_catch_methods().0,
    ));
}

/// Tests an example coming from distilled down version of
/// com/sun/corba/ee/impl/protocol/CorbaClientDelegateImpl from GlassFish 2. See issue #317823.
fn test_inline_jsr_glass_fish2_corba_client_delegate_impl_example_methods()
-> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(3, 3)
        .label(label[0])
        .jsr(label[4])
        .label(label[1])
        .go(label[5])
        .label(label[2])
        .pop()
        .jsr(label[4])
        .label(label[3])
        .aconst_null()
        .athrow()
        .label(label[4])
        .astore(1)
        .ret(1)
        .label(label[5])
        .aconst_null()
        .aconst_null()
        .aconst_null()
        .pop()
        .pop()
        .pop()
        .label(label[6])
        .go(label[8])
        .label(label[7])
        .pop()
        .go(label[8])
        .aconst_null()
        .athrow()
        .label(label[8])
        .iconst_0()
        .ifne(label[0])
        .jsr(label[12])
        .label(label[9])
        .vreturn()
        .label(label[10])
        .pop()
        .jsr(label[12])
        .label(label[11])
        .aconst_null()
        .athrow()
        .label(label[12])
        .astore(2)
        .ret(2)
        .trycatch(label[0], label[1], label[2])
        .trycatch(label[2], label[3], label[2])
        .trycatch(label[0], label[6], label[7])
        .trycatch(label[0], label[9], label[10])
        .trycatch(label[10], label[11], label[10])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(3, 3)
        // --- Main Subroutine ---
        .label(expected[0])
        .aconst_null()
        .go(expected[15])
        .label(expected[1])
        .label(expected[2])
        .go(expected[6])
        .label(expected[3])
        .pop()
        .aconst_null()
        .go(expected[17])
        .label(expected[4])
        .label(expected[5])
        .aconst_null()
        .athrow()
        .label(expected[6])
        .aconst_null()
        .aconst_null()
        .aconst_null()
        .pop()
        .pop()
        .pop()
        .label(expected[7])
        .go(expected[9])
        .label(expected[8])
        .pop()
        .go(expected[9])
        // [some dead code skipped here]
        .label(expected[9])
        .iconst_0()
        .ifne(expected[0])
        .aconst_null()
        .go(expected[19])
        .label(expected[10])
        .label(expected[11])
        .vreturn()
        .label(expected[12])
        .pop()
        .aconst_null()
        .go(expected[20])
        .label(expected[13])
        .label(expected[14])
        .aconst_null()
        .athrow()
        // --- First instantiation of first subroutine ---
        .new_label()
        .label(expected[15])
        .astore(1)
        .go(expected[1])
        .label(expected[16])
        // --- Second instantiation of first subroutine ---
        .label(expected[17])
        .astore(1)
        .go(expected[4])
        .label(expected[18])
        // --- First instantiation of second subroutine ---
        .label(expected[19])
        .astore(2)
        .go(expected[10])
        // --- Second instantiation of second subroutine ---
        .label(expected[20])
        .astore(2)
        .go(expected[13])
        .trycatch(expected[0], expected[2], expected[3])
        .trycatch(expected[3], expected[5], expected[3])
        .trycatch(expected[0], expected[7], expected[8])
        .trycatch(expected[0], expected[11], expected[12])
        .trycatch(expected[12], expected[14], expected[12])
        .trycatch(expected[15], expected[16], expected[8])
        .trycatch(expected[15], expected[16], expected[12])
        .trycatch(expected[17], expected[18], expected[8])
        .trycatch(expected[17], expected[18], expected[12])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_glass_fish2_corba_client_delegate_impl_example() {
    let (inlined_method, expected_method) =
        test_inline_jsr_glass_fish2_corba_client_delegate_impl_example_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_glass_fish2_corba_client_delegate_impl_example_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_glass_fish2_corba_client_delegate_impl_example_methods().0,
    ));
}

/// Tests a method which has line numbers and local variable declarations. More specifically:
///
/// ```java
///   public void a() {
/// 1   int a = 0;
/// 2   try {
/// 3     a++;
/// 4   } finally {
/// 5     a--;
/// 6   }
///   }
///   LV "a" from 1 to 6
/// ```
fn test_inline_jsr_basic_line_number_and_local_vars_methods() -> (MethodNode, MethodNode) {
    let label: [Label; 13] = labels();
    let expected: [Label; 29] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .label(label[0])
        .line(1, label[0])
        .iconst_0()
        .istore(1)
        .label(label[1]) // Body of try block.
        .line(3, label[1])
        .iinc(1, 1)
        .go(label[4])
        .label(label[2]) // Exception handler.
        .astore(3)
        .jsr(label[3])
        .aload(3)
        .athrow()
        .label(label[3]) // Subroutine.
        .line(5, label[3])
        .astore(2)
        .iinc(1, -1)
        .ret(2)
        .label(label[4]) // Non-exceptional exit from try block.
        .jsr(label[3])
        .label(label[5])
        .vreturn()
        .trycatch(label[1], label[2], label[2])
        .trycatch(label[4], label[5], label[2])
        .localvar("a", "I", 1, label[0], label[5])
        .build();
    let inlined_method = inline(&input_method);

    let expected_method = MethodNodeBuilder::new(1, 4)
        .label(expected[0])
        .line(1, expected[0])
        .iconst_0()
        .istore(1)
        .label(expected[1]) // Try/catch block.
        .line(3, expected[1])
        .iinc(1, 1)
        .go(expected[4])
        .label(expected[2]) // Exception handler.
        .astore(3)
        .aconst_null()
        .go(expected[7])
        .label(expected[3])
        .aload(3)
        .athrow()
        .label(expected[4]) // On non-exceptional exit, try block leads here.
        .aconst_null()
        .go(expected[9])
        .label(expected[5])
        .label(expected[6])
        .vreturn()
        .label(expected[7]) // First instantiation of subroutine.
        .line(5, expected[7])
        .astore(2)
        .iinc(1, -1)
        .go(expected[3])
        .label(expected[8])
        .label(expected[9]) // Second instantiation of subroutine.
        .line(5, expected[9])
        .astore(2)
        .iinc(1, -1)
        .go(expected[5])
        .label(expected[10])
        .trycatch(expected[1], expected[2], expected[2])
        .trycatch(expected[4], expected[6], expected[2])
        .localvar("a", "I", 1, expected[0], expected[6])
        .localvar("a", "I", 1, expected[7], expected[8])
        .localvar("a", "I", 1, expected[9], expected[10])
        .build();
    (inlined_method, expected_method)
}

#[test]
fn test_inline_jsr_basic_line_number_and_local_vars() {
    let (inlined_method, expected_method) =
        test_inline_jsr_basic_line_number_and_local_vars_methods();
    assert_eq!(to_text(&expected_method), to_text(&inlined_method));
    build_class_with_method(&inlined_method);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_basic_line_number_and_local_vars_new_instance() {
    assert_new_instance(&build_class_with_method(
        &test_inline_jsr_basic_line_number_and_local_vars_methods().0,
    ));
}

/// ASM's `JsrInlinerClassAdapter`: inlines the subroutines of every method.
struct JsrInlinerClassAdapter<V>(V);

impl<V: ClassVisitor> ClassVisitor for JsrInlinerClassAdapter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let method_visitor = self
            .0
            .visit_method(access, name, descriptor, signature, exceptions);
        Some(Box::new(JsrInlinerAdapter::new(
            method_visitor,
            access,
            name,
            descriptor,
            signature,
            exceptions,
        )))
    }
}

/// A precompiled class, with the subroutines of its methods inlined, written with a
/// `ClassWriter(0)`.
fn inline_precompiled_class(class_parameter: PrecompiledClass) -> Vec<u8> {
    let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_reader
        .accept(
            &mut JsrInlinerClassAdapter(&mut class_writer),
            ParsingOptions::empty(),
        )
        .unwrap();
    class_writer.to_bytes().unwrap()
}

/// Tests that classes transformed with JsrInlinerAdapter can be written (and, in the ignored
/// twin below, loaded and instantiated).
#[test]
fn test_inline_jsr_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        inline_precompiled_class(class_parameter);
    }
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_inline_jsr_precompiled_class_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        assert_precompiled_new_instance(
            class_parameter,
            &inline_precompiled_class(class_parameter),
        );
    }
}

/// ASM's `newInstance` expectation: the JVM verifies the class, except for preview classes of
/// another JDK, which it rejects with an `UnsupportedClassVersionError`. ASM loads the class with
/// the test classes on the class path; here it is verified along with the T0 classes of its
/// package, and only its own result counts.
fn assert_precompiled_new_instance(class_parameter: PrecompiledClass, class_file: &[u8]) {
    let siblings: Vec<Vec<u8>> = match class_parameter.name().rsplit_once('.') {
        Some((package, _)) => std::fs::read_dir(oxasm_testkit::corpus_t0().join(package))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "class"))
            .map(|path| std::fs::read(path).unwrap())
            .filter(|bytes| bytes.as_slice() != class_parameter.bytes())
            .collect(),
        None => Vec::new(),
    };
    let mut classes = vec![class_file];
    classes.extend(siblings.iter().map(Vec::as_slice));
    // The class under test is C0.class, the first one checked.
    let result = match oxasm_testkit::jvm::verify_classes(&classes) {
        Err(error) if !error.contains("C0.class:") => Ok(()),
        result => result,
    };
    if class_parameter.is_preview() {
        let error = result.expect_err("a preview class of another JDK");
        assert!(
            error.contains("UnsupportedClassVersionError"),
            "{class_parameter}: {error}"
        );
    } else {
        result.unwrap_or_else(|e| panic!("{class_parameter}: {e}"));
    }
}

// -------------------------------------------------------------------------------------------------
// oxasm-specific tests
// -------------------------------------------------------------------------------------------------

/// Without a downstream visitor, the inlined code stays in the adapter's method node (ASM's
/// adapter *is* the method node).
#[test]
fn test_inline_jsr_without_method_visitor() {
    let (inlined_method, _) = test_inline_jsr_basic_try_finally_methods();
    let label: [Label; 5] = labels();
    let input_method = MethodNodeBuilder::new(1, 4)
        .iconst_0()
        .istore(1)
        .label(label[0])
        .iinc(1, 1)
        .go(label[3])
        .label(label[1])
        .astore(3)
        .jsr(label[2])
        .aload(3)
        .athrow()
        .label(label[2])
        .astore(2)
        .iinc(1, -1)
        .ret(2)
        .label(label[3])
        .jsr(label[2])
        .label(label[4])
        .vreturn()
        .trycatch(label[0], label[1], label[1])
        .trycatch(label[3], label[4], label[1])
        .build();
    let mut adapter = JsrInlinerAdapter::new(None, ACC_PUBLIC, jstr!("m"), jstr!("()V"), None, &[]);
    input_method.accept_method_visitor(&mut adapter);

    assert_eq!(to_text(&inlined_method), to_text(adapter.method_node()));
    assert_eq!(adapter.into_method_node().try_catch_blocks.len(), 2);
}

/// A method without JSR instructions is passed on unchanged.
#[test]
fn test_no_jsr() {
    let label: [Label; 2] = labels();
    let input_method = MethodNodeBuilder::new(1, 2)
        .label(label[0])
        .iload(1)
        .ifne(label[1])
        .iinc(1, 1)
        .label(label[1])
        .vreturn()
        .localvar("a", "I", 1, label[0], label[1])
        .build();

    let inlined_method = inline(&input_method);

    assert_eq!(to_text(&input_method), to_text(&inlined_method));
    assert_eq!(input_method.instructions, inlined_method.instructions);
}

/// ASM throws an `IllegalArgumentException` on a subroutine that invokes itself.
#[test]
fn test_recursive_subroutine() {
    let label: [Label; 1] = labels();
    let input_method = MethodNodeBuilder::new(1, 2)
        .jsr(label[0])
        .vreturn()
        .label(label[0])
        .astore(1)
        .jsr(label[0])
        .ret(1)
        .build();

    let message = oxasm_testkit::panic_message(|| inline(&input_method));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Recursive invocation of {2, 3, 4, 5}"
    );
}

/// ASM throws an `IllegalArgumentException` on a RET instruction of the main "subroutine".
#[test]
fn test_ret_not_owned_by_any_subroutine() {
    let label: [Label; 1] = labels();
    let input_method = MethodNodeBuilder::new(1, 2)
        .jsr(label[0])
        .ret(1)
        .label(label[0])
        .astore(1)
        .ret(1)
        .build();

    let message = oxasm_testkit::panic_message(|| inline(&input_method));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Instruction #1 is a RET not owned by any subroutine"
    );
}

/// A JSR to a label that is not in the code: ASM's `BitSet` throws on the label's index (-1).
#[test]
fn test_jsr_to_unvisited_label() {
    let input_method = MethodNodeBuilder::new(1, 2)
        .jsr(Label::new())
        .vreturn()
        .build();

    let message = oxasm_testkit::panic_message(|| inline(&input_method));

    assert_eq!(
        message,
        "java.lang.IndexOutOfBoundsException: bitIndex < 0: -1"
    );
}

/// The search of the instructions of a subroutine does not recurse on jumps (ASM's does, once
/// per jump of a chain), so a long chain of jumps does not overflow the stack.
#[test]
fn test_long_jump_chain() {
    const CHAIN: usize = 100_000;
    let label: Vec<Label> = (0..=CHAIN).map(|_| Label::new()).collect();
    let subroutine = Label::new();
    let mut builder = MethodNodeBuilder::new(1, 2)
        .jsr(subroutine)
        .vreturn()
        .label(subroutine)
        .astore(1);
    for &link in &label[..CHAIN] {
        builder = builder.go(link).label(link);
    }
    let input_method = builder.go(label[CHAIN]).label(label[CHAIN]).ret(1).build();

    let inlined_method = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || inline(&input_method))
        .unwrap()
        .join()
        .unwrap();

    // The JSR becomes ACONST_NULL, GOTO and a return label; the RET a GOTO.
    assert!(to_text(&inlined_method).contains("ACONST_NULL"));
    assert!(!to_text(&inlined_method).contains("JSR"));
}
