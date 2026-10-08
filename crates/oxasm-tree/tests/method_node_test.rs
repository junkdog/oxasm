//! Port of ASM's `MethodNodeTest`.
//!
//! - `testConstructor_illegalState` is dropped: there is no subclassing, nor `api` (ADR-09).
//! - `testVisitAndAccept_withUninitializedMethodNode` compares `ClassWriter` outputs with
//!   `ClassFile` dumps; its Textifier form (P3, against ASM's tree path) is kept as
//!   `test_visit_and_accept_with_uninitialized_method_node_textify`.

mod common;

use std::sync::Arc;

use oxasm::opcodes::NOP;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Interner, JavaStr, Label,
    MethodVisitor, ParsingOptions,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_tree::{ClassNode, MethodNode};

use common::{assert_same_class_file, attributes, textify_node};

#[test]
fn test_constructor() {
    let interner = Arc::new(Interner::new());
    let method_node = MethodNode::new(
        interner.clone(),
        123,
        interner.intern("method"),
        interner.intern("()V"),
        None,
        Vec::new(),
    );

    assert_eq!(method_node.access, 123);
    assert_eq!(method_node.resolve(method_node.name), "method");
    assert_eq!(method_node.resolve(method_node.desc), "()V");
}

/// A class node whose `visit_method` creates "uninitialized" method nodes.
struct UninitializedMethods(ClassNode);
impl ClassVisitor for UninitializedMethods {
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
        let interner = self.0.interner().clone();
        let mut method = MethodNode::with_interner(interner.clone());
        method.access = access;
        method.name = interner.intern(name);
        method.desc = interner.intern(descriptor);
        method.signature = signature.map(|s| interner.intern(s));
        method.exceptions = exceptions.iter().map(|&e| interner.intern(e)).collect();
        self.0.methods.push(method);
        Some(Box::new(self.0.methods.last_mut().unwrap()))
    }
}

/// Tests that an uninitialized MethodNode can receive any visit method call.
#[test]
fn test_visit_and_accept_with_uninitialized_method_node() {
    for &class in PrecompiledClass::ALL {
        let class_file = class.bytes();
        let mut class_node = UninitializedMethods(ClassNode::new());
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        ClassReader::new(class_file)
            .unwrap()
            .accept_with_codecs(&mut class_node, &attributes(), ParsingOptions::empty())
            .unwrap();
        class_node.0.accept(&mut class_writer);

        assert_same_class_file(class_file, &class_writer.to_bytes().unwrap(), &class);
    }
}

/// The Textifier form of `test_visit_and_accept_with_uninitialized_method_node` (P3): a class
/// node whose method nodes are created with `MethodNode::with_interner` and then filled in prints like
/// ASM's tree path.
#[test]
fn test_visit_and_accept_with_uninitialized_method_node_textify() {
    for &class in PrecompiledClass::ALL {
        let Golden::Ok(expected) = golden::textifier_tree(class.name(), ReaderOptions::Default)
        else {
            continue;
        };
        let mut class_node = UninitializedMethods(ClassNode::new());

        ClassReader::new(class.bytes())
            .unwrap()
            .accept_with_codecs(&mut class_node, &attributes(), ParsingOptions::empty())
            .unwrap();

        assert_eq!(
            textify_node(&class_node.0),
            String::from_utf8(expected).unwrap(),
            "{class}"
        );
    }
}

#[test]
fn test_clone() {
    let mut method_node = MethodNode::default();
    method_node.visit_code();
    method_node.visit_label(Label::new());
    method_node.visit_insn(NOP);
    method_node.visit_label(Label::new());
    method_node.visit_end();
    let mut clone_method_node = MethodNode::default();

    method_node.accept_method_visitor(&mut clone_method_node);
    method_node.accept_method_visitor(&mut clone_method_node);

    // Unlike ASM, which hands out fresh labels on each accept, both copies have the same labels.
    assert_eq!(clone_method_node.instructions.len(), 6);
}

/// oxasm-specific: the try catch block annotations are visited with their block's current
/// index, as ASM's `updateIndex` sets it, without changing the node; `update_index` changes it.
#[test]
fn try_catch_annotations_follow_the_block_index() {
    use oxasm::TypeReference;
    use oxasm_tree::{TryCatchBlockNode, TypeAnnotationNode};

    let (start, end, handler) = (Label::new(), Label::new(), Label::new());
    let mut method_node = MethodNode::default();
    let mut block = TryCatchBlockNode::new(start, end, handler, None);
    block.visible_type_annotations.push(TypeAnnotationNode::new(
        TypeReference::new_try_catch_reference(7),
        None,
        method_node.intern("LA;"),
    ));
    method_node.try_catch_blocks.push(block.clone());
    method_node.try_catch_blocks.insert(0, block);
    method_node.visit_insn(NOP);
    let mut copy = MethodNode::default();

    method_node.accept_method_visitor(&mut copy);

    let type_refs: Vec<u32> = copy
        .try_catch_blocks
        .iter()
        .map(|b| b.visible_type_annotations[0].type_ref.get_value())
        .collect();
    assert_eq!(
        type_refs,
        [
            TypeReference::new_try_catch_reference(0).get_value(),
            TypeReference::new_try_catch_reference(1).get_value()
        ]
    );
    assert_eq!(
        method_node.try_catch_blocks[1].visible_type_annotations[0]
            .type_ref
            .get_try_catch_block_index(),
        7
    );
    method_node.try_catch_blocks[1].update_index(1);
    assert_eq!(
        method_node.try_catch_blocks[1].visible_type_annotations[0]
            .type_ref
            .get_try_catch_block_index(),
        1
    );
}

/// oxasm-specific: like ASM's `LabelNode`, which hands out fresh labels, a method node clears
/// the reader's debug-only flag of the labels it stores (ADR-03).
#[test]
fn stored_labels_are_not_debug_only() {
    let label = Label::new().with_debug_only(true);
    let mut method_node = MethodNode::default();

    method_node.visit_label(label);
    method_node.visit_line_number(1, label);

    for node in &method_node.instructions {
        match node.insn {
            oxasm_tree::Insn::Label(l) | oxasm_tree::Insn::Line { start: l, .. } => {
                assert_eq!(l, label);
                assert!(!l.is_debug_only());
            }
            _ => unreachable!(),
        }
    }
}

/// oxasm-specific: where ASM's method node throws on events only a malformed class file causes,
/// this one stores what it can, or drops the annotation (a visitor cannot fail).
#[test]
fn malformed_events_do_not_panic() {
    use oxasm::{TypeReference, jstr};

    let interner = Arc::new(Interner::new());
    let new_method = |desc: &str| {
        MethodNode::from_strings(interner.clone(), 0, jstr!("m"), desc.as_ref(), None, &[])
    };
    let mut method_node = new_method("(I)V");
    // A parameter index past the descriptor's parameters (ASM: ArrayIndexOutOfBoundsException).
    assert!(
        method_node
            .visit_parameter_annotation(3, jstr!("LA;"), true)
            .is_some()
    );
    assert_eq!(method_node.visible_parameter_annotations.len(), 4);
    assert_eq!(
        interner.resolve(method_node.visible_parameter_annotations[3][0].desc),
        "LA;"
    );
    // A malformed descriptor (ASM: StringIndexOutOfBoundsException).
    let mut malformed = new_method("(");
    assert!(
        malformed
            .visit_parameter_annotation(0, jstr!("LA;"), false)
            .is_some()
    );
    assert_eq!(malformed.invisible_parameter_annotations.len(), 1);
    // An instruction annotation before any instruction (ASM: NullPointerException).
    method_node.visit_label(Label::new());
    let type_ref = TypeReference::new(0x4300_0000);
    assert!(
        method_node
            .visit_insn_annotation(type_ref, None, jstr!("LA;"), true)
            .is_none()
    );
    // A try catch annotation without its block (ASM: IndexOutOfBoundsException).
    assert!(
        method_node
            .visit_try_catch_annotation(
                TypeReference::new_try_catch_reference(0),
                None,
                jstr!("LA;"),
                true
            )
            .is_none()
    );
    // ASM's internal F_INSERT frame (ASM: IllegalArgumentException) is kept.
    method_node.visit_frame(oxasm::FrameKind::Insert, &[], &[]);
    assert!(matches!(
        method_node.instructions.last().map(|id| &method_node.instructions[id].insn),
        Some(oxasm_tree::Insn::Frame(frame)) if frame.kind == oxasm::FrameKind::Insert
    ));
}
