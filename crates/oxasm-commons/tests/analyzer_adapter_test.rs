//! ASM's `AnalyzerAdapterTest`.

mod common;

use std::borrow::Cow;

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, FrameKind, FrameValue,
    Handle, JavaStr, JavaString, Label, MethodVisitor, ParsingOptions, jstr,
};
use oxasm_commons::AnalyzerAdapter;
use oxasm_testkit::{PrecompiledClass, panic_message};

#[test]
fn test_constructor() {
    let _ = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC,
        jstr!("name"),
        jstr!("()V"),
        None,
    );
    // Dropped: ASM's constructor throws an IllegalStateException when a subclass calls it (Rust
    // has no subclasses).
}

#[test]
fn test_visit_frame_empty_frame() {
    let mut analyzer_adapter = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC,
        jstr!("name"),
        jstr!("()V"),
        None,
    );

    analyzer_adapter.visit_frame(FrameKind::New, &[], &[]);
}

#[test]
fn test_visit_frame_invalid_frame_type() {
    let mut analyzer_adapter = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC,
        jstr!("name"),
        jstr!("()V"),
        None,
    );

    let message = panic_message(|| analyzer_adapter.visit_frame(FrameKind::Full, &[], &[]));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: AnalyzerAdapter only accepts expanded frames (see \
         ClassReader.EXPAND_FRAMES)"
    );
}

/// Tests that classes with additional frames inserted at each instruction, using the results of
/// an AnalyzerAdapter, can be instantiated and loaded. This makes sure the intermediate frames
/// computed by AnalyzerAdapter are correct, i.e. pass bytecode verification.
///
/// (The `api` dimension of ASM's test disappears, ADR-09.) The JVM's verification is
/// [`test_all_methods_precompiled_class_new_instance`].
#[test]
fn test_all_methods_precompiled_class() {
    let _ = transformed_precompiled_classes();
}

/// [`test_all_methods_precompiled_class`]'s `newInstance()` (the JVM through the oracle).
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_precompiled_class_new_instance() {
    let (mut verified_classes, preview_classes) = transformed_precompiled_classes();
    // ASM's newInstance(), for all the classes at once. ASM loads them with the test classes on
    // the class path: their siblings in T0 (inner classes, ...) are verified with them.
    verified_classes.extend(t0_siblings());
    let classes: Vec<&[u8]> = verified_classes.iter().map(Vec::as_slice).collect();
    oxasm_testkit::jvm::verify_classes(&classes).unwrap();
    // Preview classes of another JDK: an UnsupportedClassVersionError.
    for (class_parameter, bytes) in preview_classes {
        let error = oxasm_testkit::jvm::verify_class(&bytes).expect_err("a preview class");
        assert!(
            error.contains("UnsupportedClassVersionError"),
            "{class_parameter}: {error}"
        );
    }
}

/// The precompiled classes with a frame inserted before each instruction (or, for the two with
/// JSR/RET, which the adapter rejects with ASM's message, the original class): the classes to
/// verify, and the preview classes.
#[allow(clippy::type_complexity)]
fn transformed_precompiled_classes() -> (Vec<Vec<u8>>, Vec<(PrecompiledClass, Vec<u8>)>) {
    let mut verified_classes = Vec::new();
    let mut preview_classes = Vec::new();
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        let mut accept = || {
            let mut class_analyzer_adapter = ClassAnalyzerAdapter::new(&mut class_writer);
            class_reader
                .accept(&mut class_analyzer_adapter, ParsingOptions::EXPAND_FRAMES)
                .unwrap();
        };

        // jdk3.AllInstructions and jdk3.LargeMethod contain unsupported jsr/ret instructions.
        if class_parameter == PrecompiledClass::Jdk3AllInstructions
            || class_parameter == PrecompiledClass::Jdk3LargeMethod
        {
            assert_eq!(
                panic_message(accept),
                "java.lang.IllegalArgumentException: JSR/RET are not supported",
                "{class_parameter}"
            );
            // Keep the original class, which the other classes of its package may refer to.
            verified_classes.push(class_file.to_vec());
        } else {
            accept();
            let bytes = class_writer.to_bytes().unwrap();
            if class_parameter.is_preview() {
                preview_classes.push((class_parameter, bytes));
            } else {
                verified_classes.push(bytes);
            }
        }
    }
    (verified_classes, preview_classes)
}

/// The class files of the T0 packages of the precompiled classes that are not precompiled classes
/// themselves (inner classes, ...), except preview classes.
fn t0_siblings() -> Vec<Vec<u8>> {
    let mut packages: Vec<&str> = PrecompiledClass::ALL
        .iter()
        .filter_map(|class_parameter| class_parameter.name().rsplit_once('.'))
        .map(|(package, _)| package)
        .collect();
    packages.sort_unstable();
    packages.dedup();
    let mut siblings = Vec::new();
    for package in packages {
        let mut paths: Vec<_> = std::fs::read_dir(oxasm_testkit::corpus_t0().join(package))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "class"))
            .collect();
        paths.sort();
        for path in paths {
            let bytes = std::fs::read(path).unwrap();
            let is_precompiled = PrecompiledClass::ALL
                .iter()
                .any(|class_parameter| class_parameter.bytes() == bytes.as_slice());
            let is_preview = bytes[4..6] == [0xFF, 0xFF];
            if !is_precompiled && !is_preview {
                siblings.push(bytes);
            }
        }
    }
    siblings
}

/// oxasm-specific: the frames AnalyzerAdapter computes, around a constructor call and a jump.
#[test]
fn test_locals_and_stack() {
    let mut analyzer_adapter = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC,
        jstr!("m"),
        jstr!("(J[I)V"),
        None,
    );
    let object = |name: &str| FrameValue::Object(Cow::Owned(JavaString::from(name)));
    let initial_locals = vec![
        object("pkg/Class"),
        FrameValue::Long,
        FrameValue::Top,
        object("[I"),
    ];
    assert_eq!(analyzer_adapter.locals, Some(initial_locals.clone()));
    assert_eq!(analyzer_adapter.stack, Some(vec![]));

    let label = Label::new();
    analyzer_adapter.visit_label(label);
    analyzer_adapter.visit_type_insn(NEW, jstr!("java/lang/Object"));
    analyzer_adapter.visit_insn(DUP);
    assert_eq!(
        analyzer_adapter.stack,
        Some(vec![
            FrameValue::Uninitialized(label),
            FrameValue::Uninitialized(label)
        ])
    );
    assert_eq!(
        analyzer_adapter.uninitialized_types.get(&label),
        Some(&JavaString::from("java/lang/Object"))
    );
    analyzer_adapter.visit_method_insn(
        INVOKESPECIAL,
        jstr!("java/lang/Object"),
        jstr!("<init>"),
        jstr!("()V"),
        false,
    );
    assert_eq!(
        analyzer_adapter.stack,
        Some(vec![object("java/lang/Object")])
    );
    analyzer_adapter.visit_var_insn(ASTORE, 2);
    assert_eq!(
        analyzer_adapter.locals,
        Some(vec![
            object("pkg/Class"),
            FrameValue::Top,
            object("java/lang/Object"),
            object("[I"),
        ])
    );
    analyzer_adapter.visit_var_insn(ALOAD, 3);
    analyzer_adapter.visit_insn(ICONST_0);
    analyzer_adapter.visit_insn(IALOAD);
    analyzer_adapter.visit_ldc_insn(&Constant::Long(1));
    assert_eq!(
        analyzer_adapter.stack,
        Some(vec![FrameValue::Integer, FrameValue::Long, FrameValue::Top])
    );
    analyzer_adapter.visit_jump_insn(GOTO, label);
    assert_eq!(analyzer_adapter.locals, None);
    assert_eq!(analyzer_adapter.stack, None);
    analyzer_adapter.visit_frame(FrameKind::New, &[FrameValue::Long], &[]);
    assert_eq!(
        analyzer_adapter.locals,
        Some(vec![FrameValue::Long, FrameValue::Top])
    );
    assert_eq!(analyzer_adapter.stack, Some(vec![]));
}

/// A ClassVisitor that inserts intermediate frames before each instruction of each method, using
/// the types computed with an AnalyzerAdapter (in order to check that these intermediate frames
/// are correct).
struct ClassAnalyzerAdapter<V> {
    cv: V,
    owner: JavaString,
}

impl<V: ClassVisitor> ClassAnalyzerAdapter<V> {
    fn new(class_visitor: V) -> Self {
        ClassAnalyzerAdapter {
            cv: class_visitor,
            owner: JavaString::new(),
        }
    }
}

impl<V: ClassVisitor> ClassVisitor for ClassAnalyzerAdapter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.cv)
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.owner = name.to_owned();
        self.cv
            .visit(version, access, name, signature, super_name, interfaces);
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let method_visitor = self
            .cv
            .visit_method(access, name, descriptor, signature, exceptions);
        let analyzer_adapter =
            AnalyzerAdapter::new(&self.owner, access, name, descriptor, method_visitor);
        Some(Box::new(AnalyzedFramesInserter {
            analyzer_adapter,
            has_original_frame: false,
        }))
    }
}

/// Inserts intermediate frames before each instruction, using the types computed with an
/// AnalyzerAdapter.
///
/// ASM's inserter is the adapter's downstream visitor, and reads the adapter's frame while the
/// adapter delegates an instruction to it. A Rust visitor cannot reach the adapter that owns it,
/// so this one is the adapter's *upstream* visitor: it reads the same frame just before it passes
/// an instruction on, and inserts the frame into the adapter's downstream visitor, past the
/// adapter. (Inserting it *through* the adapter would reset the adapter's frame from it, which
/// changes it when the code is malformed, e.g. when a `double` lost its second slot.)
struct AnalyzedFramesInserter<'a> {
    analyzer_adapter: AnalyzerAdapter<'a>,
    has_original_frame: bool,
}

impl AnalyzedFramesInserter<'_> {
    fn maybe_insert_frame(&mut self) {
        // Don't insert a frame if we already have one for this instruction, from the original
        // class.
        if !self.has_original_frame
            && let (Some(locals), Some(stack)) =
                (&self.analyzer_adapter.locals, &self.analyzer_adapter.stack)
        {
            let local = to_frame_types(locals);
            let stack = to_frame_types(stack);
            // Straight to the adapter's downstream visitor, as ASM's inserter (which is that
            // visitor) does: the adapter does not see the inserted frame.
            if let Some(method_visitor) = self.analyzer_adapter.delegate() {
                method_visitor.visit_frame(FrameKind::New, &local, &stack);
            }
        }
        self.has_original_frame = false;
    }
}

/// Converts local and stack types from AnalyzerAdapter to visitFrame format (long and double are
/// represented with one element in visitFrame, but with two elements in AnalyzerAdapter).
fn to_frame_types(analyzer_types: &[FrameValue<'static>]) -> Vec<FrameValue<'static>> {
    let mut frame_types = Vec::new();
    for (i, value) in analyzer_types.iter().enumerate() {
        if i > 0 && matches!(analyzer_types[i - 1], FrameValue::Long | FrameValue::Double) {
            continue;
        }
        frame_types.push(value.clone());
    }
    frame_types
}

impl MethodVisitor for AnalyzedFramesInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.analyzer_adapter)
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        self.analyzer_adapter.visit_frame(kind, locals, stack);
        self.has_original_frame = true;
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_insn(opcode);
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_int_insn(opcode, operand);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_var_insn(opcode, var_index);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_type_insn(opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_field_insn(opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_jump_insn(opcode, label);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_ldc_insn(value);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.maybe_insert_frame();
        self.analyzer_adapter.visit_iinc_insn(var_index, increment);
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_table_switch_insn(min, max, dflt, labels);
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_lookup_switch_insn(dflt, keys, labels);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.maybe_insert_frame();
        self.analyzer_adapter
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
    }

    fn visit_maxs(&mut self, max_stack: u16, _max_locals: u16) {
        // AnalyzerAdapter should correctly recompute maxLocals from scratch.
        self.analyzer_adapter.visit_maxs(max_stack, 0);
    }
}

/// oxasm-specific (a `commons` fuzz finding): an object descriptor without a name or `;`, here a
/// field's, throws Java's `substring` exception, as in ASM, and not a Rust slice panic. So does an
/// `AALOAD` on an empty array descriptor.
#[test]
fn test_truncated_object_descriptor() {
    let mut analyzer_adapter = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC | ACC_STATIC,
        jstr!("m"),
        jstr!("()V"),
        None,
    );

    let message = panic_message(|| {
        analyzer_adapter.visit_field_insn(GETSTATIC, jstr!("pkg/Class"), jstr!("f"), jstr!("L"));
    });

    assert_eq!(
        message,
        "java.lang.StringIndexOutOfBoundsException: Range [1, 0) out of bounds for length 1"
    );

    let mut analyzer_adapter = AnalyzerAdapter::new(
        jstr!("pkg/Class"),
        ACC_PUBLIC | ACC_STATIC,
        jstr!("m"),
        jstr!("()V"),
        None,
    );
    analyzer_adapter.visit_field_insn(GETSTATIC, jstr!("pkg/Class"), jstr!("f"), jstr!("[I"));
    analyzer_adapter.stack = Some(vec![
        FrameValue::Object(Cow::Owned(JavaString::new())),
        FrameValue::Integer,
    ]);

    let message = panic_message(|| analyzer_adapter.visit_insn(AALOAD));

    assert_eq!(
        message,
        "java.lang.StringIndexOutOfBoundsException: Range [1, 0) out of bounds for length 0"
    );
}
