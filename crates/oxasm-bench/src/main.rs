//! oxasm's side of `cargo xtask bench`: a warm loop over one scenario, as the oracle's `bench`
//! command runs ASM's side.
//!
//! ```text
//! oxasm-bench run --scenario <name> --pack <file> [--hierarchy <file>] [--warmup <n>]
//!                 [--iterations <n>]
//! oxasm-bench filter --pack <file> --hierarchy <file> --out <file>
//! oxasm-bench list
//! ```
//!
//! `run` prints one line per measured pass, in the oracle's format:
//! `#bench <scenario> <pass> <ops> <nanoseconds> <check>`. `filter` writes the classes of a pack
//! for which every scenario succeeds to a new pack, and reports the others on stderr.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use oxasm::ClassHierarchy;
use oxasm_bench::{Fixture, Scenario, read_hierarchy, read_pack, run, run_class, write_pack};

const USAGE: &str = "\
usage: oxasm-bench run --scenario <name> --pack <file> [--hierarchy <file>] [--warmup <n>]
                       [--iterations <n>]
       oxasm-bench filter --pack <file> --hierarchy <file> --out <file>
       oxasm-bench list";

fn main() -> ExitCode {
    match cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxasm-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cli() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err(format!("missing command\n{USAGE}"));
    }
    let command = args.remove(0);
    let mut value = |name: &str| -> Result<Option<String>, String> {
        let Some(index) = args.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if index + 1 >= args.len() {
            return Err(format!("missing value for {name}"));
        }
        let value = args.remove(index + 1);
        args.remove(index);
        Ok(Some(value))
    };
    let number = |text: Option<String>, default: usize| -> Result<usize, String> {
        text.map_or(Ok(default), |t| t.parse().map_err(|e| format!("{t}: {e}")))
    };
    match command.as_str() {
        "run" => {
            let scenario = value("--scenario")?.ok_or("missing --scenario")?;
            let scenario = Scenario::from_name(&scenario)
                .ok_or_else(|| format!("unknown scenario {scenario}"))?;
            let pack = PathBuf::from(value("--pack")?.ok_or("missing --pack")?);
            let hierarchy = value("--hierarchy")?.map(PathBuf::from);
            let warmup = number(value("--warmup")?, 10)?;
            let iterations = number(value("--iterations")?, 20)?;
            finish(&args)?;
            bench(scenario, &pack, hierarchy.as_deref(), warmup, iterations)
        }
        "filter" => {
            let pack = PathBuf::from(value("--pack")?.ok_or("missing --pack")?);
            let hierarchy = PathBuf::from(value("--hierarchy")?.ok_or("missing --hierarchy")?);
            let out = PathBuf::from(value("--out")?.ok_or("missing --out")?);
            finish(&args)?;
            filter(&pack, &hierarchy, &out)
        }
        "list" => {
            finish(&args)?;
            for scenario in Scenario::ALL {
                println!("{}", scenario.name());
            }
            Ok(())
        }
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

fn finish(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected arguments: {}", args.join(" ")))
    }
}

fn bench(
    scenario: Scenario,
    pack: &Path,
    hierarchy: Option<&Path>,
    warmup: usize,
    iterations: usize,
) -> Result<(), String> {
    if scenario.needs_hierarchy() && hierarchy.is_none() {
        return Err(format!("{} needs --hierarchy", scenario.name()));
    }
    let fixture = Fixture::load(pack, hierarchy)?;
    let ops = scenario.ops(&fixture);
    fixture.prepare(scenario);
    for pass in 0..warmup + iterations {
        let start = Instant::now();
        let check = std::hint::black_box(run(scenario, std::hint::black_box(&fixture)));
        let nanos = start.elapsed().as_nanos();
        if pass >= warmup {
            println!(
                "#bench {} {} {ops} {nanos} {check}",
                scenario.name(),
                pass - warmup
            );
        }
    }
    Ok(())
}

/// Keeps the classes on which every scenario succeeds. A malformed class, a class with JSR
/// instructions (which `COMPUTE_FRAMES` rejects) or one that refers to classes missing from the
/// hierarchy would otherwise stop a pass, or make the sides do different work.
fn filter(pack: &Path, hierarchy: &Path, out: &Path) -> Result<(), String> {
    let classes = read_pack(pack)?;
    let hierarchy = read_hierarchy(hierarchy)?;
    let hierarchy = Some(&hierarchy as &dyn ClassHierarchy);
    // The type scenarios report ASM's exceptions as panics: keep them off stderr.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut kept = Vec::new();
    let mut dropped = 0;
    for (index, class) in classes.iter().enumerate() {
        let failure = Scenario::ALL.into_iter().find_map(|scenario| {
            run_class(scenario, class, hierarchy)
                .err()
                .map(|e| format!("{}: {e}", scenario.name()))
        });
        match failure {
            None => kept.push(class.as_slice()),
            Some(failure) => {
                dropped += 1;
                eprintln!("oxasm-bench: dropping class {index}: {failure}");
            }
        }
    }
    std::panic::set_hook(default_hook);
    write_pack(out, kept.iter().copied())?;
    eprintln!(
        "oxasm-bench: kept {} of {} classes ({dropped} dropped)",
        kept.len(),
        classes.len()
    );
    Ok(())
}
