//! T0 goldens: oracle output committed under `crates/oxasm-testkit/goldens/`, which offline
//! `cargo test` runs compare oxasm against.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::args::Args;
use crate::corpus::UPSTREAM_TEXTIFIER;
use crate::oracle::{Oracle, Status};
use crate::{relative_id, root_path, walk_files};

const GOLDENS: &str = "crates/oxasm-testkit/goldens";
const T0: &str = "corpus/t0";

/// A directory of goldens and the oracle command that produces them.
struct GoldenSet {
    dir: &'static str,
    command: &'static [&'static str],
    /// For a delta set, the set it is a delta of: only the goldens that differ from that set's
    /// are committed, and the others are that set's (`oxasm_testkit::golden` falls back to it).
    base: Option<&'static str>,
}

impl GoldenSet {
    const fn new(dir: &'static str, command: &'static [&'static str]) -> GoldenSet {
        GoldenSet {
            dir,
            command,
            base: None,
        }
    }

    const fn delta(
        dir: &'static str,
        command: &'static [&'static str],
        base: &'static str,
    ) -> GoldenSet {
        GoldenSet {
            dir,
            command,
            base: Some(base),
        }
    }
}

const SETS: [GoldenSet; 12] = [
    GoldenSet::new("textifier/default", &["textify"]),
    GoldenSet::new("textifier/skip_code", &["textify", "--opts", "SKIP_CODE"]),
    GoldenSet::new("textifier/skip_debug", &["textify", "--opts", "SKIP_DEBUG"]),
    GoldenSet::new(
        "textifier/skip_frames",
        &["textify", "--opts", "SKIP_FRAMES"],
    ),
    GoldenSet::new(
        "textifier/expand_frames",
        &["textify", "--opts", "EXPAND_FRAMES"],
    ),
    GoldenSet::new("dump", &["dump"]),
    GoldenSet::new("asmifier/default", &["asmify"]),
    // ASM's tree path (reader -> ClassNode -> Textifier) prints some classes differently from its
    // direct path (Code attributes and try catch block annotations move), hence deltas.
    GoldenSet::delta(
        "textifier_tree/default",
        &["tree-textify"],
        "textifier/default",
    ),
    GoldenSet::delta(
        "textifier_tree/skip_code",
        &["tree-textify", "--opts", "SKIP_CODE"],
        "textifier/skip_code",
    ),
    GoldenSet::delta(
        "textifier_tree/skip_debug",
        &["tree-textify", "--opts", "SKIP_DEBUG"],
        "textifier/skip_debug",
    ),
    GoldenSet::delta(
        "textifier_tree/skip_frames",
        &["tree-textify", "--opts", "SKIP_FRAMES"],
        "textifier/skip_frames",
    ),
    GoldenSet::delta(
        "textifier_tree/expand_frames",
        &["tree-textify", "--opts", "EXPAND_FRAMES"],
        "textifier/expand_frames",
    ),
];

/// File name → content, for one golden set.
type Goldens = BTreeMap<String, Vec<u8>>;

pub fn cli(mut args: Args) -> Result<()> {
    let regen = args.flag("--regen");
    let check = args.flag("--check");
    args.finish()?;
    ensure!(!(regen && check), "pass either --check or --regen");

    let t0 = root_path(T0);
    ensure!(
        t0.is_dir(),
        "{} is missing; run `cargo xtask corpus t0`",
        t0.display()
    );
    let oracle = Oracle::locate()?;

    // One JVM per set, in parallel; each compiles the oracle once and reads all of T0.
    let generated: Vec<Result<Goldens>> = std::thread::scope(|scope| {
        let handles: Vec<_> = SETS
            .iter()
            .map(|set| scope.spawn(|| generate(&oracle, set, &t0)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("golden thread panicked"))
            .collect()
    });

    let generated: Vec<Goldens> = SETS
        .iter()
        .zip(generated)
        .map(|(set, goldens)| goldens.with_context(|| format!("generating {}", set.dir)))
        .collect::<Result<_>>()?;
    let mut problems = 0usize;
    for (set, goldens) in SETS.iter().zip(&generated) {
        let delta;
        let goldens = match set.base {
            None => goldens,
            Some(base) => {
                let base = SETS
                    .iter()
                    .position(|s| s.dir == base)
                    .map(|i| &generated[i])
                    .with_context(|| format!("no golden set {base}"))?;
                delta = goldens
                    .iter()
                    .filter(|(name, content)| base.get(*name) != Some(content))
                    .map(|(name, content)| (name.clone(), content.clone()))
                    .collect();
                &delta
            }
        };
        let dir = root_path(GOLDENS).join(set.dir);
        problems += if regen {
            write_set(set, &dir, goldens)?;
            0
        } else {
            check_set(set, &dir, goldens)?
        };
        if set.dir == "textifier/default" {
            problems += check_upstream(goldens)?;
        }
    }
    if problems > 0 {
        bail!("{problems} golden problem(s); review, then `cargo xtask golden --regen`");
    }
    Ok(())
}

fn generate(oracle: &Oracle, set: &GoldenSet, t0: &Path) -> Result<Goldens> {
    let mut words: Vec<String> = set.command.iter().map(|w| (*w).to_owned()).collect();
    words.push(t0.to_string_lossy().into_owned());
    let mut goldens = Goldens::new();
    for record in oracle.batch(&words)? {
        let extension = match record.status {
            Status::Ok => "txt",
            Status::Err => "err",
            Status::Skip => continue,
        };
        goldens.insert(
            format!("{}.{extension}", golden_name(&record.id)),
            record.payload,
        );
    }
    Ok(goldens)
}

/// `jdk3/AllStructures$1.class` → `jdk3.AllStructures$1`, as AsmTest names classes.
fn golden_name(id: &str) -> String {
    let stem = id
        .strip_suffix(".class")
        .or_else(|| id.strip_suffix(".clazz"))
        .unwrap_or(id);
    stem.replace('/', ".")
}

fn committed(dir: &Path) -> Result<Goldens> {
    let mut goldens = Goldens::new();
    if dir.is_dir() {
        for file in walk_files(dir)? {
            goldens.insert(relative_id(dir, &file), fs::read(&file)?);
        }
    }
    Ok(goldens)
}

fn write_set(set: &GoldenSet, dir: &Path, goldens: &Goldens) -> Result<()> {
    let stale = committed(dir)?;
    for name in stale.keys().filter(|name| !goldens.contains_key(*name)) {
        fs::remove_file(dir.join(name))?;
    }
    fs::create_dir_all(dir)?;
    let mut changed = 0usize;
    for (name, content) in goldens {
        if stale.get(name) != Some(content) {
            fs::write(dir.join(name), content)?;
            changed += 1;
        }
    }
    println!(
        "golden {}: {} files, {changed} written",
        set.dir,
        goldens.len()
    );
    Ok(())
}

fn check_set(set: &GoldenSet, dir: &Path, goldens: &Goldens) -> Result<usize> {
    let committed = committed(dir)?;
    let mut problems = 0usize;
    for (name, content) in goldens {
        match committed.get(name) {
            None => {
                println!("golden {}: {name} is missing", set.dir);
                problems += 1;
            }
            Some(expected) if expected != content => {
                println!(
                    "golden {}: {name} differs: {}",
                    set.dir,
                    first_difference(expected, content)
                );
                problems += 1;
            }
            Some(_) => {}
        }
    }
    for name in committed.keys().filter(|name| !goldens.contains_key(*name)) {
        println!("golden {}: {name} is stale", set.dir);
        problems += 1;
    }
    println!(
        "golden {}: {} files, {}",
        set.dir,
        goldens.len(),
        if problems == 0 {
            "ok".to_owned()
        } else {
            format!("{problems} problem(s)")
        }
    );
    Ok(problems)
}

/// The P0 gate: freshly generated default Textifier output equals ASM's own goldens.
fn check_upstream(generated: &Goldens) -> Result<usize> {
    let upstream_dir = root_path(UPSTREAM_TEXTIFIER);
    let upstream = committed(&upstream_dir)?;
    ensure!(
        !upstream.is_empty(),
        "{} is empty; run `cargo xtask corpus t0`",
        upstream_dir.display()
    );
    let mut problems = 0usize;
    for (name, expected) in &upstream {
        match generated.get(name) {
            None => {
                println!("upstream textifier: no oracle output for {name}");
                problems += 1;
            }
            Some(actual) if actual != expected => {
                println!(
                    "upstream textifier: {name} differs: {}",
                    first_difference(expected, actual)
                );
                problems += 1;
            }
            Some(_) => {}
        }
    }
    println!(
        "upstream textifier: {}/{} equal to asm-util's test resources",
        upstream.len() - problems,
        upstream.len()
    );
    Ok(problems)
}

/// "line N: expected `…`, got `…`" for the first line that differs.
fn first_difference(expected: &[u8], actual: &[u8]) -> String {
    let expected = String::from_utf8_lossy(expected);
    let actual = String::from_utf8_lossy(actual);
    let mut expected_lines = expected.split('\n');
    let mut actual_lines = actual.split('\n');
    for line in 1.. {
        match (expected_lines.next(), actual_lines.next()) {
            (Some(e), Some(a)) if e == a => {}
            (e, a) => {
                return format!(
                    "line {line}: expected {:?}, got {:?}",
                    e.unwrap_or("<end>"),
                    a.unwrap_or("<end>")
                );
            }
        }
    }
    unreachable!("the line loop only ends by returning")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_names_follow_asm_test() {
        assert_eq!(
            golden_name("jdk3/AllStructures$1.class"),
            "jdk3.AllStructures$1"
        );
        assert_eq!(golden_name("DefaultPackage.class"), "DefaultPackage");
        assert_eq!(
            golden_name("invalid/InvalidOpcode.clazz"),
            "invalid.InvalidOpcode"
        );
        assert_eq!(
            golden_name("jdk8/Artificial$()$Structures.class"),
            "jdk8.Artificial$()$Structures"
        );
    }

    #[test]
    fn first_difference_names_the_line() {
        assert_eq!(
            first_difference(b"a\nb\nc", b"a\nx\nc"),
            "line 2: expected \"b\", got \"x\""
        );
        assert_eq!(
            first_difference(b"a\n", b"a\nb"),
            "line 2: expected \"\", got \"b\""
        );
        assert_eq!(
            first_difference(b"a", b"a\n"),
            "line 2: expected \"<end>\", got \"\""
        );
    }
}
