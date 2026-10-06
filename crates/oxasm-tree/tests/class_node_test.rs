//! Port of ASM's `ClassNodeTest`.
//!
//! - `testConstructor_illegalState` and `testCheck` are dropped: there is no subclassing, nor
//!   `api` and `check(api)` (ADR-09).
//! - `testVisitAndAccept`, `testVisitAndAccept_cloneInstructions` and
//!   `testVisitAndAccept_removeMembersVisitor` compare `ClassWriter` outputs with `ClassFile`
//!   dumps: they are pending P5 (the writer). Until then, the same transformations are checked
//!   through a Textifier: `tree_textify_test.rs` compares reader -> `ClassNode` -> Textifier
//!   with ASM's, for every T0 class and reader option, and the two other tests below compare
//!   with ASM's tree path golden, and with oxasm's direct path, respectively.

mod common;

use std::collections::HashMap;

use oxasm::constant::Constant;
use oxasm::opcodes::ACC_RECORD;
use oxasm::{
    AnnotationVisitor, Attribute, ClassReader, ClassVisitor, FieldVisitor, JavaStr, MethodVisitor,
    ModuleVisitor, ParsingOptions, RecordComponentVisitor, TypePath, TypeReference,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_tree::{ClassNode, InsnId, MethodNode};

use common::{attributes, textify, textify_node};

fn read_class_node(class: PrecompiledClass) -> ClassNode {
    let mut class_node = ClassNode::new();
    ClassReader::new(class.bytes())
        .unwrap()
        .accept_with_codecs(&mut class_node, &attributes(), ParsingOptions::empty())
        .unwrap();
    class_node
}

#[test]
fn test_constructor() {
    let class_node = ClassNode::new();

    assert!(class_node.interfaces.is_empty());
    assert!(class_node.inner_classes.is_empty());
    assert!(class_node.fields.is_empty());
    assert!(class_node.methods.is_empty());
}

/// Interim (the writer form is pending P5): replacing every instruction with its clone, with an
/// identity label map, changes nothing.
#[test]
fn test_visit_and_accept_clone_instructions() {
    for &class in PrecompiledClass::ALL {
        let Golden::Ok(expected) = golden::textifier_tree(class.name(), ReaderOptions::Default)
        else {
            continue;
        };
        let mut class_node = read_class_node(class);

        for method_node in &mut class_node.methods {
            clone_instructions(method_node);
        }

        assert_eq!(
            textify_node(&class_node),
            String::from_utf8(expected).unwrap(),
            "{class}"
        );
    }
}

/// Interim (the writer form is pending P5): a class node visited through a visitor that removes
/// its members prints like the class read through it.
#[test]
fn test_visit_and_accept_remove_members_visitor() {
    for &class in PrecompiledClass::ALL {
        let class_node = read_class_node(class);

        let actual = textify(|cv| class_node.accept(&mut RemoveMembersClassVisitor(cv)));
        let expected = textify(|cv| {
            ClassReader::new(class.bytes())
                .unwrap()
                .accept(&mut RemoveMembersClassVisitor(cv), ParsingOptions::empty())
                .unwrap();
        });

        assert_eq!(actual, expected, "{class}");
    }
}

#[test]
fn class_node_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ClassNode>();
}

fn clone_instructions(method_node: &mut MethodNode) {
    let label_clone_map = HashMap::new();
    let ids: Vec<InsnId> = method_node.instructions.ids().collect();
    for insn in ids {
        let clone = method_node.instructions[insn].clone_with(&label_clone_map);
        method_node.instructions.set(insn, clone);
    }
}

struct RemoveMembersClassVisitor<'a>(&'a mut dyn ClassVisitor);

impl ClassVisitor for RemoveMembersClassVisitor<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(self.0)
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.0.visit(
            version,
            access & !ACC_RECORD,
            name,
            signature,
            super_name,
            interfaces,
        );
    }

    fn visit_module<'a>(
        &'a mut self,
        _name: &JavaStr,
        _access: u32,
        _version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        None
    }

    fn visit_nest_host(&mut self, _nest_host: &JavaStr) {
        // Remove nest host.
    }

    fn visit_nest_member(&mut self, _nest_member: &JavaStr) {
        // Remove nest member.
    }

    fn visit_permitted_subclass(&mut self, _permitted_subclass: &JavaStr) {
        // Remove permitted subclass.
    }

    fn visit_annotation<'a>(
        &'a mut self,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        None
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        _type_ref: TypeReference,
        _type_path: Option<&TypePath>,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        None
    }

    fn visit_attribute(&mut self, _attribute: &Attribute) {
        // Remove attribute.
    }

    fn visit_record_component<'a>(
        &'a mut self,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        None
    }

    fn visit_field<'a>(
        &'a mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        None
    }

    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        None
    }
}
