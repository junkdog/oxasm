//! Port of ASM's `ConstantsTest` (asm/src/test/java/org/objectweb/asm/ConstantsTest.java), as a
//! unit test because `constants` is crate-private. Java lists the constants by reflection; here
//! they are listed explicitly (generated from `opcodes.rs` and `constants.rs`).
//!
//! Dropped (ADR-09, the `api` version mechanism is not ported): `testAsmVersions`,
//! `testIsWhitelisted`, `testCheckIsPreview_nullStream`, `testCheckIsPreview_invalidStream`,
//! `testCheckIsPreview_nonPreviewClass`, `testCheckIsPreview_previewClass`.

use std::collections::HashSet;

use crate::constants::*;
use crate::opcodes::*;

const CLASS_VERSIONS: &[(&str, u32)] = &[
    ("V1_1", V1_1),
    ("V1_2", V1_2),
    ("V1_3", V1_3),
    ("V1_4", V1_4),
    ("V1_5", V1_5),
    ("V1_6", V1_6),
    ("V1_7", V1_7),
    ("V1_8", V1_8),
    ("V9", V9),
    ("V10", V10),
    ("V11", V11),
    ("V12", V12),
    ("V13", V13),
    ("V14", V14),
    ("V15", V15),
    ("V16", V16),
    ("V17", V17),
    ("V18", V18),
    ("V19", V19),
    ("V20", V20),
    ("V21", V21),
    ("V22", V22),
    ("V23", V23),
    ("V24", V24),
    ("V25", V25),
    ("V26", V26),
    ("V27", V27),
    ("V_PREVIEW", V_PREVIEW),
];

const ACCESS_FLAGS: &[(&str, u32)] = &[
    ("ACC_PUBLIC", ACC_PUBLIC),
    ("ACC_PRIVATE", ACC_PRIVATE),
    ("ACC_PROTECTED", ACC_PROTECTED),
    ("ACC_STATIC", ACC_STATIC),
    ("ACC_FINAL", ACC_FINAL),
    ("ACC_SUPER", ACC_SUPER),
    ("ACC_SYNCHRONIZED", ACC_SYNCHRONIZED),
    ("ACC_OPEN", ACC_OPEN),
    ("ACC_TRANSITIVE", ACC_TRANSITIVE),
    ("ACC_VOLATILE", ACC_VOLATILE),
    ("ACC_BRIDGE", ACC_BRIDGE),
    ("ACC_STATIC_PHASE", ACC_STATIC_PHASE),
    ("ACC_VARARGS", ACC_VARARGS),
    ("ACC_TRANSIENT", ACC_TRANSIENT),
    ("ACC_NATIVE", ACC_NATIVE),
    ("ACC_INTERFACE", ACC_INTERFACE),
    ("ACC_ABSTRACT", ACC_ABSTRACT),
    ("ACC_STRICT", ACC_STRICT),
    ("ACC_SYNTHETIC", ACC_SYNTHETIC),
    ("ACC_ANNOTATION", ACC_ANNOTATION),
    ("ACC_ENUM", ACC_ENUM),
    ("ACC_MANDATED", ACC_MANDATED),
    ("ACC_MODULE", ACC_MODULE),
];

const ASM_ACCESS_FLAGS: &[(&str, u32)] = &[
    ("ACC_RECORD", ACC_RECORD),
    ("ACC_DEPRECATED", ACC_DEPRECATED),
    ("ACC_CONSTRUCTOR", ACC_CONSTRUCTOR),
];

const NEW_ARRAY_TYPES: &[(&str, i32)] = &[
    ("T_BOOLEAN", T_BOOLEAN),
    ("T_CHAR", T_CHAR),
    ("T_FLOAT", T_FLOAT),
    ("T_DOUBLE", T_DOUBLE),
    ("T_BYTE", T_BYTE),
    ("T_SHORT", T_SHORT),
    ("T_INT", T_INT),
    ("T_LONG", T_LONG),
];

const VERIFICATION_TYPE_INFO_TAGS: &[(&str, u8)] = &[
    ("ITEM_TOP", ITEM_TOP),
    ("ITEM_INTEGER", ITEM_INTEGER),
    ("ITEM_FLOAT", ITEM_FLOAT),
    ("ITEM_DOUBLE", ITEM_DOUBLE),
    ("ITEM_LONG", ITEM_LONG),
    ("ITEM_NULL", ITEM_NULL),
    ("ITEM_UNINITIALIZED_THIS", ITEM_UNINITIALIZED_THIS),
    ("ITEM_OBJECT", ITEM_OBJECT),
    ("ITEM_UNINITIALIZED", ITEM_UNINITIALIZED),
];

const OPCODES: &[(&str, u8)] = &[
    ("NOP", NOP),
    ("ACONST_NULL", ACONST_NULL),
    ("ICONST_M1", ICONST_M1),
    ("ICONST_0", ICONST_0),
    ("ICONST_1", ICONST_1),
    ("ICONST_2", ICONST_2),
    ("ICONST_3", ICONST_3),
    ("ICONST_4", ICONST_4),
    ("ICONST_5", ICONST_5),
    ("LCONST_0", LCONST_0),
    ("LCONST_1", LCONST_1),
    ("FCONST_0", FCONST_0),
    ("FCONST_1", FCONST_1),
    ("FCONST_2", FCONST_2),
    ("DCONST_0", DCONST_0),
    ("DCONST_1", DCONST_1),
    ("BIPUSH", BIPUSH),
    ("SIPUSH", SIPUSH),
    ("LDC", LDC),
    ("ILOAD", ILOAD),
    ("LLOAD", LLOAD),
    ("FLOAD", FLOAD),
    ("DLOAD", DLOAD),
    ("ALOAD", ALOAD),
    ("IALOAD", IALOAD),
    ("LALOAD", LALOAD),
    ("FALOAD", FALOAD),
    ("DALOAD", DALOAD),
    ("AALOAD", AALOAD),
    ("BALOAD", BALOAD),
    ("CALOAD", CALOAD),
    ("SALOAD", SALOAD),
    ("ISTORE", ISTORE),
    ("LSTORE", LSTORE),
    ("FSTORE", FSTORE),
    ("DSTORE", DSTORE),
    ("ASTORE", ASTORE),
    ("IASTORE", IASTORE),
    ("LASTORE", LASTORE),
    ("FASTORE", FASTORE),
    ("DASTORE", DASTORE),
    ("AASTORE", AASTORE),
    ("BASTORE", BASTORE),
    ("CASTORE", CASTORE),
    ("SASTORE", SASTORE),
    ("POP", POP),
    ("POP2", POP2),
    ("DUP", DUP),
    ("DUP_X1", DUP_X1),
    ("DUP_X2", DUP_X2),
    ("DUP2", DUP2),
    ("DUP2_X1", DUP2_X1),
    ("DUP2_X2", DUP2_X2),
    ("SWAP", SWAP),
    ("IADD", IADD),
    ("LADD", LADD),
    ("FADD", FADD),
    ("DADD", DADD),
    ("ISUB", ISUB),
    ("LSUB", LSUB),
    ("FSUB", FSUB),
    ("DSUB", DSUB),
    ("IMUL", IMUL),
    ("LMUL", LMUL),
    ("FMUL", FMUL),
    ("DMUL", DMUL),
    ("IDIV", IDIV),
    ("LDIV", LDIV),
    ("FDIV", FDIV),
    ("DDIV", DDIV),
    ("IREM", IREM),
    ("LREM", LREM),
    ("FREM", FREM),
    ("DREM", DREM),
    ("INEG", INEG),
    ("LNEG", LNEG),
    ("FNEG", FNEG),
    ("DNEG", DNEG),
    ("ISHL", ISHL),
    ("LSHL", LSHL),
    ("ISHR", ISHR),
    ("LSHR", LSHR),
    ("IUSHR", IUSHR),
    ("LUSHR", LUSHR),
    ("IAND", IAND),
    ("LAND", LAND),
    ("IOR", IOR),
    ("LOR", LOR),
    ("IXOR", IXOR),
    ("LXOR", LXOR),
    ("IINC", IINC),
    ("I2L", I2L),
    ("I2F", I2F),
    ("I2D", I2D),
    ("L2I", L2I),
    ("L2F", L2F),
    ("L2D", L2D),
    ("F2I", F2I),
    ("F2L", F2L),
    ("F2D", F2D),
    ("D2I", D2I),
    ("D2L", D2L),
    ("D2F", D2F),
    ("I2B", I2B),
    ("I2C", I2C),
    ("I2S", I2S),
    ("LCMP", LCMP),
    ("FCMPL", FCMPL),
    ("FCMPG", FCMPG),
    ("DCMPL", DCMPL),
    ("DCMPG", DCMPG),
    ("IFEQ", IFEQ),
    ("IFNE", IFNE),
    ("IFLT", IFLT),
    ("IFGE", IFGE),
    ("IFGT", IFGT),
    ("IFLE", IFLE),
    ("IF_ICMPEQ", IF_ICMPEQ),
    ("IF_ICMPNE", IF_ICMPNE),
    ("IF_ICMPLT", IF_ICMPLT),
    ("IF_ICMPGE", IF_ICMPGE),
    ("IF_ICMPGT", IF_ICMPGT),
    ("IF_ICMPLE", IF_ICMPLE),
    ("IF_ACMPEQ", IF_ACMPEQ),
    ("IF_ACMPNE", IF_ACMPNE),
    ("GOTO", GOTO),
    ("JSR", JSR),
    ("RET", RET),
    ("TABLESWITCH", TABLESWITCH),
    ("LOOKUPSWITCH", LOOKUPSWITCH),
    ("IRETURN", IRETURN),
    ("LRETURN", LRETURN),
    ("FRETURN", FRETURN),
    ("DRETURN", DRETURN),
    ("ARETURN", ARETURN),
    ("RETURN", RETURN),
    ("GETSTATIC", GETSTATIC),
    ("PUTSTATIC", PUTSTATIC),
    ("GETFIELD", GETFIELD),
    ("PUTFIELD", PUTFIELD),
    ("INVOKEVIRTUAL", INVOKEVIRTUAL),
    ("INVOKESPECIAL", INVOKESPECIAL),
    ("INVOKESTATIC", INVOKESTATIC),
    ("INVOKEINTERFACE", INVOKEINTERFACE),
    ("INVOKEDYNAMIC", INVOKEDYNAMIC),
    ("NEW", NEW),
    ("NEWARRAY", NEWARRAY),
    ("ANEWARRAY", ANEWARRAY),
    ("ARRAYLENGTH", ARRAYLENGTH),
    ("ATHROW", ATHROW),
    ("CHECKCAST", CHECKCAST),
    ("INSTANCEOF", INSTANCEOF),
    ("MONITORENTER", MONITORENTER),
    ("MONITOREXIT", MONITOREXIT),
    ("MULTIANEWARRAY", MULTIANEWARRAY),
    ("IFNULL", IFNULL),
    ("IFNONNULL", IFNONNULL),
    ("LDC_W", LDC_W),
    ("LDC2_W", LDC2_W),
    ("ILOAD_0", ILOAD_0),
    ("ILOAD_1", ILOAD_1),
    ("ILOAD_2", ILOAD_2),
    ("ILOAD_3", ILOAD_3),
    ("LLOAD_0", LLOAD_0),
    ("LLOAD_1", LLOAD_1),
    ("LLOAD_2", LLOAD_2),
    ("LLOAD_3", LLOAD_3),
    ("FLOAD_0", FLOAD_0),
    ("FLOAD_1", FLOAD_1),
    ("FLOAD_2", FLOAD_2),
    ("FLOAD_3", FLOAD_3),
    ("DLOAD_0", DLOAD_0),
    ("DLOAD_1", DLOAD_1),
    ("DLOAD_2", DLOAD_2),
    ("DLOAD_3", DLOAD_3),
    ("ALOAD_0", ALOAD_0),
    ("ALOAD_1", ALOAD_1),
    ("ALOAD_2", ALOAD_2),
    ("ALOAD_3", ALOAD_3),
    ("ISTORE_0", ISTORE_0),
    ("ISTORE_1", ISTORE_1),
    ("ISTORE_2", ISTORE_2),
    ("ISTORE_3", ISTORE_3),
    ("LSTORE_0", LSTORE_0),
    ("LSTORE_1", LSTORE_1),
    ("LSTORE_2", LSTORE_2),
    ("LSTORE_3", LSTORE_3),
    ("FSTORE_0", FSTORE_0),
    ("FSTORE_1", FSTORE_1),
    ("FSTORE_2", FSTORE_2),
    ("FSTORE_3", FSTORE_3),
    ("DSTORE_0", DSTORE_0),
    ("DSTORE_1", DSTORE_1),
    ("DSTORE_2", DSTORE_2),
    ("DSTORE_3", DSTORE_3),
    ("ASTORE_0", ASTORE_0),
    ("ASTORE_1", ASTORE_1),
    ("ASTORE_2", ASTORE_2),
    ("ASTORE_3", ASTORE_3),
    ("WIDE", WIDE),
    ("GOTO_W", GOTO_W),
    ("JSR_W", JSR_W),
    ("ASM_IFEQ", ASM_IFEQ),
    ("ASM_IFNE", ASM_IFNE),
    ("ASM_IFLT", ASM_IFLT),
    ("ASM_IFGE", ASM_IFGE),
    ("ASM_IFGT", ASM_IFGT),
    ("ASM_IFLE", ASM_IFLE),
    ("ASM_IF_ICMPEQ", ASM_IF_ICMPEQ),
    ("ASM_IF_ICMPNE", ASM_IF_ICMPNE),
    ("ASM_IF_ICMPLT", ASM_IF_ICMPLT),
    ("ASM_IF_ICMPGE", ASM_IF_ICMPGE),
    ("ASM_IF_ICMPGT", ASM_IF_ICMPGT),
    ("ASM_IF_ICMPLE", ASM_IF_ICMPLE),
    ("ASM_IF_ACMPEQ", ASM_IF_ACMPEQ),
    ("ASM_IF_ACMPNE", ASM_IF_ACMPNE),
    ("ASM_GOTO", ASM_GOTO),
    ("ASM_JSR", ASM_JSR),
    ("ASM_IFNULL", ASM_IFNULL),
    ("ASM_IFNONNULL", ASM_IFNONNULL),
    ("ASM_GOTO_W", ASM_GOTO_W),
];

fn assert_distinct<T: Copy + Eq + std::hash::Hash + std::fmt::Debug>(constants: &[(&str, T)]) {
    let values: HashSet<T> = constants.iter().map(|&(_, v)| v).collect();
    assert_eq!(
        constants.len(),
        values.len(),
        "duplicate values in {constants:?}"
    );
}

#[test]
fn test_class_versions() {
    assert_distinct(CLASS_VERSIONS);
}

#[test]
fn test_access_flags() {
    for &(name, flag) in ACCESS_FLAGS {
        assert_eq!(0, flag & !0xffff, "{name}");
        assert_eq!(1, flag.count_ones(), "{name}");
    }
}

#[test]
fn test_asm_access_flags() {
    for &(name, flag) in ASM_ACCESS_FLAGS {
        assert_eq!(0, flag & 0xffff, "{name}");
        assert_eq!(1, flag.count_ones(), "{name}");
    }
}

#[test]
fn test_new_array_types() {
    assert_distinct(NEW_ARRAY_TYPES);
    for &(name, t) in NEW_ARRAY_TYPES {
        assert_eq!(0, t & !0xff, "{name}");
    }
}

#[test]
fn test_reference_kinds() {
    // `HandleKind` is a `repr(u8)` enum, so every kind fits a byte; check distinctness.
    let tags: HashSet<u8> = HandleKind::ALL.iter().map(|k| k.tag()).collect();
    assert_eq!(HandleKind::ALL.len(), tags.len());
}

#[test]
fn test_frame_types() {
    let kinds = [
        FrameKind::New,
        FrameKind::Full,
        FrameKind::Append,
        FrameKind::Chop(1),
        FrameKind::Same,
        FrameKind::Same1,
        FrameKind::Insert,
    ];
    let values: HashSet<i32> = kinds.iter().map(|k| k.to_i32()).collect();
    assert_eq!(kinds.len(), values.len());
}

#[test]
fn test_verification_type_info_tags() {
    // Opcodes.TOP ... UNINITIALIZED_THIS are the `FrameValue` variants; their tags are these.
    assert_distinct(VERIFICATION_TYPE_INFO_TAGS);
}

#[test]
fn test_opcodes() {
    // Fitting in a byte is the `u8` type; ASM's SOURCE_MASK check has no analogue (ADR-09).
    assert_distinct(OPCODES);
}
