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
| `ClassReaderTest` | P2 | 26 | 0 | 0 | pending |  |
| `AttributeTest` | P2 (read) / P5 (write) | 4 | 0 | 0 | pending |  |
| `AnnotationVisitorTest` | P5 | 4 | 0 | 0 | pending |  |
| `ByteVectorTest` | P5 | 13 | 0 | 0 | pending |  |
| `ClassVisitorTest` | P5 | 11 | 0 | 0 | pending |  |
| `ClassWriterFlagsTest` | P5 | 4 | 0 | 0 | pending |  |
| `ClassWriterTest` | P5 | 36 | 0 | 0 | pending |  |
| `FieldVisitorTest` | P5 | 3 | 0 | 0 | pending |  |
| `HandlerTest` | P5 | 9 | 0 | 0 | pending |  |
| `LabelTest` | P5 | 3 | 0 | 0 | pending |  |
| `MethodVisitorTest` | P5 | 28 | 0 | 0 | pending |  |
| `MethodWriterTest` | P5 | 2 | 0 | 0 | pending |  |
| `ModuleVisitorTest` | P5 | 3 | 0 | 0 | pending |  |
| `RecordComponentVisitorTest` | P5 | 3 | 0 | 0 | pending |  |
| `ClassWriterComputeMaxsTest` | P6 | 14 | 0 | 0 | pending |  |
| **total** | | **234** | **56** | **15** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `SignaturesProviders` → `oxasm-testkit::signatures`: the corpus is read from the committed Textifier
  goldens (`// signature` comments) until P2's `ClassReader` can collect it as ASM does
