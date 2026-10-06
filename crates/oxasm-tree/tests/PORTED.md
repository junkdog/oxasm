# Ported tests: `asm-tree` → `oxasm-tree`

The test-porting ledger for this crate (oxasm-verification.md, "Test porting conventions"). One row per JUnit
test class of ASM at tag `ASM_9_10_1` (`asm-tree/src/test/java`). A phase is done only when none of its rows is
*pending*. Each class ports to `tests/<snake_case>.rs`, with the same test names in `snake_case`.

- **Java tests:** `@Test`/`@ParameterizedTest` methods in the Java class.
- **Ported / Dropped:** Java tests covered by Rust tests / deliberately not ported (each drop names its reason
  in a comment in the Rust file, e.g. ADR-09 for `api`-version tests).
- **Status:** *pending*, *partial* or *ported*.

| Java test class | Phase | Java tests | Ported | Dropped | Status | Notes |
|---|---|---:|---:|---:|---|---|
| `AnnotationNodeTest` | P3 | 4 | 0 | 0 | pending |  |
| `ClassNodeTest` | P3 | 6 | 0 | 0 | pending |  |
| `FieldInsnNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `FieldNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `FrameNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `IincInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `InsnListTest` | P3 | 63 | 0 | 0 | pending |  |
| `InsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `IntInsnNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `InvokeDynamicInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `JumpInsnNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `LabelNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `LdcInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `LineNumberNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `LookupSwitchInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `MethodInsnNodeTest` | P3 | 3 | 0 | 0 | pending |  |
| `MethodNodeTest` | P3 | 4 | 0 | 0 | pending |  |
| `ModuleNodeTest` | P3 | 3 | 0 | 0 | pending |  |
| `MultiANewArrayInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `RecordComponentNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `TableSwitchInsnNodeTest` | P3 | 1 | 0 | 0 | pending |  |
| `TypeAnnotationNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `TypeInsnNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| `UtilTest` | P3 | 3 | 0 | 0 | pending |  |
| `VarInsnNodeTest` | P3 | 2 | 0 | 0 | pending |  |
| **total** | | **113** | **0** | **0** | | |

**Test support classes** (no tests of their own):

- `CodeComment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
- `Comment` → `oxasm-testkit` `CustomAttribute` impl (ADR-10)
