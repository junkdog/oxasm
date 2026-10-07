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
| `AnalyzerTest` | P4 / P5 | 34 | 34 | 0 | ported | `testAnalyze_runtimeExceptions` throws from the `init` hook (in ASM's same `try`) instead of `newFrame` (no `newFrame` hook); `testAnalyze_invalidOpcode` uses opcode 255 (`u8`); `testAnalyze_invalidOverlappingSubroutines` is ignored, as ASM `@Disabled`s it; the `buildClassWithMethod(...).newInstance()` lines of 15 tests wait for P5 (writer), as `// P5:` comments |
| `AnalyzerWithBasicInterpreterTest` | P4 | 7 | 7 | 0 | ported | `testConstructor`'s `new BasicInterpreter() {}` half dropped (no subclassing, ADR-09); `testAnalyze_basicInterpreter` checks that the `init_jump_target` hook is called instead of `frame instanceof CustomFrame` (no `newFrame`); `testAnalyze_badInterpreter` runs on a thread with a timeout |
| `AnalyzerWithBasicVerifierTest` | P4 | 16 | 16 | 0 | ported | `testConstructor`'s `new BasicVerifier() {}` half dropped (ADR-09) |
| `AnalyzerWithSimpleVerifierTest` | P4 / P5 | 10 | 10 | 0 | ported | JDK classes from a `MapHierarchy` of the oracle's facts (ADR-07); `testAnalyze_classNotFound` expects the hierarchy's `Type D not present` (ASM: `Type java.lang.ClassNotFoundException: D not present`); the `buildClassWithMethod(...).newInstance()` lines of 4 tests wait for P5 (writer), as `// P5:` comments |
| `AnalyzerWithSourceInterpreterTest` | P4 | 3 | 3 | 0 | ported | `testConstructor`'s `new SourceInterpreter() {}` half dropped (ADR-09) |
| `BasicValueTest` | P4 | 4 | 4 | 0 | ported | `equals(new Object())` assertion dropped; `testHashCode` compares hashes of equal and different values (no numeric `hashCode`) |
| `SimpleVerifierTest` | P4 | 4 | 4 | 0 | ported | `testConstructor`'s `new SimpleVerifier() {}` half dropped (ADR-09); `testMerge_objectTypes` uses a `MapHierarchy` of the oracle's facts; the `getClass`/`getSuperClass` overrides of the `testIsAssignableFrom_*` tests are an `ObjectHierarchy` |
| `SmallSetTest` | P4 | 10 | 9 | 1 | ported | `testIterator_remove` dropped (immutable set); sets compared by content, as `union` does not return its operands; `NoSuchElementException` is `None` |
| `SourceValueTest` | P4 | 3 | 3 | 0 | ported | `testHashcode` compares hashes (no numeric `hashCode`); `equals(null)` is a comparison with `None` |
| **total** | | **91** | **90** | **1** | | the `newInstance()` checks of 19 tests wait for P5 |

**Test support classes** (no tests of their own):

- `MethodNodeBuilder` → `tests/common/mod.rs`; its `buildClassWithMethod` (a class made with a `ClassWriter`) waits for P5
