//! ASM's `ClassFile.newInstance()` assertions ("the JVM accepts this class"), through the oracle's
//! `verify` command (oxasm-verification, "Test porting conventions").
//!
//! The JVM runs out of process (`cargo xtask oracle verify`), so the tests that use this are
//! `#[ignore = "needs the JVM oracle"]` and run by CI's oracle job (`cargo test -- --ignored`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::oracle_records::{OracleStatus, parse_oracle_records};
use crate::workspace_root;

/// Defines the given classes in a fresh class loader of a JVM (which resolves each one's
/// references to the others), and links each of them, which verifies its bytecode (with
/// `-XX:+BytecodeVerificationLocal`). Returns the first class the JVM rejects, with its error.
///
/// # Panics
///
/// If the oracle cannot run (no Java, no ASM jars).
pub fn verify_classes(classes: &[&[u8]]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir: PathBuf = workspace_root().join("target/testkit-verify").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create the class directory");
    for (i, class) in classes.iter().enumerate() {
        std::fs::write(dir.join(format!("C{i}.class")), class).expect("write a class file");
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["xtask", "oracle", "verify"])
        .arg(&dir)
        .output()
        .expect("run cargo xtask oracle verify");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "the oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for record in parse_oracle_records(&output.stdout) {
        if record.status == OracleStatus::Err {
            return Err(format!(
                "{}: {}",
                record.id,
                String::from_utf8_lossy(&record.payload)
            ));
        }
    }
    Ok(())
}

/// [`verify_classes`] for one class: ASM's `new ClassFile(bytes).newInstance()`.
pub fn verify_class(class: &[u8]) -> Result<(), String> {
    verify_classes(&[class])
}
