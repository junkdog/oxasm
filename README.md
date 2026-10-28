# oxasm

Read, analyze, transform and write JVM class files from Rust.

oxasm is a port of [OW2 ASM](https://asm.ow2.io/) 9.10.1, the bytecode library behind much of the Java
ecosystem. It has the same capabilities and produces the same output, byte for byte, behind an API shaped
for Rust: visitors are traits, errors are `Result`s, and the tree API stores instructions in an arena
instead of a linked list.

## Crates

| Crate | ASM module | What it does |
|---|---|---|
| [`oxasm`](https://docs.rs/oxasm) | `asm` | `ClassReader`, `ClassWriter`, the visitor traits, `Type`, `Handle`, signatures |
| [`oxasm-tree`](https://docs.rs/oxasm-tree) | `asm-tree` | `ClassNode`, `MethodNode`, `InsnList`: classes as an editable object model |
| [`oxasm-analysis`](https://docs.rs/oxasm-analysis) | `asm-analysis` | `Analyzer` and its interpreters: the stack and locals at each instruction |
| [`oxasm-commons`](https://docs.rs/oxasm-commons) | `asm-commons` | remapping, `GeneratorAdapter`, `AdviceAdapter`, `LocalVariablesSorter`, ... |
| [`oxasm-util`](https://docs.rs/oxasm-util) | `asm-util` | `Textifier` and `Asmifier` printers, tracing and checking visitors |

```toml
[dependencies]
oxasm = "0.1"
oxasm-tree = "0.1"   # and the other crates as needed
```

## Examples

### Transform a class

A visitor overrides the events it cares about and forwards the rest to its `delegate`, like a subclass of
ASM's `ClassVisitor`. This one drops every `toString` method:

```rust
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, MethodVisitor,
    ParsingOptions,
};

struct DropToString<V>(V);

impl<V: ClassVisitor> ClassVisitor for DropToString<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        if name == "toString" {
            return None;
        }
        self.0.visit_method(access, name, descriptor, signature, exceptions)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::fs::read("Foo.class")?;
    let reader = ClassReader::new(&input)?;
    let mut writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
    reader.accept(&mut DropToString(&mut writer), ParsingOptions::empty())?;
    std::fs::write("Foo.class", writer.to_bytes()?)?;
    Ok(())
}
```

### Rename classes and members

```rust
use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ParsingOptions};
use oxasm_commons::{ClassRemapper, SimpleRemapper};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::fs::read("Foo.class")?;
    let remapper = SimpleRemapper::from_iter([
        ("com/example/Foo", "com/example/internal/Foo"),
        ("com/example/Foo.bar()V", "baz"),
    ]);
    let mut writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut remap = ClassRemapper::new(&mut writer, remapper);
    ClassReader::new(&input)?.accept(&mut remap, ParsingOptions::empty())?;
    std::fs::write("Foo.class", writer.to_bytes()?)?;
    Ok(())
}
```

### Read a class into a tree

The tree API interns its names, descriptors and signatures as 4-byte `Sym`s. Share one `Interner` between
the classes of a classpath to store each string once:

```rust
use std::sync::Arc;

use oxasm::{ClassReader, Interner, ParsingOptions};
use oxasm_tree::ClassNode;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let interner = Arc::new(Interner::new());
    let input = std::fs::read("Foo.class")?;
    let mut class = ClassNode::with_interner(interner.clone());
    ClassReader::new(&input)?.accept(&mut class, ParsingOptions::empty())?;

    println!("{}", class.resolve(class.name));
    for method in &class.methods {
        println!(
            "  {}{}: {} instructions",
            method.resolve(method.name),
            method.resolve(method.desc),
            method.instructions.len(),
        );
    }
    Ok(())
}
```

### Analyze a method

```rust
use oxasm::{ClassReader, ParsingOptions};
use oxasm_analysis::{Analyzer, BasicVerifier};
use oxasm_tree::ClassNode;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::fs::read("Foo.class")?;
    let mut class = ClassNode::with_interner(Default::default());
    ClassReader::new(&input)?.accept(&mut class, ParsingOptions::empty())?;

    let owner = class.resolve(class.name);
    for method in &class.methods {
        let mut analyzer = Analyzer::new(BasicVerifier::new());
        match analyzer.analyze(owner, method) {
            Ok(frames) => {
                let max_depth = frames.iter().flatten().map(|f| f.stack().len()).max();
                println!("{}: max stack depth {max_depth:?}", method.resolve(method.name));
            }
            Err(error) => println!("{}: {}", method.resolve(method.name), error.message),
        }
    }
    Ok(())
}
```

### Disassemble a class

`TraceClassVisitor` prints a class as ASM's `Textifier` does, byte for byte:

```rust
use oxasm::{ClassReader, ParsingOptions};
use oxasm_util::TraceClassVisitor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::fs::read("Foo.class")?;
    let mut trace = TraceClassVisitor::new(std::io::stdout().lock());
    ClassReader::new(&input)?.accept(&mut trace, ParsingOptions::empty())?;
    trace.take_io_error()?;
    Ok(())
}
```

## Coming from ASM

The class and method names are ASM's, in snake case (`visitMethodInsn` is `visit_method_insn`), and the
documentation is ASM's where the behavior is the same. What differs:

- **Visitors are traits.** Each has a `delegate()` method, and every other method forwards to it by
  default. A child visitor (`visit_method`, `visit_annotation`, ...) is an `Option<Box<dyn ...>>` that
  borrows its parent until it is dropped; `None` lets the reader skip the content.
- **Strings are `JavaStr`.** Class files can hold strings that Rust's `str` cannot, such as unpaired
  surrogates in obfuscated names. `JavaStr` keeps them, so a class always reads and writes back without loss,
  and any `&str` converts to it for free.
- **Constants are enums.** ASM's `Object` arguments (`visitLdcInsn`, frame types, annotation values) are
  `Constant`, `FrameValue` and `AnnotationValue`.
- **Errors are values.** Reading and writing return `ReadError` and `WriteError`, and analysis returns an
  `AnalyzerError` with ASM's message. The `Check*` adapters are the exception: they are assertion tools, and
  panic with ASM's message.
- **Class hierarchies are explicit.** Computing stack map frames (`COMPUTE_FRAMES`) and `SimpleVerifier`
  need to know the super classes of the classes involved. ASM loads classes for that; oxasm asks a
  `ClassHierarchy`, which you give the writer with `ClassWriter::with_hierarchy`. `MapHierarchy` holds the
  facts of a classpath, and `ObjectHierarchy` answers `java/lang/Object` for any two unrelated classes.
- **Instructions live in an arena.** A method's `InsnList` owns its nodes, and `InsnId` handles designate
  them: ASM's `getNext()`, `getPrevious()` and `indexOf` are methods of the list. A handle to a removed
  instruction is detected, not reused.
- **Tree nodes are thread safe.** Every node is `Send + Sync`, and an `Interner` can be shared between
  threads reading different classes.

## Correctness

oxasm is checked against ASM 9.10.1 itself, running on the JVM, over every class of a JDK runtime image
(about 32,000 classes) and about 33,000 classes from Maven jars picked for their bytecode diversity, pre-Java 6
jars included (`corpus/t2.txt`). The comparisons cover the printed text, the frames of every method, the
class files written back (with a new or a copied constant pool, with expanded frames, with `COMPUTE_MAXS` or
`COMPUTE_FRAMES`, directly or through a `ClassNode`), the verdicts of the check adapters, and the output of
the `commons` adapters. All of them match byte for byte. Every crate is also fuzzed, and the JVM verifies the
classes the tests write.

## Minimum supported Rust version

Rust 1.95, edition 2024.

## Development

```zsh
cargo test --workspace            # offline: the test goldens are committed
cargo xtask golden --check        # the goldens == a fresh ASM 9.10.1 run == ASM's own goldens
cargo xtask corpus t1             # extract $JAVA_HOME's runtime image (about 32k classes)
cargo xtask parity --tier t1      # textifier, analysis and class writer parity vs ASM (target/parity/)
cargo xtask parity --tier t1 --sample 2000 --oracle-only   # only fill the oracle cache (all modes)
cargo xtask oracle textify corpus/t0/jdk8/AllFrames.class  # ASM's output for one class
cargo test --workspace -- --ignored new_instance written_classes_are_valid --skip compute_maxs
                                  # the JVM verifies the classes the tests write (needs Java)
```

The oracle (`oracle/Oracle.java`) runs ASM itself. It needs Java 22 or newer (CI uses 25) and the released
`org.ow2.asm:*:9.10.1` jars, which xtask takes from `~/.m2` (fetching them with `mvn` if missing) and checks
against the SHA-256 sums in `xtask/src/pin.rs`.

The `unsafe` code (Java strings, the reader's string cache and the `Sym` interner) runs under Miri:

```zsh
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 cargo +nightly miri test -p oxasm --lib jstr
MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p oxasm --lib reader
MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p oxasm --lib sym
```

Fuzzing uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) with nightly. `fuzz/` is its own
workspace; seed `fuzz/corpus/<target>/` with class files (e.g. from `corpus/t0`):

```zsh
cd fuzz && cargo +nightly fuzz run -O -a read_textify -- -max_total_time=60
# also: read_noop, read_asmify, read_tree, tree_ops, analyze, roundtrip, compute, check, commons
```

## License

BSD-3-Clause, the same as ASM. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
