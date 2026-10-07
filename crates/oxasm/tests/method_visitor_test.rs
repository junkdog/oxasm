//! Port of ASM's `MethodVisitorTest`.
//!
//! oxasm has no api versions (ADR-09), so the tests of what an ASM4 visitor rejects
//! (`UnsupportedOperationException`) and of the deprecated 4-argument `visitMethodInsn` are
//! dropped. The other `visitMethodInsn` chains keep their ASM4/ASM5 names, but the "ASM4"
//! visitors are ordinary visitors: overriding `visit_method_insn` is the only way to intercept
//! the instruction, so these tests check that overrides and delegation compose as in ASM.

use std::cell::RefCell;
use std::ptr;

use oxasm::error::WriteError;
use oxasm::opcodes::{ACC_PUBLIC, ACC_STATIC, V1_7};
use oxasm::{
    ClassVisitor, ClassWriter, ClassWriterFlags, FrameKind, FrameValue, JavaStr, MethodVisitor,
    jstr,
};

/// A method visitor with no delegate.
struct NoDelegate;

impl MethodVisitor for NoDelegate {}

/// A method visitor that delegates to another.
struct Delegating<'a>(&'a mut dyn MethodVisitor);

impl MethodVisitor for Delegating<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }
}

#[test]
fn test_constructor_valid_api() {
    // A visitor without a delegate accepts every event, and is not interested in any content.
    let mut visitor = NoDelegate;
    assert!(visitor.delegate().is_none());
    visitor.visit_code();
    visitor.visit_insn(oxasm::opcodes::NOP);
    assert!(visitor.visit_annotation_default().is_none());
    visitor.visit_maxs(0, 0);
    visitor.visit_end();
}

// testConstructor_invalidApi: dropped (ADR-09, there are no api versions).

#[test]
fn test_get_delegate() {
    let mut delegate = NoDelegate;
    let delegate_address = ptr::from_mut(&mut delegate).cast::<()>();
    let mut visitor = Delegating(&mut delegate);
    let address = visitor
        .delegate()
        .map(|d| ptr::from_mut(d).cast::<()>())
        .expect("a delegate");
    assert_eq!(address, delegate_address);
}

// testVisitParameter_asm4Visitor, testVisitTypeAnnotation_asm4Visitor,
// testVisitInvokeDynamicInsn_asm4Visitor, testVisitInsnAnnotation_asm4Visitor,
// testVisitTryCatchAnnotation_asm4Visitor and testVisitLocalVariableAnnotation_asm4Visitor:
// dropped (ADR-09, there are no api versions).

/// A class writer with a class `C` (of the given version, if any) and a static method `m`,
/// whose method visitor is passed to `visit`; returns the result of `to_bytes`.
fn with_method_writer(
    version: Option<u32>,
    visit: impl FnOnce(&mut dyn MethodVisitor),
) -> Result<Vec<u8>, WriteError> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    if let Some(version) = version {
        class_writer.visit(version, ACC_PUBLIC, jstr!("C"), None, Some(jstr!("D")), &[]);
    }
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("()V"), None, &[])
            .expect("a method writer");
        visit(&mut *method_visitor);
    }
    class_writer.to_bytes()
}

#[test]
fn test_visit_frame_consecutive_frames_same_frame() {
    let result = with_method_writer(Some(V1_7), |method_visitor| {
        method_visitor.visit_code();
        method_visitor.visit_frame(FrameKind::Same, &[], &[]);
        method_visitor.visit_frame(FrameKind::Same, &[], &[]);
    });
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn test_visit_frame_consecutive_frames() {
    let result = with_method_writer(Some(V1_7), |method_visitor| {
        method_visitor.visit_code();
        method_visitor.visit_frame(FrameKind::Same, &[], &[]);
        method_visitor.visit_frame(FrameKind::Append, &[FrameValue::Integer], &[]);
    });
    assert!(
        matches!(result, Err(WriteError::IllegalState(_))),
        "{result:?}"
    );
}

#[test]
fn test_visit_frame_compressed_frame_with_v1_5_class() {
    // As in ASM, the method writer comes from a class writer whose class was not visited (whose
    // version is therefore less than V1_6).
    let result = with_method_writer(None, |method_visitor| {
        method_visitor.visit_code();
        method_visitor.visit_frame(FrameKind::Same, &[], &[]);
    });
    match result {
        Err(WriteError::InvalidArgument(message)) => {
            assert!(message.contains("versions V1_5 or less must use F_NEW frames."));
        }
        other => panic!("expected an invalid argument error, got {other:?}"),
    }
}

// testDeprecatedVisitMethodInsn_asm4Visitor, testDeprecatedVisitMethodInsn_overridenAsm4Visitor,
// testDeprecatedVisitMethodInsn_asm5Visitor, testDeprecatedVisitMethodInsn_overridenAsm5Visitor,
// testDeprecatedVisitMethodInsn_userTraceMethodVisitor4,
// testDeprecatedVisitMethodInsn_userTraceMethodVisitor5 and
// testDeprecatedVisitMethodInsn_mixedVisitorChain: dropped (ADR-09, the deprecated ASM4
// visitMethodInsn without isInterface is not ported).

/// Tests visitMethodInsn on an "ASM4" visitor, with isInterface set to false.
#[test]
fn test_visit_method_insn_asm4_visitor_is_not_interface() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = MethodVisitor4(&mut log_method_visitor);
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(*log.borrow(), "LogMethodVisitor:m()V;");
}

// testVisitMethodInsn_asm4Visitor_isInterface: dropped (ADR-09, an ASM4 visitor rejecting
// isInterface).

/// Tests visitMethodInsn on an "ASM4" visitor which overrides it, with isInterface set to false.
#[test]
fn test_visit_method_insn_overriden_asm4_visitor_is_not_interface() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = MethodVisitorOverride(&mut log_method_visitor, &log, "4");
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(*log.borrow(), "LogMethodVisitor:m4()V;MethodVisitor4:m()V;");
}

// testVisitMethodInsn_overridenAsm4Visitor_isInterface: dropped (ADR-09, an ASM4 visitor
// rejecting isInterface).

/// Tests visitMethodInsn on an ASM5 visitor.
#[test]
fn test_visit_method_insn_asm5_visitor() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = MethodVisitor5(&mut log_method_visitor);
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(*log.borrow(), "LogMethodVisitor:m()V;");
}

/// Tests visitMethodInsn on an ASM5 visitor which overrides this method.
#[test]
fn test_visit_method_insn_overriden_asm5_visitor() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = MethodVisitorOverride(&mut log_method_visitor, &log, "5");
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(*log.borrow(), "LogMethodVisitor:m5()V;MethodVisitor5:m()V;");
}

/// Tests visitMethodInsn on an "ASM4" visitor which overrides this method, and is a subclass of
/// an ASM subclass of MethodVisitor (here: wraps a `TraceMethodVisitor`).
#[test]
fn test_visit_method_insn_user_trace_method_visitor4() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = UserTraceMethodVisitor(
        TraceMethodVisitor(&mut log_method_visitor, &log),
        &log,
        "UserTraceMethodVisitor4",
    );
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(
        *log.borrow(),
        "LogMethodVisitor:m()V;TraceMethodVisitor:m()V;UserTraceMethodVisitor4:m()V;"
    );
}

/// Tests visitMethodInsn on an ASM5 visitor which overrides this method, and is a subclass of an
/// ASM subclass of MethodVisitor (here: wraps a `TraceMethodVisitor`).
#[test]
fn test_visit_method_insn_user_trace_method_visitor5() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut method_visitor = UserTraceMethodVisitor(
        TraceMethodVisitor(&mut log_method_visitor, &log),
        &log,
        "UserTraceMethodVisitor5",
    );
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(
        *log.borrow(),
        "LogMethodVisitor:m()V;TraceMethodVisitor:m()V;UserTraceMethodVisitor5:m()V;"
    );
}

/// Tests that method visitors can be chained together and produce the expected result, when
/// calling visitMethodInsn on the first visitor.
#[test]
fn test_visit_method_insn_mixed_visitor_chain() {
    let log = RefCell::new(String::new());
    let mut log_method_visitor = LogMethodVisitor(&log);
    let mut visitor4 = MethodVisitor4(&mut log_method_visitor);
    let mut visitor4_override = MethodVisitorOverride(&mut visitor4, &log, "4");
    let mut visitor5 = MethodVisitor5(&mut visitor4_override);
    let mut method_visitor = MethodVisitorOverride(&mut visitor5, &log, "5");
    method_visitor.visit_method_insn(0, jstr!("C"), jstr!("m"), jstr!("()V"), false);
    assert_eq!(
        *log.borrow(),
        "LogMethodVisitor:m54()V;MethodVisitor4:m5()V;MethodVisitor5:m()V;"
    );
}

/// An "ASM4" method visitor which does not override visitMethodInsn.
struct MethodVisitor4<'a>(&'a mut dyn MethodVisitor);

impl MethodVisitor for MethodVisitor4<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }
}

/// An ASM5 method visitor which does not override visitMethodInsn.
struct MethodVisitor5<'a>(&'a mut dyn MethodVisitor);

impl MethodVisitor for MethodVisitor5<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }
}

/// A method visitor which overrides visitMethodInsn, by adding a suffix at the end of method
/// names (ASM's `MethodVisitor4Override` with "4", `MethodVisitor5Override` with "5").
struct MethodVisitorOverride<'a>(&'a mut dyn MethodVisitor, &'a RefCell<String>, &'static str);

impl MethodVisitor for MethodVisitorOverride<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        let mut new_name = name.to_owned();
        new_name.push_str(self.2);
        self.0
            .visit_method_insn(opcode, owner, &new_name, descriptor, is_interface);
        self.1
            .borrow_mut()
            .push_str(&format!("MethodVisitor{}:{name}{descriptor};", self.2));
    }
}

/// An ASM-like method visitor which overrides visitMethodInsn (ASM's `TraceMethodVisitor`, here
/// without the redirection to the deprecated ASM4 method).
struct TraceMethodVisitor<'a>(&'a mut dyn MethodVisitor, &'a RefCell<String>);

impl MethodVisitor for TraceMethodVisitor<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.0
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
        self.1
            .borrow_mut()
            .push_str(&format!("TraceMethodVisitor:{name}{descriptor};"));
    }
}

/// A user "subclass" of `TraceMethodVisitor` (ASM's `UserTraceMethodVisitor4` and
/// `UserTraceMethodVisitor5`): it overrides visitMethodInsn, and calls the `TraceMethodVisitor`
/// it extends.
struct UserTraceMethodVisitor<'a>(TraceMethodVisitor<'a>, &'a RefCell<String>, &'static str);

impl MethodVisitor for UserTraceMethodVisitor<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.0)
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.0
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
        self.1
            .borrow_mut()
            .push_str(&format!("{}:{name}{descriptor};", self.2));
    }
}

/// A method visitor that logs the calls to its visitMethodInsn method.
struct LogMethodVisitor<'a>(&'a RefCell<String>);

impl MethodVisitor for LogMethodVisitor<'_> {
    fn visit_method_insn(
        &mut self,
        _opcode: u8,
        _owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        _is_interface: bool,
    ) {
        self.0
            .borrow_mut()
            .push_str(&format!("LogMethodVisitor:{name}{descriptor};"));
    }
}
