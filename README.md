# oxasm

A Rust port of [OW2 ASM](https://asm.ow2.io/) 9.10.1, the JVM bytecode read/analyze/transform/write
library: same capabilities, same output (byte for byte), with a Rust-shaped API.

**Status: P6 (the class writer's computations) done.** `oxasm` reads class files (`ClassReader`, with ASM's
options and leniencies) into the visitor traits and writes them back (`ClassWriter`, computing the maximum stack
sizes and the stack map frames on request, through a pluggable `ClassHierarchy`), `oxasm-tree` holds them as
`ClassNode`s (an arena-backed `InsnList` with generational handles), `oxasm-analysis` computes their frames
(`Analyzer` with ASM's four stock interpreters), and `oxasm-util` prints them with ASM's `Textifier`, byte for
byte: the Textifier output, the frames of every method and the class files written back (with a new or a copied
constant pool, with expanded frames, with `COMPUTE_MAXS` or `COMPUTE_FRAMES`, directly or through a `ClassNode`)
equal ASM 9.10.1's, on every class of the JDK runtime image and of the T2 jars. Next: M1's acceptance check (a
probe of sift's analyses), then P7 (util's checkers and `ASMifier`) and P8 (commons).

| Crate | Ports |
|---|---|
| `oxasm` | `asm` (reader, writer, visitors, `Type`, signatures) |
| `oxasm-tree` | `asm-tree` |
| `oxasm-analysis` | `asm-analysis` |
| `oxasm-commons` | `asm-commons` |
| `oxasm-util` | `asm-util` (Textifier, tracing, checks) |
| `oxasm-testkit` | `asm-test` (unpublished: T0 corpus, goldens) |

## Development

```zsh
cargo test --workspace            # offline: T0 goldens are committed
cargo xtask golden --check        # goldens == a fresh ASM 9.10.1 oracle run == ASM's own goldens
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 \
  cargo +nightly miri test -p oxasm --lib jstr   # the unsafe in jstr, under Miri (ADR-18)
cargo xtask oracle textify corpus/t0/jdk8/AllFrames.class
cargo xtask corpus t1             # extract $JAVA_HOME's runtime image (about 32k classes)
cargo xtask parity --tier t1      # Textifier, analysis and class writer parity vs ASM (target/parity/)
cargo test --workspace -- --ignored new_instance written_classes_are_valid --skip compute_maxs
                                  # the JVM verifies the classes the tests write (needs Java)
cargo xtask parity --tier t1 --sample 2000 --oracle-only   # only fill the oracle cache (all modes)
```

Fuzzing uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) with nightly; `fuzz/` is its own workspace
(seed `fuzz/corpus/<target>/` with class files, e.g. `corpus/t0`):

```zsh
cd fuzz && cargo +nightly fuzz run -O -a read_textify -- -max_total_time=60   # or read_noop, read_tree, tree_ops, analyze, roundtrip
```

The oracle (`oracle/Oracle.java`) needs Java 22 or newer (CI uses 25) and the released
`org.ow2.asm:*:9.10.1` jars, which xtask takes from `~/.m2` (fetching them with `mvn` if missing) and
checks against the SHA-256 sums in `xtask/src/pin.rs`.

## License

BSD-3-Clause, as ASM. See `LICENSE` and `NOTICE`.
