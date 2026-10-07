//! A rough single-threaded benchmark of the analysis of every method of a set of classes, read
//! into `ClassNode`s beforehand: `cargo run --release -p oxasm-analysis --example bench_analyze
//! <dir> <basic|source>`.

use oxasm::{ClassReader, ParsingOptions};
use oxasm_analysis::{Analyzer, BasicInterpreter, Interpreter, SourceInterpreter};
use oxasm_tree::ClassNode;

fn walk(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "class") {
            out.push(std::fs::read(&path).unwrap());
        }
    }
}

/// Analyzes every method, and returns the number of frames.
fn analyze_all<I: Interpreter>(classes: &[ClassNode], interpreter: impl Fn() -> I) -> usize {
    let mut frames = 0;
    for class_node in classes {
        for method in &class_node.methods {
            let mut analyzer = Analyzer::new(interpreter());
            if let Ok(result) = analyzer.analyze(&class_node.name, method) {
                frames += result.iter().flatten().count();
            }
        }
    }
    frames
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("a directory of class files");
    let interpreter = args.next().unwrap_or_else(|| "basic".to_owned());
    let mut files = Vec::new();
    walk(std::path::Path::new(&dir), &mut files);
    let classes: Vec<ClassNode> = files
        .iter()
        .map(|class| {
            let mut class_node = ClassNode::new();
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
                .unwrap();
            class_node
        })
        .collect();
    for iteration in 0..15 {
        let start = std::time::Instant::now();
        let frames = match interpreter.as_str() {
            "basic" => analyze_all(&classes, || BasicInterpreter),
            "source" => analyze_all(&classes, || SourceInterpreter),
            other => panic!("unknown interpreter {other}"),
        };
        if iteration >= 10 {
            println!(
                "oxasm {interpreter}: {} classes, {frames} frames in {:?}",
                classes.len(),
                start.elapsed()
            );
        }
    }
}
