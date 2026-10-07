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
| `AdviceAdapterTest` | P8 | 20 | 20 | 0 | ported | `tests/advice_adapter_test.rs`; the advice is an `Advice` impl that gets the adapter (ADR-13); the `newInstance()` checks of the constructor tests are one ignored JVM test; the api half of `testAllMethods_precompiledClass` is gone (ADR-09) |
| `AnalyzerAdapterTest` | P8 | 4 | 4 | 0 | ported | `tests/analyzer_adapter_test.rs`; the subclass half of `testConstructor` is dropped (ADR-13); the frame inserter is the adapter's upstream visitor, inserting into the adapter's downstream (a Rust visitor cannot reach the adapter that owns it); JVM-verified with each class's T0 siblings |
| `ClassRemapperTest` | P8 | 14 | 12 | 2 | ported | `tests/class_remapper_test.rs`; dropped: `testVisitLdcInsn_constantDynamic_deprecated`, `testInvokeDynamicInsn_field_deprecated` (ADR-09: the deprecated api-0 indy mapping); the two `testAllMethods_*` have ignored `*_new_instance` JVM twins |
| `CodeSizeEvaluatorTest` | P8 | 1 | 1 | 0 | ported | `tests/code_size_evaluator_test.rs`; the actual sizes are the written `code_length`s (ASM's `Label.getOffset()`) |
| `GeneratorAdapterTest` | P8 | 74 | 73 | 1 | ported | `tests/generator_adapter_test.rs`; dropped: `testConstructor_illegalState` (subclassing, ADR-13); Java overloads are renamed (`push_int`, `push_type`, ...) |
| `InstructionAdapterTest` | P8 | 19 | 13 | 6 | ported | `tests/instruction_adapter_test.rs`; dropped: `testConstructor` (subclass check), `testDeprecatedVisitMethodInsn_illegalArgument` and the three `testInvoke*_unsupportedOperation` (ADR-09), `testVisitLdcInsn_illegalArgument` (a non-constant `Object`, ADR-05) |
| `JsrInlinerAdapterTest` | P8 | 15 | 15 | 0 | ported | `tests/jsr_inliner_adapter_test.rs`; `testConstructor` loses its subclass check (ADR-13); each test has an ignored `<test>_new_instance` JVM twin; 6 oxasm-specific tests (no downstream, recursion, unowned RET, a 100k-jump chain on a 256 KB stack, ...) |
| `LambdaRemapTest` | P8 | 6 | 3 | 3 | ported | `tests/lambda_remap_test.rs`; dropped: `DeprecatedApi.{normal,serializable,advanced}Lambda` (ADR-09); the remapped lambdas are checked structurally and JVM-verified instead of run |
| `LocalVariablesSorterTest` | P8 | 15 | 15 | 0 | ported | `tests/local_variables_sorter_test.rs`; `testConstructor` loses its subclass check; `testAllMethods_precompiledClass` has an ignored JVM twin |
| `MethodTest` | P8 | 9 | 7 | 2 | ported | `tests/method_test.rs`; dropped: `getMethod(java.lang.reflect.Method/Constructor)` (Java reflection) |
| `SerialVersionUidAdderTest` | P8 | 4 | 4 | 0 | ported | `tests/serial_version_uid_adder_test.rs`; `testConstructor` loses its subclass check; plus Java string order and modified UTF-8 checks against an ASM-computed value |
| `SimpleRemapperTest` | P8 | 4 | 4 | 0 | ported | `tests/simple_remapper_test.rs`; plus `test_key_conventions` |
| `StaticInitMergerTest` | P8 | 1 | 1 | 0 | ported | `tests/static_init_merger_test.rs`; ASM's `counter == 5` needs class initialization: Textifier checks instead, plus an ignored JVM twin; the merged `<clinit>` comes last (ADR-02) |
| `TryCatchBlockSorterTest` | P8 | 2 | 2 | 0 | ported | `tests/try_catch_block_sorter_test.rs`; `testConstructor` loses its subclass check; the all-classes test has an ignored JVM twin |
| **total** | | **191** | **177** | **14** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `tests/common/mod.rs`, a `CustomAttribute` that is its own codec (ADR-10)
- `Comment` → `tests/common/mod.rs`, a `CustomAttribute` that is its own codec (ADR-10)
- `MethodNodeBuilder` → `tests/common/mod.rs` (ASM-commons' own builder, with `to_text` and `build_class_with_method`)
