# Ported tests: `asm` → `oxasm`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `ConstantsTest` | P1 | 14 | 8 | 6 | ported | `src/constants_test.rs` (a unit test: `constants` is crate-private); the constants are listed explicitly instead of by reflection; `testAsmVersions`, `testIsWhitelisted` and the four `testCheckIsPreview_*` dropped (ADR-09) |
| `HandleTest` | P1 | 4 | 3 | 1 | ported | `tests/handle_test.rs`; `testDeprecatedConstructor` dropped (the deprecated 4-argument constructor is not ported) |
| `SignatureReaderTest` | P1 | 3 | 3 | 0 | ported | `tests/signature_reader_test.rs`, plus the oxasm depth-bound and malformed-input tests (ADR-06) |
| `SignatureVisitorTest` | P1 | 1 | 0 | 1 | ported | `tests/signature_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09), replaced by a test of the default methods |
| `SignatureWriterTest` | P1 | 2 | 2 | 0 | ported | `tests/signature_writer_test.rs`, plus round-trip proptests over generated class, method and type signatures |
| `TypePathTest` | P1 | 3 | 3 | 0 | ported | `tests/type_path_test.rs` |
| `TypeReferenceTest` | P1 | 8 | 8 | 0 | ported | `tests/type_reference_test.rs` |
| `TypeTest` | P1 | 36 | 29 | 7 | ported | `tests/type_test.rs`, plus descriptor proptests; the 7 reflection tests (`getType(Class/Method/Constructor)`, ...) dropped |
| `ClassReaderTest` | P2 | 26 | 23 | 3 | ported | `tests/class_reader_test.rs`; the class-loader and `InputStream` constructor tests (`testStringConstructor`, `testStreamConstructor`, `testStreamConstructor_nullStream`) dropped; `testStreamConstructor_emptyStream` reads an empty slice; plus a `Send + Sync` test |
| `AttributeTest` | P2 (read) / P5 (write) | 4 | 4 | 0 | ported | `tests/attribute_test.rs`: the read half (`testIsUnknown`, `testGetLabels`) plus oxasm's codec read tests (ADR-10), and the write half (`testStaticWrite`, `testCachedContent`) through a `ClassWriter`, since `Attribute.write` is not public here |
| `AnnotationVisitorTest` | P5 | 4 | 3 | 1 | ported | `tests/annotation_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09) |
| `ByteVectorTest` | P5 | 13 | 13 | 0 | ported | `tests/byte_vector_test.rs`, plus the crate-private `put11`/`put12`/`put112`/`put122` in a unit test of `writer/byte_vector.rs`; plus an oxasm test of a supplementary character's 6 bytes against the 65535 bytes limit |
| `ClassVisitorTest` | P5 | 11 | 10 | 1 | ported | `tests/class_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09); `testReadAndWrite_copyPool_changeMethodDescriptor` (it computes maxs) with P6 |
| `ClassWriterFlagsTest` | P5 / P6 | 4 | 4 | 0 | ported | `tests/class_writer_flags_test.rs`; `switchesModesThreeTimes`, `computesFramesAsUsual` and `computesMaxsAsUsual` with P6 |
| `ClassWriterTest` | P5 / P6 | 36 | 32 | 4 | ported | `tests/class_writer_test.rs`, plus 2 JVM-verify twins (`newInstance()`, ignored without the oracle); `testInstanceFields` and `testVisitMethods_final` (reflection), `testNewConst_illegalArgument` (`Constant` is an enum, ADR-05) and `testDeprecatedNewHandle` (not ported) dropped; `testGetCommonSuperClass` runs against a `MapHierarchy` (ADR-07); the 9 COMPUTE_MAXS/COMPUTE_FRAMES tests with P6: those whose result depends on the class hierarchy (byte equality of recomputed frames, JVM verification) use the hierarchy of T0 and the JDK, from the oracle (`oxasm_testkit::jvm::class_loader_hierarchy_tsv`, needs `corpus/t1`), where ASM loads classes |
| `FieldVisitorTest` | P5 | 3 | 2 | 1 | ported | `tests/field_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09) |
| `HandlerTest` | P5 (list) / P6 (`removeRange`) | 9 | 9 | 0 | ported | a unit test of `writer/handler.rs`; a handler list is a `Vec` (ASM's `null` list is empty); the copy constructor is `Handler::with_range` |
| `LabelTest` | P5 | 3 | 3 | 0 | ported | a unit test of `writer/method_writer` (`Label` is an id, ADR-03: `getOffset` is the method writer's `label_offset`) |
| `MethodVisitorTest` | P5 | 28 | 12 | 16 | ported | `tests/method_visitor_test.rs`; dropped (ADR-09): `testConstructor_invalidApi`, the six `test*_asm4Visitor`, the seven `testDeprecatedVisitMethodInsn_*` (the 4-argument `visitMethodInsn` is not ported) and the two `testVisitMethodInsn_*asm4Visitor_isInterface` |
| `MethodWriterTest` | P5 | 2 | 2 | 0 | ported | a unit test of `writer/method_writer`; `testCanCopyMethodAttributesUpdated` reads the attribute names of `constants.rs` (no reflection) |
| `ModuleVisitorTest` | P5 | 3 | 2 | 1 | ported | `tests/module_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09) |
| `RecordComponentVisitorTest` | P5 | 3 | 2 | 1 | ported | `tests/record_component_visitor_test.rs`; `testConstructor_invalidApi` dropped (ADR-09) |
| `ClassWriterComputeMaxsTest` | P6 | 14 | 14 | 0 | ported | `tests/class_writer_compute_maxs_test.rs`; the `TestCaseBuilder` records the instructions and replays them into a class writer; the control flow graph assertions read the graph through the hidden `ClassWriter::last_method_control_flow_graph` (ASM reads package-private `Label` fields); the `newInstance()` checks of 12 tests are one ignored JVM twin, `test_visit_maxs_new_instance` |
| **total** | | **234** | **191** | **43** | | |

**oxasm-specific tests** (no Java counterpart):

- `tests/t3_synthetic_test.rs`: the T3 synthetic classes (jumps beyond ±32 KiB, with and without frames, a
  method and a constant pool too large, a constant pool of 65535 entries), generated with the same calls as
  the oracle's `gen` command and compared with ASM's output, committed in `oxasm-testkit/goldens/t3/`.
- `tests/method_copy_test.rs`: the method copy optimization never bypasses a method adapter, even one that
  overrides nothing (`try_copy_method` is not forwarded, ADR-08).
- `fuzz/regressions/compute/`: the frame computation is not idempotent on the first rewrite (in ASM too: the
  maximum number of locals counts the variable instructions of dead code, which the first write replaces with
  `NOP ... ATHROW`); the `compute` fuzz target checks stability from the second rewrite on.

**Test support classes** (no tests of their own):

- `CodeComment`, `Comment` → `CustomAttribute` + `AttributeCodec` impls in the test files that use them (ADR-10)
- `ClassFile.newInstance()` → `oxasm_testkit::jvm::verify_class` (the oracle's `verify`), in `#[ignore = "needs the JVM oracle"]` tests
- `SignaturesProviders` → `oxasm-testkit::signatures`: the corpus is read from the committed Textifier
  goldens (`// signature` comments) until P2's `ClassReader` can collect it as ASM does
