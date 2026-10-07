//! Port of ASM's `AdviceAdapterTest`.
//!
//! - ASM's subclasses of `AdviceAdapter` are [`Advice`] implementations (ADR-13).
//! - The constructor tests compare the Textifier text of the instrumented method with the
//!   expected one, then check that the JVM verifies it (`assertDoesNotThrow(() ->
//!   buildClassWithMethod(outputMethod).newInstance())`). The JVM checks run the oracle's
//!   `verify`, so they are gathered in one `#[ignore = "needs the JVM oracle"]` test,
//!   [`test_all_methods_constructors_new_instance`], run by CI's oracle job with `--ignored`.
//! - The `Api` dimension of the parameterized tests is gone (ADR-09), and with it the
//!   `UnsupportedOperationException` half of `testAllMethods_precompiledClass`.
//! - `BasicAdviceAdapter.visitLdcInsn`'s check of the constant types is not needed: `Constant`
//!   is an enum (ADR-05).

mod common;

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, JavaStr, Label,
    MethodVisitor, ParsingOptions, jstr,
};
use oxasm_commons::{Advice, AdviceAdapter};
use oxasm_testkit::{PrecompiledClass, panic_message};
use oxasm_tree::MethodNode;

use common::{
    LocalVariablesSorterClassAdapter, MethodNodeBuilder, assert_new_instance,
    assert_same_class_file, build_class_with_method, to_text,
};

/// ASM's `newBasicAdvice`: prints "enter" or "exit" on `System.err`.
fn emit_basic_advice(method_visitor: &mut dyn MethodVisitor, is_after_advice: bool) {
    method_visitor.visit_field_insn(
        GETSTATIC,
        jstr!("java/lang/System"),
        jstr!("err"),
        jstr!("Ljava/io/PrintStream;"),
    );
    method_visitor.visit_ldc_insn(&Constant::from(if is_after_advice {
        "exit"
    } else {
        "enter"
    }));
    method_visitor.visit_method_insn(
        INVOKEVIRTUAL,
        jstr!("java/io/PrintStream"),
        jstr!("println"),
        jstr!("(Ljava/lang/String;)V"),
        /* is_interface= */ false,
    );
}

/// ASM's `BasicAdviceAdapter`: the basic advices, visited through the adapter itself (ASM's
/// `newBasicAdvice(...).accept(this)`).
struct BasicAdvice;

impl<M: MethodVisitor> Advice<M> for BasicAdvice {
    fn on_method_enter(&mut self, adapter: &mut AdviceAdapter<M, Self>) {
        emit_basic_advice(adapter, /* is_after_advice= */ false);
    }

    fn on_method_exit(&mut self, adapter: &mut AdviceAdapter<M, Self>, _opcode: u8) {
        emit_basic_advice(adapter, /* is_after_advice= */ true);
    }
}

/// `inputMethod.accept(new BasicAdviceAdapter(outputMethod))`, with an `<init>(I)V` output.
fn instrument(input_method: &MethodNode) -> MethodNode {
    let mut output_method = MethodNode::new(ACC_PUBLIC, "<init>", "(I)V", None, Vec::new());
    input_method.accept_method_visitor(&mut AdviceAdapter::new(
        &mut output_method,
        ACC_PUBLIC,
        jstr!("<init>"),
        jstr!("(I)V"),
        BasicAdvice,
    ));
    output_method
}

/// ASM's `ExpectedMethodBuilder`: the input method, with the basic advices inserted before the
/// given instructions.
fn expected_method(
    input_method: &MethodNode,
    before_advice_at: &[usize],
    after_advice_at: &[usize],
) -> MethodNode {
    let mut output_method = MethodNode::new(
        input_method.access,
        input_method.name.clone(),
        input_method.desc.clone(),
        None,
        Vec::new(),
    );
    input_method.accept_method_visitor(&mut output_method);

    // ASM's Advice order: by decreasing instruction index, the after advices first.
    let mut advices: Vec<(usize, bool)> = before_advice_at
        .iter()
        .map(|&index| (index, false))
        .chain(after_advice_at.iter().map(|&index| (index, true)))
        .collect();
    advices.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (insn_index, is_after_advice) in advices {
        let mut advice = MethodNode::default();
        emit_basic_advice(&mut advice, is_after_advice);
        let next = output_method.instructions.id_at(insn_index).unwrap();
        output_method
            .instructions
            .insert_list_before(next, &mut advice.instructions);
    }
    output_method
}

/// One of ASM's constructor tests: an input method, and where the instrumented method has its
/// before and after advices.
struct ConstructorCase {
    name: String,
    input_method: MethodNode,
    before_advice_at: Vec<usize>,
    after_advice_at: Vec<usize>,
}

impl ConstructorCase {
    fn new(name: &str, input_method: MethodNode, before: &[usize], after: &[usize]) -> Self {
        ConstructorCase {
            name: name.to_owned(),
            input_method,
            before_advice_at: before.to_vec(),
            after_advice_at: after.to_vec(),
        }
    }

    /// `assertEquals(toText(expectedMethod), toText(outputMethod))`; returns the output method.
    #[track_caller]
    fn check(&self) -> MethodNode {
        let output_method = instrument(&self.input_method);
        let expected_method = expected_method(
            &self.input_method,
            &self.before_advice_at,
            &self.after_advice_at,
        );
        assert_eq!(
            to_text(&expected_method),
            to_text(&output_method),
            "{}",
            self.name
        );
        output_method
    }
}

fn builder(max_stack: u16, max_locals: u16) -> MethodNodeBuilder {
    MethodNodeBuilder::with_name("<init>", "(I)V", max_stack, max_locals)
}

fn object_init(builder: MethodNodeBuilder) -> MethodNodeBuilder {
    builder.method_insn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false)
}

fn c_init(builder: MethodNodeBuilder) -> MethodNodeBuilder {
    builder.method_insn(INVOKESPECIAL, "C", "<init>", "()V", false)
}

fn simple_constructor() -> ConstructorCase {
    let input_method = object_init(builder(2, 2).aload(0))
        // After instrumentation, expect a before advice here, before instruction #2.
        // After instrumentation, expect an after advice here, before instruction #2.
        .vreturn()
        .build();
    ConstructorCase::new("simpleConstructor", input_method, &[2], &[2])
}

fn constructor_with_two_super_init_in_two_branches() -> ConstructorCase {
    let label0 = Label::new();
    let label1 = Label::new();
    let input_method = object_init(
        builder(3, 2)
            .iload(1)
            .insn(ICONST_1)
            .insn(ICONST_2)
            .insn(IADD)
            .jump_insn(IF_ICMPEQ, label0)
            .aload(0),
    )
    // After instrumentation, expect a before advice here, before instruction #7.
    .go(label1)
    .label(label0)
    .aload(0);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #11.
        .label(label1)
        .type_insn(NEW, "java/lang/RuntimeException")
        .insn(DUP)
        .method_insn(
            INVOKESPECIAL,
            "java/lang/RuntimeException",
            "<init>",
            "()V",
            false,
        )
        // After instrumentation, expect an after advice here, before instruction #15.
        .athrow()
        .build();
    ConstructorCase::new(
        "constructorWithTwoSuperInitInTwoBranches",
        input_method,
        &[7, 11],
        &[15],
    )
}

fn constructor_with_two_super_init_in_two_switch_branches(
    use_table_switch: bool,
) -> ConstructorCase {
    let label0 = Label::new();
    let label1 = Label::new();
    let label2 = Label::new();
    let input_method = builder(4, 2)
        .iload(1)
        .insn(ICONST_1)
        .insn(ICONST_2)
        .insn(IADD)
        .switchto(label0, label1, use_table_switch)
        .label(label0)
        .aload(0);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #8.
        .go(label2)
        .label(label1)
        .aload(0);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #12.
        .go(label2)
        .label(label2)
        .type_insn(NEW, "java/lang/RuntimeException")
        .insn(DUP)
        .method_insn(
            INVOKESPECIAL,
            "java/lang/RuntimeException",
            "<init>",
            "()V",
            false,
        )
        // After instrumentation, expect an after advice here, before instruction #17.
        .athrow()
        .build();
    ConstructorCase::new(
        &format!("constructorWithTwoSuperInitInTwoSwitchBranches({use_table_switch})"),
        input_method,
        &[8, 12],
        &[17],
    )
}

fn constructor_with_super_inits_in_normal_and_handler_branches() -> ConstructorCase {
    let label0 = Label::new();
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let input_method = builder(2, 2)
        .trycatch(label0, label1, label2)
        .label(label0)
        .insn(NOP)
        .label(label1)
        .aload(0);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #5.
        .go(label3)
        .label(label2)
        .insn(POP)
        .aload(0);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #10.
        .label(label3)
        // After instrumentation, expect an after advice here, before instruction #11.
        .vreturn()
        .build();
    ConstructorCase::new(
        "constructorWithSuperInitsInNormalAndHandlerBranches",
        input_method,
        &[5, 10],
        &[11],
    )
}

fn constructor_with_uninit_this_in_two_branches() -> ConstructorCase {
    let label0 = Label::new();
    let label1 = Label::new();
    let input_method = builder(6, 2)
        .iload(1)
        .insn(I2L)
        .insn(LCONST_0)
        .insn(LCONST_1)
        .insn(LADD)
        .insn(LCMP)
        .ifne(label0)
        .insn(NOP)
        .type_insn(NEW, "C")
        .insn(DUP);
    let input_method = c_init(input_method)
        // After instrumentation, DON'T expect a before advice here, because the above call does
        // not initialize 'this'.
        .astore(1)
        .aload(0)
        .go(label1)
        .label(label0)
        .iconst_0()
        .aload(0)
        .insn(SWAP)
        .insn(POP)
        .label(label1);
    let input_method = object_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #21.
        // After instrumentation, expect an after advice here, before instruction #21.
        .vreturn()
        .build();
    ConstructorCase::new(
        "constructorWithUninitThisInTwoBranches",
        input_method,
        &[21],
        &[21],
    )
}

fn constructor_with_dup_x1() -> ConstructorCase {
    let input_method = builder(3, 2)
        .type_insn(NEW, "C")
        .astore(1)
        .aload(1)
        .aload(0)
        .insn(DUP_X1)
        .insn(POP);
    // After instrumentation, DON'T expect a before advice after the first call, because it does
    // not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #8.
        // After instrumentation, expect an after advice here, before instruction #8.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithDupX1", input_method, &[8], &[8])
}

fn constructor_with_dup_x2() -> ConstructorCase {
    let input_method = builder(4, 2)
        .type_insn(NEW, "C")
        .astore(1)
        .aload(1)
        .aconst_null()
        .aload(0)
        .insn(DUP_X2)
        .insn(POP2);
    // No method enter after the first call because it does not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #9.
        // After instrumentation, expect an after advice here, before instruction #9.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithDupX2", input_method, &[9], &[9])
}

fn constructor_with_dup2() -> ConstructorCase {
    let input_method = builder(4, 2)
        .type_insn(NEW, "C")
        .astore(1)
        .aload(0)
        .aload(1)
        .insn(DUP2)
        .insn(POP2);
    // After instrumentation, DON'T expect a before advice after the first call, because it does
    // not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #8.
        // After instrumentation, expect an after advice here, before instruction #8.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithDup2", input_method, &[8], &[8])
}

fn constructor_with_dup2_x1() -> ConstructorCase {
    let input_method = builder(5, 2)
        .type_insn(NEW, "C")
        .astore(1)
        .aconst_null()
        .aload(0)
        .aload(1)
        .insn(DUP2_X1)
        .insn(POP2)
        .insn(POP);
    // After instrumentation, DON'T expect a before advice after the first call, because it does
    // not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #10.
        // After instrumentation, expect an after advice here, before instruction #10.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithDup2X1", input_method, &[10], &[10])
}

fn constructor_with_dup2_x2() -> ConstructorCase {
    let input_method = builder(6, 2)
        .type_insn(NEW, "C")
        .astore(1)
        .insn(LCONST_0)
        .aload(0)
        .aload(1)
        .insn(DUP2_X2)
        .insn(POP2)
        .insn(POP2);
    // After instrumentation, DON'T expect a before advice after the first call, because it does
    // not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #10.
        // After instrumentation, expect an after advice here, before instruction #10.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithDup2X2", input_method, &[10], &[10])
}

fn constructor_with_jsr_ret() -> ConstructorCase {
    let label0 = Label::new();
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let input_method = builder(3, 3)
        .trycatch(label0, label1, label1)
        .label(label0)
        .aload(0);
    let input_method = c_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #3.
        .go(label3)
        .label(label1)
        .jsr(label2)
        // After instrumentation, expect an after advice here, before instruction #6.
        .athrow()
        .label(label2)
        .astore(2)
        .ret(2)
        .label(label3)
        // After instrumentation, expect an after advice here, before instruction #11.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithJsrRet", input_method, &[3], &[6, 11])
}

fn constructor_with_longs_and_arrays() -> ConstructorCase {
    let input_method = builder(6, 4)
        .aload(0)
        .type_insn(NEW, "C")
        .field_insn(GETSTATIC, "java/lang/Long", "MAX_VALUE", "J")
        .var_insn(LSTORE, 2)
        .insn(ICONST_1)
        .int_insn(NEWARRAY, T_LONG)
        .iconst_0()
        .ldc_insn(&Constant::Long(123))
        .insn(LASTORE);
    // After instrumentation, DON'T expect a before advice after the first call, because it does
    // not initialize 'this'.
    let input_method = c_init(c_init(input_method))
        // After instrumentation, expect a before advice here, before instruction #11.
        // After instrumentation, expect an after advice here, before instruction #11.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithLongsAndArrays", input_method, &[11], &[11])
}

fn constructor_with_multi_anew_array() -> ConstructorCase {
    let input_method = builder(3, 2)
        .aload(0)
        .insn(ICONST_1)
        .insn(ICONST_2)
        .multi_a_new_array_insn("[[I", 2)
        .field_insn(PUTSTATIC, "C", "f", "[[I");
    let input_method = c_init(input_method)
        // After instrumentation, expect a before advice here, before instruction #6.
        // After instrumentation, expect an after advice here, before instruction #6.
        .vreturn()
        .build();
    ConstructorCase::new("constructorWithMultiAnewArray", input_method, &[6], &[6])
}

fn constructor_with_branches_after_super_init() -> ConstructorCase {
    let label1 = Label::new();
    let label2 = Label::new();
    let input_method = object_init(builder(2, 2).aload(0))
        // After instrumentation, expect a before advice here, before instruction #2.
        .go(label1)
        .label(label2)
        .insn(POP)
        // After instrumentation, expect an after advice here, before instruction #5.
        .vreturn()
        .label(label1)
        .iconst_0()
        .go(label2)
        .build();
    ConstructorCase::new(
        "constructorWithBranchesAfterSuperInit",
        input_method,
        &[2],
        &[5],
    )
}

fn constructor_with_forward_goto_after_block_without_successor() -> ConstructorCase {
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let input_method = object_init(builder(3, 2).trycatch(label1, label2, label2).aload(0))
        // After instrumentation, expect a before advice here, before instruction #2.
        .label(label1)
        .iconst_0()
        .go(label3)
        .label(label2)
        // After instrumentation, expect an after advice here, before instruction #6.
        .athrow()
        .label(label3)
        .pop()
        // After instrumentation, expect an after advice here, before instruction #9.
        .vreturn()
        .build();
    ConstructorCase::new(
        "constructorWithForwardGotoAfterBlockWithoutSuccessor",
        input_method,
        &[2],
        &[6, 9],
    )
}

fn constructor_with_forward_switch_after_block_without_successor(
    parameter: &str,
) -> ConstructorCase {
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let input_method = object_init(builder(3, 2).trycatch(label1, label2, label2).aload(0))
        // After instrumentation, expect a before advice here, before instruction #2.
        .label(label1)
        .iconst_0()
        .iconst_0()
        .switchto(
            label3,
            label3,
            /* use_table_switch= */ parameter == "tableswitch",
        )
        .label(label2)
        // After instrumentation, expect an after advice here, before instruction #7.
        .athrow()
        .label(label3)
        .pop()
        // After instrumentation, expect an after advice here, before instruction #10.
        .vreturn()
        .build();
    ConstructorCase::new(
        &format!("constructorWithForwardSwitchAfterBlockWithoutSuccessor({parameter})"),
        input_method,
        &[2],
        &[7, 10],
    )
}

fn constructor_with_handler_fallthrough_to_private_method_call() -> ConstructorCase {
    let label1 = Label::new();
    let label2 = Label::new();
    let label3 = Label::new();
    let input_method = object_init(builder(2, 2).trycatch(label1, label2, label2).aload(0))
        // After instrumentation, expect a before advice here, before instruction #2.
        .label(label1)
        .go(label3)
        .label(label2)
        .astore(1)
        .label(label3)
        .aload(0)
        .method_insn(INVOKESPECIAL, "C", "privateMethod", "()V", false)
        // After instrumentation, expect an after advice here, before instruction #9.
        .vreturn()
        .build();
    ConstructorCase::new(
        "constructorWithHandlerFallthroughToPrivateMethodCall",
        input_method,
        &[2],
        &[9],
    )
}

/// Every constructor case, for the JVM checks.
fn all_constructor_cases() -> Vec<ConstructorCase> {
    vec![
        simple_constructor(),
        constructor_with_two_super_init_in_two_branches(),
        constructor_with_two_super_init_in_two_switch_branches(true),
        constructor_with_two_super_init_in_two_switch_branches(false),
        constructor_with_super_inits_in_normal_and_handler_branches(),
        constructor_with_uninit_this_in_two_branches(),
        constructor_with_dup_x1(),
        constructor_with_dup_x2(),
        constructor_with_dup2(),
        constructor_with_dup2_x1(),
        constructor_with_dup2_x2(),
        constructor_with_jsr_ret(),
        constructor_with_longs_and_arrays(),
        constructor_with_multi_anew_array(),
        constructor_with_branches_after_super_init(),
        constructor_with_forward_goto_after_block_without_successor(),
        constructor_with_forward_switch_after_block_without_successor("lookupswitch"),
        constructor_with_forward_switch_after_block_without_successor("tableswitch"),
        constructor_with_handler_fallthrough_to_private_method_call(),
    ]
}

#[test]
fn test_all_methods_invalid_constructor() {
    let input_method = builder(2, 2).insn(IRETURN).build();

    let message = panic_message(|| instrument(&input_method));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid return in constructor"
    );
}

#[test]
fn test_all_methods_simple_constructor() {
    simple_constructor().check();
}

#[test]
fn test_all_methods_constructor_with_two_super_init_in_two_branches() {
    constructor_with_two_super_init_in_two_branches().check();
}

#[test]
fn test_all_methods_constructor_with_two_super_init_in_two_switch_branches() {
    for use_table_switch in [true, false] {
        constructor_with_two_super_init_in_two_switch_branches(use_table_switch).check();
    }
}

#[test]
fn test_all_methods_constructor_with_super_inits_in_normal_and_handler_branches() {
    constructor_with_super_inits_in_normal_and_handler_branches().check();
}

#[test]
fn test_all_methods_constructor_with_uninit_this_in_two_branches() {
    constructor_with_uninit_this_in_two_branches().check();
}

#[test]
fn test_all_methods_constructor_with_dup_x1() {
    constructor_with_dup_x1().check();
}

#[test]
fn test_all_methods_constructor_with_dup_x2() {
    constructor_with_dup_x2().check();
}

#[test]
fn test_all_methods_constructor_with_dup2() {
    constructor_with_dup2().check();
}

#[test]
fn test_all_methods_constructor_with_dup2_x1() {
    constructor_with_dup2_x1().check();
}

#[test]
fn test_all_methods_constructor_with_dup2_x2() {
    constructor_with_dup2_x2().check();
}

#[test]
fn test_all_methods_constructor_with_jsr_ret() {
    constructor_with_jsr_ret().check();
}

#[test]
fn test_all_methods_constructor_with_longs_and_arrays() {
    constructor_with_longs_and_arrays().check();
}

#[test]
fn test_all_methods_constructor_with_multi_anew_array() {
    constructor_with_multi_anew_array().check();
}

#[test]
fn test_all_methods_constructor_with_branches_after_super_init() {
    constructor_with_branches_after_super_init().check();
}

#[test]
fn test_all_methods_constructor_with_forward_goto_after_block_without_successor() {
    constructor_with_forward_goto_after_block_without_successor().check();
}

#[test]
fn test_all_methods_constructor_with_forward_switch_after_block_without_successor() {
    for parameter in ["lookupswitch", "tableswitch"] {
        constructor_with_forward_switch_after_block_without_successor(parameter).check();
    }
}

#[test]
fn test_all_methods_constructor_with_handler_fallthrough_to_private_method_call() {
    constructor_with_handler_fallthrough_to_private_method_call().check();
}

/// The `assertDoesNotThrow(() -> buildClassWithMethod(outputMethod).newInstance())` half of the
/// constructor tests.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_constructors_new_instance() {
    for case in all_constructor_cases() {
        let output_method = case.check();
        let class_file = build_class_with_method(&output_method);
        if let Err(error) = oxasm_testkit::jvm::verify_class(&class_file) {
            panic!("{}: the JVM rejects the class: {error}", case.name);
        }
    }
    // The same check, through the shared helper, for the simplest case.
    assert_new_instance(&build_class_with_method(&simple_constructor().check()));
}

/// ASM's `EmptyAdviceClassAdapter`: wraps every method with code in an `AdviceAdapter` with no
/// advice.
struct EmptyAdviceClassAdapter<V>(V);

impl<V: ClassVisitor> ClassVisitor for EmptyAdviceClassAdapter<V> {
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
            .visit_method(access, name, descriptor, signature, exceptions)?;
        if access & (ACC_ABSTRACT | ACC_NATIVE) != 0 {
            return Some(method_visitor);
        }
        Some(Box::new(AdviceAdapter::new(
            method_visitor,
            access,
            name,
            descriptor,
            (),
        )))
    }
}

#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        class_reader
            .accept(
                &mut EmptyAdviceClassAdapter(&mut class_writer),
                ParsingOptions::EXPAND_FRAMES,
            )
            .unwrap();

        let mut expected_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        class_reader
            .accept(
                &mut LocalVariablesSorterClassAdapter(&mut expected_class_writer),
                ParsingOptions::EXPAND_FRAMES,
            )
            .unwrap();
        assert_same_class_file(
            &expected_class_writer.to_bytes().unwrap(),
            &class_writer.to_bytes().unwrap(),
            &class_parameter,
        );
    }
}

/// An advice that generates code both through the adapter and directly with the downstream
/// visitor.
struct MixedVisitorsAdvice;

impl<M: MethodVisitor> Advice<M> for MixedVisitorsAdvice {
    fn on_method_enter(&mut self, adapter: &mut AdviceAdapter<M, Self>) {
        let label = Label::new();
        adapter.visit_label(label);
        // Generate ICONST_1 with the delegate visitor. The advice adapter does not 'see' this
        // and therefore cannot update its stack state, if it were doing so.
        adapter.mv().visit_insn(ICONST_1);
        // Generate IFEQ with the advice adapter itself. If the stack was updated here, it would
        // pop from an empty stack because the previous ICONST_1 was not simulated.
        adapter.visit_jump_insn(IFEQ, label);
    }
}

#[test]
fn test_on_method_enter_mixed_visitors() {
    let mut output_method = MethodNode::new(ACC_PUBLIC, "<init>", "(I)V", None, Vec::new());
    {
        let mut advice_adapter = AdviceAdapter::new(
            &mut output_method,
            ACC_PUBLIC,
            jstr!("<init>"),
            jstr!("()V"),
            MixedVisitorsAdvice,
        );

        advice_adapter.visit_code();
        advice_adapter.visit_var_insn(ALOAD, 0);
        advice_adapter.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        advice_adapter.visit_insn(RETURN);
        advice_adapter.visit_maxs(0, 0);
        advice_adapter.visit_end();
    }

    assert_eq!(
        to_text(&output_method),
        "    ALOAD 0\n    INVOKESPECIAL java/lang/Object.<init> ()V\n   L0\n    ICONST_1\n    \
         IFEQ L0\n    RETURN\n    MAXSTACK = 0\n    MAXLOCALS = 1\n"
    );
}

/// oxasm-specific: the advice hooks are not called again while one of them runs, and the
/// adapter dereferences to its generator adapter.
#[test]
fn test_hooks_are_not_reentrant() {
    struct ReturningAdvice(u32);
    impl<M: MethodVisitor> Advice<M> for ReturningAdvice {
        fn on_method_exit(&mut self, adapter: &mut AdviceAdapter<M, Self>, opcode: u8) {
            self.0 += 1;
            // In ASM, this would call onMethodExit again, recursively.
            adapter.visit_insn(opcode);
            adapter.push_int(7);
            adapter.pop();
        }
    }
    let mut output_method = MethodNode::new(ACC_PUBLIC, "m", "()V", None, Vec::new());
    let mut advice_adapter = AdviceAdapter::new(
        &mut output_method,
        ACC_PUBLIC,
        jstr!("m"),
        jstr!("()V"),
        ReturningAdvice(0),
    );
    advice_adapter.visit_code();
    advice_adapter.visit_insn(RETURN);
    advice_adapter.visit_maxs(0, 0);
    advice_adapter.visit_end();
    let (_, advice) = advice_adapter.into_parts();
    assert_eq!(advice.0, 1);
    assert_eq!(
        to_text(&output_method),
        "    RETURN\n    BIPUSH 7\n    POP\n    RETURN\n    MAXSTACK = 0\n    MAXLOCALS = 1\n"
    );
}
