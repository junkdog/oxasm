//! Shape tests for the visitor traits: default forwarding through `delegate()`, the
//! non-forwarding `try_copy_method` hook (ADR-08), and the `&mut V`/`Box<V>` impls.

use super::*;
use crate::jstr;
use crate::jstr::JavaStr;

/// Records the events it receives; accepts method copies, like a method writer would.
#[derive(Default)]
struct Sink {
    events: Vec<String>,
    copied: bool,
}

impl ClassVisitor for Sink {
    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        name: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.events.push(format!("method {name}"));
        Some(Box::new(self))
    }
    fn visit_end(&mut self) {
        self.events.push("end".into());
    }
}

impl MethodVisitor for Sink {
    fn try_copy_method(&mut self, _: &MethodCopySource<'_>) -> bool {
        self.copied = true;
        true
    }
    fn visit_insn(&mut self, opcode: u8) {
        self.events.push(format!("insn {opcode}"));
    }
}

/// Overrides nothing: everything forwards to `next`.
struct PassThrough<V>(V);

impl<V: ClassVisitor> ClassVisitor for PassThrough<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }
}

/// Wraps the downstream method visitor, overriding nothing.
struct MethodPassThrough<V>(V);

impl<V: ClassVisitor> ClassVisitor for MethodPassThrough<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let mv = self
            .0
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(Wrapped(mv)))
    }
}

struct Wrapped<'a>(Box<dyn MethodVisitor + 'a>);

impl MethodVisitor for Wrapped<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }
}

fn source() -> MethodCopySource<'static> {
    MethodCopySource {
        reader_id: 1,
        method_info: &[],
    }
}

fn drive(cv: &mut dyn ClassVisitor) {
    if let Some(mut mv) = cv.visit_method(0, jstr!("m"), jstr!("()V"), None, &[]) {
        if !mv.try_copy_method(&source()) {
            mv.visit_insn(0xb1);
        }
        mv.visit_end();
    }
    cv.visit_end();
}

#[test]
fn defaults_forward_to_the_delegate() {
    let mut sink = Sink::default();
    drive(&mut PassThrough(&mut sink));
    // The class-level pass-through hands out the sink's own method visitor, so the copy reaches
    // it: no method adapter is in between.
    assert!(sink.copied);
    assert_eq!(sink.events, ["method m", "end"]);
}

#[test]
fn try_copy_method_is_not_forwarded_through_a_method_adapter() {
    let mut sink = Sink::default();
    drive(&mut MethodPassThrough(&mut sink));
    // The wrapper does not override `try_copy_method`, and the default does not forward, so the
    // method is visited instruction by instruction through the adapter.
    assert!(!sink.copied);
    assert_eq!(sink.events, ["method m", "insn 177", "end"]);
}

#[test]
fn references_and_boxes_forward_everything() {
    let mut sink = Sink::default();
    {
        let mut boxed: Box<dyn ClassVisitor + '_> = Box::new(&mut sink);
        drive(&mut boxed);
    }
    assert!(
        sink.copied,
        "`&mut V` and `Box<V>` must forward try_copy_method"
    );
    assert_eq!(sink.events, ["method m", "end"]);
}

#[test]
fn visitors_are_object_safe() {
    fn assert_dyn(
        _: Option<&mut dyn ClassVisitor>,
        _: Option<&mut dyn MethodVisitor>,
        _: Option<&mut dyn FieldVisitor>,
        _: Option<&mut dyn AnnotationVisitor>,
        _: Option<&mut dyn ModuleVisitor>,
        _: Option<&mut dyn RecordComponentVisitor>,
    ) {
    }
    assert_dyn(None, None, None, None, None, None);
}
