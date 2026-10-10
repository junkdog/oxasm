//! Test support shared by the check adapter tests: ASM-util's `MethodNodeBuilder`, ASM's
//! `Comment` and `CodeComment` test attributes (as codecs, ASM's attribute prototypes), and the
//! class hierarchy ASM's tests get from class loading.

#![allow(dead_code)] // each test file uses part of it

use std::sync::{Arc, OnceLock};

use oxasm::constant::FrameValue;
use oxasm::error::ReadError;
use oxasm::opcodes::*;
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{
    AttributeCodec, CustomAttribute, FrameKind, JavaStr, Label, MapHierarchy, MethodVisitor, jstr,
};
use oxasm_testkit::ClassFile;
use oxasm_tree::{FrameNode, MethodNode, SymFrameValue};

/// ASM's `Comment` test attribute: a class attribute with no content.
#[derive(Clone, Debug)]
pub struct Comment;

impl CustomAttribute for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn codec(&self) -> Option<Arc<dyn AttributeCodec>> {
        Some(Arc::new(Comment))
    }
}

impl AttributeCodec for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(Comment))
    }
}

/// ASM's `CodeComment` test attribute: a Code attribute with no content.
#[derive(Clone, Debug)]
pub struct CodeComment;

impl CustomAttribute for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn codec(&self) -> Option<Arc<dyn AttributeCodec>> {
        Some(Arc::new(CodeComment))
    }
}

impl AttributeCodec for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(CodeComment))
    }
}

/// ASM's `attributes()`: the prototypes of the test attributes.
pub fn attributes() -> [&'static dyn AttributeCodec; 2] {
    [&Comment, &CodeComment]
}

/// ASM's `assertEquals(new ClassFile(expected), new ClassFile(actual))`: the two class files have
/// the same `ClassFile` dump (shown as a diff if they do not).
#[track_caller]
pub fn assert_same_class_file(expected: &[u8], actual: &[u8], context: &dyn std::fmt::Display) {
    let (expected, actual) = (ClassFile::new(expected), ClassFile::new(actual));
    if expected != actual {
        assert_eq!(expected.dump_lossy(), actual.dump_lossy(), "{context}");
        panic!("{context}: the class files differ");
    }
}

/// The class hierarchy ASM's tests get from class loading: the classes of T0 and of the JDK, as
/// the oracle sees them (it needs the JVM oracle and `corpus/t1`).
pub fn class_loader_hierarchy() -> &'static MapHierarchy {
    static HIERARCHY: OnceLock<MapHierarchy> = OnceLock::new();
    HIERARCHY.get_or_init(|| {
        MapHierarchy::from_tsv(oxasm_testkit::jvm::class_loader_hierarchy_tsv()).unwrap()
    })
}

/// A builder of `MethodNode`s for tests (ASM-util's `MethodNodeBuilder`): a public method `m`
/// of class `C`, built through its visitor methods.
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
            Arc::default(),
            ACC_PUBLIC,
            "m".as_ref(),
            descriptor.as_ref(),
            None,
            &[],
        );
        method_node.max_stack = max_stack;
        method_node.max_locals = max_locals;
        method_node.visit_code();
        MethodNodeBuilder { method_node }
    }

    fn insn(mut self, opcode: u8) -> Self {
        self.method_node.visit_insn(opcode);
        self
    }

    fn var_insn(mut self, opcode: u8, var_index: u16) -> Self {
        self.method_node.visit_var_insn(opcode, var_index);
        self
    }

    fn jump_insn(mut self, opcode: u8, label: Label) -> Self {
        self.method_node.visit_jump_insn(opcode, label);
        self
    }

    pub fn nop(self) -> Self {
        self.insn(NOP)
    }

    pub fn iconst_0(self) -> Self {
        self.insn(ICONST_0)
    }

    pub fn istore(self, var_index: u16) -> Self {
        self.var_insn(ISTORE, var_index)
    }

    pub fn aload(self, var_index: u16) -> Self {
        self.var_insn(ALOAD, var_index)
    }

    pub fn astore(self, var_index: u16) -> Self {
        self.var_insn(ASTORE, var_index)
    }

    pub fn vreturn(self) -> Self {
        self.insn(RETURN)
    }

    pub fn label(mut self, label: Label) -> Self {
        self.method_node.visit_label(label);
        self
    }

    pub fn go(self, label: Label) -> Self {
        self.jump_insn(GOTO, label)
    }

    pub fn jsr(self, label: Label) -> Self {
        self.jump_insn(JSR, label)
    }

    pub fn ret(self, var_index: u16) -> Self {
        self.var_insn(RET, var_index)
    }

    pub fn ifne(self, label: Label) -> Self {
        self.jump_insn(IFNE, label)
    }

    /// Adds a frame node with the given content as is (ASM sets the fields of its `FrameNode`
    /// directly, without the trimming of its constructor).
    pub fn frame(
        mut self,
        kind: FrameKind,
        local: &[FrameValue<'static>],
        stack: &[FrameValue<'static>],
    ) -> Self {
        let interner = self.method_node.interner().clone();
        let intern = |values: &[FrameValue<'static>]| -> Vec<_> {
            values
                .iter()
                .map(|v| SymFrameValue::intern(v, &interner))
                .collect()
        };
        self.method_node.instructions.add(FrameNode::from_parts(
            kind,
            &intern(local),
            &intern(stack),
        ));
        self
    }

    pub fn build(mut self) -> MethodNode {
        self.method_node.visit_end();
        self.method_node
    }
}
