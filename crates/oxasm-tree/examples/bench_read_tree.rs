//! A rough single-threaded benchmark of reading classes into `ClassNode`s (the `read_tree`
//! scenario): `cargo run --release -p oxasm-tree --example bench_read_tree <dir> [mode]`.
//!
//! The mode says where the class nodes intern their strings:
//!
//! - `fresh` (the default): `ClassNode::new()` per class, each with a new interner;
//! - `shared`: one interner per pass over the classes, which starts empty (cold);
//! - `warm`: one interner for every pass, so that all but the first find their strings.

use std::sync::Arc;

use oxasm::{ClassReader, Interner, ParsingOptions};
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

fn main() {
    let dir = std::env::args().nth(1).expect("a directory of class files");
    let mode = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "fresh".to_owned());
    let mut classes = Vec::new();
    walk(std::path::Path::new(&dir), &mut classes);
    let warm = Arc::new(Interner::new());
    for iteration in 0..15 {
        let pass = match mode.as_str() {
            "fresh" => None,
            "shared" => Some(Arc::new(Interner::new())),
            "warm" => Some(warm.clone()),
            mode => panic!("unknown mode {mode}: fresh, shared or warm"),
        };
        let start = std::time::Instant::now();
        for class in &classes {
            let mut class_node = match &pass {
                Some(interner) => ClassNode::with_interner(interner.clone()),
                None => ClassNode::new(),
            };
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
                .unwrap();
            std::hint::black_box(&class_node);
        }
        if iteration >= 10 {
            println!(
                "oxasm ({mode}): {} classes in {:?}",
                classes.len(),
                start.elapsed()
            );
        }
    }
}
