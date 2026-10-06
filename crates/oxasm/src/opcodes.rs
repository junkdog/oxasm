//! The JVM opcodes, access flags, class file versions and array type codes (ASM's `Opcodes`).
//!
//! This module does not define all the JVM opcodes because some opcodes are automatically handled.
//! For example, the xLOAD and xSTORE opcodes are automatically replaced by xLOAD_n and xSTORE_n
//! opcodes when possible. The xLOAD_n and xSTORE_n opcodes are therefore not defined here. Likewise
//! for LDC, automatically replaced by LDC_W or LDC2_W when necessary, WIDE, GOTO_W and JSR_W.
//!
//! Following ADR-09, opcodes are `u8`, access flags and class versions are `u32`, and the small
//! closed sets (the `H_*` reference kinds and the `F_*` frame types) are the enums [`HandleKind`]
//! and [`FrameKind`]. ASM's `ASMx` API versions and the `SOURCE_DEPRECATED`/`SOURCE_MASK` flags
//! are not ported: they only serve Java's binary-compatibility mechanism. The stack map frame
//! element types (`TOP`, `INTEGER`, ...) are the variants of
//! [`FrameValue`](crate::constant::FrameValue).
//!
//! See [JVMS 6](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-6.html).

use std::fmt;

// Java ClassFile versions (the minor version is stored in the 16 most significant bits, and the
// major version in the 16 least significant bits).
pub const V1_1: u32 = 3 << 16 | 45;
pub const V1_2: u32 = 46;
pub const V1_3: u32 = 47;
pub const V1_4: u32 = 48;
pub const V1_5: u32 = 49;
pub const V1_6: u32 = 50;
pub const V1_7: u32 = 51;
pub const V1_8: u32 = 52;
pub const V9: u32 = 53;
pub const V10: u32 = 54;
pub const V11: u32 = 55;
pub const V12: u32 = 56;
pub const V13: u32 = 57;
pub const V14: u32 = 58;
pub const V15: u32 = 59;
pub const V16: u32 = 60;
pub const V17: u32 = 61;
pub const V18: u32 = 62;
pub const V19: u32 = 63;
pub const V20: u32 = 64;
pub const V21: u32 = 65;
pub const V22: u32 = 66;
pub const V23: u32 = 67;
pub const V24: u32 = 68;
pub const V25: u32 = 69;
pub const V26: u32 = 70;
pub const V27: u32 = 71;

/// Version flag indicating that the class is using 'preview' features.
///
/// `version & V_PREVIEW == V_PREVIEW` tests if a version is flagged with `V_PREVIEW`.
pub const V_PREVIEW: u32 = 0xffff_0000;

// Access flags values, defined in
// - https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.1-200-E.1
// - https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.5-200-A.1
// - https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.6-200-A.1
// - https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-4.html#jvms-4.7.25

pub const ACC_PUBLIC: u32 = 0x0001; // class, field, method
pub const ACC_PRIVATE: u32 = 0x0002; // class, field, method
pub const ACC_PROTECTED: u32 = 0x0004; // class, field, method
pub const ACC_STATIC: u32 = 0x0008; // field, method
pub const ACC_FINAL: u32 = 0x0010; // class, field, method, parameter
pub const ACC_SUPER: u32 = 0x0020; // class
pub const ACC_SYNCHRONIZED: u32 = 0x0020; // method
pub const ACC_OPEN: u32 = 0x0020; // module
pub const ACC_TRANSITIVE: u32 = 0x0020; // module requires
pub const ACC_VOLATILE: u32 = 0x0040; // field
pub const ACC_BRIDGE: u32 = 0x0040; // method
pub const ACC_STATIC_PHASE: u32 = 0x0040; // module requires
pub const ACC_VARARGS: u32 = 0x0080; // method
pub const ACC_TRANSIENT: u32 = 0x0080; // field
pub const ACC_NATIVE: u32 = 0x0100; // method
pub const ACC_INTERFACE: u32 = 0x0200; // class
pub const ACC_ABSTRACT: u32 = 0x0400; // class, method
pub const ACC_STRICT: u32 = 0x0800; // method
pub const ACC_SYNTHETIC: u32 = 0x1000; // class, field, method, parameter, module *
pub const ACC_ANNOTATION: u32 = 0x2000; // class
pub const ACC_ENUM: u32 = 0x4000; // class(?) field inner
pub const ACC_MANDATED: u32 = 0x8000; // field, method, parameter, module, module *
pub const ACC_MODULE: u32 = 0x8000; // class

// ASM specific access flags.
// WARNING: the 16 least significant bits must NOT be used, to avoid conflicts with standard
// access flags, and also to make sure that these flags are automatically filtered out when
// written in class files (because access flags are stored using 16 bits only).

pub const ACC_RECORD: u32 = 0x10000; // class
pub const ACC_DEPRECATED: u32 = 0x20000; // class, field, method

// Possible values for the type operand of the NEWARRAY instruction (`visit_int_insn` operands).
// See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-6.html#jvms-6.5.newarray.

pub const T_BOOLEAN: i32 = 4;
pub const T_CHAR: i32 = 5;
pub const T_FLOAT: i32 = 6;
pub const T_DOUBLE: i32 = 7;
pub const T_BYTE: i32 = 8;
pub const T_SHORT: i32 = 9;
pub const T_INT: i32 = 10;
pub const T_LONG: i32 = 11;

// The JVM opcode values (with the MethodVisitor method name used to visit them in comment, and
// where '-' means 'same method name as on the previous line').
// See https://docs.oracle.com/javase/specs/jvms/se9/html/jvms-6.html.

pub const NOP: u8 = 0; // visitInsn
pub const ACONST_NULL: u8 = 1; // -
pub const ICONST_M1: u8 = 2; // -
pub const ICONST_0: u8 = 3; // -
pub const ICONST_1: u8 = 4; // -
pub const ICONST_2: u8 = 5; // -
pub const ICONST_3: u8 = 6; // -
pub const ICONST_4: u8 = 7; // -
pub const ICONST_5: u8 = 8; // -
pub const LCONST_0: u8 = 9; // -
pub const LCONST_1: u8 = 10; // -
pub const FCONST_0: u8 = 11; // -
pub const FCONST_1: u8 = 12; // -
pub const FCONST_2: u8 = 13; // -
pub const DCONST_0: u8 = 14; // -
pub const DCONST_1: u8 = 15; // -
pub const BIPUSH: u8 = 16; // visitIntInsn
pub const SIPUSH: u8 = 17; // -
pub const LDC: u8 = 18; // visitLdcInsn
pub const ILOAD: u8 = 21; // visitVarInsn
pub const LLOAD: u8 = 22; // -
pub const FLOAD: u8 = 23; // -
pub const DLOAD: u8 = 24; // -
pub const ALOAD: u8 = 25; // -
pub const IALOAD: u8 = 46; // visitInsn
pub const LALOAD: u8 = 47; // -
pub const FALOAD: u8 = 48; // -
pub const DALOAD: u8 = 49; // -
pub const AALOAD: u8 = 50; // -
pub const BALOAD: u8 = 51; // -
pub const CALOAD: u8 = 52; // -
pub const SALOAD: u8 = 53; // -
pub const ISTORE: u8 = 54; // visitVarInsn
pub const LSTORE: u8 = 55; // -
pub const FSTORE: u8 = 56; // -
pub const DSTORE: u8 = 57; // -
pub const ASTORE: u8 = 58; // -
pub const IASTORE: u8 = 79; // visitInsn
pub const LASTORE: u8 = 80; // -
pub const FASTORE: u8 = 81; // -
pub const DASTORE: u8 = 82; // -
pub const AASTORE: u8 = 83; // -
pub const BASTORE: u8 = 84; // -
pub const CASTORE: u8 = 85; // -
pub const SASTORE: u8 = 86; // -
pub const POP: u8 = 87; // -
pub const POP2: u8 = 88; // -
pub const DUP: u8 = 89; // -
pub const DUP_X1: u8 = 90; // -
pub const DUP_X2: u8 = 91; // -
pub const DUP2: u8 = 92; // -
pub const DUP2_X1: u8 = 93; // -
pub const DUP2_X2: u8 = 94; // -
pub const SWAP: u8 = 95; // -
pub const IADD: u8 = 96; // -
pub const LADD: u8 = 97; // -
pub const FADD: u8 = 98; // -
pub const DADD: u8 = 99; // -
pub const ISUB: u8 = 100; // -
pub const LSUB: u8 = 101; // -
pub const FSUB: u8 = 102; // -
pub const DSUB: u8 = 103; // -
pub const IMUL: u8 = 104; // -
pub const LMUL: u8 = 105; // -
pub const FMUL: u8 = 106; // -
pub const DMUL: u8 = 107; // -
pub const IDIV: u8 = 108; // -
pub const LDIV: u8 = 109; // -
pub const FDIV: u8 = 110; // -
pub const DDIV: u8 = 111; // -
pub const IREM: u8 = 112; // -
pub const LREM: u8 = 113; // -
pub const FREM: u8 = 114; // -
pub const DREM: u8 = 115; // -
pub const INEG: u8 = 116; // -
pub const LNEG: u8 = 117; // -
pub const FNEG: u8 = 118; // -
pub const DNEG: u8 = 119; // -
pub const ISHL: u8 = 120; // -
pub const LSHL: u8 = 121; // -
pub const ISHR: u8 = 122; // -
pub const LSHR: u8 = 123; // -
pub const IUSHR: u8 = 124; // -
pub const LUSHR: u8 = 125; // -
pub const IAND: u8 = 126; // -
pub const LAND: u8 = 127; // -
pub const IOR: u8 = 128; // -
pub const LOR: u8 = 129; // -
pub const IXOR: u8 = 130; // -
pub const LXOR: u8 = 131; // -
pub const IINC: u8 = 132; // visitIincInsn
pub const I2L: u8 = 133; // visitInsn
pub const I2F: u8 = 134; // -
pub const I2D: u8 = 135; // -
pub const L2I: u8 = 136; // -
pub const L2F: u8 = 137; // -
pub const L2D: u8 = 138; // -
pub const F2I: u8 = 139; // -
pub const F2L: u8 = 140; // -
pub const F2D: u8 = 141; // -
pub const D2I: u8 = 142; // -
pub const D2L: u8 = 143; // -
pub const D2F: u8 = 144; // -
pub const I2B: u8 = 145; // -
pub const I2C: u8 = 146; // -
pub const I2S: u8 = 147; // -
pub const LCMP: u8 = 148; // -
pub const FCMPL: u8 = 149; // -
pub const FCMPG: u8 = 150; // -
pub const DCMPL: u8 = 151; // -
pub const DCMPG: u8 = 152; // -
pub const IFEQ: u8 = 153; // visitJumpInsn
pub const IFNE: u8 = 154; // -
pub const IFLT: u8 = 155; // -
pub const IFGE: u8 = 156; // -
pub const IFGT: u8 = 157; // -
pub const IFLE: u8 = 158; // -
pub const IF_ICMPEQ: u8 = 159; // -
pub const IF_ICMPNE: u8 = 160; // -
pub const IF_ICMPLT: u8 = 161; // -
pub const IF_ICMPGE: u8 = 162; // -
pub const IF_ICMPGT: u8 = 163; // -
pub const IF_ICMPLE: u8 = 164; // -
pub const IF_ACMPEQ: u8 = 165; // -
pub const IF_ACMPNE: u8 = 166; // -
pub const GOTO: u8 = 167; // -
pub const JSR: u8 = 168; // -
pub const RET: u8 = 169; // visitVarInsn
pub const TABLESWITCH: u8 = 170; // visiTableSwitchInsn
pub const LOOKUPSWITCH: u8 = 171; // visitLookupSwitch
pub const IRETURN: u8 = 172; // visitInsn
pub const LRETURN: u8 = 173; // -
pub const FRETURN: u8 = 174; // -
pub const DRETURN: u8 = 175; // -
pub const ARETURN: u8 = 176; // -
pub const RETURN: u8 = 177; // -
pub const GETSTATIC: u8 = 178; // visitFieldInsn
pub const PUTSTATIC: u8 = 179; // -
pub const GETFIELD: u8 = 180; // -
pub const PUTFIELD: u8 = 181; // -
pub const INVOKEVIRTUAL: u8 = 182; // visitMethodInsn
pub const INVOKESPECIAL: u8 = 183; // -
pub const INVOKESTATIC: u8 = 184; // -
pub const INVOKEINTERFACE: u8 = 185; // -
pub const INVOKEDYNAMIC: u8 = 186; // visitInvokeDynamicInsn
pub const NEW: u8 = 187; // visitTypeInsn
pub const NEWARRAY: u8 = 188; // visitIntInsn
pub const ANEWARRAY: u8 = 189; // visitTypeInsn
pub const ARRAYLENGTH: u8 = 190; // visitInsn
pub const ATHROW: u8 = 191; // -
pub const CHECKCAST: u8 = 192; // visitTypeInsn
pub const INSTANCEOF: u8 = 193; // -
pub const MONITORENTER: u8 = 194; // visitInsn
pub const MONITOREXIT: u8 = 195; // -
pub const MULTIANEWARRAY: u8 = 197; // visitMultiANewArrayInsn
pub const IFNULL: u8 = 198; // visitJumpInsn
pub const IFNONNULL: u8 = 199; // -

/// The `reference_kind` of a `CONSTANT_MethodHandle_info` (ASM's `H_*` constants).
///
/// See [JVMS 4.4.8](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HandleKind {
    GetField = 1,
    GetStatic = 2,
    PutField = 3,
    PutStatic = 4,
    InvokeVirtual = 5,
    InvokeStatic = 6,
    InvokeSpecial = 7,
    NewInvokeSpecial = 8,
    InvokeInterface = 9,
}

impl HandleKind {
    /// Every kind, in tag order.
    pub const ALL: [HandleKind; 9] = [
        HandleKind::GetField,
        HandleKind::GetStatic,
        HandleKind::PutField,
        HandleKind::PutStatic,
        HandleKind::InvokeVirtual,
        HandleKind::InvokeStatic,
        HandleKind::InvokeSpecial,
        HandleKind::NewInvokeSpecial,
        HandleKind::InvokeInterface,
    ];

    /// The kind with the given `reference_kind` tag, if valid (1 to 9).
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<HandleKind> {
        match tag {
            1..=9 => Some(HandleKind::ALL[tag as usize - 1]),
            _ => None,
        }
    }

    /// The `reference_kind` tag (ASM's `H_*` value).
    #[inline]
    #[must_use]
    pub const fn tag(self) -> u8 {
        self as u8
    }

    /// Whether the handle refers to a field (`H_GETFIELD` to `H_PUTSTATIC`).
    #[inline]
    #[must_use]
    pub const fn is_field(self) -> bool {
        (self as u8) <= HandleKind::PutStatic as u8
    }
}

/// The ASM `H_*` name, e.g. `H_INVOKESTATIC`.
impl fmt::Display for HandleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HandleKind::GetField => "H_GETFIELD",
            HandleKind::GetStatic => "H_GETSTATIC",
            HandleKind::PutField => "H_PUTFIELD",
            HandleKind::PutStatic => "H_PUTSTATIC",
            HandleKind::InvokeVirtual => "H_INVOKEVIRTUAL",
            HandleKind::InvokeStatic => "H_INVOKESTATIC",
            HandleKind::InvokeSpecial => "H_INVOKESPECIAL",
            HandleKind::NewInvokeSpecial => "H_NEWINVOKESPECIAL",
            HandleKind::InvokeInterface => "H_INVOKEINTERFACE",
        })
    }
}

/// The type of a stack map frame passed to `MethodVisitor::visit_frame` (ASM's `F_*` constants).
///
/// Unlike Java, the number of chopped locals travels with [`FrameKind::Chop`]: the locals and
/// stack *contents* are the `visit_frame` slices, but a chop frame has no content, only a count.
// `Insert` is hidden because it is internal, not to make the enum non-exhaustive.
#[allow(clippy::manual_non_exhaustive)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// An expanded frame. See `ClassReader`'s `EXPAND_FRAMES`.
    New,
    /// A compressed frame with complete frame data.
    Full,
    /// A compressed frame where locals are the same as the locals in the previous frame, except
    /// that additional 1-3 locals are defined, and with an empty stack.
    Append,
    /// A compressed frame where locals are the same as the locals in the previous frame, except
    /// that the last 1-3 locals (the count) are absent and with an empty stack.
    Chop(u8),
    /// A compressed frame with exactly the same locals as the previous frame and with an empty
    /// stack.
    Same,
    /// A compressed frame with exactly the same locals as the previous frame and with a single
    /// value on the stack.
    Same1,
    /// ASM's internal `F_INSERT`: a frame inserted between already existing frames, whose
    /// content can be computed from the previous existing frame and from the instructions between
    /// this existing frame and the inserted one, without any knowledge of the type hierarchy. This
    /// kind of frame is only used when an unconditional jump is inserted in a method while
    /// expanding an ASM specific instruction.
    #[doc(hidden)]
    Insert,
}

impl FrameKind {
    /// ASM's `F_*` value: `F_NEW` = -1, `F_FULL` = 0, `F_APPEND` = 1, `F_CHOP` = 2, `F_SAME` = 3,
    /// `F_SAME1` = 4 (and the internal `F_INSERT` = 256).
    #[must_use]
    pub const fn to_i32(self) -> i32 {
        match self {
            FrameKind::New => -1,
            FrameKind::Full => 0,
            FrameKind::Append => 1,
            FrameKind::Chop(_) => 2,
            FrameKind::Same => 3,
            FrameKind::Same1 => 4,
            FrameKind::Insert => 256,
        }
    }
}
