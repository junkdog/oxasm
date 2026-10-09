//! Port of ASM's `ClassNodeTest`.
//!
//! - `testConstructor_illegalState` and `testCheck` are dropped: there is no subclassing, nor
//!   `api` and `check(api)` (ADR-09).
//! - `testVisitAndAccept`, `testVisitAndAccept_cloneInstructions` and
//!   `testVisitAndAccept_removeMembersVisitor` compare `ClassWriter` outputs with `ClassFile`
//!   dumps, over every T0 class (the `Api` axis is dropped). The Textifier forms that preceded
//!   the writer (P3) are kept as `*_textify` tests: they compare with ASM's tree path golden, and
//!   with oxasm's direct path, respectively.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use oxasm::constant::Constant;
use oxasm::opcodes::ACC_RECORD;
use oxasm::{
    AnnotationVisitor, Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags,
    FieldVisitor, JavaStr, MethodVisitor, ModuleVisitor, ParsingOptions, RecordComponentVisitor,
    TypePath, TypeReference,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_tree::{ClassNode, InsnId, MethodNode};

use common::{assert_same_class_file, attributes, textify, textify_node};

fn read_class_node(class: PrecompiledClass) -> ClassNode {
    let mut class_node = ClassNode::with_interner(Arc::default());
    ClassReader::new(class.bytes())
        .unwrap()
        .accept_with_codecs(&mut class_node, &attributes(), ParsingOptions::empty())
        .unwrap();
    class_node
}

#[test]
fn test_constructor() {
    let class_node = ClassNode::with_interner(Arc::default());

    assert!(class_node.interfaces.is_empty());
    assert!(class_node.inner_classes.is_empty());
    assert!(class_node.fields.is_empty());
    assert!(class_node.methods.is_empty());
}

/// Tests that classes are unchanged with a ClassReader->ClassNode->ClassWriter transform.
#[test]
fn test_visit_and_accept() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        let class_node = read_class_node(class);
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_node.accept(&mut class_writer);

        assert_same_class_file(class_file, &class_writer.to_bytes().unwrap(), &class);
    }
}

/// Tests that classes are unchanged with a ClassReader->ClassNode->ClassWriter transform, when
/// all instructions are cloned.
#[test]
fn test_visit_and_accept_clone_instructions() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        let mut class_node = read_class_node(class);
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        for method_node in &mut class_node.methods {
            clone_instructions(method_node);
        }
        class_node.accept(&mut class_writer);

        assert_same_class_file(class_file, &class_writer.to_bytes().unwrap(), &class);
    }
}

/// Tests that ClassNode accepts visitors that remove class elements.
#[test]
fn test_visit_and_accept_remove_members_visitor() {
    for &class in PrecompiledClass::ALL {
        let class_node = read_class_node(class);
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_node.accept(&mut RemoveMembersClassVisitor(&mut class_writer));
        let mut expected_class_writer = ClassWriter::new(ClassWriterFlags::empty());
        ClassReader::new(class.bytes())
            .unwrap()
            .accept(
                &mut RemoveMembersClassVisitor(&mut expected_class_writer),
                ParsingOptions::empty(),
            )
            .unwrap();

        assert_same_class_file(
            &expected_class_writer.to_bytes().unwrap(),
            &class_writer.to_bytes().unwrap(),
            &class,
        );
    }
}

/// The Textifier form of `test_visit_and_accept_clone_instructions` (P3): replacing every
/// instruction with its clone, with an identity label map, changes nothing.
#[test]
fn test_visit_and_accept_clone_instructions_textify() {
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

/// The Textifier form of `test_visit_and_accept_remove_members_visitor` (P3): a class node
/// visited through a visitor that removes its members prints like the class read through it.
#[test]
fn test_visit_and_accept_remove_members_visitor_textify() {
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
        // ASM's clone copies the type annotations of a real instruction; here the list holds
        // them, and `set` drops those of the replaced instruction.
        let annotations = method_node.instructions.type_annotations(insn).cloned();
        let (clone, _) = method_node.instructions.set(insn, clone);
        if let Some(annotations) = annotations
            && method_node.instructions[clone].opcode().is_some()
        {
            *method_node.instructions.type_annotations_mut(clone) = annotations;
        }
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

/// Records the member counts hint.
#[derive(Default)]
struct MemberCounts(Option<(u16, u16)>);

impl ClassVisitor for MemberCounts {
    fn visit_member_counts(&mut self, fields_count: u16, methods_count: u16) {
        self.0 = Some((fields_count, methods_count));
    }
}

/// oxasm-specific: a class node visits its number of fields and methods, as the class reader
/// does (W12 in oxasm-optimization.md: a writer sizes its member storage from it).
#[test]
fn test_accept_visits_the_member_counts() {
    for &class in PrecompiledClass::ALL {
        let mut read = MemberCounts::default();
        ClassReader::new(class.bytes())
            .unwrap()
            .accept(&mut read, ParsingOptions::empty())
            .unwrap();
        let mut class_node = read_class_node(class);
        let mut accepted = MemberCounts::default();

        class_node.accept(&mut accepted);
        assert_eq!(accepted.0, read.0, "{class}");

        // The counts of the node's members, not those it was read with.
        class_node.methods.pop();
        class_node.accept(&mut accepted);
        assert_eq!(
            accepted.0,
            read.0
                .map(|(fields, methods)| (fields, methods.saturating_sub(1))),
            "{class}"
        );
    }
}
