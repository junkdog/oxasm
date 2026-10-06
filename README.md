# oxasm

A Rust port of [OW2 ASM](https://asm.ow2.io/) 9.10.1, the JVM bytecode read/analyze/transform/write
library: same capabilities, same output (byte for byte), with a Rust-shaped API.

**Status: P1 (value types, visitor traits, signatures) done.** The `oxasm` crate has its value types
(`JavaStr`, `Type`, `Handle`, `Label`, ...), the six visitor traits and the signature package; the class
reader (P2) is next. The oracle, corpus and golden machinery that the port is verified against is in place.

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
cargo xtask parity --tier t1 --sample 2000 --oracle-only
```

The oracle (`oracle/Oracle.java`) needs Java 22 or newer (CI uses 25) and the released
`org.ow2.asm:*:9.10.1` jars, which xtask takes from `~/.m2` (fetching them with `mvn` if missing) and
checks against the SHA-256 sums in `xtask/src/pin.rs`.

## License

BSD-3-Clause, as ASM. See `LICENSE` and `NOTICE`.
