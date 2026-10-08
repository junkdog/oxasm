# oxasm

A Rust port of [OW2 ASM](https://asm.ow2.io/) 9.10.1, the JVM bytecode read/analyze/transform/write
library: same capabilities, same output (byte for byte), with a Rust-shaped API.

**Status: P8 (commons) done: every ASM module is ported.** `oxasm` reads class files (`ClassReader`, with
ASM's options and leniencies) into the visitor traits and writes them back (`ClassWriter`, computing the maximum
stack sizes and the stack map frames on request, through a pluggable `ClassHierarchy`), `oxasm-tree` holds them as
`ClassNode`s (an arena-backed `InsnList` with generational handles, strings interned as 4-byte `Sym`s), `oxasm-analysis` computes their frames
(`Analyzer` with ASM's four stock interpreters), `oxasm-util` prints them (ASM's `Textifier` and `ASMifier`)
and checks them (ASM's `Check*` adapters, which panic with ASM's exception messages), and `oxasm-commons`
transforms them (remapping, `GeneratorAdapter`/`AdviceAdapter`, `LocalVariablesSorter`, `InstructionAdapter`,
`AnalyzerAdapter`, `SerialVersionUidAdder`, JSR inlining, ...). All of it matches ASM 9.10.1 on every class of
the JDK runtime image and of the T2 jars: the printed text, the frames of every method, the class files written
back (with a new or a copied constant pool, with expanded frames, with `COMPUTE_MAXS` or `COMPUTE_FRAMES`,
directly or through a `ClassNode`), byte for byte, the check adapters' verdicts, and the output of the commons
adapters. M1's acceptance holds: ports of sift's `TextifierLines` and `OriginSourceInterpreter`
(`crates/oxasm-util/examples/sift_probe.rs`) give sift's own output on every class sift compiles. P9
(hardening, performance, API review) is under way: the reader's string cache is 21% faster, and the tree stores
interned `Sym`s from a caller-owned `Interner` (ADR-04). Share one interner between the class nodes of a classpath
(`ClassNode::with_interner`): reading `java.base` is then 3–4% faster than before, while a new interner per class
(`ClassNode::with_interner(Arc::default())`) costs 8% more.

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
MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p oxasm --lib reader   # the reader's string cache
MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p oxasm --lib sym      # the Sym interner
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
