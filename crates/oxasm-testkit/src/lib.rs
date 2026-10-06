//! Test support shared by the oxasm crates: the T0 corpus, the committed oracle goldens, and
//! (from P2) the `ClassFile` structural dumper. This is the port of ASM's `asm-test` module.
//!
//! Not published. It deliberately does not depend on `oxasm`, so every oxasm crate can use it
//! as a dev-dependency without a dependency cycle.

pub mod golden;
pub mod precompiled;
pub mod signatures;

use std::path::{Path, PathBuf};

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
