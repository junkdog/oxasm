//! A rough benchmark of reading classes into `ClassNode`s (the `read_tree` scenario):
//! `cargo run --release -p oxasm-tree --example bench_read_tree <dir> [mode] [threads]`.
//!
//! The mode says where the class nodes intern their strings:
//!
//! - `fresh` (the default): a new interner per class;
//! - `shared`: one interner per pass over the classes, which starts empty (cold);
//! - `warm`: one interner for every pass, so that all but the first find their strings;
//! - `per-thread`: one interner per thread and pass, which starts empty: `shared` without the
//!   sharing between threads.
//!
//! With `threads` (1 by default), that many threads take the classes of a pass from a common
//! counter.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Reads the classes the counter hands out, until there are none left.
fn read(classes: &[Vec<u8>], next: &AtomicUsize, interner: Option<&Arc<Interner>>) {
    loop {
        let Some(class) = classes.get(next.fetch_add(1, Ordering::Relaxed)) else {
            return;
        };
        let mut class_node = ClassNode::with_interner(interner.cloned().unwrap_or_default());
        ClassReader::new(class)
            .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
            .unwrap();
        std::hint::black_box(&class_node);
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("a directory of class files");
    let mode = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "fresh".to_owned());
    let threads: usize = std::env::args().nth(3).map_or(1, |t| t.parse().unwrap());
    let mut classes = Vec::new();
    walk(std::path::Path::new(&dir), &mut classes);
    let warm = Arc::new(Interner::new());
    for iteration in 0..15 {
        let pass = match mode.as_str() {
            "fresh" | "per-thread" => None,
            "shared" => Some(Arc::new(Interner::new())),
            "warm" => Some(warm.clone()),
            mode => panic!("unknown mode {mode}: fresh, shared, warm or per-thread"),
        };
        let next = AtomicUsize::new(0);
        let start = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    let own = (mode == "per-thread").then(|| Arc::new(Interner::new()));
                    read(&classes, &next, own.as_ref().or(pass.as_ref()));
                });
            }
        });
        if iteration >= 10 {
            println!(
                "oxasm ({mode}, {threads} threads): {} classes in {:?}",
                classes.len(),
                start.elapsed()
            );
        }
    }
}
