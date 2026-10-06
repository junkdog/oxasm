//! `ClassReader::accept` of a `ClassNode`, then `ClassNode::accept` of a `TraceClassVisitor`
//! (Textifier), never panics, hangs or runs out of memory: the tree's visitor implementations
//! see every visitor argument path, including the malformed inputs where ASM's tree throws
//! (oxasm-verification, "Fuzzing").

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ParsingOptions};
use oxasm_tree::ClassNode;
use oxasm_util::TraceClassVisitor;

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
        let mut class_node = ClassNode::new();
        // A class node visited from a prefix of the events (the reader failed) must work too.
        let _ = reader.accept(&mut class_node, options);
        let mut trace_class_visitor = TraceClassVisitor::new(std::io::sink());
        class_node.accept(&mut trace_class_visitor);
        // Every instruction has a position, and every label node is found.
        for method in &class_node.methods {
            let instructions = &method.instructions;
            for (index, id) in instructions.ids().enumerate() {
                assert_eq!(instructions.index_of(id), Some(index));
                if let Some(label) = instructions[id].insn.as_label() {
                    assert!(instructions.label_position(label).is_some());
                }
            }
        }
    }
});
