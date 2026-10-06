# Ported tests: `asm-util` → `oxasm-util`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-util/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `PrinterTest` | P2 | 23 | 22 | 1 | ported | `tests/printer_test.rs`; `testVisitMethodInsn_asm4_unsupportedByDefault` dropped (ADR-09); plus `append_string` and text-tree tests |
| `TextifierTest` | P2 | 10 | 10 | 0 | ported | `tests/textifier_test.rs`; `testConstructor`'s subclass half dropped (no subclassing); `testMain_className` adapted (no class path: a class name is "Class not found"; the content checks run on a class file); plus the oracle-golden test over all T0 classes × 5 reader options (`.err` goldens must fail to read) and hand-driven tests checked against ASM's output |
| `TraceClassVisitorTest` | P2 | 3 | 3 | 0 | ported | `tests/trace_class_visitor_test.rs`; `test_visit_methods` `#[ignore = "needs ClassWriter (P5)"]` |
| `TraceSignatureVisitorTest` | P2 | 4 | 4 | 0 | ported | `tests/trace_signature_visitor_test.rs`, plus a malformed-signature no-panic test |
| `ASMifierTest` | P7 | 10 | 0 | 0 | pending |  |
| `CheckAnnotationAdapterTest` | P7 | 5 | 0 | 0 | pending |  |
| `CheckClassAdapterTest` | P7 | 44 | 0 | 0 | pending |  |
| `CheckFieldAdapterTest` | P7 | 4 | 0 | 0 | pending |  |
| `CheckFrameAnalyzerTest` | P7 | 15 | 0 | 0 | pending |  |
| `CheckMethodAdapterTest` | P7 | 93 | 0 | 0 | pending |  |
| `CheckModuleAdapterTest` | P7 | 12 | 0 | 0 | pending |  |
| `CheckRecordComponentAdapterTest` | P7 | 4 | 0 | 0 | pending |  |
| `CheckSignatureAdapterTest` | P7 | 44 | 0 | 0 | pending |  |
| **total** | | **271** | **39** | **1** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10); for now a local impl in
  `tests/textifier_test.rs` (with a Textifier representation, as `TextifierSupport`)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10); likewise local for now
- `MethodNodeBuilder` → `oxasm-testkit`, when P4 needs it
- `SignaturesProviders` → `oxasm-testkit` (signature corpus), when P1 needs it

**oxasm-specific:** `tests/java_fmt_test.rs` checks `java_fmt` (Java's `Float`/`Double.toString`)
against 2,450 values printed by Java 25 (`tests/data/java_fmt.txt`).
