# Ported tests: `asm-tree` → `oxasm-tree`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-tree/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*. The 4 tests that compare `ClassWriter` outputs (with
  `ClassFile` dumps) were ported with the writer (P5); the Textifier forms that stood in for them in P3 are
  kept as `*_textify` tests.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `AnnotationNodeTest` | P3 | 4 | 3 | 1 | ported | `testConstructor_illegalState` dropped (no subclassing, ADR-09); plus a test of every kind of value |
| `ClassNodeTest` | P3 / P5 | 6 | 4 | 2 | ported | `testConstructor_illegalState` and `testCheck` dropped (ADR-09); `testVisitAndAccept`, `testVisitAndAccept_cloneInstructions` and `testVisitAndAccept_removeMembersVisitor` over every T0 class (no `Api` axis), comparing `ClassFile` dumps, with their P3 Textifier forms kept (`*_textify`, and `tree_textify_test.rs`); plus a `Send + Sync` test |
| `FieldInsnNodeTest` | P3 | 2 | 2 | 0 | ported |  |
| `FieldNodeTest` | P3 | 2 | 1 | 1 | ported | `testConstructor_illegalState` dropped (ADR-09) |
| `FrameNodeTest` | P3 | 2 | 1 | 1 | ported | `testConstructor_illegalArgument` dropped (`FrameKind` cannot hold an unknown type); plus a test of what each kind keeps |
| `IincInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `InsnListTest` | P3 | 63 | 57 | 6 | ported | the 4 `*_illegalArgument` tests dropped (a list owns its nodes, and cannot be added to itself), `testIterator_cacheIsNull` (private cache) and `testResetLabels` (ADR-03) dropped; `NoSuchElementException` cases are panics or `None`; plus handle, label cache, iterator and cursor tests, and a proptest of random edits against a `Vec` model |
| `InsnNodeTest` | P3 | 1 | 1 | 0 | ported | plus `InsnKind` values and an `Insn` size bound |
| `IntInsnNodeTest` | P3 | 2 | 2 | 0 | ported |  |
| `InvokeDynamicInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `JumpInsnNodeTest` | P3 | 2 | 2 | 0 | ported |  |
| `LabelNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `LdcInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `LineNumberNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `LookupSwitchInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `MethodInsnNodeTest` | P3 | 3 | 3 | 0 | ported |  |
| `MethodNodeTest` | P3 / P5 | 4 | 3 | 1 | ported | `testConstructor_illegalState` dropped (ADR-09); `testVisitAndAccept_withUninitializedMethodNode` compares `ClassFile` dumps, its P3 Textifier form kept (`*_textify`); plus try catch annotation index and label flag tests |
| `ModuleNodeTest` | P3 | 3 | 2 | 1 | ported | `testConstructor_illegalState` dropped (ADR-09); plus a test of every directive |
| `MultiANewArrayInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `RecordComponentNodeTest` | P3 | 2 | 1 | 1 | ported | `testConstructor_illegalState` dropped (ADR-09) |
| `TableSwitchInsnNodeTest` | P3 | 1 | 1 | 0 | ported |  |
| `TypeAnnotationNodeTest` | P3 | 2 | 1 | 1 | ported | `testConstructor_illegalState` dropped (ADR-09) |
| `TypeInsnNodeTest` | P3 | 2 | 2 | 0 | ported |  |
| `UtilTest` | P3 | 3 | 1 | 2 | ported | `Util` is not ported; `testAsArrayList_nullArray` and `testAsArrayList_withLength` dropped (no null arrays; a chop frame carries its count); `testAsArrayList_withLengthAndArray` is `FrameNode::new` keeping one stack item |
| `VarInsnNodeTest` | P3 | 2 | 2 | 0 | ported |  |
| **total** | | **113** | **96** | **17** | | |

**oxasm-specific:** `tree_textify_test.rs`, the P3 gate on T0: reader → `ClassNode` → Textifier equals ASM's
tree path (`textifier_tree` goldens) for every T0 class and reader option.

**Test support classes** (no tests of their own):

- `CodeComment` → `tests/common/mod.rs`, a `CustomAttribute` + `AttributeCodec` (ADR-10)
- `Comment` → `tests/common/mod.rs`, a `CustomAttribute` + `AttributeCodec` (ADR-10)
