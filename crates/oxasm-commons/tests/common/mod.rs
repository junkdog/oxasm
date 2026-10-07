//! Test support shared by the commons tests: ASM-commons' `MethodNodeBuilder`, ASM's `Comment`
//! and `CodeComment` test attributes (as codecs, ASM's attribute prototypes), and the class file
//! comparison of ASM's tests.

#![allow(dead_code)] // each test file uses part of it

use std::sync::{Arc, OnceLock};

use oxasm::error::ReadError;
use oxasm::opcodes::*;
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{
    AttributeCodec, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, CustomAttribute,
    JavaStr, Label, MapHierarchy, MethodVisitor, jstr,
};
use oxasm_testkit::ClassFile;
use oxasm_tree::MethodNode;
use oxasm_util::printer::text_to_java_string;
use oxasm_util::{Printer, Textifier, TraceMethodVisitor};

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

/// ASM's `assertDoesNotThrow(() -> new ClassFile(bytes).newInstance())`: the JVM defines and
/// verifies the class (through the oracle).
#[track_caller]
pub fn assert_new_instance(class_file: &[u8]) {
    if let Err(error) = oxasm_testkit::jvm::verify_class(class_file) {
        panic!("the JVM rejects the class: {error}");
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

/// ASM-commons' `MethodNodeBuilder.toText`: the Textifier text of a method's code.
pub fn to_text(method_node: &MethodNode) -> String {
    let mut textifier = Textifier::new();
    method_node.accept_method_visitor(&mut TraceMethodVisitor::new(&mut textifier));
    text_to_java_string(textifier.get_text()).to_string()
}

/// ASM-commons' `MethodNodeBuilder.buildClassWithMethod`: a class `C` with a static field `f` of
/// type `int[][]`, a default constructor and the given method, written with a `ClassWriter(0)`
/// (the bytes, which ASM wraps in a `ClassFile`).
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
    if let Some(mut fv) = class_writer.visit_field(ACC_STATIC, jstr!("f"), jstr!("[[I"), None, None)
    {
        fv.visit_end();
    }
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

/// A builder of `MethodNode`s for tests (ASM-commons' `MethodNodeBuilder`): a public method,
/// `m()V` by default, built through its visitor methods.
pub struct MethodNodeBuilder {
    method_node: MethodNode,
}

impl MethodNodeBuilder {
    /// A `m()V` method with the given maximum stack size and number of locals.
    pub fn new(max_stack: u16, max_locals: u16) -> MethodNodeBuilder {
        MethodNodeBuilder::with_name("m", "()V", max_stack, max_locals)
    }

    /// A method with the given name, descriptor, maximum stack size and number of locals.
    pub fn with_name(
        name: &str,
        descriptor: &str,
        max_stack: u16,
        max_locals: u16,
    ) -> MethodNodeBuilder {
        let mut method_node = MethodNode::new(ACC_PUBLIC, name, descriptor, None, Vec::new());
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

    pub fn var_insn(mut self, opcode: u8, var_index: u16) -> Self {
        self.method_node.visit_var_insn(opcode, var_index);
        self
    }

    pub fn type_insn(mut self, opcode: u8, operand: &str) -> Self {
        self.method_node.visit_type_insn(opcode, jstr!(operand));
        self
    }

    pub fn field_insn(mut self, opcode: u8, owner: &str, name: &str, descriptor: &str) -> Self {
        self.method_node
            .visit_field_insn(opcode, jstr!(owner), jstr!(name), jstr!(descriptor));
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

    pub fn jump_insn(mut self, opcode: u8, label: Label) -> Self {
        self.method_node.visit_jump_insn(opcode, label);
        self
    }

    pub fn ldc_insn(mut self, value: &Constant<'_>) -> Self {
        self.method_node.visit_ldc_insn(value);
        self
    }

    pub fn iconst_0(self) -> Self {
        self.insn(ICONST_0)
    }

    pub fn pop(self) -> Self {
        self.insn(POP)
    }

    pub fn istore(self, var_index: u16) -> Self {
        self.var_insn(ISTORE, var_index)
    }

    pub fn aload(self, var_index: u16) -> Self {
        self.var_insn(ALOAD, var_index)
    }

    pub fn iload(self, var_index: u16) -> Self {
        self.var_insn(ILOAD, var_index)
    }

    pub fn astore(self, var_index: u16) -> Self {
        self.var_insn(ASTORE, var_index)
    }

    pub fn ret(self, var_index: u16) -> Self {
        self.var_insn(RET, var_index)
    }

    pub fn athrow(self) -> Self {
        self.insn(ATHROW)
    }

    pub fn aconst_null(self) -> Self {
        self.insn(ACONST_NULL)
    }

    pub fn vreturn(self) -> Self {
        self.insn(RETURN)
    }

    /// Visits a new label (ASM's `label()`).
    pub fn new_label(self) -> Self {
        self.label(Label::new())
    }

    pub fn label(mut self, label: Label) -> Self {
        self.method_node.visit_label(label);
        self
    }

    pub fn iinc(mut self, var_index: u16, increment: i16) -> Self {
        self.method_node.visit_iinc_insn(var_index, increment);
        self
    }

    pub fn go(self, label: Label) -> Self {
        self.jump_insn(GOTO, label)
    }

    pub fn jsr(self, label: Label) -> Self {
        self.jump_insn(JSR, label)
    }

    pub fn ifnonnull(self, label: Label) -> Self {
        self.jump_insn(IFNONNULL, label)
    }

    pub fn ifne(self, label: Label) -> Self {
        self.jump_insn(IFNE, label)
    }

    /// ASM's `switchto(defaultLabel, key, target, useTableSwitch)`.
    pub fn switchto_key(
        mut self,
        default_label: Label,
        key: i32,
        target: Label,
        use_table_switch: bool,
    ) -> Self {
        if use_table_switch {
            self.method_node
                .visit_table_switch_insn(key, key, default_label, &[target]);
        } else {
            self.method_node
                .visit_lookup_switch_insn(default_label, &[key], &[target]);
        }
        self
    }

    /// ASM's `switchto(label0, label1, useTableSwitch)`.
    pub fn switchto(mut self, label0: Label, label1: Label, use_table_switch: bool) -> Self {
        if use_table_switch {
            self.method_node
                .visit_table_switch_insn(0, 1, label0, &[label0, label1]);
        } else {
            self.method_node
                .visit_lookup_switch_insn(label0, &[1], &[label1]);
        }
        self
    }

    pub fn multi_a_new_array_insn(mut self, descriptor: &str, num_dimensions: u8) -> Self {
        self.method_node
            .visit_multi_a_new_array_insn(jstr!(descriptor), num_dimensions);
        self
    }

    pub fn trycatch(mut self, start: Label, end: Label, handler: Label) -> Self {
        self.method_node
            .visit_try_catch_block(start, end, handler, None);
        self
    }

    pub fn line(mut self, line: u16, start: Label) -> Self {
        self.method_node.visit_line_number(line, start);
        self
    }

    pub fn localvar(
        mut self,
        name: &str,
        descriptor: &str,
        index: u16,
        start: Label,
        end: Label,
    ) -> Self {
        self.method_node.visit_local_variable(
            jstr!(name),
            jstr!(descriptor),
            None,
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

/// ASM's `newInstance` expectation for a transformed precompiled class: the JVM verifies the
/// class, except for preview classes of another JDK, which it rejects with an
/// `UnsupportedClassVersionError` (`isNotCompatibleWithCurrentJdk`). ASM loads the class with the
/// test classes on the class path; here it is verified along with the T0 classes of its package,
/// and only its own result counts.
#[track_caller]
pub fn assert_precompiled_new_instance(
    class_parameter: oxasm_testkit::PrecompiledClass,
    class_file: &[u8],
) {
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

/// ASM's `LocalVariablesSorterTest.LocalVariablesSorterClassAdapter`: wraps every method in a
/// `LocalVariablesSorter`.
pub struct LocalVariablesSorterClassAdapter<V>(pub V);

impl<V: ClassVisitor> ClassVisitor for LocalVariablesSorterClassAdapter<V> {
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
        Some(Box::new(oxasm_commons::LocalVariablesSorter::new(
            access,
            descriptor,
            method_visitor,
        )))
    }
}
