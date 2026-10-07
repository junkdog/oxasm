//! `ClassReader::accept` of a `ClassWriter` (with a new constant pool, with the reader's pool
//! copied, and with expanded frames), then `to_bytes`, never panics, hangs or runs out of memory:
//! it succeeds or returns a `WriteError` (oxasm-verification, "Fuzzing"). When it succeeds,
//! writing the class it wrote again gives the same Textifier dump (idempotence). The bytes need
//! not be the same: on malformed input, ASM's writer reorders constants and attributes from one
//! round trip to the next, and oxasm's does exactly the same (checked against the oracle).

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ParsingOptions};
use oxasm_util::TraceClassVisitor;

/// The Textifier dump of a class, if it can be read.
fn textify(class_file: &[u8]) -> Option<Vec<u8>> {
    let reader = ClassReader::new(class_file).ok()?;
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    reader.accept(&mut trace_class_visitor, ParsingOptions::empty()).ok()?;
    drop(trace_class_visitor);
    Some(output)
}

/// The class written by a class writer from the given reader, if the read and the write succeed.
fn write(reader: &ClassReader<'_>, copy_pool: bool, options: ParsingOptions) -> Option<Vec<u8>> {
    let mut writer = if copy_pool {
        ClassWriter::from_reader(reader, ClassWriterFlags::empty()).ok()?
    } else {
        ClassWriter::new(ClassWriterFlags::empty())
    };
    reader.accept(&mut writer, options).ok()?;
    writer.to_bytes().ok()
}

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    for (copy_pool, options) in [
        (false, ParsingOptions::empty()),
        (true, ParsingOptions::empty()),
        (false, ParsingOptions::EXPAND_FRAMES),
    ] {
        let Some(written) = write(&reader, copy_pool, options) else {
            continue;
        };
        // Writing the written class again changes nothing visible (if it can be read: the
        // writer writes what it is given, like ASM, so the output of a malformed class may be
        // malformed).
        let Ok(reread) = ClassReader::new(&written) else {
            continue;
        };
        if let Some(rewritten) = write(&reread, copy_pool, options) {
            assert!(
                textify(&written) == textify(&rewritten),
                "rewriting the written class changes its Textifier dump"
            );
        }
    }
});
