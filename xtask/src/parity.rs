//! Parity sweeps over a corpus tier. Each sweep first fills the oracle result cache,
//! `target/oracle/<tier>/<mode>/<sha256>.{ok,err}`, keyed by the SHA-256 of each class file, so
//! that later sweeps only pay for classes they have not seen; then (unless `--oracle-only`) it
//! computes oxasm's side of each mode and compares the two.
//!
//! The oxasm side exists for the Textifier modes (P2), direct and through a `ClassNode` (P3), the
//! analysis modes (P4), the writer modes that compute nothing (P5), `maxs` and `frames` (P6), and
//! the ASMifier and check adapter modes (P7).
//! A class on which ASM fails agrees with oxasm iff oxasm fails too ("both reject").

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail, ensure};
use oxasm::{
    ClassHierarchy, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Interner, JavaStr,
    MapHierarchy, ParsingOptions,
};
use oxasm_tree::ClassNode;
use oxasm_util::check::CheckClassAdapter;
use oxasm_util::trace::NoDelegate;
use oxasm_util::{Asmifier, Printer, Textifier, TraceClassVisitor};

use crate::analysis::{self, Interp};
use crate::args::Args;
use crate::commons::{self, Commons, Mapping};
use crate::oracle::{Oracle, ServeRequest};
use crate::{is_class_file, relative_id, root_path, sha256_hex, walk_files};

/// A mode of the byte-parity matrix (oxasm-verification, "Byte parity"), and the oracle command
/// that produces ASM's side of it.
struct Mode {
    name: &'static str,
    command: &'static str,
    needs_hierarchy: bool,
    /// For a Textifier mode, the reader options: oxasm's side is a `TraceClassVisitor` output.
    textify: Option<ParsingOptions>,
    /// For a Textifier mode, whether the class is read into a `ClassNode` first (which then
    /// visits the `TraceClassVisitor`).
    tree: bool,
    /// For a Textifier mode, whether the printer is an ASMifier instead (P7).
    asmify: bool,
    /// For an analysis mode, the interpreter: oxasm's side is the oracle's `analyze` output.
    analyze: Option<Interp>,
    /// For a writer mode, how the class is written: oxasm's side is the class file.
    roundtrip: Option<Roundtrip>,
    /// For a check mode, how the class is checked: oxasm's side is "OK", or the panic message
    /// of a check adapter (ASM's exception), or the output of `verify`.
    check: Option<Check>,
    /// For a commons mode, the adapter the class goes through (P8): oxasm's side is the class
    /// file, or the text of `code-size`.
    commons: Option<Commons>,
    /// Whether the mode needs the tier's remapping (`remap`).
    needs_mapping: bool,
}

/// How a check mode checks a class (the oracle's `check --mode`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    /// A `CheckClassAdapter` with data flow checks, without a downstream visitor.
    Plain,
    /// A `CheckClassAdapter` without data flow checks.
    NoDataFlow,
    /// A `CheckClassAdapter` with data flow checks, and a `ClassWriter` without flags
    /// downstream (so that the max stack sizes and the frames are checked too).
    Writer,
    /// `check::verify`, with the frames of each method.
    Verify,
}

/// How a writer mode writes a class.
#[derive(Clone, Copy)]
struct Roundtrip {
    /// The class writer flags.
    flags: ClassWriterFlags,
    /// The reader options (`frames` adds `SKIP_FRAMES`, as the oracle does).
    options: ParsingOptions,
    /// Whether the class writer copies the constant pool of the reader.
    copy_pool: bool,
}

impl Mode {
    /// Whether oxasm computes its side of this mode (yet).
    fn has_oxasm_side(&self) -> bool {
        self.textify.is_some()
            || self.analyze.is_some()
            || self.roundtrip.is_some()
            || self.check.is_some()
            || self.commons.is_some_and(|c| c.has_oxasm_side())
    }
}

const MODES: [Mode; 45] = [
    Mode::textify("textify", "textify", ParsingOptions::empty()),
    Mode::textify(
        "textify-skip-code",
        "textify --opts SKIP_CODE",
        ParsingOptions::SKIP_CODE,
    ),
    Mode::textify(
        "textify-skip-debug",
        "textify --opts SKIP_DEBUG",
        ParsingOptions::SKIP_DEBUG,
    ),
    Mode::textify(
        "textify-skip-frames",
        "textify --opts SKIP_FRAMES",
        ParsingOptions::SKIP_FRAMES,
    ),
    Mode::textify(
        "textify-expand-frames",
        "textify --opts EXPAND_FRAMES",
        ParsingOptions::EXPAND_FRAMES,
    ),
    Mode::tree_textify("tree-textify", "tree-textify", ParsingOptions::empty()),
    Mode::tree_textify(
        "tree-textify-skip-code",
        "tree-textify --opts SKIP_CODE",
        ParsingOptions::SKIP_CODE,
    ),
    Mode::tree_textify(
        "tree-textify-skip-debug",
        "tree-textify --opts SKIP_DEBUG",
        ParsingOptions::SKIP_DEBUG,
    ),
    Mode::tree_textify(
        "tree-textify-skip-frames",
        "tree-textify --opts SKIP_FRAMES",
        ParsingOptions::SKIP_FRAMES,
    ),
    Mode::tree_textify(
        "tree-textify-expand-frames",
        "tree-textify --opts EXPAND_FRAMES",
        ParsingOptions::EXPAND_FRAMES,
    ),
    Mode::asmify("asmify", "asmify", ParsingOptions::empty()),
    Mode::asmify(
        "asmify-skip-code",
        "asmify --opts SKIP_CODE",
        ParsingOptions::SKIP_CODE,
    ),
    Mode::asmify(
        "asmify-skip-debug",
        "asmify --opts SKIP_DEBUG",
        ParsingOptions::SKIP_DEBUG,
    ),
    Mode::asmify(
        "asmify-skip-frames",
        "asmify --opts SKIP_FRAMES",
        ParsingOptions::SKIP_FRAMES,
    ),
    Mode::asmify(
        "asmify-expand-frames",
        "asmify --opts EXPAND_FRAMES",
        ParsingOptions::EXPAND_FRAMES,
    ),
    Mode {
        tree: true,
        ..Mode::asmify("tree-asmify", "tree-asmify", ParsingOptions::empty())
    },
    Mode::roundtrip("copy", "roundtrip --mode 0 --copy-pool", Roundtrip::copy()),
    Mode::roundtrip("plain", "roundtrip --mode 0", Roundtrip::plain()),
    Mode::roundtrip(
        "maxs",
        "roundtrip --mode maxs",
        Roundtrip::flags(ClassWriterFlags::COMPUTE_MAXS),
    ),
    Mode {
        needs_hierarchy: true,
        ..Mode::roundtrip(
            "frames",
            "roundtrip --mode frames",
            Roundtrip::flags(ClassWriterFlags::COMPUTE_FRAMES),
        )
    },
    Mode::roundtrip(
        "expand",
        "roundtrip --mode 0 --opts EXPAND_FRAMES",
        Roundtrip::options(ParsingOptions::EXPAND_FRAMES),
    ),
    Mode::roundtrip(
        "skip-debug",
        "roundtrip --mode 0 --opts SKIP_DEBUG",
        Roundtrip::options(ParsingOptions::SKIP_DEBUG),
    ),
    Mode::roundtrip(
        "skip-code",
        "roundtrip --mode 0 --opts SKIP_CODE",
        Roundtrip::options(ParsingOptions::SKIP_CODE),
    ),
    Mode::tree_roundtrip(
        "tree-copy",
        "tree-roundtrip --mode 0 --copy-pool",
        Roundtrip::copy(),
    ),
    Mode::tree_roundtrip("tree-plain", "tree-roundtrip --mode 0", Roundtrip::plain()),
    Mode::tree_roundtrip(
        "tree-maxs",
        "tree-roundtrip --mode maxs",
        Roundtrip::flags(ClassWriterFlags::COMPUTE_MAXS),
    ),
    Mode {
        needs_hierarchy: true,
        ..Mode::tree_roundtrip(
            "tree-frames",
            "tree-roundtrip --mode frames",
            Roundtrip::flags(ClassWriterFlags::COMPUTE_FRAMES),
        )
    },
    Mode::analyze("analyze-basic", "analyze --interp basic", Interp::Basic),
    Mode::analyze(
        "analyze-basicverifier",
        "analyze --interp basicverifier",
        Interp::BasicVerifier,
    ),
    Mode {
        needs_hierarchy: true,
        ..Mode::analyze(
            "analyze-simpleverifier",
            "analyze --interp simpleverifier",
            Interp::SimpleVerifier,
        )
    },
    Mode::analyze("analyze-source", "analyze --interp source", Interp::Source),
    Mode::check("check-plain", "check --mode plain", Check::Plain),
    Mode::check(
        "check-nodataflow",
        "check --mode nodataflow",
        Check::NoDataFlow,
    ),
    Mode::check("check-writer", "check --mode writer", Check::Writer),
    Mode {
        needs_hierarchy: true,
        ..Mode::check("check-verify", "check --mode verify", Check::Verify)
    },
    Mode {
        needs_mapping: true,
        ..Mode::commons("remap", "commons --mode remap", Commons::Remap)
    },
    Mode::commons("svuid", "commons --mode svuid", Commons::Svuid),
    Mode::commons(
        "static-init-merger",
        "commons --mode static-init-merger",
        Commons::StaticInitMerger,
    ),
    Mode::commons("tcb-sort", "commons --mode tcb-sort", Commons::TcbSort),
    Mode::commons("jsr", "commons --mode jsr", Commons::Jsr),
    Mode::commons("lvs", "commons --mode lvs", Commons::Lvs),
    Mode::commons("advice", "commons --mode advice", Commons::Advice),
    Mode::commons(
        "instruction-adapter",
        "commons --mode instruction-adapter",
        Commons::InstructionAdapter,
    ),
    Mode::commons(
        "analyzer-adapter",
        "commons --mode analyzer-adapter",
        Commons::AnalyzerAdapter,
    ),
    Mode::commons("code-size", "commons --mode code-size", Commons::CodeSize),
];

impl Mode {
    const fn new(name: &'static str, command: &'static str) -> Mode {
        Mode {
            name,
            command,
            needs_hierarchy: false,
            textify: None,
            tree: false,
            asmify: false,
            analyze: None,
            roundtrip: None,
            check: None,
            commons: None,
            needs_mapping: false,
        }
    }

    const fn commons(name: &'static str, command: &'static str, commons: Commons) -> Mode {
        Mode {
            commons: Some(commons),
            ..Mode::new(name, command)
        }
    }

    const fn check(name: &'static str, command: &'static str, check: Check) -> Mode {
        Mode {
            check: Some(check),
            ..Mode::new(name, command)
        }
    }

    const fn textify(name: &'static str, command: &'static str, options: ParsingOptions) -> Mode {
        Mode {
            textify: Some(options),
            ..Mode::new(name, command)
        }
    }

    const fn asmify(name: &'static str, command: &'static str, options: ParsingOptions) -> Mode {
        Mode {
            asmify: true,
            ..Mode::textify(name, command, options)
        }
    }

    const fn roundtrip(name: &'static str, command: &'static str, roundtrip: Roundtrip) -> Mode {
        Mode {
            roundtrip: Some(roundtrip),
            ..Mode::new(name, command)
        }
    }

    const fn tree_roundtrip(
        name: &'static str,
        command: &'static str,
        roundtrip: Roundtrip,
    ) -> Mode {
        Mode {
            tree: true,
            ..Mode::roundtrip(name, command, roundtrip)
        }
    }

    const fn analyze(name: &'static str, command: &'static str, interp: Interp) -> Mode {
        Mode {
            analyze: Some(interp),
            ..Mode::new(name, command)
        }
    }

    const fn tree_textify(
        name: &'static str,
        command: &'static str,
        options: ParsingOptions,
    ) -> Mode {
        Mode {
            tree: true,
            ..Mode::textify(name, command, options)
        }
    }
}

impl Roundtrip {
    const fn plain() -> Roundtrip {
        Roundtrip {
            flags: ClassWriterFlags::empty(),
            options: ParsingOptions::empty(),
            copy_pool: false,
        }
    }

    const fn copy() -> Roundtrip {
        Roundtrip {
            copy_pool: true,
            ..Roundtrip::plain()
        }
    }

    const fn flags(flags: ClassWriterFlags) -> Roundtrip {
        Roundtrip {
            flags,
            // As the oracle: frames are recomputed, so they are not read.
            options: if flags.contains(ClassWriterFlags::COMPUTE_FRAMES) {
                ParsingOptions::SKIP_FRAMES
            } else {
                ParsingOptions::empty()
            },
            ..Roundtrip::plain()
        }
    }

    const fn options(options: ParsingOptions) -> Roundtrip {
        Roundtrip {
            options,
            ..Roundtrip::plain()
        }
    }
}

/// The stack size of the threads that compute oxasm's side of a mode.
const COMPARISON_STACK_SIZE: usize = 64 << 20;

/// The fewest classes worth starting another oracle JVM for.
const MIN_CLASSES_PER_JVM: usize = 500;

/// One class of a tier.
pub(crate) struct Class {
    pub(crate) id: String,
    sha256: String,
    pub(crate) bytes: Vec<u8>,
}

pub fn cli(mut args: Args) -> Result<()> {
    let tier = args
        .value("--tier")?
        .context("missing --tier <t0|t1|t2|dir:<path>>")?;
    let mode_names = args.values("--mode")?;
    let sample = args
        .value("--sample")?
        .map(|n| n.parse::<usize>())
        .transpose()
        .context("--sample")?;
    let jobs = args
        .value("--jobs")?
        .map(|n| n.parse::<usize>())
        .transpose()
        .context("--jobs")?;
    let hierarchy = args.value("--hierarchy")?.map(PathBuf::from);
    let mapping = args.value("--mapping")?.map(PathBuf::from);
    let oracle_only = args.flag("--oracle-only");
    args.finish()?;

    let modes: Vec<&Mode> = if mode_names.is_empty() {
        // Without --oracle-only, the modes that have an oxasm side.
        MODES
            .iter()
            .filter(|m| oracle_only || m.has_oxasm_side())
            .collect()
    } else {
        mode_names
            .iter()
            .map(|name| {
                MODES.iter().find(|m| m.name == name).with_context(|| {
                    let known: Vec<_> = MODES.iter().map(|m| m.name).collect();
                    format!("unknown --mode {name}; known: {}", known.join(", "))
                })
            })
            .collect::<Result<_>>()?
    };

    if modes
        .iter()
        .any(|m| m.check.is_some() || m.commons.is_some())
    {
        // The check adapters panic on the classes they reject, and the commons adapters where
        // ASM throws, which is an expected outcome: only report the other panics.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied());
            if !message.is_some_and(|m| m.starts_with("java.lang.")) {
                default_hook(info);
            }
        }));
    }

    let oracle = Oracle::locate()?;
    let mut classes = tier_classes(&tier)?;
    if let Some(n) = sample {
        stable_sample(&mut classes, n);
    }
    let jobs = jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().div_ceil(2)));

    let hierarchy = if modes.iter().any(|m| m.needs_hierarchy) {
        Some(match hierarchy {
            Some(path) => path,
            None => tier_hierarchy(&oracle, &tier)?,
        })
    } else {
        None
    };

    let mapping = if modes.iter().any(|m| m.needs_mapping) {
        Some(match mapping {
            Some(path) => path,
            None => tier_mapping(&oracle, &tier)?,
        })
    } else {
        None
    };

    if !oracle_only && let Some(mode) = modes.iter().find(|m| !m.has_oxasm_side()) {
        bail!(
            "mode {} has no oxasm side yet; pass --oracle-only to only fill the oracle cache",
            mode.name
        );
    }
    // oxasm's side of the modes that need a hierarchy reads the same file.
    let map_hierarchy = match &hierarchy {
        Some(path) if !oracle_only => {
            let tsv = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            Some(MapHierarchy::from_tsv(&tsv).with_context(|| path.display().to_string())?)
        }
        _ => None,
    };
    // oxasm's side of the remap mode reads the same mapping.
    let remapping = match &mapping {
        Some(path) if !oracle_only => Some(commons::read_mapping(path)?),
        _ => None,
    };
    let mut mismatched_modes = Vec::new();
    for mode in modes {
        let cache = fill_mode(
            &oracle,
            &tier,
            mode,
            &classes,
            hierarchy.as_deref(),
            mapping.as_deref(),
            jobs,
        )?;
        if !oracle_only
            && compare_mode(
                &tier,
                mode,
                &classes,
                &cache,
                map_hierarchy.as_ref(),
                remapping.as_ref(),
                jobs,
            )? > 0
        {
            mismatched_modes.push(mode.name);
        }
    }
    ensure!(
        mismatched_modes.is_empty(),
        "mismatches in {}",
        mismatched_modes.join(", ")
    );
    Ok(())
}

/// The result of one side of a mode for one class.
#[derive(PartialEq, Eq)]
enum Outcome {
    Ok(Vec<u8>),
    /// The side failed: the oracle's exception, or oxasm's error (or panic).
    Err(String),
    /// oxasm panicked as ASM throws (a commons adapter): agrees with the oracle's exception
    /// only if it is the same one.
    Thrown(String),
}

/// oxasm's side of a mode for one class.
fn oxasm_outcome(
    mode: &Mode,
    class_file: &[u8],
    hierarchy: Option<&MapHierarchy>,
    mapping: Option<&Mapping>,
) -> Outcome {
    let result = std::panic::catch_unwind(|| -> Result<Vec<u8>, String> {
        if let Some(interp) = mode.analyze {
            let hierarchy = hierarchy.map(|h| h as &dyn ClassHierarchy);
            return analysis::analyze(class_file, interp, ParsingOptions::empty(), hierarchy);
        }
        if let Some(roundtrip) = mode.roundtrip {
            return write_class(class_file, roundtrip, mode.tree, hierarchy);
        }
        if let Some(check) = mode.check {
            return check_class(class_file, check, hierarchy);
        }
        if let Some(adapter) = mode.commons {
            return commons::run(class_file, adapter, mapping);
        }
        let options = mode
            .textify
            .expect("only Textifier and analysis modes have an oxasm side");
        if mode.asmify {
            print_class(class_file, Asmifier::new(), options, mode.tree)
        } else {
            print_class(class_file, Textifier::new(), options, mode.tree)
        }
    });
    match result {
        Ok(Ok(output)) => Outcome::Ok(output),
        Ok(Err(error)) => Outcome::Err(error),
        // A commons adapter panics where ASM throws, with the `toString()` of ASM's exception
        // (ADR-06): an expected failure, compared with ASM's.
        Err(ref panic)
            if mode.commons.is_some()
                && let Some(message) = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .filter(|m| m.starts_with("java.lang.")) =>
        {
            Outcome::Thrown(message.to_owned())
        }
        Err(panic) => Outcome::Err(format!(
            "PANIC: {}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("?")
        )),
    }
}

/// An empty class node, whose strings are interned in the interner of this worker thread: shared
/// by every class the thread reads, as sift shares one for a classpath.
pub(crate) fn class_node() -> ClassNode {
    thread_local!(static INTERNER: Arc<Interner> = Arc::default());
    ClassNode::with_interner(INTERNER.with(Arc::clone))
}

/// oxasm's side of a Textifier or ASMifier mode: the text printed by a `TraceClassVisitor` with
/// the given printer (ASM's `Oracle.print`).
fn print_class(
    class_file: &[u8],
    printer: impl Printer,
    options: ParsingOptions,
    tree: bool,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut trace_class_visitor = TraceClassVisitor::with_printer(printer, &mut output);
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    if tree {
        let mut class_node = class_node();
        reader
            .accept(&mut class_node, options)
            .map_err(|e| e.to_string())?;
        class_node.accept(&mut trace_class_visitor);
    } else {
        reader
            .accept(&mut trace_class_visitor, options)
            .map_err(|e| e.to_string())?;
    }
    trace_class_visitor
        .take_io_error()
        .map_err(|e| e.to_string())?;
    Ok(output)
}

/// oxasm's side of a writer mode: the class read and written back (ASM's
/// `Oracle.roundtrip`).
fn write_class(
    class_file: &[u8],
    roundtrip: Roundtrip,
    tree: bool,
    hierarchy: Option<&MapHierarchy>,
) -> Result<Vec<u8>, String> {
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    let class_writer = if roundtrip.copy_pool {
        ClassWriter::from_reader(&reader, roundtrip.flags).map_err(|e| e.to_string())?
    } else {
        ClassWriter::new(roundtrip.flags)
    };
    let mut class_writer = match hierarchy {
        Some(hierarchy) => class_writer.with_hierarchy(hierarchy),
        None => class_writer,
    };
    if tree {
        let mut class_node = class_node();
        reader
            .accept(&mut class_node, roundtrip.options)
            .map_err(|e| e.to_string())?;
        class_node.accept(&mut class_writer as &mut dyn ClassVisitor);
    } else {
        reader
            .accept(&mut class_writer, roundtrip.options)
            .map_err(|e| e.to_string())?;
    }
    class_writer.to_bytes().map_err(|e| e.to_string())
}

/// oxasm's side of a check mode (ASM's `Oracle.check`): "OK", or the message of the panic of a
/// check adapter (the `toString()` of ASM's exception), or the output of `verify`.
fn check_class(
    class_file: &[u8],
    check: Check,
    hierarchy: Option<&MapHierarchy>,
) -> Result<Vec<u8>, String> {
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match check {
        Check::Verify => {
            let hierarchy = hierarchy.ok_or("check-verify needs a hierarchy")?;
            let mut output = Vec::new();
            oxasm_util::check::verify(&reader, hierarchy, true, &mut output)
                .map_err(|e| e.to_string())?;
            Ok(output)
        }
        Check::Plain | Check::NoDataFlow => {
            let mut adapter =
                CheckClassAdapter::with_check_data_flow(NoDelegate, check == Check::Plain);
            reader
                .accept(&mut adapter, ParsingOptions::empty())
                .map_err(|e| e.to_string())?;
            Ok(b"OK\n".to_vec())
        }
        Check::Writer => {
            let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
            reader
                .accept(
                    &mut CheckClassAdapter::new(&mut class_writer),
                    ParsingOptions::empty(),
                )
                .map_err(|e| e.to_string())?;
            Ok(b"OK\n".to_vec())
        }
    }));
    match result {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("?");
            if message.starts_with("java.lang.") {
                Ok(format!("{message}\n").into_bytes())
            } else {
                std::panic::resume_unwind(panic)
            }
        }
    }
}

/// Compares oxasm's side of a mode with the cached oracle results, prints a summary and the
/// first mismatches, and writes each mismatch's outputs under `target/parity/<tier>/<mode>/`.
/// Returns the number of mismatched classes.
fn compare_mode(
    tier: &str,
    mode: &Mode,
    classes: &[Class],
    cache: &Path,
    hierarchy: Option<&MapHierarchy>,
    mapping: Option<&Mapping>,
    jobs: usize,
) -> Result<usize> {
    let out = root_path("target/parity")
        .join(tier_key(tier))
        .join(mode.name);
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    // Threads take classes one at a time: the costliest classes of a tier sit next to each other
    // (the same package or jar), and fixed chunks left most threads idle behind the one that
    // drew them.
    let next = AtomicUsize::new(0);
    let results: Vec<Result<Comparison>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..jobs.clamp(1, classes.len().max(1)))
            .map(|_| {
                let out = out.as_path();
                let next = &next;
                // The reader's depth bounds (ADR-06) hold 256 levels of nesting in a few KB of
                // stack per level when optimized, but xtask's unoptimized frames are larger:
                // more than the 2 MiB default for a thread at 256 levels of annotations.
                std::thread::Builder::new()
                    .stack_size(COMPARISON_STACK_SIZE)
                    .spawn_scoped(scope, move || -> Result<Comparison> {
                        let mut mismatches = Vec::new();
                        let mut categories = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(class) = classes.get(i) else {
                                break;
                            };
                            let mut oracle = oracle_outcome(cache, class)?;
                            let mut oxasm = oxasm_outcome(mode, &class.bytes, hierarchy, mapping);
                            if mode.check.is_some() {
                                // oxasm's panic messages are Rust strings, where each unpaired
                                // surrogate of a name became one U+FFFD (JavaStr's Display): compare
                                // both sides so.
                                for outcome in [&mut oracle, &mut oxasm] {
                                    if let Outcome::Ok(text) = outcome
                                        && let Ok(java_text) = JavaStr::from_semi_utf8(text)
                                    {
                                        *text = java_text.to_string().into_bytes();
                                    }
                                }
                            }
                            let initialized_init_receiver = mode.commons
                                == Some(Commons::AnalyzerAdapter)
                                && commons::initialized_init_receiver();
                            if let Some(category) =
                                documented_category(&oracle, &oxasm, initialized_init_receiver)
                            {
                                categories.push(category);
                            } else if let Some(report) = mismatch(&oracle, &oxasm) {
                                fs::create_dir_all(out)?;
                                let base = out.join(&class.sha256);
                                fs::write(base.with_extension("class"), &class.bytes)?;
                                for (side, outcome) in [("oracle", &oracle), ("oxasm", &oxasm)] {
                                    let bytes = match outcome {
                                        Outcome::Ok(bytes) => bytes.as_slice(),
                                        Outcome::Err(error) | Outcome::Thrown(error) => {
                                            error.as_bytes()
                                        }
                                    };
                                    fs::write(base.with_extension(side), bytes)?;
                                }
                                mismatches.push((i, report));
                            }
                        }
                        Ok((mismatches, categories))
                    })
                    .expect("spawning a comparison thread")
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("comparison thread panicked"))
            .collect()
    });
    let mut mismatches = Vec::new();
    let mut categories = std::collections::BTreeMap::<&str, usize>::new();
    for result in results {
        let (chunk_mismatches, chunk_categories) = result?;
        mismatches.extend(chunk_mismatches);
        for category in chunk_categories {
            *categories.entry(category).or_default() += 1;
        }
    }
    mismatches.sort();
    println!(
        "{tier}/{}: {} classes, {} mismatched",
        mode.name,
        classes.len(),
        mismatches.len()
    );
    for (category, count) in &categories {
        println!("  documented divergence, {count} classes: {category}");
    }
    for (index, report) in mismatches.iter().take(20) {
        let class = &classes[*index];
        println!("  {} ({}): {report}", class.id, &class.sha256[..12]);
    }
    if !mismatches.is_empty() {
        println!("  outputs: {}", out.display());
    }
    Ok(mismatches.len())
}

/// The cached oracle result of a class.
fn oracle_outcome(cache: &Path, class: &Class) -> Result<Outcome> {
    let ok = cache.join(format!("{}.ok", class.sha256));
    if ok.exists() {
        return Ok(Outcome::Ok(fs::read(ok)?));
    }
    let err = cache.join(format!("{}.err", class.sha256));
    Ok(Outcome::Err(
        String::from_utf8_lossy(&fs::read(&err).with_context(|| {
            format!("no oracle result for {} in {}", class.id, cache.display())
        })?)
        .into_owned(),
    ))
}

/// The mismatches of a chunk of classes (their indices and reports), and the documented
/// divergences found in it.
type Comparison = (Vec<(usize, String)>, Vec<&'static str>);

/// The documented divergence (oxasm-verification, "Known nondeterminism") the two outcomes
/// disagree by, if any: such a class counts as agreeing, and is reported by category.
fn documented_category(
    oracle: &Outcome,
    oxasm: &Outcome,
    initialized_init_receiver: bool,
) -> Option<&'static str> {
    // ASM's AnalyzerAdapter compares an `INVOKESPECIAL <init>` receiver by reference identity,
    // oxasm's by equality: on an initialized receiver (unverifiable code), the frames of the
    // `analyzer-adapter` mode differ (accepted, 2026-10-09).
    if initialized_init_receiver
        && let (Outcome::Ok(a), Outcome::Ok(b)) = (oracle, oxasm)
        && a != b
    {
        return Some(
            "INVOKESPECIAL <init> on an initialized reference (ASM's AnalyzerAdapter compares by \
             identity)",
        );
    }
    // The reader categories (P2, ADR-06 and ADR-09): oxasm's reader rejects a class that ASM's
    // reader passes on.
    if let Outcome::Err(error) = oxasm {
        if error.starts_with("invalid constant pool index 0 ") {
            return Some("a null string (oxasm's reader rejects it, ASM's passes null on)");
        }
        if error.starts_with("invalid method handle reference kind ") {
            return Some("an invalid method handle kind (oxasm's reader rejects it)");
        }
    }
    // Where ASM's commons modes say where ASM threw, and oxasm does not fail.
    if let (Outcome::Err(error), Outcome::Ok(_)) = (oracle, oxasm)
        && let Some(site) = oracle_exception(error).1
    {
        // ASM reads a type path's length as a signed byte (P6, the user chose the JVMS reading).
        if site.starts_with("org.objectweb.asm.TypePath.") {
            return Some("a type path of 128 steps or more (ASM's signed path_length throws)");
        }
        // ASM's tree throws on malformed input where oxasm's stores (P3).
        if site.starts_with("org.objectweb.asm.tree.") {
            return Some("ASM's tree throws where oxasm's stores (malformed input)");
        }
        // A chop frame removing more locals than there are, read with EXPAND_FRAMES: ASM's
        // writer throws on the negative count, oxasm's visitors get no locals (P5).
        if site == "org.objectweb.asm.MethodWriter.visitFrameStart" {
            return Some("a negative frame local count in expanded mode (ASM's writer throws)");
        }
    }
    let (Outcome::Err(error), Outcome::Thrown(thrown)) = (oracle, oxasm) else {
        return None;
    };
    // ASM's class writer throws in the middle of the visit, where oxasm's records the error for
    // `to_bytes` (ADR-06): the adapter goes on, and may panic on a later event (P7).
    let (exception, site) = oracle_exception(error);
    if exception
        == "java.lang.IllegalArgumentException: Class versions V1_5 or less must use F_NEW frames."
        || site.is_some_and(|site| ASM_WRITER_CLASSES.iter().any(|c| site.starts_with(c)))
    {
        return Some("the class writer's deferred error (ASM's writer throws first, both fail)");
    }
    if !error.starts_with("java.lang.") {
        return None;
    }
    // ASM's Type parses a method descriptor lazily (an `L` without its `;` gives a type whose
    // name fails later, if ever asked for), where oxasm's fails at once.
    if thrown.starts_with("java.lang.IllegalArgumentException: Invalid descriptor: ")
        && !same_exception(error, thrown)
    {
        return Some("a malformed descriptor that ASM's Type reads lazily (both fail)");
    }
    // oxasm's signature reader keeps the offset of a malformed signature, not ASM's exception
    // (ADR-14).
    if thrown.starts_with("java.lang.IllegalArgumentException: invalid signature at offset ")
        || thrown == "java.lang.IllegalArgumentException: Too many nested type arguments"
    {
        return Some("a malformed signature (both throw, with different exceptions)");
    }
    None
}

/// The classes of ASM's class writer (with a `.`, to match the start of a throw site).
const ASM_WRITER_CLASSES: [&str; 12] = [
    "org.objectweb.asm.AnnotationWriter.",
    "org.objectweb.asm.ClassWriter.",
    "org.objectweb.asm.CurrentFrame.",
    "org.objectweb.asm.FieldWriter.",
    "org.objectweb.asm.Frame.",
    "org.objectweb.asm.Handler.",
    "org.objectweb.asm.Label.",
    "org.objectweb.asm.MethodWriter.",
    "org.objectweb.asm.ModuleWriter.",
    "org.objectweb.asm.RecordComponentWriter.",
    "org.objectweb.asm.Symbol.",
    "org.objectweb.asm.SymbolTable.",
];

/// An oracle error (`<class>: <message>`), and the ASM method that threw it if the oracle says
/// (the commons modes print it on a second line, `\tat <class>.<method>`).
fn oracle_exception(error: &str) -> (&str, Option<&str>) {
    match error.split_once("\n\tat ") {
        Some((exception, site)) => (exception, Some(site.trim_end())),
        None => (error, None),
    }
}

/// Whether oxasm's panic (`thrown`) stands for the oracle's exception (`error`, printed as
/// `<class>: <message>`).
fn same_exception(error: &str, thrown: &str) -> bool {
    let error = oracle_exception(error).0;
    // The oracle prints each unpaired surrogate of a message as '?', and a panic message has a
    // U+FFFD (JavaStr's Display).
    let thrown = thrown.replace('\u{FFFD}', "?");
    error == thrown
        // An exception without a message, which the oracle prints as "<class>: null".
        || (!thrown.contains(": ") && error.strip_suffix(": null") == Some(&thrown))
        // Java's helpful NullPointerException messages describe the JVM's bytecode.
        || (thrown == "java.lang.NullPointerException"
            && error.starts_with("java.lang.NullPointerException"))
}

/// A description of how the two outcomes disagree, or `None` if they agree.
fn mismatch(oracle: &Outcome, oxasm: &Outcome) -> Option<String> {
    match (oracle, oxasm) {
        (Outcome::Ok(a), Outcome::Ok(b)) if a == b => None,
        (Outcome::Err(_), Outcome::Err(error)) if !error.starts_with("PANIC") => None,
        // The oracle prints each unpaired surrogate of a message as '?', and a panic message has
        // a U+FFFD (JavaStr's Display).
        (Outcome::Err(error), Outcome::Thrown(thrown)) if same_exception(error, thrown) => None,
        (Outcome::Err(error), Outcome::Thrown(thrown)) => Some(format!(
            "oxasm panics {thrown:?} where ASM throws {error:?}"
        )),
        (Outcome::Ok(_), Outcome::Thrown(thrown)) => Some(format!("only oxasm fails: {thrown}")),
        (Outcome::Ok(a), Outcome::Ok(b))
            if a.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
                && b.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]) =>
        {
            // Byte parity: locate the difference in the ClassFile dumps (rung 3).
            let offset = a
                .iter()
                .zip(b)
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            let dump = |bytes: &[u8]| {
                oxasm_testkit::classfile_dump::dump(bytes)
                    .map(|d| String::from_utf8_lossy(&d).into_owned())
                    .unwrap_or_else(|e| format!("<no dump: {e}>"))
            };
            let (a_dump, b_dump) = (dump(a), dump(b));
            let line = a_dump.lines().zip(b_dump.lines()).position(|(x, y)| x != y);
            Some(match line {
                Some(line) => format!(
                    "byte {offset} (sizes {}/{}), dump line {}: oracle {:?} / oxasm {:?}",
                    a.len(),
                    b.len(),
                    line + 1,
                    a_dump.lines().nth(line).unwrap_or("<end>"),
                    b_dump.lines().nth(line).unwrap_or("<end>")
                ),
                None => format!("byte {offset} (sizes {}/{}), same dump", a.len(), b.len()),
            })
        }
        (Outcome::Ok(a), Outcome::Ok(b)) => {
            let (a, b) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
            let line = a
                .lines()
                .zip(b.lines())
                .position(|(x, y)| x != y)
                .unwrap_or_else(|| a.lines().count().min(b.lines().count()));
            Some(format!(
                "line {}: oracle {:?} / oxasm {:?}",
                line + 1,
                a.lines().nth(line).unwrap_or("<end>"),
                b.lines().nth(line).unwrap_or("<end>")
            ))
        }
        (Outcome::Ok(_), Outcome::Err(error)) => Some(format!("only oxasm fails: {error}")),
        (Outcome::Err(error), Outcome::Ok(_)) => Some(format!("only ASM fails: {error}")),
        (Outcome::Err(error), Outcome::Err(oxasm)) => Some(format!("{oxasm} (ASM: {error})")),
        (Outcome::Thrown(_), _) => unreachable!("only oxasm's side panics"),
    }
}

/// Fills the oracle cache of a mode, and returns its directory.
fn fill_mode(
    oracle: &Oracle,
    tier: &str,
    mode: &Mode,
    classes: &[Class],
    hierarchy: Option<&Path>,
    mapping: Option<&Path>,
    jobs: usize,
) -> Result<PathBuf> {
    let mut command = mode.command.to_owned();
    let mut key = mode.name.to_owned();
    if mode.needs_hierarchy {
        let hierarchy = hierarchy.expect("hierarchy resolved for this mode");
        let tsv =
            fs::read(hierarchy).with_context(|| format!("cannot read {}", hierarchy.display()))?;
        ensure!(
            !hierarchy.to_string_lossy().contains(char::is_whitespace),
            "the hierarchy path must not contain whitespace: {}",
            hierarchy.display()
        );
        command.push_str(&format!(" --hierarchy {}", hierarchy.display()));
        // Results depend on the hierarchy, so it is part of the cache key.
        key.push_str(&format!("-h{}", &sha256_hex(&tsv)[..12]));
    }
    if mode.needs_mapping {
        let mapping = mapping.expect("mapping resolved for this mode");
        let tsv =
            fs::read(mapping).with_context(|| format!("cannot read {}", mapping.display()))?;
        ensure!(
            !mapping.to_string_lossy().contains(char::is_whitespace),
            "the mapping path must not contain whitespace: {}",
            mapping.display()
        );
        command.push_str(&format!(" --mapping {}", mapping.display()));
        // Results depend on the mapping, so it is part of the cache key.
        key.push_str(&format!("-m{}", &sha256_hex(&tsv)[..12]));
    }
    let cache = root_path("target/oracle").join(tier_key(tier)).join(&key);
    fs::create_dir_all(&cache)?;

    let misses: Vec<&Class> = classes
        .iter()
        .filter(|c| {
            !cache.join(format!("{}.ok", c.sha256)).exists()
                && !cache.join(format!("{}.err", c.sha256)).exists()
        })
        .collect();

    // Enough classes per JVM to amortize its start-up.
    let chunk_size = misses.len().div_ceil(jobs.max(1)).max(MIN_CLASSES_PER_JVM);
    let errors: Vec<Result<usize>> = std::thread::scope(|scope| {
        let handles: Vec<_> = misses
            .chunks(chunk_size)
            .map(|chunk| {
                let command = command.as_str();
                let cache = cache.as_path();
                scope.spawn(move || -> Result<usize> {
                    let requests: Vec<ServeRequest<'_>> = chunk
                        .iter()
                        .map(|c| ServeRequest {
                            command,
                            class_file: &c.bytes,
                        })
                        .collect();
                    let mut errors = 0usize;
                    oracle.serve(&requests, |index, response| {
                        let extension = if response.ok { "ok" } else { "err" };
                        errors += usize::from(!response.ok);
                        let path = cache.join(format!("{}.{extension}", chunk[index].sha256));
                        fs::write(&path, &response.payload)
                            .with_context(|| format!("cannot write {}", path.display()))
                    })?;
                    Ok(errors)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("oracle thread panicked"))
            .collect()
    });
    let mut oracle_errors = 0usize;
    for result in errors {
        oracle_errors += result?;
    }

    println!(
        "{tier}/{}: {} classes, {} cached, {} filled ({oracle_errors} ASM errors)",
        mode.name,
        classes.len(),
        classes.len() - misses.len(),
        misses.len()
    );
    Ok(cache)
}

/// `target/oracle/<tier>/hierarchy.tsv`: the hierarchy of the tier plus, for tiers other than T1,
/// the JDK classes of T1 (when extracted). It is regenerated on every run, so it follows the
/// corpus; cached results stay valid because their key includes its hash.
pub(crate) fn tier_hierarchy(oracle: &Oracle, tier: &str) -> Result<PathBuf> {
    let path = root_path("target/oracle")
        .join(tier_key(tier))
        .join("hierarchy.tsv");
    let mut words = vec!["hierarchy".to_owned()];
    words.extend(tier_inputs(tier)?);
    let t1 = root_path("corpus/t1");
    if tier != "t1" {
        if t1.is_dir() {
            words.push(t1.to_string_lossy().into_owned());
        } else {
            eprintln!(
                "xtask: corpus/t1 is not extracted, so {} has no JDK classes; frames modes will \
                 fail on them (run `cargo xtask corpus t1`)",
                path.display()
            );
        }
    }
    let tsv = oracle.text(&words)?;
    fs::create_dir_all(path.parent().expect("file has a parent"))?;
    fs::write(&path, tsv)?;
    Ok(path)
}

/// `target/oracle/<tier>/mapping.tsv`: the remapping of the `remap` mode, printed by the oracle's
/// `mapping` command over the tier (every class, member and invokedynamic name of the tier gets a
/// new name; see `Oracle.mapping`). Like the hierarchy, it is regenerated on every run, and
/// cached results are keyed by its hash.
fn tier_mapping(oracle: &Oracle, tier: &str) -> Result<PathBuf> {
    let path = root_path("target/oracle")
        .join(tier_key(tier))
        .join("mapping.tsv");
    let mut words = vec!["mapping".to_owned()];
    words.extend(tier_inputs(tier)?);
    let tsv = oracle.text(&words)?;
    fs::create_dir_all(path.parent().expect("file has a parent"))?;
    fs::write(&path, tsv)?;
    Ok(path)
}

/// The directory name of a tier under `target/oracle` and `target/parity`: the tier itself, or
/// `dir-<hash of the path>` for a `dir:<path>` tier.
pub(crate) fn tier_key(tier: &str) -> String {
    match tier.strip_prefix("dir:") {
        Some(path) => format!("dir-{}", &sha256_hex(path.as_bytes())[..12]),
        None => tier.to_owned(),
    }
}

/// The oracle inputs (directories or jars) that make up a tier: `corpus/<tier>`, or the
/// directory of a `dir:<path>` tier (any directory of class files, e.g. fuzzer mutants).
fn tier_inputs(tier: &str) -> Result<Vec<String>> {
    if let Some(path) = tier.strip_prefix("dir:") {
        ensure!(Path::new(path).is_dir(), "{path} is not a directory");
        return Ok(vec![path.to_owned()]);
    }
    let dir = root_path(&format!("corpus/{tier}"));
    ensure!(
        dir.is_dir(),
        "{} is missing; run `cargo xtask corpus {tier}`",
        dir.display()
    );
    Ok(match tier {
        "t0" | "t1" => vec![dir.to_string_lossy().into_owned()],
        "t2" => jars(&dir)?
            .into_iter()
            .map(|j| j.to_string_lossy().into_owned())
            .collect(),
        other => bail!("unknown tier {other}"),
    })
}

fn jars(dir: &Path) -> Result<Vec<PathBuf>> {
    Ok(walk_files(dir)?
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "jar"))
        .collect())
}

/// Keeps a stable sample of `n` classes: the classes whose ids hash lowest.
pub(crate) fn stable_sample(classes: &mut Vec<Class>, n: usize) {
    classes.sort_by_cached_key(|c| sha256_hex(c.id.as_bytes()));
    classes.truncate(n);
}

/// Every class file of a tier, with the same ids the oracle gives them.
pub(crate) fn tier_classes(tier: &str) -> Result<Vec<Class>> {
    let mut classes = Vec::new();
    for input in tier_inputs(tier)? {
        let input = PathBuf::from(input);
        if input.is_dir() {
            for file in walk_files(&input)?.into_iter().filter(|f| is_class_file(f)) {
                let bytes = fs::read(&file)?;
                classes.push(Class {
                    id: relative_id(&input, &file),
                    sha256: sha256_hex(&bytes),
                    bytes,
                });
            }
        } else {
            let mut zip = zip::ZipArchive::new(fs::File::open(&input)?)
                .with_context(|| format!("cannot open {}", input.display()))?;
            let mut names: Vec<String> = zip
                .file_names()
                .filter(|name| name.ends_with(".class"))
                .map(str::to_owned)
                .collect();
            names.sort();
            for name in names {
                let mut entry = zip.by_name(&name)?;
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                classes.push(Class {
                    id: format!("{}!/{name}", input.display()),
                    sha256: sha256_hex(&bytes),
                    bytes,
                });
            }
        }
    }
    Ok(classes)
}
