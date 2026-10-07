//! Test support shared by the tree tests: ASM's `Comment` and `CodeComment` test attributes (as
//! codecs, ASM's attribute prototypes) and Textifier helpers.

#![allow(dead_code)] // each test file uses part of it

use oxasm::error::ReadError;
use std::sync::Arc;

use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{AttributeCodec, ClassVisitor, CustomAttribute, JavaStr, jstr};
use oxasm_testkit::ClassFile;
use oxasm_tree::ClassNode;
use oxasm_util::TraceClassVisitor;

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

/// The Textifier output of whatever `accept` makes a class visitor visit.
pub fn textify(accept: impl FnOnce(&mut dyn ClassVisitor)) -> String {
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    accept(&mut trace_class_visitor);
    trace_class_visitor.take_io_error().unwrap();
    String::from_utf8(output).unwrap()
}

/// The Textifier output of a class node.
pub fn textify_node(class_node: &ClassNode) -> String {
    textify(|cv| class_node.accept(cv))
}
