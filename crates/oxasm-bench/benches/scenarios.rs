//! The scenarios of `cargo xtask bench`, under criterion, to compare oxasm with itself between
//! versions (`--save-baseline`, `critcmp`). Comparing with ASM is xtask's job.
//!
//! The classes come from the pack `cargo xtask bench --prepare` writes:
//! `target/bench/default/classes.pack` and `hierarchy.tsv` next to it, or the pack named by
//! `$OXASM_BENCH_PACK`. Without a pack, no benchmark runs.

use std::path::PathBuf;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use oxasm_bench::{Fixture, Scenario, run};

fn scenarios(c: &mut Criterion) {
    let pack = std::env::var_os("OXASM_BENCH_PACK").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/bench/default/classes.pack")
        },
        PathBuf::from,
    );
    if !pack.is_file() {
        eprintln!(
            "no pack at {}: run `cargo xtask bench --prepare` (or set OXASM_BENCH_PACK)",
            pack.display()
        );
        return;
    }
    let hierarchy = pack.with_file_name("hierarchy.tsv");
    let fixture = Fixture::load(&pack, hierarchy.is_file().then_some(hierarchy.as_path()))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut group = c.benchmark_group("oxasm");
    for scenario in Scenario::ALL {
        if scenario.needs_hierarchy() && fixture.hierarchy.is_none() {
            continue;
        }
        group.throughput(Throughput::Elements(scenario.ops(&fixture) as u64));
        group.bench_function(scenario.name(), |b| b.iter(|| run(scenario, &fixture)));
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = scenarios
}
criterion_main!(benches);
