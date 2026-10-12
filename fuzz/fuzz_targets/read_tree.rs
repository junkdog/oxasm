//! `ClassReader::accept` of a `ClassNode`, then `ClassNode::accept` of a `TraceClassVisitor`
//! (Textifier), never panics, hangs or runs out of memory: the tree's visitor implementations
//! see every visitor argument path, including the malformed inputs where ASM's tree throws
//! (oxasm-verification, "Fuzzing").
//!
//! Each class is read twice per option: into a node with a new interner, and into one with an
//! interner shared across inputs, as sift shares one across a classpath (the memo's hits on
//! strings of earlier classes, its thread cache's generations). Both must print the same.

#![no_main]

use std::cell::RefCell;
use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use oxasm::sym::Interner;
use oxasm::{ClassReader, ParsingOptions};
use oxasm_tree::ClassNode;
use oxasm_util::TraceClassVisitor;

/// A shared interner is replaced after this many inputs, or this many input bytes, so that it
/// stays well under the RSS limit and the memo sees new generations.
const SHARED_INPUTS: usize = 4096;
const SHARED_BYTES: usize = 64 << 20;

struct Shared {
    interner: Arc<Interner>,
    generation: usize,
    inputs: usize,
    bytes: usize,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared {
        interner: Arc::default(),
        generation: 0,
        inputs: 0,
        bytes: 0,
    });
}

/// The shared interner for an input of `len` bytes; every other generation has a capacity hint.
fn shared_interner(len: usize) -> Arc<Interner> {
    SHARED.with_borrow_mut(|shared| {
        if shared.inputs == SHARED_INPUTS || shared.bytes > SHARED_BYTES {
            shared.generation += 1;
            shared.interner = Arc::new(if shared.generation % 2 == 0 {
                Interner::new()
            } else {
                Interner::with_capacity(SHARED_INPUTS * 40)
            });
            shared.inputs = 0;
            shared.bytes = 0;
        }
        shared.inputs += 1;
        shared.bytes += len;
        Arc::clone(&shared.interner)
    })
}

fn read_and_print(reader: &ClassReader, options: ParsingOptions, interner: Arc<Interner>) -> Vec<u8> {
    let mut class_node = ClassNode::with_interner(interner);
    // A class node visited from a prefix of the events (the reader failed) must work too.
    let _ = reader.accept(&mut class_node, options);
    let mut text = Vec::new();
    class_node.accept(&mut TraceClassVisitor::new(&mut text));
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
    text
}

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    let shared = shared_interner(data.len());
    for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
        let fresh = read_and_print(&reader, options, Arc::default());
        let warm = read_and_print(&reader, options, Arc::clone(&shared));
        assert!(fresh == warm, "a shared interner prints a different tree");
    }
});
