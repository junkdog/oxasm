//! Port of ASM's `AnalyzerTest` (`asm-analysis/src/test/java/org/objectweb/asm/tree/analysis/
//! AnalyzerTest.java`, tag `ASM_9_10_1`): unit tests for [`Analyzer`].
//!
//! The `assertDoesNotThrow(() -> MethodNodeBuilder.buildClassWithMethod(methodNode)
//! .newInstance())` checks (the JVM accepts the analyzed method) are split in two: the tests write
//! the class (`build_class_with_method`), and an ignored twin, `<test>_new_instance`, has the JVM
//! verify it (`cargo test -- --ignored`, which needs the oracle).

mod common;

use common::{MethodNodeBuilder, assert_new_instance, build_class_with_method};
use oxasm::opcodes::*;
use oxasm::{JavaStr, Label, Type, TypeSort, jstr};
use oxasm_analysis::{
    Analyzer, AnalyzerError, AnalyzerHooks, Frame, Frames, InsnRef, Interpreter, Value,
};
use oxasm_tree::MethodNode;

const CLASS_NAME: &JavaStr = jstr!("C");

// Some local variable numbers used in tests.
const LOCAL1: u16 = 1;
const LOCAL2: u16 = 2;
const LOCAL3: u16 = 3;
const LOCAL4: u16 = 4;
const LOCAL5: u16 = 5;

/// Labels used to generate test cases (ASM's `label0` ... `label12` fields, new for each test).
fn labels<const N: usize>() -> [Label; N] {
    std::array::from_fn(|_| Label::new())
}

/// The message of the error of a failed analysis of `method_node` with `analyzer`.
fn analyze_error<I: Interpreter, H: AnalyzerHooks<I::V>>(
    analyzer: &mut Analyzer<I, H>,
    method_node: &MethodNode,
) -> String {
    let error = analyzer
        .analyze(CLASS_NAME, method_node)
        .err()
        .expect("the analysis should fail");
    error.message.to_string()
}

// ASM's newFrame cannot be overridden in oxasm (there is no newFrame hook): the init hook, which
// is called in the same try block of ASM's analyze (the one wrapping RuntimeExceptions at
// instruction 0), tests the same wrapping behavior.
#[test]
fn test_analyze_runtime_exceptions() {
    struct NewFrameError;
    impl AnalyzerHooks<MockValue> for NewFrameError {
        fn init(
            &mut self,
            _owner: &JavaStr,
            _method: &MethodNode,
            _frames: &mut [Option<Frame<MockValue>>],
            _handlers: &[Vec<usize>],
        ) -> Result<(), AnalyzerError> {
            Err(AnalyzerError::runtime("newFrame error"))
        }
    }
    let method_node = MethodNodeBuilder::new().insn(NOP).vreturn().build();

    let mut analyzer = Analyzer::with_hooks(MockInterpreter, NewFrameError);
    let message = analyze_error(&mut analyzer, &method_node);

    assert!(message.contains("newFrame error"), "{message}");
}

#[test]
fn test_analyze_invalid_opcode() {
    // insn(-1) cannot be written with u8 opcodes: 255 is an unknown opcode too.
    let method_node = MethodNodeBuilder::new().insn(255).vreturn().build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal opcode 255"), "{message}");
}

#[test]
fn test_analyze_invalid_pop() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(POP)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of POP"), "{message}");
}

#[test]
fn test_analyze_invalid_pop2() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(POP2)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of POP2"), "{message}");
}

#[test]
fn test_analyze_invalid_dup() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(DUP)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP"), "{message}");
}

#[test]
fn test_analyze_invalid_dupx1() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(DUP_X1)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP_X1"), "{message}");
}

#[test]
fn test_analyze_invalid_dupx2() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(ICONST_0)
        .insn(DUP_X2)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP_X2"), "{message}");
}

#[test]
fn test_analyze_invalid_dup2() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(DUP2)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP2"), "{message}");
}

#[test]
fn test_analyze_invalid_dup2x1() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(ICONST_0)
        .insn(DUP2_X1)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP2_X1"), "{message}");
}

#[test]
fn test_analyze_invalid_dup2x2() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(ICONST_0)
        .insn(ICONST_0)
        .insn(DUP2_X2)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of DUP2_X2"), "{message}");
}

#[test]
fn test_analyze_invalid_swap() {
    let method_node = MethodNodeBuilder::new()
        .insn(LCONST_0)
        .insn(ICONST_0)
        .insn(SWAP)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Illegal use of SWAP"), "{message}");
}

#[test]
fn test_analyze_invalid_get_local() {
    let method_node = MethodNodeBuilder::new().aload(10).vreturn().build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Trying to get an inexistant local variable"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_set_local() {
    let method_node = MethodNodeBuilder::new()
        .aconst_null()
        .astore(10)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Trying to set an inexistant local variable"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_pop_from_empty_stack() {
    let method_node = MethodNodeBuilder::new().insn(POP).vreturn().build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Cannot pop operand off an empty stack."),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_push_on_full_stack() {
    let method_node = MethodNodeBuilder::with_maxs(3, 3)
        .iconst_0()
        .iconst_0()
        .iconst_0()
        .iconst_0()
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Insufficient maximum stack size."),
        "{message}"
    );
}

#[test]
fn test_analyze_inconsistent_stack_heights() {
    let if_label = Label::new();
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .ifne(if_label)
        .iconst_0()
        .label(if_label)
        .vreturn()
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(message.contains("Incompatible stack heights"), "{message}");
}

#[test]
fn test_analyze_invalid_ret() {
    let method_node = MethodNodeBuilder::new().ret(1).vreturn().build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("RET instruction outside of a subroutine"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_falloff_end_of_method() {
    let method_node = MethodNodeBuilder::new().build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Execution can fall off the end of the code"),
        "{message}"
    );
}

#[test]
fn test_analyze_invalid_falloff_subroutine() {
    let goto_label = Label::new();
    let jsr_label = Label::new();
    let method_node = MethodNodeBuilder::new()
        .go(goto_label)
        .label(jsr_label)
        .astore(1)
        .ret(1)
        .label(goto_label)
        .jsr(jsr_label)
        .build();

    let message = analyze_error(&mut new_analyzer(), &method_node);

    assert!(
        message.contains("Execution can fall off the end of the code"),
        "{message}"
    );
}

#[test]
#[ignore = "TODO currently Analyzer can not detect this situation"]
fn test_analyze_invalid_overlapping_subroutines() {
    // The problem is that other overlapping subroutine situations are valid, such as
    // when a nested subroutine implicitly returns to its parent subroutine, without a RET.
    let subroutine1_label = Label::new();
    let subroutine2_label = Label::new();
    let end_subroutine_label = Label::new();
    let method_node = MethodNodeBuilder::new()
        .jsr(subroutine1_label)
        .jsr(subroutine2_label)
        .vreturn()
        .label(subroutine1_label)
        .astore(1)
        .go(end_subroutine_label)
        .label(subroutine2_label)
        .astore(1)
        .label(end_subroutine_label)
        .ret(1)
        .build();

    let result = new_analyzer().analyze(CLASS_NAME, &method_node).map(|_| ());

    assert!(result.is_err());
}

/// Tests a method which has the most basic <code>try{}finally{}</code> form imaginable. That is:
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
fn test_analyze_basic_try_finally_method_node() -> MethodNode {
    let [label0, label1, label2, label3, label4] = labels();
    MethodNodeBuilder::with_maxs(4, 4)
        .iconst_0()
        .istore(1)
        // Body of try block.
        .label(label0)
        .iinc(1, 1)
        .go(label3)
        // Exception handler.
        .label(label1)
        .astore(3)
        .jsr(label2)
        .aload(3)
        .athrow()
        // Subroutine.
        .label(label2)
        .astore(2)
        .iinc(1, -1)
        .push()
        .push()
        .ret(2)
        // Non-exceptional exit from try block.
        .label(label3)
        .jsr(label2)
        .push()
        .push()
        .label(label4)
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label3, label4, label1)
        .build()
}

#[test]
fn test_analyze_basic_try_finally() {
    let method_node = test_analyze_basic_try_finally_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_basic_try_finally_new_instance() {
    assert_new_instance(&test_analyze_basic_try_finally_method_node());
}

/// Tests a method which has an if/else-if w/in the finally clause. More specifically:
///
/// ```java
/// public void a() {
///   int a = 0;
///   try {
///       a++;
///   } finally {
///     if (a == 0) {
///       a += 2;
///     } else {
///       a += 3;
///     }
///   }
/// }
/// ```
fn test_analyze_if_else_in_finally_method_node() -> MethodNode {
    let [label0, label1, label2, label3, label4, label5, label6] = labels();
    MethodNodeBuilder::with_maxs(5, 4)
        .iconst_0()
        .istore(1)
        // Body of try block.
        .label(label0)
        .iinc(1, 1)
        .go(label5)
        // Exception handler.
        .label(label1)
        .astore(3)
        .jsr(label2)
        .push()
        .push()
        .aload(3)
        .athrow()
        // Subroutine.
        .label(label2)
        .astore(2)
        .push()
        .push()
        .iload(1)
        .ifne(label3)
        .iinc(1, 2)
        .go(label4)
        .label(label3)
        .iinc(1, 3)
        .label(label4)
        .ret(2)
        // Non-exceptional exit from try block.
        .label(label5)
        .jsr(label2)
        .label(label6)
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label5, label6, label1)
        .build()
}

#[test]
fn test_analyze_if_else_in_finally() {
    let method_node = test_analyze_if_else_in_finally_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_if_else_in_finally_new_instance() {
    assert_new_instance(&test_analyze_if_else_in_finally_method_node());
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
fn test_analyze_simple_nested_finally_method_node() -> MethodNode {
    let [label0, label1, label2, label3, label4, label5] = labels();
    MethodNodeBuilder::with_maxs(5, 6)
        .iconst_0()
        .istore(1)
        // Body of try block.
        .label(label0)
        .iinc(1, 1)
        .jsr(label2)
        .go(label5)
        // First exception handler.
        .label(label1)
        .astore(4)
        .jsr(label2)
        .aload(4)
        .athrow()
        // First subroutine.
        .label(label2)
        .astore(2)
        .iinc(1, 2)
        .jsr(label4)
        .push()
        .push()
        .ret(2)
        // Second exception handler.
        .label(label3)
        .astore(5)
        .jsr(label4)
        .aload(5)
        .athrow()
        // Second subroutine.
        .label(label4)
        .astore(3)
        .push()
        .push()
        .iinc(1, 3)
        .ret(3)
        // On normal exit, try block jumps here.
        .label(label5)
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label2, label3, label3)
        .build()
}

#[test]
fn test_analyze_simple_nested_finally() {
    let method_node = test_analyze_simple_nested_finally_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_simple_nested_finally_new_instance() {
    assert_new_instance(&test_analyze_simple_nested_finally_method_node());
}

fn test_analyze_nested_subroutines_method_node() -> MethodNode {
    let subroutine1_label = Label::new();
    let subroutine2_label = Label::new();
    MethodNodeBuilder::with_maxs(1, 3)
        .jsr(subroutine1_label)
        .vreturn()
        .label(subroutine1_label)
        .astore(1)
        .jsr(subroutine2_label)
        .jsr(subroutine2_label)
        .ret(1)
        .label(subroutine2_label)
        .astore(2)
        .ret(2)
        .build()
}

#[test]
fn test_analyze_nested_subroutines() {
    let method_node = test_analyze_nested_subroutines_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_nested_subroutines_new_instance() {
    assert_new_instance(&test_analyze_nested_subroutines_method_node());
}

/// This tests a subroutine which has no ret statement, but ends in a "return" instead.
///
/// We structure this as a try/finally with a break in the finally. Because the while loop is
/// infinite, it's clear from the byte code that the only path which reaches the RETURN
/// instruction is through the subroutine.
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
fn test_analyze_subroutine_with_no_ret_method_node() -> MethodNode {
    let [label0, label1, label2, label3, label4] = labels();
    MethodNodeBuilder::with_maxs(1, 4)
        .iconst_0()
        .istore(1)
        // While loop header/try block.
        .label(label0)
        .iinc(1, 1)
        .jsr(label2)
        .go(label3)
        // Implicit catch block.
        .label(label1)
        .astore(2)
        .jsr(label2)
        .push()
        .push()
        .aload(2)
        .athrow()
        // Subroutine which does not return.
        .label(label2)
        .astore(3)
        .iinc(1, 2)
        .go(label4)
        // End of the loop, goes back to the top.
        .label(label3)
        .go(label0)
        .label(label4)
        .vreturn()
        .trycatch(label0, label1, label1)
        .build()
}

#[test]
fn test_analyze_subroutine_with_no_ret() {
    let method_node = test_analyze_subroutine_with_no_ret_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_subroutine_with_no_ret_new_instance() {
    assert_new_instance(&test_analyze_subroutine_with_no_ret_method_node());
}

/// This tests a subroutine which has no ret statement, but ends in a "return" instead.
///
/// ```text
///   aconst_null
///   jsr l0
/// l0:
///   astore 0
///   astore 0
///   return
/// ```
fn test_analyze_subroutine_with_no_ret2_method_node() -> MethodNode {
    let [label0, label1] = labels();
    MethodNodeBuilder::with_maxs(2, 2)
        .aconst_null()
        .jsr(label0)
        .nop()
        .label(label0)
        .astore(0)
        .astore(0)
        .vreturn()
        .label(label1)
        .local_variable("i", "I", None, label0, label1, 1)
        .build()
}

#[test]
fn test_analyze_subroutine_with_no_ret2() {
    let method_node = test_analyze_subroutine_with_no_ret2_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_subroutine_with_no_ret2_new_instance() {
    assert_new_instance(&test_analyze_subroutine_with_no_ret2_method_node());
}

fn test_analyze_subroutine_locals_access_method_node() -> MethodNode {
    let start_label = Label::new();
    let exception_handler1_label = Label::new();
    let exception_handler2_label = Label::new();
    let subroutine_label = Label::new();
    MethodNodeBuilder::with_maxs(1, 5)
        .label(start_label)
        .jsr(subroutine_label)
        .vreturn()
        .label(exception_handler1_label)
        .astore(1)
        .jsr(subroutine_label)
        .aload(1)
        .athrow()
        .label(subroutine_label)
        .astore(2)
        .aconst_null()
        .astore(3)
        .ret(2)
        .label(exception_handler2_label)
        .astore(4)
        .aload(4)
        .athrow()
        .trycatch(
            start_label,
            exception_handler1_label,
            exception_handler1_label,
        )
        .trycatch_type(
            start_label,
            exception_handler2_label,
            exception_handler2_label,
            Some("java/lang/RuntimeException"),
        )
        .build()
}

#[test]
fn test_analyze_subroutine_locals_access() {
    let method_node = test_analyze_subroutine_locals_access_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_subroutine_locals_access_new_instance() {
    assert_new_instance(&test_analyze_subroutine_locals_access_method_node());
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
fn test_analyze_implicit_exit_method_node() -> MethodNode {
    let [label0, label1, label2, label3, label4, label5] = labels();
    MethodNodeBuilder::with_maxs(1, 4)
        .iconst_0()
        .istore(1)
        // While loop header.
        .label(label0)
        .aconst_null()
        .ifnonnull(label5)
        // Try block.
        .label(label1)
        .iinc(1, 1)
        .jsr(label3)
        .go(label4)
        // Implicit catch block.
        .label(label2)
        .astore(2)
        .jsr(label3)
        .aload(2)
        .push()
        .push()
        .athrow()
        // Subroutine which does not return.
        .label(label3)
        .astore(3)
        .iinc(1, 2)
        .go(label5)
        // End of the loop, goes back to the top.
        .label(label4)
        .go(label1)
        .label(label5)
        .vreturn()
        .trycatch(label1, label2, label2)
        .build()
}

#[test]
fn test_analyze_implicit_exit() {
    let method_node = test_analyze_implicit_exit_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_implicit_exit_new_instance() {
    assert_new_instance(&test_analyze_implicit_exit_method_node());
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
fn test_analyze_implicit_exit_to_another_subroutine_method_node() -> MethodNode {
    let [
        label0,
        label1,
        label2,
        label3,
        label4,
        label5,
        label6,
        label7,
    ] = labels();
    MethodNodeBuilder::with_maxs(5, 6)
        .iconst_0()
        .istore(1)
        // First try.
        .label(label0)
        .jsr(label2)
        .vreturn()
        // Exception handler for first try.
        .label(label1)
        .astore(LOCAL2)
        .jsr(label2)
        .push()
        .push()
        .aload(LOCAL2)
        .athrow()
        // First finally handler.
        .label(label2)
        .astore(LOCAL4)
        .push()
        .push()
        .go(label6)
        // Body of while loop, also second try.
        .label(label3)
        .jsr(label5)
        .vreturn()
        // Exception handler for second try.
        .label(label4)
        .astore(LOCAL3)
        .push()
        .push()
        .jsr(label5)
        .aload(LOCAL3)
        .athrow()
        // Second finally handler.
        .label(label5)
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label7)
        .ret(LOCAL5)
        // Test for the while loop.
        .label(label6)
        .iload(LOCAL1)
        .ifne(label3)
        // Exit from finally block.
        .label(label7)
        .ret(LOCAL4)
        .trycatch(label0, label1, label1)
        .trycatch(label3, label4, label4)
        .build()
}

#[test]
fn test_analyze_implicit_exit_to_another_subroutine() {
    let method_node = test_analyze_implicit_exit_to_another_subroutine_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_implicit_exit_to_another_subroutine_new_instance() {
    assert_new_instance(&test_analyze_implicit_exit_to_another_subroutine_method_node());
}

// Sic: the Java name is testanalyze_implicitExitToAnotherSubroutine2 (lowercase 'a').
fn testanalyze_implicit_exit_to_another_subroutine2_method_node() -> MethodNode {
    let [label0, label1, label2] = labels();
    MethodNodeBuilder::with_maxs(1, 4)
        .iconst_0()
        .istore(1)
        .jsr(label0)
        .vreturn()
        .label(label0)
        .astore(2)
        .jsr(label1)
        .go(label2)
        .label(label1)
        .astore(3)
        .iload(1)
        .ifne(label2)
        .ret(3)
        .label(label2)
        .ret(2)
        .build()
}

#[test]
fn testanalyze_implicit_exit_to_another_subroutine2() {
    let method_node = testanalyze_implicit_exit_to_another_subroutine2_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_implicit_exit_to_another_subroutine2_new_instance() {
    assert_new_instance(&testanalyze_implicit_exit_to_another_subroutine2_method_node());
}

/// This tests a simple subroutine where the control flow jumps back and forth between the
/// subroutine and the caller.
///
/// This would not normally be produced by a java compiler.
fn test_analyze_interleaved_code_method_node() -> MethodNode {
    let [label0, label1, label2, label3] = labels();
    MethodNodeBuilder::with_maxs(4, 3)
        .iconst_0()
        .istore(1)
        .jsr(label0)
        .go(label1)
        // Subroutine 1.
        .label(label0)
        .astore(2)
        .iinc(1, 1)
        .go(label2)
        // Second part of main subroutine.
        .label(label1)
        .iinc(1, 2)
        .go(label3)
        // Second part of subroutine 1.
        .label(label2)
        .iinc(1, 4)
        .push()
        .push()
        .ret(2)
        // Third part of main subroutine.
        .label(label3)
        .push()
        .push()
        .vreturn()
        .build()
}

#[test]
fn test_analyze_interleaved_code() {
    let method_node = test_analyze_interleaved_code_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_interleaved_code_new_instance() {
    assert_new_instance(&test_analyze_interleaved_code_method_node());
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
fn test_analyze_implicit_exit_in_try_catch_method_node() -> MethodNode {
    let [
        label0,
        label1,
        label2,
        label3,
        label4,
        label5,
        label6,
        label7,
        label8,
    ] = labels();
    MethodNodeBuilder::with_maxs(4, 6)
        .iconst_0()
        .istore(1)
        // First try.
        .label(label0)
        .jsr(label2)
        .vreturn()
        // Exception handler for first try.
        .label(label1)
        .astore(LOCAL2)
        .jsr(label2)
        .aload(LOCAL2)
        .athrow()
        // First finally handler.
        .label(label2)
        .astore(LOCAL4)
        .go(label6)
        // Body of while loop, also second try.
        .label(label3)
        .jsr(label5)
        .push()
        .push()
        .vreturn()
        // Exception handler for second try.
        .label(label4)
        .astore(LOCAL3)
        .jsr(label5)
        .aload(LOCAL3)
        .athrow()
        // Second finally handler.
        .label(label5)
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label7)
        .push()
        .push()
        .ret(LOCAL5)
        // Test for the while loop.
        .label(label6)
        .iload(LOCAL1)
        .ifne(label3)
        // Exit from finally{} block.
        .label(label7)
        .ret(LOCAL4)
        // Outermost catch.
        .label(label8)
        .iinc(LOCAL1, 3)
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label3, label4, label4)
        .trycatch(label0, label8, label8)
        .build()
}

#[test]
fn test_analyze_implicit_exit_in_try_catch() {
    let method_node = test_analyze_implicit_exit_in_try_catch_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_implicit_exit_in_try_catch_new_instance() {
    assert_new_instance(&test_analyze_implicit_exit_in_try_catch_method_node());
}

/// Tests that Analyzer works correctly on classes with many labels.
fn test_analyze_many_labels_method_node() -> MethodNode {
    let target = Label::new();
    let mut method_node_builder = MethodNodeBuilder::with_maxs(1, 1).jsr(target).label(target);
    for _ in 0..8192 {
        let label = Label::new();
        method_node_builder = method_node_builder.go(label).label(label);
    }
    method_node_builder.vreturn().build()
}

#[test]
fn test_analyze_many_labels() {
    let method_node = test_analyze_many_labels_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_many_labels_new_instance() {
    assert_new_instance(&test_analyze_many_labels_method_node());
}

/// Tests an example coming from distilled down version of
/// com/sun/corba/ee/impl/protocol/CorbaClientDelegateImpl from GlassFish 2. See issue #317823.
fn test_analyze_glass_fish2_corba_client_delegate_impl_example_method_node() -> MethodNode {
    let [
        label0,
        label1,
        label2,
        label3,
        label4,
        label5,
        label6,
        label7,
        label8,
        label9,
        label10,
        label11,
        label12,
    ] = labels();
    MethodNodeBuilder::with_maxs(3, 3)
        .label(label0)
        .jsr(label4)
        .label(label1)
        .go(label5)
        .label(label2)
        .pop()
        .jsr(label4)
        .label(label3)
        .aconst_null()
        .athrow()
        .label(label4)
        .astore(1)
        .ret(1)
        .label(label5)
        .aconst_null()
        .aconst_null()
        .aconst_null()
        .pop()
        .pop()
        .pop()
        .label(label6)
        .go(label8)
        .label(label7)
        .pop()
        .go(label8)
        .aconst_null()
        .athrow()
        .label(label8)
        .iconst_0()
        .ifne(label0)
        .jsr(label12)
        .label(label9)
        .vreturn()
        .label(label10)
        .pop()
        .jsr(label12)
        .label(label11)
        .aconst_null()
        .athrow()
        .label(label12)
        .astore(2)
        .ret(2)
        .trycatch(label0, label1, label2)
        .trycatch(label2, label3, label2)
        .trycatch(label0, label6, label7)
        .trycatch(label0, label9, label10)
        .trycatch(label10, label11, label10)
        .build()
}

#[test]
fn test_analyze_glass_fish2_corba_client_delegate_impl_example() {
    let method_node = test_analyze_glass_fish2_corba_client_delegate_impl_example_method_node();

    assert_maxs_from_frames(&method_node);
    // ASM: assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance()): the class
    // is written here, and verified by the JVM in the ignored test below.
    build_class_with_method(&method_node);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_analyze_glass_fish2_corba_client_delegate_impl_example_new_instance() {
    assert_new_instance(&test_analyze_glass_fish2_corba_client_delegate_impl_example_method_node());
}

fn new_analyzer() -> Analyzer<MockInterpreter> {
    Analyzer::new(MockInterpreter)
}

/// What each successful test of ASM does with the frames: `analyze`, then check that
/// `computeMaxStackAndLocalsFromFrames` gives the method's `maxStack` and `maxLocals`.
fn assert_maxs_from_frames(method_node: &MethodNode) {
    let mut analyzer = new_analyzer();
    let frames = analyzer
        .analyze(CLASS_NAME, method_node)
        .unwrap_or_else(|error| panic!("{error}"));

    let method_maxs = compute_max_stack_and_locals_from_frames(frames);
    assert_eq!(usize::from(method_node.max_stack), method_maxs.max_stack);
    assert_eq!(usize::from(method_node.max_locals), method_maxs.max_locals);
}

fn compute_max_stack_and_locals_from_frames<V: Value>(frames: &Frames<V>) -> MethodMaxs {
    let mut max_stack = 0;
    let mut max_locals = 0;
    for frame in frames.iter().flatten() {
        let mut stack_size = 0;
        for i in 0..frame.get_stack_size() {
            stack_size += frame.get_stack(i).get_size();
        }
        max_stack = max_stack.max(stack_size);
        max_locals = max_locals.max(frame.get_locals());
    }
    MethodMaxs {
        max_stack,
        max_locals,
    }
}

struct MethodMaxs {
    max_stack: usize,
    max_locals: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MockValue {
    Int,
    Long,
    Reference,
    ReturnAddress,
    Top,
}

impl Value for MockValue {
    fn get_size(&self) -> usize {
        if *self == MockValue::Long { 2 } else { 1 }
    }
}

/// ASM's `MockInterpreter` (its `ASM10_EXPERIMENTAL` api version has no counterpart). Its
/// `UnsupportedOperationException`s are runtime errors.
struct MockInterpreter;

fn unsupported() -> AnalyzerError {
    AnalyzerError::runtime("java.lang.UnsupportedOperationException")
}

impl Interpreter for MockInterpreter {
    type V = MockValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<MockValue>, AnalyzerError> {
        let Some(ty) = ty else {
            return Ok(Some(MockValue::Top));
        };
        match ty.get_sort() {
            TypeSort::Void => Ok(None),
            TypeSort::Int => Ok(Some(MockValue::Int)),
            // Sic: ASM's MockInterpreter returns INT for LONG.
            TypeSort::Long => Ok(Some(MockValue::Int)),
            TypeSort::Object => Ok(Some(MockValue::Reference)),
            _ => Err(unsupported()),
        }
    }

    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<MockValue>, AnalyzerError> {
        match insn.opcode() {
            Some(ACONST_NULL) => Ok(Some(MockValue::Reference)),
            Some(ICONST_0) => Ok(Some(MockValue::Int)),
            Some(JSR) => Ok(Some(MockValue::ReturnAddress)),
            Some(LCONST_0) => Ok(Some(MockValue::Long)),
            _ => Err(unsupported()),
        }
    }

    fn copy_operation(
        &mut self,
        _insn: InsnRef<'_>,
        value: &MockValue,
    ) -> Result<MockValue, AnalyzerError> {
        Ok(*value)
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        _value: &MockValue,
    ) -> Result<Option<MockValue>, AnalyzerError> {
        match insn.opcode() {
            Some(IFNE | IFNONNULL | ATHROW) => Ok(None),
            Some(IINC) => Ok(Some(MockValue::Int)),
            Some(NEWARRAY) => Ok(Some(MockValue::Reference)),
            _ => Err(unsupported()),
        }
    }

    fn binary_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value1: &MockValue,
        _value2: &MockValue,
    ) -> Result<Option<MockValue>, AnalyzerError> {
        Err(unsupported())
    }

    fn ternary_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value1: &MockValue,
        _value2: &MockValue,
        _value3: &MockValue,
    ) -> Result<Option<MockValue>, AnalyzerError> {
        Err(unsupported())
    }

    fn nary_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _values: &[MockValue],
    ) -> Result<Option<MockValue>, AnalyzerError> {
        Err(unsupported())
    }

    fn return_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value: &MockValue,
        _expected: Option<&MockValue>,
    ) -> Result<(), AnalyzerError> {
        // Nothing to do.
        Ok(())
    }

    fn merge(
        &mut self,
        value1: &MockValue,
        value2: &MockValue,
    ) -> Result<Option<MockValue>, AnalyzerError> {
        if value1 != value2 {
            return Ok(Some(MockValue::Top));
        }
        // value1.
        Ok(None)
    }
}
