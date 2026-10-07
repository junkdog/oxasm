//! Port of ASM's `ClassWriterFlagsTest`: a class writer can be configured to compute maxs and
//! frames for each method.

use std::cell::Cell;

use oxasm::opcodes::{ACC_PUBLIC, ACONST_NULL, GOTO, LCONST_0, RETURN, V1_7};
use oxasm::{
    ClassHierarchy, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, FrameKind,
    FrameValue, JavaStr, Label, MethodVisitor, ObjectHierarchy, ParsingOptions, jstr,
};

#[test]
fn switches_modes_three_times() {
    let hierarchy = ObjectHierarchy;
    let mut writer = ClassWriter::new(ClassWriterFlags::empty()).with_hierarchy(&hierarchy);
    let compute_maxs = "computeMaxs";
    let compute_nothing = "computeNothing";
    let compute_frames = "computeFrames";
    let mut clazz = DummyClass::new(&mut writer);
    clazz.writer.set_flags(ClassWriterFlags::COMPUTE_MAXS);
    clazz.with_dummy_method(compute_maxs);
    clazz.writer.set_flags(ClassWriterFlags::empty());
    clazz.with_dummy_method(compute_nothing);
    clazz.writer.set_flags(ClassWriterFlags::COMPUTE_FRAMES);
    clazz.with_dummy_method(compute_frames);
    let compiled = clazz.compile();

    assert_eq!(
        compiled.maxs(compute_maxs),
        Maxs {
            stack: 3,
            locals: 1
        }
    );
    assert_eq!(
        compiled.maxs(compute_nothing),
        Maxs {
            stack: 1,
            locals: 0
        }
    );
    assert_eq!(
        compiled.maxs(compute_frames),
        Maxs {
            stack: 3,
            locals: 1
        }
    );
}

#[test]
fn computes_frames_as_usual() {
    computes_as_usual(
        ClassWriterFlags::COMPUTE_FRAMES,
        "frames",
        Maxs {
            stack: 3,
            locals: 1,
        },
    );
}

#[test]
fn computes_maxs_as_usual() {
    computes_as_usual(
        ClassWriterFlags::COMPUTE_MAXS,
        "maxs",
        Maxs {
            stack: 3,
            locals: 1,
        },
    );
}

#[test]
fn computes_nothing_as_usual() {
    computes_as_usual(
        ClassWriterFlags::empty(),
        "nothing",
        Maxs {
            stack: 1,
            locals: 0,
        },
    );
}

fn computes_as_usual(flags: ClassWriterFlags, method: &str, expected: Maxs) {
    let hierarchy: &dyn ClassHierarchy = &ObjectHierarchy;
    let mut writer = ClassWriter::new(flags).with_hierarchy(hierarchy);
    let mut clazz = DummyClass::new(&mut writer);
    clazz.with_dummy_method(method);
    assert_eq!(clazz.compile().maxs(method), expected);
}

/// The max stack and locals of a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Maxs {
    stack: u16,
    locals: u16,
}

/// A compiled class file.
struct CompiledClass {
    clazz: Vec<u8>,
}

impl CompiledClass {
    /// The max stack and locals of the given method.
    fn maxs(&self, method: &str) -> Maxs {
        let maxs = Cell::new(None);
        let mut visitor = MaxsReader {
            method,
            maxs: &maxs,
        };
        ClassReader::new(&self.clazz)
            .unwrap()
            .accept(&mut visitor, ParsingOptions::empty())
            .unwrap();
        maxs.get().expect("the method's maxs")
    }
}

/// Records the maxs of a method.
struct MaxsReader<'a> {
    method: &'a str,
    maxs: &'a Cell<Option<Maxs>>,
}

impl ClassVisitor for MaxsReader<'_> {
    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        (name == self.method).then(|| Box::new(MethodMaxsReader { maxs: self.maxs }) as _)
    }
}

struct MethodMaxsReader<'a> {
    maxs: &'a Cell<Option<Maxs>>,
}

impl MethodVisitor for MethodMaxsReader<'_> {
    fn visit_maxs(&mut self, stack: u16, locals: u16) {
        self.maxs.set(Some(Maxs { stack, locals }));
    }
}

/// A class to which dummy methods are added.
struct DummyClass<'w, 'h> {
    /// Class writer to use in a test.
    writer: &'w mut ClassWriter<'h>,
}

impl<'w, 'h> DummyClass<'w, 'h> {
    fn new(writer: &'w mut ClassWriter<'h>) -> DummyClass<'w, 'h> {
        writer.visit(
            V1_7,
            ACC_PUBLIC,
            jstr!("SomeClass"),
            None,
            Some(jstr!("java/lang/Object")),
            &[],
        );
        DummyClass { writer }
    }

    fn compile(self) -> CompiledClass {
        self.writer.visit_end();
        CompiledClass {
            clazz: self.writer.to_bytes().unwrap(),
        }
    }

    fn with_dummy_method(&mut self, method: &str) -> &mut Self {
        let start = Label::new();
        let mut mvisitor = self
            .writer
            .visit_method(ACC_PUBLIC, method.into(), jstr!("()V"), None, &[])
            .unwrap();
        mvisitor.visit_code();
        mvisitor.visit_label(start);
        mvisitor.visit_insn(LCONST_0);
        let label = Label::new();
        mvisitor.visit_jump_insn(GOTO, label);
        mvisitor.visit_label(label);
        mvisitor.visit_frame(FrameKind::New, &[], &[FrameValue::Long]);
        mvisitor.visit_insn(ACONST_NULL);
        mvisitor.visit_insn(RETURN);
        mvisitor.visit_maxs(1, 0);
        mvisitor.visit_end();
        drop(mvisitor);
        self
    }
}
