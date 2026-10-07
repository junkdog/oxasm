//! Analyzing every method of a class with the four stock interpreters never panics, hangs or
//! runs out of memory, and a failed analysis designates an instruction of the method, if any
//! (oxasm-verification, "Fuzzing").
//!
//! Like ASM's, an analysis takes memory in the number of instructions times the number of locals
//! and stack slots, which a method's header sets freely: the methods where that product is large
//! are skipped.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::opcodes::ACC_INTERFACE;
use oxasm::{ClassReader, ObjectHierarchy, ParsingOptions, Type};
use oxasm_analysis::{
    Analyzer, AnalyzerError, BasicInterpreter, BasicVerifier, Interpreter, SimpleVerifier,
    SourceInterpreter,
};
use oxasm_tree::{ClassNode, MethodNode};

/// The largest number of instructions times frame slots analyzed.
const MAX_FRAME_SLOTS: usize = 1 << 18;

fn check<I: Interpreter>(class_node: &ClassNode, method: &MethodNode, interpreter: I) {
    let mut analyzer = Analyzer::new(interpreter);
    match analyzer.analyze(&class_node.name, method) {
        Ok(frames) => assert!(frames.is_empty() || frames.len() == method.instructions.len()),
        Err(AnalyzerError { insn: Some(id), .. }) => {
            assert!(method.instructions.contains(id));
        }
        Err(_) => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    let mut class_node = ClassNode::new();
    let _ = reader.accept(&mut class_node, ParsingOptions::empty());
    for method in &class_node.methods {
        let slots = usize::from(method.max_locals) + usize::from(method.max_stack);
        if method.instructions.len().saturating_mul(slots) > MAX_FRAME_SLOTS {
            continue;
        }
        check(&class_node, method, BasicInterpreter);
        check(&class_node, method, BasicVerifier);
        check(&class_node, method, SourceInterpreter);
        let verifier = SimpleVerifier::for_class(
            Type::get_object_type(&class_node.name),
            class_node.super_name.as_ref().map(Type::get_object_type),
            class_node.interfaces.iter().map(Type::get_object_type).collect(),
            class_node.access & ACC_INTERFACE != 0,
            ObjectHierarchy,
        );
        check(&class_node, method, verifier);
        let mut method = method.clone();
        let _ = Analyzer::new(BasicInterpreter).analyze_and_compute_maxs(&class_node.name, &mut method);
    }
});
