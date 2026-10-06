# Ported tests: `asm-analysis` → `oxasm-analysis`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-analysis/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `AnalyzerTest` | P4 | 34 | 0 | 0 | pending |  |
| `AnalyzerWithBasicInterpreterTest` | P4 | 7 | 0 | 0 | pending |  |
| `AnalyzerWithBasicVerifierTest` | P4 | 16 | 0 | 0 | pending |  |
| `AnalyzerWithSimpleVerifierTest` | P4 | 10 | 0 | 0 | pending |  |
| `AnalyzerWithSourceInterpreterTest` | P4 | 3 | 0 | 0 | pending |  |
| `BasicValueTest` | P4 | 4 | 0 | 0 | pending |  |
| `SimpleVerifierTest` | P4 | 4 | 0 | 0 | pending |  |
| `SmallSetTest` | P4 | 10 | 0 | 0 | pending |  |
| `SourceValueTest` | P4 | 3 | 0 | 0 | pending |  |
| **total** | | **91** | **0** | **0** | | |

**Test support classes** (no tests of their own):

- `MethodNodeBuilder` → `oxasm-testkit`, when P4 needs it
