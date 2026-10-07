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
| `PrinterTest` | P2 | 23 | 22 | 1 | ported | `tests/printer_test.rs`; `testVisitMethodInsn_asm4_unsupportedByDefault` dropped (ADR-09); plus `append_string` and text-tree tests |
| `TextifierTest` | P2 | 10 | 10 | 0 | ported | `tests/textifier_test.rs`; `testConstructor`'s subclass half dropped (no subclassing); `testMain_className` adapted (no class path: a class name is "Class not found"; the content checks run on a class file); plus the oracle-golden test over all T0 classes × 5 reader options (`.err` goldens must fail to read) and hand-driven tests checked against ASM's output |
| `TraceClassVisitorTest` | P2 | 3 | 3 | 0 | ported | `tests/trace_class_visitor_test.rs`; `test_visit_methods` reads with `Comment`/`CodeComment` codecs, as ASM does with its prototypes (un-ignored in P7) |
| `TraceSignatureVisitorTest` | P2 | 4 | 4 | 0 | ported | `tests/trace_signature_visitor_test.rs`, plus a malformed-signature no-panic test |
| `ASMifierTest` | P7 | 10 | 10 | 0 | ported | `tests/asmifier_test.rs`; `testConstructor`'s subclass half dropped (no subclassing); `testAsmify_precompiledClass` adapted (no Java compiler: the output of every T0 class must equal ASM's, from the `asmifier/default` oracle goldens); `testMain_className*` adapted as in `TextifierTest`; plus a hand-driven test of every event against ASM's output (`data/asmifier_hand_driven.txt`, from `data/AsmifierHand.java`), `with_names` and malformed-event tests |
| `CheckAnnotationAdapterTest` | P7 | 5 | 3 | 2 | ported | `tests/check_annotation_adapter_test.rs`; a null enum value and a `new Object()` value dropped (unrepresentable, ADR-05); `testConstructor`'s subclass half dropped |
| `CheckClassAdapterTest` | P7 | 44 | 41 | 3 | ported | `tests/check_class_adapter_test.rs`; three null-argument tests dropped (a null class name, outer class owner and attribute); the all-API tests run on the latest API (ADR-09); `verify`/`main` run over the T0 + JDK hierarchy (oracle), and their outputs and the data flow messages are compared in full with ASM 9.10.1's; `testMain_className` adapted ("Class not found") |
| `CheckFieldAdapterTest` | P7 | 4 | 3 | 1 | ported | `tests/check_field_adapter_test.rs`; a null attribute dropped |
| `CheckFrameAnalyzerTest` | P7 | 15 | 14 | 1 | ported | `tests/check_frame_analyzer_test.rs` (with ASM-util's `MethodNodeBuilder` in `tests/common/mod.rs`); a `java.lang.Object` frame value dropped (unrepresentable); the illegal frame type is `F_INSERT` (256); the precompiled-class tests compute frames over the T0 + JDK hierarchy (oracle) |
| `CheckMethodAdapterTest` | P7 | 93 | 79 | 14 | ported | `tests/check_method_adapter_test.rs`; dropped: arguments the Rust types rule out (null labels, attributes and arrays, arrays shorter than their counts, a negative variable index, invalid frame values, an `ldc` of an `Object`, handle tag -1) and the deprecated `visitMethodInsn` (ADR-09); null names are empty strings (same message); the `MethodWriterWrapper` tests go through a `CheckClassAdapter` over a `ClassWriter`; plus a shared `label_insn_indices` test |
| `CheckModuleAdapterTest` | P7 | 12 | 12 | 0 | ported | `tests/check_module_adapter_test.rs`; null arrays are empty slices (ADR-02); the `java.base` tests get their adapter from a `CheckClassAdapter` (for the class version) |
| `CheckRecordComponentAdapterTest` | P7 | 4 | 3 | 1 | ported | `tests/check_record_component_adapter_test.rs`; a null attribute dropped |
| `CheckSignatureAdapterTest` | P7 | 44 | 42 | 2 | ported | `tests/check_signature_adapter_test.rs`; null type variable and class names dropped (the empty-name tests give the same message); plus tests with delegates that return distinct nested visitors (`TraceSignatureVisitor`, `SignatureWriter`), run under Miri (Stacked and Tree Borrows) |
| **total** | | **271** | **246** | **25** | | |

**Test support classes** (no tests of their own):

- `CodeComment`, `Comment` → `CustomAttribute` impls (ADR-10), local to the tests that need them:
  `tests/asmifier_test.rs` (with an ASMifier representation, as `ASMifierSupport`),
  `tests/textifier_test.rs` (with a Textifier representation, as `TextifierSupport`), and, with their
  codecs (ASM's prototypes), `tests/trace_class_visitor_test.rs` and `tests/common/mod.rs`
- `MethodNodeBuilder` → `tests/common/mod.rs` (a copy of `oxasm-analysis`'s, for `CheckFrameAnalyzerTest`)
- `SignaturesProviders` → `oxasm-testkit` (`signatures`)

**Exceptions:** the check adapters panic where ASM throws, with the exception's `toString()` as the
message; the tests compare it in full with `oxasm_testkit::panic_message`, which checks the exception
class as well.

**oxasm-specific:** `tests/java_fmt_test.rs` checks `java_fmt` (Java's `Float`/`Double.toString`)
against 2,450 values printed by Java 25 (`tests/data/java_fmt.txt`).
