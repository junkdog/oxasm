//! Parity sweeps over a corpus tier. Each sweep first fills the oracle result cache,
//! `target/oracle/<tier>/<mode>/<sha256>.{ok,err}`, keyed by the SHA-256 of each class file, so
//! that later sweeps only pay for classes they have not seen; then (unless `--oracle-only`) it
//! computes oxasm's side of each mode and compares the two.
//!
//! The oxasm side exists for the Textifier modes (P2), direct and through a `ClassNode` (P3), the
//! analysis modes (P4), the writer modes that compute nothing (P5), and `maxs` and `frames` (P6).
//! A class on which ASM fails agrees with oxasm iff oxasm fails too ("both reject").

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use oxasm::{
    ClassHierarchy, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, MapHierarchy,
    ParsingOptions,
};
use oxasm_tree::ClassNode;
use oxasm_util::TraceClassVisitor;

use crate::analysis::{self, Interp};
use crate::args::Args;
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
    /// For an analysis mode, the interpreter: oxasm's side is the oracle's `analyze` output.
    analyze: Option<Interp>,
    /// For a writer mode, how the class is written: oxasm's side is the class file.
    roundtrip: Option<Roundtrip>,
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
        self.textify.is_some() || self.analyze.is_some() || self.roundtrip.is_some()
    }
}

const MODES: [Mode; 25] = [
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
];

impl Mode {
    const fn new(name: &'static str, command: &'static str) -> Mode {
        Mode {
            name,
            command,
            needs_hierarchy: false,
            textify: None,
            tree: false,
            analyze: None,
            roundtrip: None,
        }
    }

    const fn textify(name: &'static str, command: &'static str, options: ParsingOptions) -> Mode {
        Mode {
            textify: Some(options),
            ..Mode::new(name, command)
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

/// The fewest classes worth starting another oracle JVM for.
const MIN_CLASSES_PER_JVM: usize = 500;

/// One class of a tier.
struct Class {
    id: String,
    sha256: String,
    bytes: Vec<u8>,
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

    let oracle = Oracle::locate()?;
    let mut classes = tier_classes(&tier)?;
    if let Some(n) = sample {
        // A stable sample: the classes whose ids hash lowest.
        classes.sort_by_cached_key(|c| sha256_hex(c.id.as_bytes()));
        classes.truncate(n);
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

    if !oracle_only && let Some(mode) = modes.iter().find(|m| !m.has_oxasm_side()) {
        bail!(
            "mode {} has no oxasm side yet (it needs the writer, P5/P6); pass --oracle-only to \
             only fill the oracle cache",
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
    let mut mismatched_modes = Vec::new();
    for mode in modes {
        let cache = fill_mode(&oracle, &tier, mode, &classes, hierarchy.as_deref(), jobs)?;
        if !oracle_only
            && compare_mode(&tier, mode, &classes, &cache, map_hierarchy.as_ref(), jobs)? > 0
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
}

/// oxasm's side of a mode for one class.
fn oxasm_outcome(mode: &Mode, class_file: &[u8], hierarchy: Option<&MapHierarchy>) -> Outcome {
    let result = std::panic::catch_unwind(|| -> Result<Vec<u8>, String> {
        if let Some(interp) = mode.analyze {
            let hierarchy = hierarchy.map(|h| h as &dyn ClassHierarchy);
            return analysis::analyze(class_file, interp, ParsingOptions::empty(), hierarchy);
        }
        if let Some(roundtrip) = mode.roundtrip {
            return write_class(class_file, roundtrip, mode.tree, hierarchy);
        }
        let options = mode
            .textify
            .expect("only Textifier and analysis modes have an oxasm side");
        let mut output = Vec::new();
        let mut trace_class_visitor = TraceClassVisitor::new(&mut output);
        let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
        if mode.tree {
            let mut class_node = ClassNode::default();
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
    });
    match result {
        Ok(Ok(output)) => Outcome::Ok(output),
        Ok(Err(error)) => Outcome::Err(error),
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
        let mut class_node = ClassNode::default();
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

/// Compares oxasm's side of a mode with the cached oracle results, prints a summary and the
/// first mismatches, and writes each mismatch's outputs under `target/parity/<tier>/<mode>/`.
/// Returns the number of mismatched classes.
fn compare_mode(
    tier: &str,
    mode: &Mode,
    classes: &[Class],
    cache: &Path,
    hierarchy: Option<&MapHierarchy>,
    jobs: usize,
) -> Result<usize> {
    let out = root_path("target/parity")
        .join(tier_key(tier))
        .join(mode.name);
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    let chunk_size = classes.len().div_ceil(jobs.max(1)).max(1);
    let results: Vec<Result<Vec<(usize, String)>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = classes
            .chunks(chunk_size)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                let out = out.as_path();
                scope.spawn(move || -> Result<Vec<(usize, String)>> {
                    let mut mismatches = Vec::new();
                    for (i, class) in chunk.iter().enumerate() {
                        let oracle = oracle_outcome(cache, class)?;
                        let oxasm = oxasm_outcome(mode, &class.bytes, hierarchy);
                        if let Some(report) = mismatch(&oracle, &oxasm) {
                            fs::create_dir_all(out)?;
                            let base = out.join(&class.sha256);
                            fs::write(base.with_extension("class"), &class.bytes)?;
                            for (side, outcome) in [("oracle", &oracle), ("oxasm", &oxasm)] {
                                let bytes = match outcome {
                                    Outcome::Ok(bytes) => bytes.as_slice(),
                                    Outcome::Err(error) => error.as_bytes(),
                                };
                                fs::write(base.with_extension(side), bytes)?;
                            }
                            mismatches.push((chunk_index * chunk_size + i, report));
                        }
                    }
                    Ok(mismatches)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("comparison thread panicked"))
            .collect()
    });
    let mut mismatches = Vec::new();
    for result in results {
        mismatches.extend(result?);
    }
    mismatches.sort();
    println!(
        "{tier}/{}: {} classes, {} mismatched",
        mode.name,
        classes.len(),
        mismatches.len()
    );
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

/// A description of how the two outcomes disagree, or `None` if they agree.
fn mismatch(oracle: &Outcome, oxasm: &Outcome) -> Option<String> {
    match (oracle, oxasm) {
        (Outcome::Ok(a), Outcome::Ok(b)) if a == b => None,
        (Outcome::Err(_), Outcome::Err(error)) if !error.starts_with("PANIC") => None,
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
    }
}

/// Fills the oracle cache of a mode, and returns its directory.
fn fill_mode(
    oracle: &Oracle,
    tier: &str,
    mode: &Mode,
    classes: &[Class],
    hierarchy: Option<&Path>,
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
fn tier_hierarchy(oracle: &Oracle, tier: &str) -> Result<PathBuf> {
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

/// The directory name of a tier under `target/oracle` and `target/parity`: the tier itself, or
/// `dir-<hash of the path>` for a `dir:<path>` tier.
fn tier_key(tier: &str) -> String {
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

/// Every class file of a tier, with the same ids the oracle gives them.
fn tier_classes(tier: &str) -> Result<Vec<Class>> {
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
