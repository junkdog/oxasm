//! oxasm-specific: the P3 gate on T0. A class read into a `ClassNode`, then visited from it by a
//! Textifier, prints exactly as in ASM (the oracle's `tree-textify`), for every T0 class and
//! reader option. ASM's tree path differs from its direct path for a few classes (Code attributes
//! and try catch block annotations are visited in another order), so the goldens are ASM's tree
//! path's, not the direct Textifier goldens.

use oxasm::{ClassReader, ParsingOptions};
use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_tree::ClassNode;
use oxasm_util::TraceClassVisitor;

fn parsing_options(options: ReaderOptions) -> ParsingOptions {
    match options {
        ReaderOptions::Default => ParsingOptions::empty(),
        ReaderOptions::SkipCode => ParsingOptions::SKIP_CODE,
        ReaderOptions::SkipDebug => ParsingOptions::SKIP_DEBUG,
        ReaderOptions::SkipFrames => ParsingOptions::SKIP_FRAMES,
        ReaderOptions::ExpandFrames => ParsingOptions::EXPAND_FRAMES,
    }
}

/// The Textifier output of the class read into a `ClassNode`, or the reader's error.
fn tree_textify(class_file: &[u8], options: ParsingOptions) -> Result<Vec<u8>, String> {
    let mut class_node = ClassNode::default();
    ClassReader::new(class_file)
        .and_then(|reader| reader.accept(&mut class_node, options))
        .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    class_node.accept(&mut trace_class_visitor);
    trace_class_visitor.take_io_error().unwrap();
    Ok(output)
}

/// The names of the T0 classes (those of the direct Textifier goldens).
fn t0_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(golden::goldens_dir().join("textifier/default"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            path.file_stem().unwrap().to_str().unwrap().to_owned()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn tree_textify_goldens() {
    let mut checked = 0;
    for name in t0_names() {
        let base = oxasm_testkit::corpus_t0().join(name.replace('.', "/"));
        let class_file = std::fs::read(base.with_extension("class"))
            .or_else(|_| std::fs::read(base.with_extension("clazz")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for options in ReaderOptions::ALL {
            let actual = tree_textify(&class_file, parsing_options(options));
            match golden::textifier_tree(&name, options) {
                Golden::Ok(expected) => {
                    let actual = actual.unwrap_or_else(|e| panic!("{options:?} {name}: {e}"));
                    if actual != expected {
                        let (actual, expected) = (
                            String::from_utf8_lossy(&actual),
                            String::from_utf8_lossy(&expected),
                        );
                        let line = actual
                            .lines()
                            .zip(expected.lines())
                            .position(|(a, e)| a != e)
                            .map_or_else(
                                || "the line counts differ".to_owned(),
                                |i| {
                                    format!(
                                        "line {}: {:?} != {:?}",
                                        i + 1,
                                        actual.lines().nth(i).unwrap(),
                                        expected.lines().nth(i).unwrap()
                                    )
                                },
                            );
                        panic!("{options:?} {name}: tree Textifier output differs: {line}");
                    }
                }
                Golden::Err(asm_error) => {
                    assert!(
                        actual.is_err(),
                        "{options:?} {name}: ASM fails ({asm_error})"
                    );
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 300);
}
