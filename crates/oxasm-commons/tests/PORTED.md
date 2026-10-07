# Ported tests: `asm-commons` → `oxasm-commons`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-commons/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `ModuleHashesAttributeTest` | P5 | 1 | 1 | 0 | ported | `tests/module_hashes_attribute_test.rs`; the class is `visit`ed first (ASM's test writes a class with a null name, which oxasm's reader rejects, ADR-06) |
| `ModuleResolutionAttributeTest` | P5 | 1 | 1 | 0 | ported | `tests/module_resolution_attribute_test.rs`; the class is `visit`ed first (ASM's test writes a class with a null name, which oxasm's reader rejects, ADR-06) |
| `ModuleTargetAttributeTest` | P5 | 1 | 1 | 0 | ported | `tests/module_target_attribute_test.rs`; the class is `visit`ed first (ASM's test writes a class with a null name, which oxasm's reader rejects, ADR-06) |
| `AdviceAdapterTest` | P8 | 20 | 0 | 0 | pending |  |
| `AnalyzerAdapterTest` | P8 | 4 | 0 | 0 | pending |  |
| `ClassRemapperTest` | P8 | 14 | 0 | 0 | pending |  |
| `CodeSizeEvaluatorTest` | P8 | 1 | 0 | 0 | pending |  |
| `GeneratorAdapterTest` | P8 | 74 | 0 | 0 | pending |  |
| `InstructionAdapterTest` | P8 | 19 | 0 | 0 | pending |  |
| `JsrInlinerAdapterTest` | P8 | 15 | 0 | 0 | pending |  |
| `LambdaRemapTest` | P8 | 6 | 0 | 0 | pending |  |
| `LocalVariablesSorterTest` | P8 | 15 | 0 | 0 | pending |  |
| `MethodTest` | P8 | 9 | 0 | 0 | pending |  |
| `SerialVersionUidAdderTest` | P8 | 4 | 0 | 0 | pending |  |
| `SimpleRemapperTest` | P8 | 4 | 0 | 0 | pending |  |
| `StaticInitMergerTest` | P8 | 1 | 0 | 0 | pending |  |
| `TryCatchBlockSorterTest` | P8 | 2 | 0 | 0 | pending |  |
| **total** | | **191** | **3** | **0** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `MethodNodeBuilder` → `oxasm-testkit`, when P4 needs it
