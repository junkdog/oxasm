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
| `ClassFileTest` | P2 | 9 | 7 | 2 | ported | `tests/class_file_test.rs`; `testNewInstance_validClass` and `testStaticNewInstance_invalidClass` dropped (they load classes in a JVM; the oracle's `verify` replaces `newInstance`); `testNewInstance_invalidClass` checks the dump's `ClassFormatException`, which is what `newInstance` throws. Plus `tests/classfile_dump_goldens.rs` (all 73 T0 dumps == the oracle goldens), `tests/classfile_dump_robustness.rs` and the opt-in oracle cross-check `tests/classfile_dump_oracle.rs` |
| **total** | | **14** | **11** | **3** | | |
