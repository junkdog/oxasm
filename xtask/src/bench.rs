//! `cargo xtask bench`: oxasm against ASM 9.10.1 on the scenarios of `crates/oxasm-bench`
//! (oxasm-verification, "Benchmarks").
//!
//! Both sides are warm loops over the same pack of classes: `oxasm-bench run` (release) and the
//! oracle's `bench` command, one process per scenario and side, run in turn for a number of
//! rounds (the order of the sides alternates between rounds). Each process prints the time of
//! each measured pass and a check value, which must be the same on both sides.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use oxasm_bench::Scenario;

use crate::args::Args;
use crate::oracle::Oracle;
use crate::parity::{Class, stable_sample, tier_classes, tier_hierarchy, tier_key};
use crate::{root_path, workspace_root};

/// The size of the T1 sample of the default set.
const DEFAULT_T1_SAMPLE: usize = 2000;

/// The P9 gate: oxasm's time per operation at most this many times ASM's.
const GATE: f64 = 1.5;

pub fn cli(mut args: Args) -> Result<()> {
    let tier = args.value("--tier")?;
    let sample = args
        .value("--sample")?
        .map(|n| n.parse::<usize>())
        .transpose()
        .context("--sample")?;
    let scenario_names = args.values("--scenario")?;
    let number = |text: Option<String>, name: &str, default: usize| -> Result<usize> {
        text.map_or(Ok(default), |t| t.parse().with_context(|| name.to_owned()))
    };
    let rounds = number(args.value("--rounds")?, "--rounds", 5)?;
    let warmup = number(args.value("--warmup")?, "--warmup", 10)?;
    let iterations = number(args.value("--iterations")?, "--iterations", 20)?;
    let cpus = args.value("--cpus")?;
    let prepare = args.flag("--prepare");
    args.finish()?;
    ensure!(
        sample.is_none() || tier.is_some(),
        "--sample needs --tier (the default set is T0 and a {DEFAULT_T1_SAMPLE}-class T1 sample)"
    );

    let scenarios: Vec<Scenario> = if scenario_names.is_empty() {
        Scenario::ALL.to_vec()
    } else {
        scenario_names
            .iter()
            .map(|name| {
                Scenario::from_name(name).with_context(|| {
                    let known: Vec<_> = Scenario::ALL.iter().map(|s| s.name()).collect();
                    format!("unknown --scenario {name}; known: {}", known.join(", "))
                })
            })
            .collect::<Result<_>>()?
    };

    let set = Set::new(tier, sample);
    let oracle = Oracle::locate()?;
    let runner = build_runner()?;
    if prepare || !set.pack().is_file() || !set.hierarchy().is_file() {
        set.prepare(&oracle, &runner)?;
    }
    if prepare {
        return Ok(());
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let raw_path = set.dir.join(format!("runs-{stamp}.tsv"));
    let mut raw = String::from("round\tside\tscenario\tpass\tops\tnanos\tcheck\n");
    let mut samples: BTreeMap<(Side, &str), Vec<Pass>> = BTreeMap::new();

    for round in 0..rounds {
        for &scenario in &scenarios {
            let mut sides = vec![Side::Oxasm];
            // ASM's side of scenarios that share it (read_tree_shared) runs once per round.
            if !samples_this_round(&samples, scenario.asm_name(), round, iterations) {
                sides.push(Side::Asm);
            }
            if round % 2 == 1 {
                sides.reverse();
            }
            for side in sides {
                let name = match side {
                    Side::Oxasm => scenario.name(),
                    Side::Asm => scenario.asm_name(),
                };
                let mut command = match side {
                    Side::Oxasm => {
                        let mut command = Command::new(&runner);
                        command.arg("run");
                        command
                    }
                    Side::Asm => oracle.command(&["bench"]),
                };
                command
                    .args(["--scenario", name, "--pack"])
                    .arg(set.pack())
                    .arg("--hierarchy")
                    .arg(set.hierarchy())
                    .args(["--warmup", &warmup.to_string()])
                    .args(["--iterations", &iterations.to_string()]);
                let passes = run_side(pin(command, cpus.as_deref()), name)
                    .with_context(|| format!("{} {name}, round {}", side.label(), round + 1))?;
                ensure!(
                    passes.len() == iterations,
                    "{} {name} printed {} passes, not {iterations}",
                    side.label(),
                    passes.len()
                );
                for (index, pass) in passes.iter().enumerate() {
                    writeln!(
                        raw,
                        "{}\t{}\t{name}\t{index}\t{}\t{}\t{}",
                        round + 1,
                        side.label(),
                        pass.ops,
                        pass.nanos,
                        pass.check
                    )?;
                }
                let best = passes
                    .iter()
                    .map(Pass::nanos_per_op)
                    .fold(f64::INFINITY, f64::min);
                eprintln!(
                    "round {}/{rounds} {:<5} {name:<26} min {}",
                    round + 1,
                    side.label(),
                    format_nanos(best)
                );
                samples.entry((side, name)).or_default().extend(passes);
                // Written as it goes, so that a failure later keeps what was measured.
                fs::write(&raw_path, &raw)?;
            }
        }
    }

    let summary = summarize(
        &scenarios,
        &samples,
        &set,
        rounds,
        iterations,
        cpus.as_deref(),
    )?;
    print!("{summary}");
    let summary_path = set.dir.join(format!("summary-{stamp}.txt"));
    fs::write(&summary_path, &summary)?;
    eprintln!(
        "xtask: wrote {} and {}",
        summary_path.display(),
        raw_path.display()
    );
    Ok(())
}

/// Whether ASM's side of `asm_name` already ran in this round (for a scenario that shares it).
fn samples_this_round(
    samples: &BTreeMap<(Side, &str), Vec<Pass>>,
    asm_name: &str,
    round: usize,
    iterations: usize,
) -> bool {
    samples
        .get(&(Side::Asm, asm_name))
        .is_some_and(|passes| passes.len() > round * iterations)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    Oxasm,
    Asm,
}

impl Side {
    fn label(self) -> &'static str {
        match self {
            Side::Oxasm => "oxasm",
            Side::Asm => "asm",
        }
    }
}

/// One measured pass, as printed by either side.
#[derive(Clone, Copy, Debug)]
struct Pass {
    ops: u64,
    nanos: u64,
    check: u64,
}

impl Pass {
    #[allow(clippy::cast_precision_loss)] // nanoseconds and counts far below 2^52
    fn nanos_per_op(&self) -> f64 {
        self.nanos as f64 / self.ops as f64
    }
}

/// The classes a bench runs on: `target/bench/<name>/classes.pack` and `hierarchy.tsv`.
#[derive(Debug)]
struct Set {
    /// `None` for the default set (T0 and a T1 sample).
    tier: Option<String>,
    sample: Option<usize>,
    dir: PathBuf,
}

impl Set {
    fn new(tier: Option<String>, sample: Option<usize>) -> Set {
        let name = match (&tier, sample) {
            (None, _) => "default".to_owned(),
            (Some(tier), None) => tier_key(tier),
            (Some(tier), Some(n)) => format!("{}-sample{n}", tier_key(tier)),
        };
        Set {
            tier,
            sample,
            dir: root_path("target/bench").join(name),
        }
    }

    fn pack(&self) -> PathBuf {
        self.dir.join("classes.pack")
    }

    fn hierarchy(&self) -> PathBuf {
        self.dir.join("hierarchy.tsv")
    }

    fn describe(&self) -> String {
        match (&self.tier, self.sample) {
            (None, _) => format!("T0 and a {DEFAULT_T1_SAMPLE}-class T1 sample"),
            (Some(tier), None) => tier.clone(),
            (Some(tier), Some(n)) => format!("a {n}-class sample of {tier}"),
        }
    }

    /// Writes the pack of the set's classes on which every scenario succeeds, and the hierarchy
    /// of the set (with the JDK of T1).
    fn prepare(&self, oracle: &Oracle, runner: &Path) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let (classes, hierarchy) = match &self.tier {
            None => {
                let mut t1 = tier_classes("t1")?;
                stable_sample(&mut t1, DEFAULT_T1_SAMPLE);
                let mut classes = tier_classes("t0")?;
                classes.extend(t1);
                // T0's hierarchy includes T1's.
                (classes, tier_hierarchy(oracle, "t0")?)
            }
            Some(tier) => {
                let mut classes = tier_classes(tier)?;
                if let Some(n) = self.sample {
                    stable_sample(&mut classes, n);
                }
                (classes, tier_hierarchy(oracle, tier)?)
            }
        };
        fs::copy(&hierarchy, self.hierarchy())
            .with_context(|| format!("cannot copy {}", hierarchy.display()))?;
        let candidates = self.dir.join("candidates.pack");
        oxasm_bench::write_pack(
            &candidates,
            classes.iter().map(|c: &Class| c.bytes.as_slice()),
        )
        .map_err(anyhow::Error::msg)?;
        fs::write(
            self.dir.join("candidates.txt"),
            classes
                .iter()
                .map(|c| format!("{}\n", c.id))
                .collect::<String>(),
        )?;

        let output = Command::new(runner)
            .arg("filter")
            .arg("--pack")
            .arg(&candidates)
            .arg("--hierarchy")
            .arg(self.hierarchy())
            .arg("--out")
            .arg(self.pack())
            .stderr(Stdio::piped())
            .output()
            .context("cannot run oxasm-bench")?;
        let log = self.dir.join("filter.log");
        fs::write(&log, &output.stderr)?;
        ensure!(
            output.status.success(),
            "oxasm-bench filter exited with {} (see {})",
            output.status,
            log.display()
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "xtask: {} ({}): {}",
            self.describe(),
            self.dir.display(),
            stderr
                .lines()
                .last()
                .unwrap_or_default()
                .trim_start_matches("oxasm-bench: ")
        );
        Ok(())
    }
}

/// Builds `oxasm-bench` in release mode, and returns the path of the binary.
fn build_runner() -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(workspace_root())
        .args([
            "build",
            "--quiet",
            "--release",
            "-p",
            "oxasm-bench",
            "--bin",
            "oxasm-bench",
        ])
        .status()
        .context("cannot run cargo")?;
    ensure!(status.success(), "building oxasm-bench failed: {status}");
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root_path("target"), PathBuf::from);
    Ok(target.join("release").join("oxasm-bench"))
}

/// The command under `taskset -c <cpus>`, if CPUs are given.
fn pin(command: Command, cpus: Option<&str>) -> Command {
    let Some(cpus) = cpus else {
        return command;
    };
    let mut pinned = Command::new("taskset");
    pinned
        .args(["-c", cpus])
        .arg(command.get_program())
        .args(command.get_args());
    pinned
}

/// Runs one side of a scenario and parses its `#bench` lines.
fn run_side(mut command: Command, scenario: &str) -> Result<Vec<Pass>> {
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("cannot run {}", command.get_program().display()))?;
    ensure!(output.status.success(), "exited with {}", output.status);
    let stdout = String::from_utf8(output.stdout).context("output is not UTF-8")?;
    let mut passes = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("#bench ") else {
            continue;
        };
        let fields: Vec<&str> = rest.split(' ').collect();
        let [name, _pass, ops, nanos, check] = fields[..] else {
            bail!("malformed line {line:?}");
        };
        ensure!(name == scenario, "line for scenario {name}: {line:?}");
        let number = |text: &str| text.parse::<u64>().with_context(|| format!("in {line:?}"));
        passes.push(Pass {
            ops: number(ops)?,
            nanos: number(nanos)?,
            check: number(check)?,
        });
    }
    Ok(passes)
}

/// The value at quantile `q` of sorted values (nearest rank).
fn quantile(sorted: &[f64], q: f64) -> f64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )] // a rank within the slice
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[index]
}

fn format_nanos(nanos: f64) -> String {
    if nanos >= 1e6 {
        format!("{:.2} ms", nanos / 1e6)
    } else if nanos >= 1e3 {
        format!("{:.2} µs", nanos / 1e3)
    } else {
        format!("{nanos:.1} ns")
    }
}

/// The table of results: per scenario, the time per operation of each side (min and p25 over
/// every measured pass of every round), and oxasm's time over ASM's. Checks that both sides
/// did the same work: the same number of operations and the same check value in every pass.
fn summarize(
    scenarios: &[Scenario],
    samples: &BTreeMap<(Side, &str), Vec<Pass>>,
    set: &Set,
    rounds: usize,
    iterations: usize,
    cpus: Option<&str>,
) -> Result<String> {
    let mut out = String::new();
    writeln!(
        out,
        "oxasm against ASM 9.10.1 on {}: {rounds} rounds of {iterations} measured passes per side{}",
        set.describe(),
        cpus.map(|c| format!(", pinned to CPUs {c}"))
            .unwrap_or_default()
    )?;
    writeln!(
        out,
        "time per op (class, generated class or descriptor); ratio = oxasm / ASM; gate: p25 ratio ≤ {GATE}"
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "{:<26} {:>6} {:>10} {:>10} {:>10} {:>10} {:>6} {:>6}  gate",
        "scenario", "ops", "oxasm min", "oxasm p25", "asm min", "asm p25", "min×", "p25×"
    )?;
    let mut failures = Vec::new();
    for scenario in scenarios {
        let oxasm = &samples[&(Side::Oxasm, scenario.name())];
        let asm = &samples[&(Side::Asm, scenario.asm_name())];
        let reference = asm[0];
        if let Some(pass) = oxasm
            .iter()
            .chain(asm)
            .find(|p| p.ops != reference.ops || p.check != reference.check)
        {
            failures.push(format!(
                "{}: ASM's passes have {} ops and check {}, but a pass has {} ops and check {}",
                scenario.name(),
                reference.ops,
                reference.check,
                pass.ops,
                pass.check
            ));
        }
        let stats = |passes: &[Pass]| {
            let mut values: Vec<f64> = passes.iter().map(Pass::nanos_per_op).collect();
            values.sort_by(f64::total_cmp);
            (values[0], quantile(&values, 0.25))
        };
        let (oxasm_min, oxasm_p25) = stats(oxasm);
        let (asm_min, asm_p25) = stats(asm);
        let ratio = oxasm_p25 / asm_p25;
        writeln!(
            out,
            "{:<26} {:>6} {:>10} {:>10} {:>10} {:>10} {:>6.2} {:>6.2}  {}",
            scenario.name(),
            reference.ops,
            format_nanos(oxasm_min),
            format_nanos(oxasm_p25),
            format_nanos(asm_min),
            format_nanos(asm_p25),
            oxasm_min / asm_min,
            ratio,
            if ratio <= GATE { "ok" } else { "OVER" }
        )?;
    }
    if !failures.is_empty() {
        writeln!(out)?;
        for failure in &failures {
            writeln!(out, "MISMATCH {failure}")?;
        }
        print!("{out}");
        bail!(
            "the sides did different work in {} scenarios",
            failures.len()
        );
    }
    Ok(out)
}
