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
| `ConstantsTest` | P1 | 14 | 0 | 0 | pending |  |
| `HandleTest` | P1 | 4 | 0 | 0 | pending |  |
| `SignatureReaderTest` | P1 | 3 | 0 | 0 | pending |  |
| `SignatureVisitorTest` | P1 | 1 | 0 | 0 | pending |  |
| `SignatureWriterTest` | P1 | 2 | 0 | 0 | pending |  |
| `TypePathTest` | P1 | 3 | 0 | 0 | pending |  |
| `TypeReferenceTest` | P1 | 8 | 0 | 0 | pending |  |
| `TypeTest` | P1 | 36 | 0 | 0 | pending |  |
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
| **total** | | **234** | **0** | **0** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `SignaturesProviders` → `oxasm-testkit` (signature corpus), when P1 needs it
