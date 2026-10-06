//! The inputs the fuzz targets found failing (`fuzz/regressions/<target>/`), as ordinary tests
//! (oxasm-verification, "Fuzzing"): reading and textifying them must not panic.

use oxasm::{ClassReader, ParsingOptions};
use oxasm_util::TraceClassVisitor;

#[test]
fn read_textify_regressions() {
    let dir = oxasm_testkit::workspace_root().join("fuzz/regressions/read_textify");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let data = std::fs::read(&path).unwrap();
        let Ok(reader) = ClassReader::new(&data) else {
            continue;
        };
        for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
            let mut trace_class_visitor = TraceClassVisitor::new(std::io::sink());
            let _ = reader.accept(&mut trace_class_visitor, options);
        }
        count += 1;
    }
    assert!(count > 0, "no regression inputs in {}", dir.display());
}
