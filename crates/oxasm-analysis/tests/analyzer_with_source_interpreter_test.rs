//! Port of ASM's `AnalyzerWithSourceInterpreterTest`: unit tests for `Analyzer`, when used with a
//! `SourceInterpreter`.
//!
//! - `testConstructor` keeps its non-throwing half: there is no subclassing, so the
//!   `IllegalStateException` of `new SourceInterpreter() {}` cannot be written (ADR-09).
//! - The `Api` axis of `testAnalyze_sourceInterpreter` is dropped (ADR-09): it loops over
//!   `PrecompiledClass::ALL`.

mod common;

use std::sync::Arc;

use common::MethodNodeBuilder;
use oxasm::opcodes::*;
use oxasm::{ClassReader, Label, ParsingOptions, jstr};
use oxasm_analysis::{Analyzer, SetArena, SourceInterpreter};
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, InsnId};

#[test]
fn test_constructor() {
    let _ = SourceInterpreter::new(&SetArena::new());
    // ASM: assertThrows(IllegalStateException.class, () -> new SourceInterpreter() {}) -
    // dropped, there is no subclassing (ADR-09).
}

/// Tests that the precompiled classes can be successfully analyzed with a SourceInterpreter.
#[test]
fn test_analyze_source_interpreter() {
    for &class_parameter in PrecompiledClass::ALL {
        let mut class_node = ClassNode::with_interner(Arc::default());
        ClassReader::new(class_parameter.bytes())
            .unwrap()
            .accept(&mut class_node, ParsingOptions::empty())
            .unwrap();
        let sets = SetArena::new();
        let mut analyzer = Analyzer::new(SourceInterpreter::new(&sets));

        for method_node in &class_node.methods {
            if let Err(e) = analyzer.analyze(class_node.resolve(class_node.name), method_node) {
                panic!(
                    "{class_parameter} {}{}: {e}",
                    method_node.resolve(method_node.name),
                    method_node.resolve(method_node.desc)
                );
            }
        }
    }
}

/// Checks if DUP_X2 producers are correct.
#[test]
fn test_analyze_dupx2_producers() {
    let label0 = Label::new();
    let label1 = Label::new();
    let method_node = MethodNodeBuilder::with_maxs(4, 1)
        .push()
        .push()
        .iconst_0()
        .ifne(label0)
        // First case
        .insn(ICONST_M1)
        .go(label1)
        // Second case
        .label(label0)
        .iconst_0()
        // DUP_X2 value
        .label(label1)
        .insn(DUP_X2)
        .pop() // Point where the frame is checked
        .pop()
        .pop()
        .pop()
        .vreturn()
        .build();

    let sets = SetArena::new();
    let mut analyzer = Analyzer::new(SourceInterpreter::new(&sets));
    analyzer.analyze(jstr!("C"), &method_node).unwrap();

    let first_opcode = |opcode: u8| -> InsnId {
        method_node
            .instructions
            .iter_with_ids()
            .find(|(_, insn)| insn.opcode() == Some(opcode))
            .unwrap()
            .0
    };
    let first_pop = first_opcode(POP);
    let dupx2 = first_opcode(DUP_X2);
    let frame = analyzer
        .get_frames()
        .get(method_node.instructions.index_of(first_pop).unwrap())
        .unwrap();
    // Check if all source values have the DUP_X2 as a producer
    let source_value1 = frame.get_stack(frame.get_stack_size() - 4);
    let source_value2 = frame.get_stack(frame.get_stack_size() - 3);
    let source_value3 = frame.get_stack(frame.get_stack_size() - 2);
    let source_value4 = frame.get_stack(frame.get_stack_size() - 1);
    assert_eq!(source_value1.insns.iter().next(), Some(dupx2));
    assert_eq!(source_value2.insns.iter().next(), Some(dupx2));
    assert_eq!(source_value3.insns.iter().next(), Some(dupx2));
    assert_eq!(source_value4.insns.iter().next(), Some(dupx2));
}
