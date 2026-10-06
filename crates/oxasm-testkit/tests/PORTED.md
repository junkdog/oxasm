# Ported tests: `asm-test` → `oxasm-testkit`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-test/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `AsmTestTest` | P0 | 5 | 4 | 1 | ported | `tests/asm_test_test.rs`; the two `allClassesAnd*Api` tests merge into `test_get_all_classes`; `testApi_allMethods` dropped (ADR-09) |
| `ClassFileTest` | P2 | 9 | 0 | 0 | pending |  |
| **total** | | **14** | **4** | **1** | | |
