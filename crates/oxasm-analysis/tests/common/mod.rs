//! Test support shared by the analysis tests: ASM's `MethodNodeBuilder`.

#![allow(dead_code)] // each test file uses part of it

use oxasm::opcodes::*;
use oxasm::{ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, Label, MethodVisitor, jstr};
use oxasm_tree::MethodNode;

/// A class `C` with a default constructor and the given method, written with a `ClassWriter`
/// (ASM's `MethodNodeBuilder.buildClassWithMethod`, less the `ClassFile` wrapper).
///
/// # Panics
///
/// If the writer fails.
pub fn build_class_with_method(method_node: &MethodNode) -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_5,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .expect("a method writer");
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ALOAD, 0);
        method_visitor.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(1, 1);
        method_visitor.visit_end();
    }
    method_node.accept(&mut class_writer);
    class_writer.to_bytes().expect("a class file")
}

/// ASM's `assertDoesNotThrow(() -> buildClassWithMethod(methodNode).newInstance())`: the JVM
/// defines and verifies the class (through the oracle).
#[track_caller]
pub fn assert_new_instance(method_node: &MethodNode) {
    let class_file = build_class_with_method(method_node);
    if let Err(error) = oxasm_testkit::jvm::verify_class(&class_file) {
        panic!("the JVM rejects the class: {error}");
    }
}

/// A builder of `MethodNode`s for tests (ASM's `MethodNodeBuilder`): a public method `m` of
/// class `C`, built through its visitor methods.
///
pub struct MethodNodeBuilder {
    method_node: MethodNode,
}

impl Default for MethodNodeBuilder {
    fn default() -> Self {
        MethodNodeBuilder::new()
    }
}

impl MethodNodeBuilder {
    /// A `()V` method with a maximum stack size and number of locals of 10.
    pub fn new() -> MethodNodeBuilder {
        MethodNodeBuilder::with_maxs(10, 10)
    }

    /// A `()V` method with the given maximum stack size and number of locals.
    pub fn with_maxs(max_stack: u16, max_locals: u16) -> MethodNodeBuilder {
        MethodNodeBuilder::with_descriptor("()V", max_stack, max_locals)
    }

    /// A method with the given descriptor, maximum stack size and number of locals.
    pub fn with_descriptor(descriptor: &str, max_stack: u16, max_locals: u16) -> MethodNodeBuilder {
        let mut method_node = MethodNode::from_strings(
            std::sync::Arc::default(),
            ACC_PUBLIC,
            jstr!("m"),
            jstr!(descriptor),
            None,
            &[],
        );
        method_node.max_stack = max_stack;
        method_node.max_locals = max_locals;
        method_node.visit_code();
        MethodNodeBuilder { method_node }
    }

    pub fn insn(mut self, opcode: u8) -> Self {
        self.method_node.visit_insn(opcode);
        self
    }

    pub fn int_insn(mut self, opcode: u8, operand: i32) -> Self {
        self.method_node.visit_int_insn(opcode, operand);
        self
    }

    pub fn type_insn(mut self, opcode: u8, operand: &str) -> Self {
        self.method_node.visit_type_insn(opcode, jstr!(operand));
        self
    }

    pub fn method_insn(
        mut self,
        opcode: u8,
        owner: &str,
        name: &str,
        descriptor: &str,
        is_interface: bool,
    ) -> Self {
        self.method_node.visit_method_insn(
            opcode,
            jstr!(owner),
            jstr!(name),
            jstr!(descriptor),
            is_interface,
        );
        self
    }

    pub fn multi_a_new_array_insn(mut self, descriptor: &str, num_dimensions: u8) -> Self {
        self.method_node
            .visit_multi_a_new_array_insn(jstr!(descriptor), num_dimensions);
        self
    }

    pub fn nop(self) -> Self {
        self.insn(NOP)
    }

    pub fn push(self) -> Self {
        self.iconst_0()
    }

    pub fn pop(self) -> Self {
        self.insn(POP)
    }

    pub fn iconst_0(self) -> Self {
        self.insn(ICONST_0)
    }

    pub fn istore(mut self, var_index: u16) -> Self {
        self.method_node.visit_var_insn(ISTORE, var_index);
        self
    }

    pub fn aload(mut self, var_index: u16) -> Self {
        self.method_node.visit_var_insn(ALOAD, var_index);
        self
    }

    pub fn iload(mut self, var_index: u16) -> Self {
        self.method_node.visit_var_insn(ILOAD, var_index);
        self
    }

    pub fn astore(mut self, var_index: u16) -> Self {
        self.method_node.visit_var_insn(ASTORE, var_index);
        self
    }

    pub fn ret(mut self, var_index: u16) -> Self {
        self.method_node.visit_var_insn(RET, var_index);
        self
    }

    pub fn athrow(self) -> Self {
        self.insn(ATHROW)
    }

    pub fn aconst_null(self) -> Self {
        self.insn(ACONST_NULL)
    }

    pub fn areturn(self) -> Self {
        self.insn(ARETURN)
    }

    pub fn vreturn(self) -> Self {
        self.insn(RETURN)
    }

    pub fn label(mut self, label: Label) -> Self {
        self.method_node.visit_label(label);
        self
    }

    pub fn iinc(mut self, var_index: u16, increment: i16) -> Self {
        self.method_node.visit_iinc_insn(var_index, increment);
        self
    }

    pub fn go(mut self, label: Label) -> Self {
        self.method_node.visit_jump_insn(GOTO, label);
        self
    }

    pub fn jsr(mut self, label: Label) -> Self {
        self.method_node.visit_jump_insn(JSR, label);
        self
    }

    pub fn ifnonnull(mut self, label: Label) -> Self {
        self.method_node.visit_jump_insn(IFNONNULL, label);
        self
    }

    pub fn ifne(mut self, label: Label) -> Self {
        self.method_node.visit_jump_insn(IFNE, label);
        self
    }

    pub fn tableswitch(mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) -> Self {
        self.method_node
            .visit_table_switch_insn(min, max, dflt, labels);
        self
    }

    pub fn trycatch(self, start: Label, end: Label, handler: Label) -> Self {
        self.trycatch_type(start, end, handler, None)
    }

    pub fn trycatch_type(
        mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&str>,
    ) -> Self {
        self.method_node
            .visit_try_catch_block(start, end, handler, type_.map(JavaStr::from_str));
        self
    }

    pub fn local_variable(
        mut self,
        name: &str,
        descriptor: &str,
        signature: Option<&str>,
        start: Label,
        end: Label,
        index: u16,
    ) -> Self {
        self.method_node.visit_local_variable(
            jstr!(name),
            jstr!(descriptor),
            signature.map(JavaStr::from_str),
            start,
            end,
            index,
        );
        self
    }

    pub fn build(mut self) -> MethodNode {
        self.method_node.visit_end();
        self.method_node
    }
}
