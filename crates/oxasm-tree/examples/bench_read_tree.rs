//! A rough single-threaded benchmark of reading classes into `ClassNode`s (the `read_tree`
//! scenario): `cargo run --release -p oxasm-tree --example bench_read_tree <dir>`.

use oxasm::{ClassReader, ParsingOptions};
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
    let mut classes = Vec::new();
    walk(std::path::Path::new(&dir), &mut classes);
    for iteration in 0..15 {
        let start = std::time::Instant::now();
        for class in &classes {
            let mut class_node = ClassNode::new();
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
                .unwrap();
            std::hint::black_box(&class_node);
        }
        if iteration >= 10 {
            println!("oxasm: {} classes in {:?}", classes.len(), start.elapsed());
        }
    }
}
