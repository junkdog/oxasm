//! Port of ASM's `TryCatchBlockSorterTest`: unit tests for `TryCatchBlockSorter`.

mod common;

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, Label, MethodVisitor,
    ParsingOptions, jstr,
};
use oxasm_commons::TryCatchBlockSorter;
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::ClassNode;

#[test]
fn test_constructor() {
    let _ = TryCatchBlockSorter::new(None, ACC_PUBLIC, jstr!("name"), jstr!("()V"), None, &[]);
    // ASM's second assertion (a subclass must use the constructor with an api) is dropped:
    // there is no subclassing, and no api (ADR-09, ADR-13).
}

/// A class visitor that wraps each method of its downstream in a [`TryCatchBlockSorter`].
struct SortingClassVisitor<V>(V);

impl<V: ClassVisitor> ClassVisitor for SortingClassVisitor<V> {
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
        Some(Box::new(TryCatchBlockSorter::new(
            self.0
                .visit_method(access, name, descriptor, signature, exceptions),
            access,
            name,
            descriptor,
            signature,
            exceptions,
        )))
    }
}

fn sort_try_catch_blocks(class_parameter: PrecompiledClass) -> Vec<u8> {
    let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut class_visitor = SortingClassVisitor(&mut class_writer);

    class_reader
        .accept(&mut class_visitor, ParsingOptions::empty())
        .unwrap();

    class_writer.to_bytes().unwrap()
}

/// The JVM half of ASM's test (`newInstance`); the precompiled classes go through the sorter
/// without panicking in [`test_all_methods_precompile_class_transforms`].
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_precompile_class_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        common::assert_precompiled_new_instance(
            class_parameter,
            &sort_try_catch_blocks(class_parameter),
        );
    }
}

/// oxasm-specific (the offline half of ASM's `testAllMethods_precompileClass`): every
/// precompiled class goes through the sorter, and keeps its try catch blocks (sorted: javac
/// does not always emit them innermost first, e.g. for `synchronized` blocks).
#[test]
fn test_all_methods_precompile_class_transforms() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = sort_try_catch_blocks(class_parameter);
        let read = |class_file: &[u8]| {
            let mut class_node = ClassNode::default();
            ClassReader::new(class_file)
                .unwrap()
                .accept(&mut class_node, ParsingOptions::empty())
                .unwrap();
            class_node
                .methods
                .iter()
                .map(|m| m.try_catch_blocks.len())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            read(&new_class_file),
            read(class_parameter.bytes()),
            "{class_parameter}"
        );
    }
}

/// oxasm-specific: the blocks are sorted innermost-to-outermost, by the length of their "try"
/// block, in a stable way (blocks of the same length keep their order).
#[test]
fn test_sort_order() {
    let labels: Vec<Label> = (0..6).map(|_| Label::new()).collect();
    let mut sorter =
        TryCatchBlockSorter::new(None, ACC_PUBLIC, jstr!("m"), jstr!("()V"), None, &[]);
    sorter.visit_code();
    // The outer block first, then two inner blocks of the same length.
    sorter.visit_try_catch_block(labels[0], labels[5], labels[5], Some(jstr!("A")));
    sorter.visit_try_catch_block(labels[1], labels[2], labels[5], Some(jstr!("B")));
    sorter.visit_try_catch_block(labels[3], labels[4], labels[5], Some(jstr!("C")));
    for &label in &labels {
        sorter.visit_label(label);
        sorter.visit_insn(NOP);
    }
    sorter.visit_insn(RETURN);
    sorter.visit_maxs(1, 1);
    sorter.visit_end();

    let types: Vec<String> = sorter
        .method_node()
        .try_catch_blocks
        .iter()
        .map(|b| b.type_.as_deref().unwrap().to_string())
        .collect();
    assert_eq!(types, ["B", "C", "A"]);
}
