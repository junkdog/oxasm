//! `ClassReader::accept` of a `ClassWriter` that computes the maximum stack sizes
//! (`COMPUTE_MAXS`) or the stack map frames (`COMPUTE_FRAMES`, with an `ObjectHierarchy`, so
//! that every merge of two classes succeeds and the fix point runs to the end), then `to_bytes`,
//! never panics, hangs or runs out of memory: it succeeds or returns a `WriteError`
//! (oxasm-verification, "Fuzzing"). When it succeeds, computing again is stable from the second
//! write on: writing the class written from the written class gives the same Textifier dump.
//! (The first rewrite may differ, in ASM too: the maximum number of locals counts the variable
//! instructions of dead code, which the first write replaces with `NOP ... ATHROW`.)

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ObjectHierarchy, ParsingOptions};
use oxasm_util::TraceClassVisitor;

/// The Textifier dump of a class, if it can be read.
fn textify(class_file: &[u8]) -> Option<Vec<u8>> {
    let reader = ClassReader::new(class_file).ok()?;
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
    reader
        .accept(&mut trace_class_visitor, ParsingOptions::empty())
        .ok()?;
    drop(trace_class_visitor);
    Some(output)
}

/// The class written by a class writer with the given flags from the given reader, if the read
/// and the write succeed.
fn write(
    reader: &ClassReader<'_>,
    flags: ClassWriterFlags,
    options: ParsingOptions,
) -> Option<Vec<u8>> {
    let hierarchy = ObjectHierarchy;
    let mut writer = ClassWriter::new(flags).with_hierarchy(&hierarchy);
    reader.accept(&mut writer, options).ok()?;
    writer.to_bytes().ok()
}

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    for (flags, options) in [
        (ClassWriterFlags::COMPUTE_MAXS, ParsingOptions::empty()),
        (
            ClassWriterFlags::COMPUTE_FRAMES,
            ParsingOptions::SKIP_FRAMES,
        ),
        (ClassWriterFlags::COMPUTE_FRAMES, ParsingOptions::empty()),
    ] {
        let Some(written) = write(&reader, flags, options) else {
            continue;
        };
        // Computing again from the class written from the written class changes nothing
        // visible (if they can be read: the writer writes what it is given, like ASM, so the
        // output of a malformed class may be malformed).
        let Some(rewritten) = ClassReader::new(&written)
            .ok()
            .and_then(|reread| write(&reread, flags, options))
        else {
            continue;
        };
        let Ok(reread) = ClassReader::new(&rewritten) else {
            continue;
        };
        if let Some(rewritten_again) = write(&reread, flags, options) {
            assert!(
                textify(&rewritten) == textify(&rewritten_again),
                "computing again from the rewritten class changes its Textifier dump"
            );
        }
    }
});
