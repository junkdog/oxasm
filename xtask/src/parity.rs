//! Parity sweeps over a corpus tier. Until the oxasm reader (P2) and writer (P5) exist, this only
//! fills the oracle result cache, `target/oracle/<tier>/<mode>/<sha256>.{ok,err}`, keyed by the
//! SHA-256 of each class file, so that later sweeps only pay for classes they have not seen.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::args::Args;
use crate::oracle::{Oracle, ServeRequest};
use crate::{is_class_file, relative_id, root_path, sha256_hex, walk_files};

/// A mode of the byte-parity matrix (oxasm-verification, "Byte parity"), and the oracle command
/// that produces ASM's side of it.
struct Mode {
    name: &'static str,
    command: &'static str,
    needs_hierarchy: bool,
}

const MODES: [Mode; 12] = [
    Mode::new("textify", "textify"),
    Mode::new("copy", "roundtrip --mode 0 --copy-pool"),
    Mode::new("plain", "roundtrip --mode 0"),
    Mode::new("maxs", "roundtrip --mode maxs"),
    Mode::hierarchy("frames", "roundtrip --mode frames"),
    Mode::new("expand", "roundtrip --mode 0 --opts EXPAND_FRAMES"),
    Mode::new("skip-debug", "roundtrip --mode 0 --opts SKIP_DEBUG"),
    Mode::new("skip-code", "roundtrip --mode 0 --opts SKIP_CODE"),
    Mode::new("tree-copy", "tree-roundtrip --mode 0 --copy-pool"),
    Mode::new("tree-plain", "tree-roundtrip --mode 0"),
    Mode::new("tree-maxs", "tree-roundtrip --mode maxs"),
    Mode::hierarchy("tree-frames", "tree-roundtrip --mode frames"),
];

impl Mode {
    const fn new(name: &'static str, command: &'static str) -> Mode {
        Mode {
            name,
            command,
            needs_hierarchy: false,
        }
    }

    const fn hierarchy(name: &'static str, command: &'static str) -> Mode {
        Mode {
            name,
            command,
            needs_hierarchy: true,
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
    let tier = args.value("--tier")?.context("missing --tier <t0|t1|t2>")?;
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

    if !oracle_only {
        bail!(
            "the oxasm side of parity sweeps lands with the reader (P2) and writer (P5); pass \
             --oracle-only to fill the oracle cache"
        );
    }
    let modes: Vec<&Mode> = if mode_names.is_empty() {
        MODES.iter().collect()
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

    for mode in modes {
        fill_mode(&oracle, &tier, mode, &classes, hierarchy.as_deref(), jobs)?;
    }
    Ok(())
}

fn fill_mode(
    oracle: &Oracle,
    tier: &str,
    mode: &Mode,
    classes: &[Class],
    hierarchy: Option<&Path>,
    jobs: usize,
) -> Result<()> {
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
    let cache = root_path("target/oracle").join(tier).join(&key);
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
    Ok(())
}

/// `target/oracle/<tier>/hierarchy.tsv`: the hierarchy of the tier plus, for tiers other than T1,
/// the JDK classes of T1 (when extracted). It is regenerated on every run, so it follows the
/// corpus; cached results stay valid because their key includes its hash.
fn tier_hierarchy(oracle: &Oracle, tier: &str) -> Result<PathBuf> {
    let path = root_path("target/oracle").join(tier).join("hierarchy.tsv");
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

/// The oracle inputs (directories or jars) that make up a tier.
fn tier_inputs(tier: &str) -> Result<Vec<String>> {
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
