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
| `PrinterTest` | P2 | 23 | 0 | 0 | pending |  |
| `TextifierTest` | P2 | 10 | 0 | 0 | pending |  |
| `TraceClassVisitorTest` | P2 | 3 | 0 | 0 | pending |  |
| `TraceSignatureVisitorTest` | P2 | 4 | 0 | 0 | pending |  |
| `ASMifierTest` | P7 | 10 | 0 | 0 | pending |  |
| `CheckAnnotationAdapterTest` | P7 | 5 | 0 | 0 | pending |  |
| `CheckClassAdapterTest` | P7 | 44 | 0 | 0 | pending |  |
| `CheckFieldAdapterTest` | P7 | 4 | 0 | 0 | pending |  |
| `CheckFrameAnalyzerTest` | P7 | 15 | 0 | 0 | pending |  |
| `CheckMethodAdapterTest` | P7 | 93 | 0 | 0 | pending |  |
| `CheckModuleAdapterTest` | P7 | 12 | 0 | 0 | pending |  |
| `CheckRecordComponentAdapterTest` | P7 | 4 | 0 | 0 | pending |  |
| `CheckSignatureAdapterTest` | P7 | 44 | 0 | 0 | pending |  |
| **total** | | **271** | **0** | **0** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `MethodNodeBuilder` → `oxasm-testkit`, when P4 needs it
- `SignaturesProviders` → `oxasm-testkit` (signature corpus), when P1 needs it
