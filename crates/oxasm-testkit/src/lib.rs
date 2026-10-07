//! Test support shared by the oxasm crates: the T0 corpus, the committed oracle goldens, and
//! the `ClassFile` structural dumper (`classfile_dump`). This is the port of ASM's `asm-test` module.
//!
//! Not published. It deliberately does not depend on `oxasm`, so every oxasm crate can use it
//! as a dev-dependency without a dependency cycle.

pub mod classfile_dump;
pub mod golden;
pub mod java_fmt;
pub mod jvm;
pub mod oracle_records;
pub mod precompiled;
pub mod signatures;

use std::path::{Path, PathBuf};

pub use classfile_dump::{ClassFile, ClassFormatError};
pub use precompiled::{InvalidClass, JdkVersion, PrecompiledClass};

/// The workspace root.
pub fn workspace_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The T0 corpus directory (`corpus/t0`).
pub fn corpus_t0() -> PathBuf {
    workspace_root().join("corpus/t0")
}

/// A class file from another ASM module's test resources, e.g.
/// `t0_resource("asm-commons", "SerialVersionClass.class")`; panics if it does not exist.
#[track_caller]
pub fn t0_resource(module: &str, file: &str) -> Vec<u8> {
    let path = corpus_t0().join("resources").join(module).join(file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Runs `f`, which must panic, and returns its panic message: what ASM's tests get with
/// `assertThrows(...).getMessage()` from the code that panics where ASM throws (the check
/// adapters, ADR-06). Panics itself if `f` does not panic.
#[track_caller]
pub fn panic_message<R>(f: impl FnOnce() -> R) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(_) => panic!("expected a panic"),
        Err(payload) => payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "<non string panic payload>".to_owned()),
    }
}

/// Runs `f`, and returns whether it panics (ASM's `assertThrows`/`assertDoesNotThrow` pairs).
pub fn panics<R>(f: impl FnOnce() -> R) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}
