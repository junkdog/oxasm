//! `ClassReader::accept` of a `TraceClassVisitor` (Textifier) never panics, hangs or runs out of
//! memory: this exercises every visitor argument path (oxasm-verification, "Fuzzing").

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ParsingOptions};
use oxasm_util::TraceClassVisitor;

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
        let mut trace_class_visitor = TraceClassVisitor::new(std::io::sink());
        let _ = reader.accept(&mut trace_class_visitor, options);
    }
});
