//! Port of ASM's `ClassWriterComputeMaxsTest`: `COMPUTE_MAXS` with JSR instructions.
//!
//! - ASM's `TestCaseBuilder` holds a method writer while the test adds instructions; here it
//!   records them, and [`TestCaseBuilder::build`] replays them into a class writer (with the
//!   constructor and method `m` of ASM's builder, then `visitMaxs(0, 0)`).
//! - ASM's `new ClassFile(bytes).newInstance()` checks run the oracle's `verify`
//!   (`oxasm_testkit::jvm`), for all the test cases at once, in
//!   [`test_visit_maxs_new_instance`], which is `#[ignore = "needs the JVM oracle"]`.
//! - ASM reads the control flow graph from package-private fields (`Label.outgoingEdges`,
//!   `Label.nextBasicBlock`, `Label.flags`); oxasm's method writer keeps them in a crate-private
//!   side table (ADR-03), which the hidden `ClassWriter::last_method_control_flow_graph` reads.

use std::collections::{BTreeMap, BTreeSet};

use oxasm::opcodes::{
    ACC_PUBLIC, ACONST_NULL, ALOAD, ASTORE, ATHROW, GOTO, ICONST_0, IFNE, IFNONNULL, ILOAD,
    INVOKESPECIAL, ISTORE, JSR, LCONST_0, NOP, POP, RET, RETURN, V1_1, V1_7,
};
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, FrameKind, FrameValue, JavaStr,
    Label, MethodVisitor, ParsingOptions, jstr,
};

// Some local variable numbers used in tests.
const LOCAL1: u16 = 1;
const LOCAL2: u16 = 2;
const LOCAL3: u16 = 3;
const LOCAL4: u16 = 4;
const LOCAL5: u16 = 5;

/// The labels used to generate test cases (ASM's `label0` ... `label12` fields).
fn labels() -> [Label; 13] {
    std::array::from_fn(|_| Label::new())
}

/// Tests a method which has the most basic `try{}finally{}` form imaginable. That is, repeated
/// one or more times:
///
/// ```java
/// public void a() {
///   int a = 0;
///   try {
///     a++;
///   } finally {
///     a--;
///   }
///   // ... same try {} finally {} repeated 0 or more times ...
/// }
/// ```
#[test]
fn test_visit_maxs_basic() {
    for num_subroutines in [1, 31, 32, 33] {
        let (class_file, graph) = visit_maxs_basic(num_subroutines).build_and_graph();

        assert_eq!(
            read_max_stack_and_locals(&class_file),
            (4, 4),
            "{num_subroutines}"
        );
        if num_subroutines == 1 {
            let expected_control_flow_graph = control_flow_graph(&[
                "N0=N2",
                "N2=N22,N8",
                "N8=N14",
                "N12=",
                "N14=N12,N25",
                "N22=N14,N8",
                "N25=N27,N8",
                "N27=",
            ]);
            assert_eq!(graph, expected_control_flow_graph);
        }
    }
}

fn visit_maxs_basic(num_subroutines: usize) -> TestCaseBuilder {
    let mut test_case = TestCaseBuilder::new();
    for _ in 0..num_subroutines {
        let [k0, k1, k2, k3, k4] = std::array::from_fn(|_| Label::new());
        test_case
            .iconst_0() // N0
            .istore(1)
            // Body of try block.
            .label(k0) // N2
            .iinc(1, 1)
            .go(k3)
            // Exception handler.
            .label(k1) // N8
            .astore(3)
            .jsr(k2)
            .aload(3) // N12
            .athrow()
            // Subroutine.
            .label(k2) // N14
            .astore(2)
            .iinc(1, -1)
            .push()
            .push()
            .ret(2)
            // Non-exceptional exit from try block.
            .label(k3) // N22
            .jsr(k2)
            .push() // N25
            .push()
            .label(k4) // N27
            .vreturn()
            .trycatch(k0, k1, k1)
            .trycatch(k3, k4, k1);
    }
    test_case
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
#[test]
fn test_visit_maxs_if_else_in_finally() {
    let (class_file, graph) = visit_maxs_if_else_in_finally().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (5, 4));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N34,N8",
        "N8=N16",
        "N12=",
        "N16=N29,N32",
        "N29=N32",
        "N32=N37,N12",
        "N34=N16,N8",
        "N37=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_if_else_in_finally() -> TestCaseBuilder {
    let [label0, label1, label2, label3, label4, label5, label6, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // Body of try block.
        .label(label0) // N2
        .iinc(1, 1)
        .go(label5)
        // Exception handler.
        .label(label1) // N8
        .astore(3)
        .jsr(label2)
        .push() // N12
        .push()
        .aload(3)
        .athrow()
        // Subroutine.
        .label(label2) // N16
        .astore(2)
        .push()
        .push()
        .iload(1)
        .ifne(label3)
        .iinc(1, 2)
        .go(label4)
        .label(label3) // N29
        .iinc(1, 3)
        .label(label4) // N32, common exit.
        .ret(2)
        // Non-exceptional exit from try block.
        .label(label5) // N34
        .jsr(label2)
        .label(label6) // N37
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label5, label6, label1);
    test_case
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
#[test]
fn test_visit_maxs_simple_nested_finally() {
    let (class_file, graph) = visit_maxs_simple_nested_finally().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (5, 6));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N11,N19",
        "N8=N11,N46",
        "N11=N19",
        "N16=",
        "N19=N30,N38",
        "N26=N16,N30,N8",
        "N30=N38",
        "N35=",
        "N38=N26,N35",
        "N46=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_simple_nested_finally() -> TestCaseBuilder {
    let [label0, label1, label2, label3, label4, label5, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // Body of try block.
        .label(label0) // N2
        .iinc(1, 1)
        .jsr(label2)
        .go(label5) // N8
        // First exception handler.
        .label(label1) // N11
        .astore(4)
        .jsr(label2)
        .aload(4) // N16
        .athrow()
        // First subroutine.
        .label(label2) // N19
        .astore(2)
        .iinc(1, 2)
        .jsr(label4)
        .push() // N26
        .push()
        .ret(2)
        // Second exception handler.
        .label(label3) // N30
        .astore(5)
        .jsr(label4)
        .aload(5) // N35
        .athrow()
        // Second subroutine.
        .label(label4) // N38
        .astore(3)
        .push()
        .push()
        .iinc(1, 3)
        .ret(3)
        // On normal exit, try block jumps here.
        .label(label5) // N46
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label2, label3, label3);
    test_case
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
#[test]
fn test_visit_maxs_subroutine_with_no_ret() {
    let (class_file, graph) = visit_maxs_subroutine_with_no_ret().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (1, 4));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N11,N19",
        "N8=N11,N26",
        "N11=N19",
        "N15=",
        "N19=N29",
        "N26=N2",
        "N29=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_subroutine_with_no_ret() -> TestCaseBuilder {
    let [label0, label1, label2, label3, label4, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // While loop header/try block.
        .label(label0) // N2
        .iinc(1, 1)
        .jsr(label2)
        .go(label3) // N8
        // Implicit catch block.
        .label(label1) // N11
        .astore(2)
        .jsr(label2)
        .push() // N15
        .push()
        .aload(2)
        .athrow()
        // Subroutine which does not return.
        .label(label2) // N19
        .astore(3)
        .iinc(1, 2)
        .go(label4)
        // End of the loop, goes back to the top.
        .label(label3) // N26
        .go(label0)
        .label(label4) // N29
        .vreturn()
        .trycatch(label0, label1, label1);
    test_case
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
#[test]
fn test_visit_maxs_subroutine_with_no_ret2() {
    let (class_file, graph) = visit_maxs_subroutine_with_no_ret2().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (2, 2));
    let expected_control_flow_graph = control_flow_graph(&["N0=N5", "N4=N5", "N5=", "N8="]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_subroutine_with_no_ret2() -> TestCaseBuilder {
    let [label0, label1, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .aconst_null() // N0
        .jsr(label0)
        .nop() // N4
        .label(label0) // N5
        .astore(0)
        .astore(0)
        .vreturn()
        .label(label1) // N8
        .local_variable("i", "I", None, label0, label1, 1);
    test_case
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
#[test]
fn test_visit_maxs_implicit_exit() {
    let (class_file, graph) = visit_maxs_implicit_exit().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (1, 4));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N6,N33",
        "N6=N23,N15",
        "N12=N30,N15",
        "N15=N23",
        "N19=",
        "N23=N33",
        "N30=N6",
        "N33=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_implicit_exit() -> TestCaseBuilder {
    let [label0, label1, label2, label3, label4, label5, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // While loop header.
        .label(label0) // N2
        .aconst_null()
        .ifnonnull(label5)
        // Try block.
        .label(label1) // N6
        .iinc(1, 1)
        .jsr(label3)
        .go(label4) // N12
        // Implicit catch block.
        .label(label2) // N15
        .astore(2)
        .jsr(label3)
        .aload(2) // N19
        .push()
        .push()
        .athrow()
        // Subroutine which does not return.
        .label(label3) // N23
        .astore(3)
        .iinc(1, 2)
        .go(label5)
        // End of the loop, goes back to the top.
        .label(label4) // N30
        .go(label1)
        .label(label5) // N33
        .vreturn()
        .trycatch(label1, label2, label2);
    test_case
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
#[test]
fn test_visit_maxs_implicit_exit_to_another_subroutine() {
    let (class_file, graph) = visit_maxs_implicit_exit_to_another_subroutine().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (5, 6));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N6,N14",
        "N5=N6",
        "N6=N14",
        "N10=",
        "N14=N41",
        "N21=N25,N33",
        "N24=N25",
        "N25=N33",
        "N31=",
        "N33=N31,N45,N24",
        "N41=N45,N21",
        "N45=N5,N10",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_implicit_exit_to_another_subroutine() -> TestCaseBuilder {
    let [
        label0,
        label1,
        label2,
        label3,
        label4,
        label5,
        label6,
        label7,
        ..,
    ] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // First try.
        .label(label0) // N2
        .jsr(label2)
        .vreturn() // N5
        // Exception handler for first try.
        .label(label1) // N6
        .astore(LOCAL2)
        .jsr(label2)
        .push() // N10
        .push()
        .aload(LOCAL2)
        .athrow()
        // First finally handler.
        .label(label2) // N14
        .astore(LOCAL4)
        .push()
        .push()
        .go(label6)
        // Body of while loop, also second try.
        .label(label3) // N21
        .jsr(label5)
        .vreturn() // N24
        // Exception handler for second try.
        .label(label4) // N25
        .astore(LOCAL3)
        .push()
        .push()
        .jsr(label5)
        .aload(LOCAL3) // N31
        .athrow()
        // Second finally handler.
        .label(label5) // N33
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label7)
        .ret(LOCAL5)
        // Test for the while loop.
        .label(label6) // N41
        .iload(LOCAL1)
        .ifne(label3) // falls through to X.
        // Exit from finally block.
        .label(label7) // N45
        .ret(LOCAL4)
        .trycatch(label0, label1, label1)
        .trycatch(label3, label4, label4);
    test_case
}

#[test]
fn test_visit_maxs_implicit_exit_to_another_subroutine2() {
    let (class_file, graph) = visit_maxs_implicit_exit_to_another_subroutine2().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (1, 4));
    let expected_control_flow_graph =
        control_flow_graph(&["N0=N6", "N5=", "N6=N13", "N10=N20", "N13=N20,N10", "N20=N5"]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_implicit_exit_to_another_subroutine2() -> TestCaseBuilder {
    let [label0, label1, label2, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        .jsr(label0)
        .vreturn() // N5
        .label(label0) // N6
        .astore(2)
        .jsr(label1)
        .go(label2) // N10
        .label(label1) // N13
        .astore(3)
        .iload(1)
        .ifne(label2)
        .ret(3)
        .label(label2) // N20
        .ret(2);
    test_case
}

/// This tests a simple subroutine where the control flow jumps back and forth between the
/// subroutine and the caller.
///
/// This would not normally be produced by a Java compiler.
#[test]
fn test_visit_maxs_interleaved_code() {
    let (class_file, graph) = visit_maxs_interleaved_code().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (4, 3));
    let expected_control_flow_graph =
        control_flow_graph(&["N0=N8", "N5=N15", "N8=N21", "N15=N28", "N21=N5", "N28="]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_interleaved_code() -> TestCaseBuilder {
    let [label0, label1, label2, label3, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        .jsr(label0)
        .go(label1) // N5
        // Subroutine 1.
        .label(label0) // N8
        .astore(2)
        .iinc(1, 1)
        .go(label2)
        // Second part of main subroutine.
        .label(label1) // N15
        .iinc(1, 2)
        .go(label3)
        // Second part of subroutine 1.
        .label(label2) // N21
        .iinc(1, 4)
        .push()
        .push()
        .ret(2)
        // Third part of main subroutine.
        .label(label3) // N28
        .push()
        .push()
        .vreturn();
    test_case
}

/// Tests a nested try/finally with implicit exit from one subroutine to the other subroutine,
/// and with a surrounding try/catch thrown in the mix. Equivalent to the following java code:
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
#[test]
fn test_visit_maxs_implicit_exit_in_try_catch() {
    let (class_file, graph) = visit_maxs_implicit_exit_in_try_catch().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (4, 6));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N2",
        "N2=N6,N45,N12",
        "N5=N6,N45",
        "N6=N45,N12",
        "N10=N45",
        "N12=N39,N45",
        "N17=N23,N45,N29",
        "N20=N23,N45",
        "N23=N45,N29",
        "N27=N45",
        "N29=N43,N45,N20,N27",
        "N39=N43,N45,N17",
        "N43=N45,N5,N10",
        "N45=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_implicit_exit_in_try_catch() -> TestCaseBuilder {
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
        ..,
    ] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .iconst_0() // N0
        .istore(1)
        // First try.
        .label(label0) // N2
        .jsr(label2)
        .vreturn() // N5
        // Exception handler for first try.
        .label(label1) // N6
        .astore(LOCAL2)
        .jsr(label2)
        .aload(LOCAL2) // N10
        .athrow()
        // First finally handler.
        .label(label2) // N12
        .astore(LOCAL4)
        .go(label6)
        // Body of while loop, also second try.
        .label(label3) // N17
        .jsr(label5)
        .push() // N20
        .push()
        .vreturn()
        // Exception handler for second try.
        .label(label4) // N23
        .astore(LOCAL3)
        .jsr(label5)
        .aload(LOCAL3) // N27
        .athrow()
        // Second finally handler.
        .label(label5) // N29
        .astore(LOCAL5)
        .iload(LOCAL1)
        .ifne(label7)
        .push()
        .push()
        .ret(LOCAL5)
        // Test for the while loop.
        .label(label6) // N39
        .iload(LOCAL1)
        .ifne(label3) // Falls through.
        // Exit from finally block.
        .label(label7) // N43
        .ret(LOCAL4)
        // Outermost catch.
        .label(label8) // N45
        .iinc(LOCAL1, 3)
        .vreturn()
        .trycatch(label0, label1, label1)
        .trycatch(label3, label4, label4)
        .trycatch(label0, label8, label8);
    test_case
}

/// Tests an example coming from distilled down version of
/// com/sun/corba/ee/impl/protocol/CorbaClientDelegateImpl from GlassFish 2. See issue #317823.
#[test]
fn test_visit_maxs_glass_fish2_corba_client_delegate_impl_example() {
    let (class_file, graph) =
        visit_maxs_glass_fish2_corba_client_delegate_impl_example().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (3, 3));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N6,N12,N24,N38",
        "N3=N15,N24,N38",
        "N6=N6,N12,N24,N38",
        "N10=N24,N38",
        "N12=N3,N10,N24,N38",
        "N15=N21,N24,N38",
        "N21=N30,N38",
        "N24=N30,N38",
        "N30=N0,N38,N44",
        "N37=",
        "N38=N38,N44",
        "N42=",
        "N44=N37,N42",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_glass_fish2_corba_client_delegate_impl_example() -> TestCaseBuilder {
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
    let mut test_case = TestCaseBuilder::new();
    test_case
        .label(label0) // N0
        .jsr(label4)
        .label(label1) // N3
        .go(label5)
        .label(label2) // N6
        .pop()
        .jsr(label4)
        .label(label3) // N10
        .aconst_null()
        .athrow()
        .label(label4) // N12
        .astore(1)
        .ret(1)
        .label(label5) // N15
        .aconst_null()
        .aconst_null()
        .aconst_null()
        .pop()
        .pop()
        .pop()
        .label(label6) // N21
        .go(label8)
        .label(label7) // N24
        .pop()
        .go(label8)
        .aconst_null()
        .athrow()
        .label(label8) // N30
        .iconst_0()
        .ifne(label0)
        .jsr(label12)
        .label(label9) // N37
        .vreturn()
        .label(label10) // N38
        .pop()
        .jsr(label12)
        .label(label11) // N42
        .aconst_null()
        .athrow()
        .label(label12) // N44
        .astore(2)
        .ret(2)
        .trycatch(label0, label1, label2)
        .trycatch(label2, label3, label2)
        .trycatch(label0, label6, label7)
        .trycatch(label0, label9, label10)
        .trycatch(label10, label11, label10);
    test_case
}

/// Tests a nested subroutine with implicit exit from the nested subroutine to the outer one,
/// with the second subroutine coming first in the bytecode instructions sequence.
#[test]
fn test_visit_maxs_implicit_exit_to_another_subroutine_inverted() {
    let (class_file, graph) =
        visit_maxs_implicit_exit_to_another_subroutine_inverted().build_and_graph();

    assert_eq!(read_max_stack_and_locals(&class_file), (1, 3));
    let expected_control_flow_graph = control_flow_graph(&[
        "N0=N18", "N3=N4", "N4=N21", "N6=N3,N4", "N14=", "N18=N6", "N21=",
    ]);
    assert_eq!(graph, expected_control_flow_graph);
}

fn visit_maxs_implicit_exit_to_another_subroutine_inverted() -> TestCaseBuilder {
    let [label0, label1, label2, label3, label4, ..] = labels();
    let mut test_case = TestCaseBuilder::new();
    test_case
        .go(label3) // N0
        // Second subroutine, returns to caller of first subroutine.
        .label(label0) // N3
        .astore(2)
        .label(label1) // N4
        .ret(1)
        // First subroutine.
        .label(label2) // N6
        .astore(1)
        .aload(0)
        .ifnonnull(label1)
        .jsr(label0) // This JSR never returns, the following code is unreachable.
        .aconst_null() // N14
        .aconst_null()
        .aconst_null()
        .vreturn()
        // Main "subroutine".
        .label(label3) // N18
        .jsr(label2)
        .label(label4) // N21
        .vreturn();
    test_case
}

/// Tests computing the maximum stack size from the existing stack map frames and the
/// instructions in between, when dead code is present.
#[test]
fn test_visit_maxs_frames_with_dead_code() {
    let [label0, ..] = labels();
    let mut test_case = TestCaseBuilder::with_version(V1_7);
    test_case
        .vreturn()
        // With the default compute maxs algorithm, this dead code block is not considered for
        // the maximum stack size, which works fine for classes up to V1_6. Starting with V1_7,
        // stack map frames are mandatory, even for dead code, and the maximum stack size must
        // take dead code into account. Hopefully it can be computed from the stack map frames,
        // and the instructions in between (without any control flow graph construction or
        // algorithm).
        .label(label0)
        .frame(FrameKind::Same, &[], &[])
        .aconst_null()
        .vreturn();

    let class_file = test_case.build();

    assert_eq!(read_max_stack_and_locals(&class_file), (1, 1));
}

#[test]
fn test_visit_maxs_frame_with_long() {
    let [label0, ..] = labels();
    let mut test_case = TestCaseBuilder::with_version(V1_7);
    test_case
        .push2()
        .go(label0)
        .label(label0)
        .frame(FrameKind::New, &[], &[FrameValue::Long])
        .aconst_null()
        .vreturn();

    let class_file = test_case.build();

    assert_eq!(read_max_stack_and_locals(&class_file), (3, 1));
}

/// ASM's `assertDoesNotThrow(() -> new ClassFile(classFile).newInstance())`, for every test case
/// that checks it.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_visit_maxs_new_instance() {
    let mut test_cases: Vec<(String, TestCaseBuilder)> = [1, 31, 32, 33]
        .into_iter()
        .map(|n| (format!("basic({n})"), visit_maxs_basic(n)))
        .collect();
    test_cases.extend([
        (
            "ifElseInFinally".to_owned(),
            visit_maxs_if_else_in_finally(),
        ),
        (
            "simpleNestedFinally".to_owned(),
            visit_maxs_simple_nested_finally(),
        ),
        (
            "subroutineWithNoRet".to_owned(),
            visit_maxs_subroutine_with_no_ret(),
        ),
        (
            "subroutineWithNoRet2".to_owned(),
            visit_maxs_subroutine_with_no_ret2(),
        ),
        ("implicitExit".to_owned(), visit_maxs_implicit_exit()),
        (
            "implicitExitToAnotherSubroutine".to_owned(),
            visit_maxs_implicit_exit_to_another_subroutine(),
        ),
        (
            "implicitExitToAnotherSubroutine2".to_owned(),
            visit_maxs_implicit_exit_to_another_subroutine2(),
        ),
        ("interleavedCode".to_owned(), visit_maxs_interleaved_code()),
        (
            "implicitExitInTryCatch".to_owned(),
            visit_maxs_implicit_exit_in_try_catch(),
        ),
        (
            "glassFish2CorbaClientDelegateImplExample".to_owned(),
            visit_maxs_glass_fish2_corba_client_delegate_impl_example(),
        ),
        (
            "implicitExitToAnotherSubroutineInverted".to_owned(),
            visit_maxs_implicit_exit_to_another_subroutine_inverted(),
        ),
    ]);
    for (name, test_case) in test_cases {
        let class_file = test_case.build();
        if let Err(error) = oxasm_testkit::jvm::verify_class(&class_file) {
            panic!("{name}: {error}");
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Test support
// -------------------------------------------------------------------------------------------------

/// The `(max_stack, max_locals)` of the method `m` of the given class (ASM's
/// `readMaxStackAndLocals`).
fn read_max_stack_and_locals(class_file: &[u8]) -> (u16, u16) {
    let mut reader = MaxsReader { maxs: None };
    ClassReader::new(class_file)
        .unwrap()
        .accept(&mut reader, ParsingOptions::empty())
        .unwrap();
    reader.maxs.expect("the maxs of method m")
}

/// Records the maxs of the method `m`.
struct MaxsReader {
    maxs: Option<(u16, u16)>,
}

impl ClassVisitor for MaxsReader {
    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        (name == "m").then(|| {
            Box::new(MethodMaxsReader {
                maxs: &mut self.maxs,
            }) as _
        })
    }
}

struct MethodMaxsReader<'a> {
    maxs: &'a mut Option<(u16, u16)>,
}

impl MethodVisitor for MethodMaxsReader<'_> {
    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        *self.maxs = Some((max_stack, max_locals));
    }
}

/// A control flow graph, from `"N<offset>=N<successor offset>,..."` nodes (ASM's
/// `controlFlowGraph`).
fn control_flow_graph(nodes: &[&str]) -> BTreeMap<String, BTreeSet<String>> {
    nodes
        .iter()
        .map(|node| {
            let (key, values) = node.split_once('=').expect("a node");
            let values = values
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            (key.to_owned(), values)
        })
        .collect()
}

/// An instruction of the method `m` of a test case.
type Instruction = Box<dyn Fn(&mut dyn MethodVisitor)>;

/// ASM's `TestCaseBuilder`: the instructions of a method `m`, replayed by [`Self::build`] into a
/// class writer with `COMPUTE_MAXS`.
struct TestCaseBuilder {
    class_version: u32,
    instructions: Vec<Instruction>,
}

impl TestCaseBuilder {
    fn new() -> TestCaseBuilder {
        TestCaseBuilder::with_version(V1_1)
    }

    fn with_version(class_version: u32) -> TestCaseBuilder {
        TestCaseBuilder {
            class_version,
            instructions: Vec::new(),
        }
    }

    fn add(&mut self, instruction: impl Fn(&mut dyn MethodVisitor) + 'static) -> &mut Self {
        self.instructions.push(Box::new(instruction));
        self
    }

    fn nop(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(NOP))
    }

    fn push(&mut self) -> &mut Self {
        self.iconst_0()
    }

    fn push2(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(LCONST_0))
    }

    fn pop(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(POP))
    }

    fn iconst_0(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(ICONST_0))
    }

    fn istore(&mut self, var_index: u16) -> &mut Self {
        self.add(move |mv| mv.visit_var_insn(ISTORE, var_index))
    }

    fn aload(&mut self, var_index: u16) -> &mut Self {
        self.add(move |mv| mv.visit_var_insn(ALOAD, var_index))
    }

    fn iload(&mut self, var_index: u16) -> &mut Self {
        self.add(move |mv| mv.visit_var_insn(ILOAD, var_index))
    }

    fn astore(&mut self, var_index: u16) -> &mut Self {
        self.add(move |mv| mv.visit_var_insn(ASTORE, var_index))
    }

    fn ret(&mut self, var_index: u16) -> &mut Self {
        self.add(move |mv| mv.visit_var_insn(RET, var_index))
    }

    fn athrow(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(ATHROW))
    }

    fn aconst_null(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(ACONST_NULL))
    }

    fn vreturn(&mut self) -> &mut Self {
        self.add(|mv| mv.visit_insn(RETURN))
    }

    fn label(&mut self, label: Label) -> &mut Self {
        self.add(move |mv| mv.visit_label(label))
    }

    fn iinc(&mut self, var_index: u16, increment: i16) -> &mut Self {
        self.add(move |mv| mv.visit_iinc_insn(var_index, increment))
    }

    fn frame(
        &mut self,
        kind: FrameKind,
        local: &[FrameValue<'static>],
        stack: &[FrameValue<'static>],
    ) -> &mut Self {
        let (local, stack) = (local.to_vec(), stack.to_vec());
        self.add(move |mv| mv.visit_frame(kind, &local, &stack))
    }

    fn go(&mut self, label: Label) -> &mut Self {
        self.add(move |mv| mv.visit_jump_insn(GOTO, label))
    }

    fn jsr(&mut self, label: Label) -> &mut Self {
        self.add(move |mv| mv.visit_jump_insn(JSR, label))
    }

    fn ifnonnull(&mut self, label: Label) -> &mut Self {
        self.add(move |mv| mv.visit_jump_insn(IFNONNULL, label))
    }

    fn ifne(&mut self, label: Label) -> &mut Self {
        self.add(move |mv| mv.visit_jump_insn(IFNE, label))
    }

    fn trycatch(&mut self, start: Label, end: Label, handler: Label) -> &mut Self {
        self.add(move |mv| mv.visit_try_catch_block(start, end, handler, None))
    }

    fn local_variable(
        &mut self,
        name: &'static str,
        descriptor: &'static str,
        signature: Option<&'static str>,
        start: Label,
        end: Label,
        index: u16,
    ) -> &mut Self {
        self.add(move |mv| {
            mv.visit_local_variable(
                name.into(),
                descriptor.into(),
                signature.map(Into::into),
                start,
                end,
                index,
            );
        })
    }

    /// Writes the class `C`, with a constructor and the method `m` of this test case, followed
    /// by `visitMaxs(0, 0)` (ASM's constructor, `visitMaxs` and `build`).
    fn build(&self) -> Vec<u8> {
        self.build_and_graph().0
    }

    /// [`Self::build`], and the control flow graph of the method `m` (ASM's `visitMaxs`), from
    /// its first label on.
    fn build_and_graph(&self) -> (Vec<u8>, BTreeMap<String, BTreeSet<String>>) {
        let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
        class_writer.visit(
            self.class_version,
            ACC_PUBLIC,
            jstr!("C"),
            None,
            Some(jstr!("java/lang/Object")),
            &[],
        );
        let mut constructor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .unwrap();
        constructor.visit_code();
        constructor.visit_var_insn(ALOAD, 0);
        constructor.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        constructor.visit_insn(RETURN);
        constructor.visit_maxs(1, 1);
        constructor.visit_end();
        drop(constructor);
        let mut method_visitor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        let start = Label::new();
        method_visitor.visit_label(start);
        for instruction in &self.instructions {
            instruction(&mut *method_visitor);
        }
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
        drop(method_visitor);
        let graph = class_writer
            .last_method_control_flow_graph(start)
            .into_iter()
            .map(|(offset, successors)| {
                (
                    format!("N{offset}"),
                    successors.iter().map(|s| format!("N{s}")).collect(),
                )
            })
            .collect();
        class_writer.visit_end();
        (class_writer.to_bytes().unwrap(), graph)
    }
}
