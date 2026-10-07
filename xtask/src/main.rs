//! Development tasks for oxasm: `cargo xtask <task>`.
//!
//! See `vault-docs/oxasm-verification.md` for the oracle, corpus tiers and goldens these tasks
//! manage.

mod analysis;
mod args;
mod commons;
mod corpus;
mod golden;
mod oracle;
mod parity;
mod pin;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::args::Args;

const USAGE: &str = "\
usage: cargo xtask <task> [options]

tasks:
  oracle <command> [options] <inputs...>
        run the ASM 9.10.1 oracle (oracle/Oracle.java) directly; output goes to stdout
  golden [--check | --regen]
        compare the T0 goldens under crates/oxasm-testkit/goldens against a fresh oracle run
        (--check, the default), or rewrite them (--regen); either way, the default-options
        Textifier goldens must equal ASM's own asm-util test resources
  corpus t0 [--asm <dir>] [--allow-dirty]
        import the T0 corpus and upstream goldens from an ASM checkout at tag ASM_9_10_1
        (default: $OXASM_ASM_SRC, else ~/opt/dev/jvm/asm)
  corpus t1
        extract the JDK runtime image of $JAVA_HOME into corpus/t1
  corpus t2
        fetch the jars listed in corpus/t2.txt into corpus/t2
  parity --tier <t0|t1|t2|dir:<path>> [--mode <mode>]... [--sample <n>] [--jobs <n>]
         [--hierarchy <file>] [--oracle-only]
        fill the oracle result cache under target/oracle/<tier>/ for each mode, then compare
        oxasm's output with it (mismatches are written under target/parity/<tier>/<mode>/);
        default modes: those with an oxasm side, i.e. the Textifier ones (textify,
        textify-skip-code, textify-skip-debug, textify-skip-frames, textify-expand-frames),
        the same through a ClassNode (tree-textify, ...), the ASMifier ones (asmify,
        asmify-skip-code, ..., tree-asmify), the writer ones (copy, plain, maxs, frames, ...)
        and the analysis ones (analyze-basic, analyze-basicverifier, analyze-simpleverifier,
        analyze-source)
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = Args::from_env();
    let Some(task) = args.next_positional() else {
        eprint!("{USAGE}");
        bail!("missing task");
    };
    match task.as_str() {
        "oracle" => oracle::cli(args),
        "golden" => golden::cli(args),
        "corpus" => corpus::cli(args),
        "parity" => parity::cli(args),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => {
            eprint!("{USAGE}");
            bail!("unknown task {other:?}")
        }
    }
}

/// The workspace root (the parent of this crate's directory).
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
}

fn root_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

/// Lists the regular files under `dir` recursively, sorted by path.
fn walk_files(dir: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                walk(&entry.path(), out)?;
            } else if file_type.is_file() {
                out.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn is_class_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("class" | "clazz")
    )
}

/// `path` relative to `base`, with `/` separators.
fn relative_id(base: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(base).unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}
