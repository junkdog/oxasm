//! The T3 synthetic classes (oxasm-verification, "Corpus tiers"): classes that only a class
//! writer produces (jumps too far for a 2 bytes offset, a method or a constant pool too large),
//! generated here with the same calls as the oracle's `gen` command (`oracle/Oracle.java`), and
//! compared with what ASM wrote, committed under `crates/oxasm-testkit/goldens/t3/`
//! (`cargo xtask oracle gen <scenario>` regenerates them).

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, FrameKind, FrameValue,
    JavaStr, Label, MethodVisitor, ParsingOptions, WriteError, jstr,
};

/// The number of NOPs that makes a jump over them too far for a 2 bytes offset.
const FAR: usize = 40000;

/// The golden of a scenario: the class file ASM wrote, or the exception it threw.
fn golden(scenario: &str) -> Result<Vec<u8>, String> {
    let dir = oxasm_testkit::workspace_root().join("crates/oxasm-testkit/goldens/t3");
    match std::fs::read(dir.join(format!("{scenario}.class"))) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Err(std::fs::read_to_string(dir.join(format!("{scenario}.err")))
            .unwrap_or_else(|e| panic!("no golden for {scenario}: {e}"))),
    }
}

fn new_class(version: u32, name: &str) -> ClassWriter<'static> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        version,
        ACC_PUBLIC,
        JavaStr::from_str(name),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    class_writer
}

/// `static m(I)V`: IFEQ and GOTO forward over FAR NOPs (with frames if requested).
fn jump_forward(version: u32, frames: bool) -> Result<Vec<u8>, WriteError> {
    let name = if version == V1_5 {
        "t3/JumpForward5"
    } else {
        "t3/JumpForward8"
    };
    let mut class_writer = new_class(version, name);
    {
        let mut mv = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("(I)V"), None, &[])
            .unwrap();
        mv.visit_code();
        let (target, end) = (Label::new(), Label::new());
        mv.visit_var_insn(ILOAD, 0);
        mv.visit_jump_insn(IFEQ, target);
        mv.visit_jump_insn(GOTO, end);
        mv.visit_label(target);
        if frames {
            mv.visit_frame(FrameKind::Same, &[], &[]);
        }
        for _ in 0..FAR {
            mv.visit_insn(NOP);
        }
        mv.visit_label(end);
        if frames {
            mv.visit_frame(FrameKind::Same, &[], &[]);
        }
        mv.visit_insn(RETURN);
        mv.visit_maxs(1, 1);
        mv.visit_end();
    }
    class_writer.to_bytes()
}

/// `static m(I)V`: IFNE and GOTO backward over FAR NOPs (GOTO_W, and IFEQ + GOTO_W).
fn jump_backward() -> Result<Vec<u8>, WriteError> {
    let mut class_writer = new_class(V1_5, "t3/JumpBackward5");
    {
        let mut mv = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("(I)V"), None, &[])
            .unwrap();
        mv.visit_code();
        let (start, end) = (Label::new(), Label::new());
        mv.visit_label(start);
        mv.visit_var_insn(ILOAD, 0);
        mv.visit_jump_insn(IFLT, end);
        for _ in 0..FAR {
            mv.visit_insn(NOP);
        }
        mv.visit_iinc_insn(0, -1);
        mv.visit_var_insn(ILOAD, 0);
        mv.visit_jump_insn(IFNE, start);
        mv.visit_jump_insn(GOTO, start);
        mv.visit_label(end);
        mv.visit_insn(RETURN);
        mv.visit_maxs(1, 1);
        mv.visit_end();
    }
    class_writer.to_bytes()
}

/// `m(ILjava/lang/String;)J`, V1_8: a forward IFNULL over FAR bytes of instructions that change
/// the locals and the stack, then expanded frames, so that the inserted frames of the re-read
/// are computed by simulating these instructions.
fn jump_forward_locals() -> Result<Vec<u8>, WriteError> {
    let mut class_writer = new_class(V1_8, "t3/JumpForwardLocals8");
    {
        let mut mv = class_writer
            .visit_method(0, jstr!("m"), jstr!("(ILjava/lang/String;)J"), None, &[])
            .unwrap();
        mv.visit_code();
        let target = Label::new();
        mv.visit_insn(LCONST_1);
        mv.visit_var_insn(LSTORE, 3);
        mv.visit_type_insn(NEW, jstr!("java/lang/StringBuilder"));
        mv.visit_insn(DUP);
        mv.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/StringBuilder"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        mv.visit_var_insn(ASTORE, 5);
        mv.visit_var_insn(ALOAD, 2);
        mv.visit_jump_insn(IFNULL, target);
        for _ in 0..FAR / 8 {
            mv.visit_var_insn(ALOAD, 5);
            mv.visit_var_insn(ALOAD, 2);
            mv.visit_method_insn(
                INVOKEVIRTUAL,
                jstr!("java/lang/StringBuilder"),
                jstr!("append"),
                jstr!("(Ljava/lang/String;)Ljava/lang/StringBuilder;"),
                false,
            );
            mv.visit_insn(POP);
        }
        mv.visit_var_insn(LLOAD, 3);
        mv.visit_insn(L2I);
        mv.visit_var_insn(ISTORE, 1);
        mv.visit_label(target);
        mv.visit_frame(
            FrameKind::New,
            &[
                FrameValue::Object(jstr!("t3/JumpForwardLocals8").into()),
                FrameValue::Integer,
                FrameValue::Object(jstr!("java/lang/String").into()),
                FrameValue::Long,
                FrameValue::Object(jstr!("java/lang/StringBuilder").into()),
            ],
            &[],
        );
        mv.visit_var_insn(LLOAD, 3);
        mv.visit_insn(LRETURN);
        mv.visit_maxs(3, 6);
        mv.visit_end();
    }
    class_writer.to_bytes()
}

fn method_too_large() -> Result<Vec<u8>, WriteError> {
    let mut class_writer = new_class(V1_8, "t3/MethodTooLarge");
    {
        let mut mv = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        mv.visit_code();
        for _ in 0..70000 {
            mv.visit_insn(NOP);
        }
        mv.visit_insn(RETURN);
        mv.visit_maxs(0, 0);
        mv.visit_end();
    }
    class_writer.to_bytes()
}

fn class_too_large() -> Result<Vec<u8>, WriteError> {
    let mut class_writer = new_class(V1_8, "t3/ClassTooLarge");
    for i in 0..65535 {
        class_writer.new_const(&Constant::Integer(i));
    }
    class_writer.to_bytes()
}

fn pool_65535() -> Result<Vec<u8>, WriteError> {
    let mut class_writer = new_class(V1_8, "t3/Pool65535");
    let mut i = 0;
    while class_writer.new_const(&Constant::Integer(i)) < 65534 {
        i += 1;
    }
    class_writer.to_bytes()
}

fn generate(scenario: &str) -> Result<Vec<u8>, WriteError> {
    match scenario {
        "jump-forward-v5" => jump_forward(V1_5, false),
        "jump-forward-v8" => jump_forward(V1_8, true),
        "jump-backward-v5" => jump_backward(),
        "jump-forward-locals-v8" => jump_forward_locals(),
        "method-too-large" => method_too_large(),
        "class-too-large" => class_too_large(),
        "pool-65535" => pool_65535(),
        _ => panic!("unknown scenario {scenario}"),
    }
}

/// The scenarios that ASM writes successfully.
const WRITTEN: [&str; 5] = [
    "jump-forward-v5",
    "jump-forward-v8",
    "jump-backward-v5",
    "jump-forward-locals-v8",
    "pool-65535",
];

#[test]
fn written_classes_equal_asm_s() {
    for scenario in WRITTEN {
        assert_written_class_equals_asm_s(scenario);
    }
}

fn assert_written_class_equals_asm_s(scenario: &str) {
    let written = generate(scenario).unwrap();
    let expected = golden(scenario).unwrap();
    assert!(
        written == expected,
        "{scenario}: the class differs from ASM's"
    );
    // The class can be read back.
    let reader = ClassReader::new(&written).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    reader
        .accept(&mut class_writer, ParsingOptions::empty())
        .unwrap();
    assert!(class_writer.to_bytes().is_ok());
}

#[test]
fn too_large_fails_as_asm() {
    for scenario in ["method-too-large", "class-too-large"] {
        assert_fails_as_asm(scenario);
    }
}

fn assert_fails_as_asm(scenario: &str) {
    let error = generate(scenario).unwrap_err();
    let expected = golden(scenario).unwrap_err();
    let class_name = match &error {
        WriteError::MethodTooLarge { .. } => "org.objectweb.asm.MethodTooLargeException",
        WriteError::ClassTooLarge { .. } => "org.objectweb.asm.ClassTooLargeException",
        _ => panic!("unexpected error {error:?}"),
    };
    assert_eq!(format!("{class_name}: {error}"), expected);
}

#[test]
fn method_too_large_error_details() {
    let Err(WriteError::MethodTooLarge {
        class_name,
        method_name,
        descriptor,
        code_size,
    }) = method_too_large()
    else {
        panic!("expected MethodTooLarge");
    };
    assert_eq!(class_name, "t3/MethodTooLarge");
    assert_eq!(method_name, "m");
    assert_eq!(descriptor, "()V");
    assert_eq!(code_size, 70001);
}

#[test]
#[ignore = "needs the JVM oracle"]
fn written_classes_are_valid() {
    for scenario in WRITTEN {
        oxasm_testkit::jvm::verify_class(&generate(scenario).unwrap())
            .unwrap_or_else(|e| panic!("{scenario}: {e}"));
    }
}
