//! Port of ASM's `TraceClassVisitorTest`.

use oxasm::constant::Constant;
use oxasm::{
    AnnotationVisitor, ClassReader, ClassVisitor, FieldVisitor, JavaStr, MethodVisitor,
    ModuleVisitor, ParsingOptions,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_util::{
    Textifier, TraceAnnotationVisitor, TraceClassVisitor, TraceFieldVisitor, TraceMethodVisitor,
    TraceModuleVisitor,
};

/// Tests that classes are unchanged with a ClassReader->TraceClassVisitor->ClassWriter transform.
#[test]
#[ignore = "needs ClassWriter (P5)"]
fn test_visit_methods() {
    // ClassReader(classFile).accept(TraceClassVisitor(ClassWriter(0), ...), [Comment,
    // CodeComment], 0), then ClassFile(classFile) == ClassFile(classWriter.toByteArray()).
    unimplemented!("needs ClassWriter (P5) and the ClassFile dumper");
}

/// Tests that ClassReader can accept a TraceClassVisitor without delegate.
#[test]
fn test_visit_methods_no_delegate() {
    for &class in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class.bytes()).unwrap();
        let mut output = Vec::new();
        class_reader
            .accept(
                &mut TraceClassVisitor::new(&mut output),
                ParsingOptions::empty(),
            )
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&class.internal_name()), "{class}");
    }
}

/// A class visitor whose children are tracing visitors without a delegate.
struct NestedTraceVisitors;

impl ClassVisitor for NestedTraceVisitors {
    fn visit_module<'a>(
        &'a mut self,
        _name: &JavaStr,
        _access: u32,
        _version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        Some(Box::new(TraceModuleVisitor::new(Textifier::new())))
    }

    fn visit_annotation<'a>(
        &'a mut self,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(TraceAnnotationVisitor::new(Textifier::new())))
    }

    fn visit_field<'a>(
        &'a mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        Some(Box::new(TraceFieldVisitor::new(Textifier::new())))
    }

    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(TraceMethodVisitor::new(Textifier::new())))
    }
}

/// Tests that ClassReader can accept a TraceAnnotationVisitor, TraceFieldVisitor,
/// TraceMethodVisitor or TraceModuleVisitor without delegate.
#[test]
fn test_visit_methods_no_nested_delegate() {
    for &class in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class.bytes()).unwrap();
        class_reader
            .accept(&mut NestedTraceVisitors, ParsingOptions::empty())
            .unwrap();
    }
}

/// oxasm-specific: the downstream visitor of a TraceClassVisitor sees every event.
#[test]
fn test_delegates_to_downstream() {
    #[derive(Default)]
    struct Counter(usize);
    impl ClassVisitor for Counter {
        fn visit_nest_member(&mut self, _nest_member: &JavaStr) {
            self.0 += 1;
        }
        fn visit_end(&mut self) {
            self.0 += 10;
        }
    }
    let mut output = Vec::new();
    let mut tcv = TraceClassVisitor::with_delegate(
        Some(Counter::default()),
        Textifier::new(),
        Some(&mut output),
    );
    tcv.visit_nest_member(oxasm::jstr!("A"));
    tcv.visit_end();
    assert_eq!(tcv.class_visitor().unwrap().0, 11);
    drop(tcv);
    assert_eq!(String::from_utf8(output).unwrap(), "  NESTMEMBER A\n}\n");
}
