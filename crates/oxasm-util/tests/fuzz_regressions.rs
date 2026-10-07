//! The inputs the fuzz targets found failing (`fuzz/regressions/<target>/`), as ordinary tests
//! (oxasm-verification, "Fuzzing"): reading and textifying them must not panic, and the
//! properties of the `compute` target hold on its inputs.

use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ObjectHierarchy, ParsingOptions};
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

/// The Textifier dump of a class.
fn textify(class_file: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    ClassReader::new(class_file)
        .unwrap()
        .accept(&mut trace_class_visitor, ParsingOptions::empty())
        .unwrap();
    drop(trace_class_visitor);
    output
}

/// The class a class writer with the given flags writes from the given class.
fn compute(class_file: &[u8], flags: ClassWriterFlags, options: ParsingOptions) -> Vec<u8> {
    let hierarchy = ObjectHierarchy;
    let mut class_writer = ClassWriter::new(flags).with_hierarchy(&hierarchy);
    ClassReader::new(class_file)
        .unwrap()
        .accept(&mut class_writer, options)
        .unwrap();
    class_writer.to_bytes().unwrap()
}

/// The frame computation is stable from the second rewrite on, but not on the first one, in ASM
/// too: the maximum number of locals counts the variable instructions of dead code, which the
/// first write replaces with `NOP ... ATHROW` (the `compute` target's first finding).
#[test]
fn compute_regressions() {
    let dir = oxasm_testkit::workspace_root().join("fuzz/regressions/compute");
    let path = dir.join("dead-code-max-locals-not-idempotent");
    let data = std::fs::read(&path).unwrap();
    let (flags, options) = (
        ClassWriterFlags::COMPUTE_FRAMES,
        ParsingOptions::SKIP_FRAMES,
    );
    let written = compute(&data, flags, options);
    let rewritten = compute(&written, flags, options);
    let rewritten_again = compute(&rewritten, flags, options);
    assert_ne!(textify(&written), textify(&rewritten));
    assert_eq!(textify(&rewritten), textify(&rewritten_again));
}
