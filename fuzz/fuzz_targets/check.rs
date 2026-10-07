//! Checking a class with the check adapters never panics, hangs or runs out of memory, except
//! with the panics of the checks themselves, which are ASM's exceptions (`java.lang.*`): with a
//! `CheckClassAdapter` (data flow checks, without and with a class writer downstream, and without
//! data flow checks), and with `check::verify` (oxasm-verification, "Fuzzing").
//!
//! Like ASM's, the data flow checks take memory in the number of instructions times the number of
//! locals and stack slots, which a method's header (or its variable instructions) sets freely:
//! the classes where that product is large are skipped.

#![no_main]

use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use libfuzzer_sys::fuzz_target;
use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ObjectHierarchy, ParsingOptions};
use oxasm_tree::{ClassNode, Insn};
use oxasm_util::check::{self, CheckClassAdapter};
use oxasm_util::trace::NoDelegate;

/// The largest number of instructions times frame slots analyzed.
const MAX_FRAME_SLOTS: usize = 1 << 18;

/// The message of a panic, if it is a string.
fn message<'a>(payload: &'a (dyn std::any::Any + Send)) -> Option<&'a str> {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
}

/// Whether the analyses of the class's methods are small enough.
fn small_enough(reader: &ClassReader<'_>) -> bool {
    let mut class_node = ClassNode::new();
    let _ = reader.accept(&mut class_node, ParsingOptions::empty());
    class_node.methods.iter().all(|method| {
        let max_var = method
            .instructions
            .iter()
            .filter_map(|insn| match &insn.insn {
                Insn::Var { var, .. } | Insn::Iinc { var, .. } => Some(usize::from(*var) + 2),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let slots =
            usize::from(method.max_locals).max(max_var) + usize::from(method.max_stack);
        method.instructions.len().saturating_mul(slots + 4) <= MAX_FRAME_SLOTS
    })
}

/// Runs `f`, and fails on any panic other than a check's.
fn checking(f: impl FnOnce()) {
    if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(f)) {
        if !message(&*payload).is_some_and(|m| m.starts_with("java.lang.")) {
            panic::resume_unwind(payload);
        }
    }
}

static QUIET_CHECK_PANICS: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    // libfuzzer's panic hook aborts: let the checks' panics unwind (to `checking`) quietly.
    QUIET_CHECK_PANICS.call_once(|| {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !message(info.payload()).is_some_and(|m| m.starts_with("java.lang.")) {
                hook(info);
            }
        }));
    });

    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    if !small_enough(&reader) {
        return;
    }
    checking(|| {
        let _ = reader.accept(&mut CheckClassAdapter::new(NoDelegate), ParsingOptions::empty());
    });
    checking(|| {
        let mut adapter = CheckClassAdapter::with_check_data_flow(NoDelegate, false);
        let _ = reader.accept(&mut adapter, ParsingOptions::empty());
    });
    checking(|| {
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        let _ = reader.accept(
            &mut CheckClassAdapter::new(&mut class_writer),
            ParsingOptions::empty(),
        );
    });
    checking(|| {
        let _ = check::verify(&reader, &ObjectHierarchy, true, &mut std::io::sink());
    });
});
