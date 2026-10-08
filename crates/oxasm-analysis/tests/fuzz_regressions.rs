//! The inputs the `analyze` fuzz target found failing (`fuzz/regressions/analyze/`), as ordinary
//! tests (oxasm-verification, "Fuzzing"), and oxasm-specific tests of the recursion bounds.

use oxasm::opcodes::ACC_INTERFACE;
use oxasm::{ClassReader, ObjectHierarchy, ParsingOptions, Type};
use oxasm_analysis::{
    Analyzer, BasicInterpreter, BasicVerifier, Interpreter, SimpleVerifier, SourceInterpreter,
};
use oxasm_tree::ClassNode;

/// A stack small enough that an unbounded recursion overflows it quickly.
const STACK_SIZE: usize = 1024 * 1024;

fn analyze_all<I: Interpreter>(class_node: &ClassNode, interpreter: impl Fn() -> I) {
    for method in &class_node.methods {
        let _ = Analyzer::new(interpreter()).analyze(class_node.resolve(class_node.name), method);
    }
}

#[test]
fn analyze_regressions() {
    let dir = oxasm_testkit::workspace_root().join("fuzz/regressions/analyze");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let data = std::fs::read(entry.unwrap().path()).unwrap();
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let Ok(reader) = ClassReader::new(&data) else {
                    return;
                };
                let mut class_node = ClassNode::new();
                let _ = reader.accept(&mut class_node, ParsingOptions::empty());
                analyze_all(&class_node, || BasicInterpreter);
                analyze_all(&class_node, || BasicVerifier);
                analyze_all(&class_node, || SourceInterpreter);
                analyze_all(&class_node, || {
                    let resolve = |sym| Type::get_object_type(class_node.resolve(sym));
                    SimpleVerifier::for_class(
                        resolve(class_node.name),
                        class_node.super_name.map(resolve),
                        class_node.interfaces.iter().copied().map(resolve).collect(),
                        class_node.access & ACC_INTERFACE != 0,
                        ObjectHierarchy,
                    )
                });
            })
            .unwrap()
            .join()
            .unwrap();
        count += 1;
    }
    assert!(count > 0, "no regression inputs in {}", dir.display());
}

/// A class that is its own super class makes ASM's `isAssignableFrom` recurse until a
/// `StackOverflowError`; oxasm's fails at its recursion bound instead.
#[test]
fn simple_verifier_is_assignable_from_cyclic_super_class() {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(|| {
            let c = Type::get_object_type("C");
            let verifier = SimpleVerifier::for_class(
                c.clone(),
                Some(c.clone()),
                Vec::new(),
                false,
                ObjectHierarchy,
            );
            let error = verifier
                .is_assignable_from(&Type::get_object_type("D"), &c)
                .unwrap_err();
            assert_eq!("java.lang.StackOverflowError", error.message.to_string());
        })
        .unwrap()
        .join()
        .unwrap();
}
