//! A rough single-threaded read benchmark: `cargo run --release -p oxasm --example bench_read <dir>`.

use oxasm::{ClassReader, ClassVisitor, JavaStr, MethodVisitor, ParsingOptions};

struct Methods;
struct NoOp;
impl MethodVisitor for NoOp {}
impl ClassVisitor for Methods {
    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(NoOp))
    }
}

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
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut Methods, ParsingOptions::empty()))
                .unwrap();
        }
        if iteration >= 10 {
            println!("oxasm: {} classes in {:?}", classes.len(), start.elapsed());
        }
    }
}
