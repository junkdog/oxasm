//! Test support shared by the analysis tests: ASM's `MethodNodeBuilder`.

#![allow(dead_code)] // each test file uses part of it

use oxasm::opcodes::*;
use oxasm::{JavaStr, Label, MethodVisitor, jstr};
use oxasm_tree::MethodNode;

/// A builder of `MethodNode`s for tests (ASM's `MethodNodeBuilder`): a public method `m` of
/// class `C`, built through its visitor methods.
///
/// ASM's `buildClassWithMethod` (a class file made with a `ClassWriter`) waits for the writer
/// (P5).
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
        let mut method_node = MethodNode::new(ACC_PUBLIC, "m", descriptor, None, Vec::new());
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
