//! `ClassReader::accept` of a `TraceClassVisitor` with an ASMifier, directly and through a
//! `ClassNode`, never panics, hangs or runs out of memory (oxasm-verification, "Fuzzing").

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ParsingOptions};
use oxasm_tree::ClassNode;
use oxasm_util::{Asmifier, TraceClassVisitor};

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
        let mut trace_class_visitor =
            TraceClassVisitor::with_printer(Asmifier::new(), std::io::sink());
        let _ = reader.accept(&mut trace_class_visitor, options);
    }
    let mut class_node = ClassNode::default();
    if reader.accept(&mut class_node, ParsingOptions::empty()).is_ok() {
        class_node.accept(&mut TraceClassVisitor::with_printer(
            Asmifier::new(),
            std::io::sink(),
        ));
    }
});
