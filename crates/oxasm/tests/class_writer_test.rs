//! Port of ASM's `ClassWriterTest`.
//!
//! - ASM's `new ClassFile(bytes).newInstance()` checks (the JVM loads and verifies the class) run
//!   the oracle's `verify` (`oxasm_testkit::jvm`): they are in `#[ignore = "needs the JVM
//!   oracle"]` tests, run by CI's oracle job with `--ignored`.
//! - Computing frames needs a class hierarchy (ADR-07), where ASM loads classes: the tests that
//!   check the computed frames (byte equality, JVM verification) use [`class_loader_hierarchy`],
//!   the hierarchy of T0 and the JDK, which the oracle computes; the others use
//!   [`frames_hierarchy`].
//! - The `Api` dimension of the parameterized tests is gone (ADR-09).
//!
//! Dropped:
//! - `testInstanceFields` and `testVisitMethods_final`: they check the Java class by reflection
//!   (its field list, and that its visit methods are `final` for binary compatibility).
//! - `testNewConst_illegalArgument`: `newConst(new Object())` cannot be written, `Constant` is an
//!   enum (ADR-05).
//! - `testDeprecatedNewHandle`: the deprecated `newHandle` without `isInterface` is not ported
//!   (`new_handle` takes a `Handle`, see `HandleTest`).

use std::collections::HashSet;
use std::sync::Arc;

use oxasm::error::{HierarchyError, ReadError};
use oxasm::opcodes::{
    ACC_PUBLIC, ACC_STATIC, ALOAD, ARETURN, ASTORE, GOTO, ICONST_0, IFNE, ILOAD, INVOKESPECIAL,
    ISTORE, NEW, NOP, RETURN, V1_1, V1_7,
};
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{
    AttributeCodec, ClassHierarchy, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags,
    Constant, CustomAttribute, FrameKind, FrameValue, Handle, HandleKind, JavaStr, JavaString,
    Label, MapHierarchy, MethodVisitor, ObjectHierarchy, ParsingOptions, WriteError, jstr,
};
use oxasm_testkit::{ClassFile, PrecompiledClass};

// -------------------------------------------------------------------------------------------------
// Test support
// -------------------------------------------------------------------------------------------------

/// ASM's `Comment` test attribute (one of ASM's `attributes()` prototypes).
#[derive(Clone, Debug)]
struct Comment;

impl CustomAttribute for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn codec(&self) -> Option<Arc<dyn AttributeCodec>> {
        Some(Arc::new(Comment))
    }
}

impl AttributeCodec for Comment {
    fn name(&self) -> &JavaStr {
        jstr!("Comment")
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(Comment))
    }
}

/// ASM's `CodeComment` test attribute (one of ASM's `attributes()` prototypes).
#[derive(Clone, Debug)]
struct CodeComment;

impl CustomAttribute for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        Vec::new()
    }
    fn codec(&self) -> Option<Arc<dyn AttributeCodec>> {
        Some(Arc::new(CodeComment))
    }
}

impl AttributeCodec for CodeComment {
    fn name(&self) -> &JavaStr {
        jstr!("CodeComment")
    }
    fn is_code_attribute(&self) -> bool {
        true
    }
    fn read(
        &self,
        _cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        Ok(Box::new(CodeComment))
    }
}

/// ASM's `attributes()`: the attribute prototypes of the test classes.
fn attributes() -> [&'static dyn AttributeCodec; 2] {
    [&Comment, &CodeComment]
}

/// A class writer that has visited the header of a class `C` (ASM's `newEmptyClassWriter`).
fn new_empty_class_writer() -> ClassWriter<'static> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    class_writer
}

/// The constant pool dump of the class written by the given writer.
fn get_constant_pool_dump(class_writer: &mut ClassWriter<'_>) -> String {
    let class_file = class_writer.to_bytes().expect("a class file");
    String::from_utf8_lossy(&ClassFile::new(&class_file).constant_pool_dump().unwrap()).into_owned()
}

/// The dump of the class written by the given writer.
fn get_dump(class_writer: &mut ClassWriter<'_>) -> String {
    let class_file = class_writer.to_bytes().expect("a class file");
    ClassFile::new(&class_file).dump_lossy().unwrap()
}

/// Reads the given class into a class writer with the given flags and reader options, and
/// returns the written class.
fn read_and_write(
    class_file: &[u8],
    flags: ClassWriterFlags,
    copy_pool: bool,
    options: ParsingOptions,
    hierarchy: &dyn ClassHierarchy,
) -> Result<Vec<u8>, WriteError> {
    let class_reader = ClassReader::new(class_file).unwrap();
    let class_writer = if copy_pool {
        ClassWriter::from_reader(&class_reader, flags).unwrap()
    } else {
        ClassWriter::new(flags)
    };
    let mut class_writer = class_writer.with_hierarchy(hierarchy);
    class_reader
        .accept_with_codecs(&mut class_writer, &attributes(), options)
        .unwrap();
    class_writer.to_bytes()
}

/// The class hierarchy used where ASM computes frames with class loading, in the tests whose
/// result does not depend on it.
fn frames_hierarchy() -> ObjectHierarchy {
    ObjectHierarchy
}

/// The class hierarchy ASM's tests get from class loading: the classes of T0 and of the JDK, as
/// the oracle sees them (it needs the JVM oracle and `corpus/t1`).
fn class_loader_hierarchy() -> &'static MapHierarchy {
    static HIERARCHY: std::sync::OnceLock<MapHierarchy> = std::sync::OnceLock::new();
    HIERARCHY.get_or_init(|| {
        MapHierarchy::from_tsv(oxasm_testkit::jvm::class_loader_hierarchy_tsv()).unwrap()
    })
}

/// Whether the given class has JSR or RET instructions.
fn has_jsr_or_ret_instructions(class_parameter: PrecompiledClass) -> bool {
    class_parameter == PrecompiledClass::Jdk3AllInstructions
        || class_parameter == PrecompiledClass::Jdk3LargeMethod
}

/// ASM's `newInstance` expectation: the JVM verifies the class, except for preview classes of
/// another JDK, which it rejects with an `UnsupportedClassVersionError`. ASM loads the class with
/// the test classes on the class path; here it is verified along with the T0 classes of its
/// package, and only its own result counts.
fn assert_new_instance(class_parameter: PrecompiledClass, class_file: &[u8]) {
    let siblings: Vec<Vec<u8>> = match class_parameter.name().rsplit_once('.') {
        Some((package, _)) => std::fs::read_dir(oxasm_testkit::corpus_t0().join(package))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "class"))
            .map(|path| std::fs::read(path).unwrap())
            .filter(|bytes| bytes.as_slice() != class_parameter.bytes())
            .collect(),
        None => Vec::new(),
    };
    let mut classes = vec![class_file];
    classes.extend(siblings.iter().map(Vec::as_slice));
    // The class under test is C0.class, the first one checked.
    let result = match oxasm_testkit::jvm::verify_classes(&classes) {
        Err(error) if !error.contains("C0.class:") => Ok(()),
        result => result,
    };
    if class_parameter.is_preview() {
        let error = result.expect_err("a preview class of another JDK");
        assert!(
            error.contains("UnsupportedClassVersionError"),
            "{class_parameter}: {error}"
        );
    } else {
        result.unwrap_or_else(|e| panic!("{class_parameter}: {e}"));
    }
}

// -------------------------------------------------------------------------------------------------
// Constant pool
// -------------------------------------------------------------------------------------------------

#[test]
fn test_new_const() {
    let mut class_writer = new_empty_class_writer();
    // ASM's Boolean, Byte, Character and Short constants: the caller widens them (ADR-05).
    class_writer.new_const(&Constant::Integer(0));
    class_writer.new_const(&Constant::Integer(1));
    class_writer.new_const(&Constant::Integer(i32::from(b'2')));
    class_writer.new_const(&Constant::Integer(3));

    let constant_pool_dump = get_constant_pool_dump(&mut class_writer);
    assert!(constant_pool_dump.contains("constant_pool: 0"));
    assert!(constant_pool_dump.contains("constant_pool: 1"));
    assert!(constant_pool_dump.contains("constant_pool: 50"));
    assert!(constant_pool_dump.contains("constant_pool: 3"));
}

#[test]
fn test_new_utf8() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_utf8(jstr!("A"));
    assert!(get_constant_pool_dump(&mut class_writer).contains("constant_pool: A"));
}

#[test]
fn test_new_class() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_class(jstr!("A"));
    assert!(
        get_constant_pool_dump(&mut class_writer).contains("constant_pool: ConstantClassInfo A")
    );
}

#[test]
fn test_new_method_type() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_method_type(jstr!("()V"));
    assert!(
        get_constant_pool_dump(&mut class_writer)
            .contains("constant_pool: ConstantMethodTypeInfo ()V")
    );
}

#[test]
fn test_new_module() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_module(jstr!("A"));
    assert!(
        get_constant_pool_dump(&mut class_writer).contains("constant_pool: ConstantModuleInfo A")
    );
}

#[test]
fn test_new_package() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_package(jstr!("A"));
    assert!(
        get_constant_pool_dump(&mut class_writer).contains("constant_pool: ConstantPackageInfo A")
    );
}

#[test]
fn test_new_handle() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_handle(&Handle::new(HandleKind::GetField, "A", "h", "I", false));
    assert!(
        get_constant_pool_dump(&mut class_writer)
            .contains("constant_pool: ConstantMethodHandleInfo 1.ConstantFieldRefInfo A.hI")
    );
}

#[test]
fn test_new_handle_is_interface() {
    let mut class_writer = new_empty_class_writer();
    let index1 = class_writer.new_handle(&Handle::new(
        HandleKind::InvokeVirtual,
        "A",
        "m",
        "()V",
        false,
    ));
    let index2 = class_writer.new_handle(&Handle::new(
        HandleKind::InvokeVirtual,
        "A",
        "m",
        "()V",
        true,
    ));
    assert_ne!(index1, index2);
}

#[test]
fn test_new_constant_dynamic() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_constant_dynamic(
        jstr!("m"),
        jstr!("Ljava/lang/String;"),
        &Handle::new(HandleKind::InvokeStatic, "A", "m", "()V", false),
        &[],
    );

    let constant_pool_dump = get_constant_pool_dump(&mut class_writer);
    assert!(
        constant_pool_dump.contains("constant_pool: ConstantDynamicInfo 0.mLjava/lang/String;")
    );
    assert!(
        constant_pool_dump
            .contains("constant_pool: ConstantMethodHandleInfo 6.ConstantMethodRefInfo A.m()V")
    );
    assert!(constant_pool_dump.contains("constant_pool: BootstrapMethods"));
}

#[test]
fn test_new_invoke_dynamic() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_invoke_dynamic(
        jstr!("m"),
        jstr!("()V"),
        &Handle::new(HandleKind::GetField, "A", "h", "I", false),
        &[],
    );

    let constant_pool_dump = get_constant_pool_dump(&mut class_writer);
    assert!(constant_pool_dump.contains("ConstantInvokeDynamicInfo 0.m()V"));
    assert!(
        constant_pool_dump
            .contains("constant_pool: ConstantMethodHandleInfo 1.ConstantFieldRefInfo A.hI")
    );
    assert!(constant_pool_dump.contains("constant_pool: BootstrapMethods"));
}

#[test]
fn test_new_field() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_field(jstr!("A"), jstr!("f"), jstr!("I"));
    assert!(
        get_constant_pool_dump(&mut class_writer)
            .contains("constant_pool: ConstantFieldRefInfo A.fI")
    );
}

#[test]
fn test_new_method() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_method(jstr!("A"), jstr!("m"), jstr!("()V"), false);
    assert!(
        get_constant_pool_dump(&mut class_writer)
            .contains("constant_pool: ConstantMethodRefInfo A.m()V")
    );
}

#[test]
fn test_new_name_type() {
    let mut class_writer = new_empty_class_writer();
    class_writer.new_name_type(jstr!("m"), jstr!("()V"));
    assert!(
        get_constant_pool_dump(&mut class_writer)
            .contains("constant_pool: ConstantNameAndTypeInfo m()V")
    );
}

// -------------------------------------------------------------------------------------------------
// to_bytes
// -------------------------------------------------------------------------------------------------

#[test]
fn test_to_byte_array_constant_pool_size_too_large() {
    for constant_pool_count in [65535_usize, 65536] {
        let mut class_writer = new_empty_class_writer();
        let init_constant_pool_count = 5;
        for i in 0..constant_pool_count - init_constant_pool_count {
            class_writer.new_const(&Constant::Integer(i32::try_from(i).unwrap()));
        }

        let result = class_writer.to_bytes();

        if constant_pool_count > 65535 {
            let error = result.expect_err("a class too large");
            assert_eq!(
                error,
                WriteError::ClassTooLarge {
                    class_name: JavaString::from("C"),
                    constant_pool_count,
                }
            );
            assert_eq!(error.to_string(), "Class too large: C");
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn test_to_byte_array_method_code_size_too_large() {
    for method_code_size in [65535_usize, 65536] {
        let mut class_writer = new_empty_class_writer();
        let method_name = jstr!("m");
        let descriptor = jstr!("()V");
        {
            let mut method_visitor = class_writer
                .visit_method(ACC_STATIC, method_name, descriptor, None, &[])
                .unwrap();
            method_visitor.visit_code();
            for _ in 0..method_code_size - 1 {
                method_visitor.visit_insn(NOP);
            }
            method_visitor.visit_insn(RETURN);
            method_visitor.visit_maxs(0, 0);
            method_visitor.visit_end();
        }

        let result = class_writer.to_bytes();

        if method_code_size > 65535 {
            let error = result.expect_err("a method too large");
            assert_eq!(
                error,
                WriteError::MethodTooLarge {
                    class_name: JavaString::from("C"),
                    method_name: method_name.to_owned(),
                    descriptor: descriptor.to_owned(),
                    code_size: method_code_size,
                }
            );
            assert_eq!(error.to_string(), "Method too large: C.m ()V");
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn test_to_byte_array_large_source_debug_extension() {
    let mut class_writer = new_empty_class_writer();
    let debug: JavaString = "\0".repeat(100_000).into();
    class_writer.visit_source(Some(jstr!("Test.java")), Some(&debug));

    assert!(get_dump(&mut class_writer).contains("attribute_name_index: SourceDebugExtension"));
}

/// Tests that the COMPUTE_MAXS option works correctly on classes with very large or deeply
/// nested subroutines (#307600, #311642).
#[test]
fn test_to_byte_array_compute_maxs_large_subroutines() {
    for class_file_name in ["Issue307600.class", "Issue311642.class"] {
        let class_file = oxasm_testkit::t0_resource("asm", class_file_name);
        let result = read_and_write(
            &class_file,
            ClassWriterFlags::COMPUTE_MAXS,
            false,
            ParsingOptions::empty(),
            &frames_hierarchy(),
        );
        assert!(result.is_ok(), "{class_file_name}: {result:?}");
    }
}

/// A class `A` with a default constructor and a static method `m` with a long local variable
/// using slots 0 and 1, with an int stored in slot 1 in a branch (ASM's
/// `testToByteArray_computeFrames_mergeLongOrDouble`).
fn merge_long_or_double_class() -> Result<Vec<u8>, WriteError> {
    let hierarchy = frames_hierarchy();
    let mut class_writer =
        ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES).with_hierarchy(&hierarchy);
    class_writer.visit(
        V1_7,
        ACC_PUBLIC,
        jstr!("A"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    // Generate a default constructor, so that we can instantiate the class.
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ALOAD, 0);
        method_visitor.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    // A method with a long local variable using slots 0 and 1, with an int stored in slot 1 in
    // a branch. At the end of the method, the stack map frame should contain 'TOP' for slot 0,
    // otherwise the class instantiation fails with a verification error.
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("(J)V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_insn(ICONST_0);
        let label = Label::new();
        method_visitor.visit_jump_insn(IFNE, label);
        method_visitor.visit_insn(ICONST_0);
        method_visitor.visit_var_insn(ISTORE, 1);
        method_visitor.visit_label(label);
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    class_writer.to_bytes()
}

#[test]
fn test_to_byte_array_compute_frames_merge_long_or_double() {
    let class_file = merge_long_or_double_class().unwrap();
    oxasm_testkit::jvm::verify_class(&class_file).unwrap();
}

#[test]
fn test_to_byte_array_compute_frames_high_dimension_arrays() {
    // ASM loads the classes; the hierarchy says the same.
    let mut hierarchy = MapHierarchy::new();
    hierarchy.insert("java/lang/Object", None, false);
    hierarchy.insert("java/lang/Number", Some("java/lang/Object".into()), false);
    hierarchy.insert("java/lang/Integer", Some("java/lang/Number".into()), false);
    hierarchy.insert("java/lang/Long", Some("java/lang/Number".into()), false);
    let mut class_writer =
        ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES).with_hierarchy(&hierarchy);
    class_writer.visit(
        V1_7,
        ACC_PUBLIC,
        jstr!("A"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(
                ACC_STATIC,
                jstr!("m"),
                jstr!("(I[[[[[[[[Ljava/lang/Integer;[[[[[[[[Ljava/lang/Long;)Ljava/lang/Object;"),
                None,
                &[],
            )
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ILOAD, 0);
        let then_label = Label::new();
        let end_if_label = Label::new();
        method_visitor.visit_jump_insn(IFNE, then_label);
        method_visitor.visit_var_insn(ALOAD, 1);
        method_visitor.visit_jump_insn(GOTO, end_if_label);
        method_visitor.visit_label(then_label);
        method_visitor.visit_var_insn(ALOAD, 2);
        // At this point the stack can contain either an 8 dimensions Integer array or an 8
        // dimensions Long array. The merged type computed with the COMPUTE_FRAMES option should
        // therefore be an 8 dimensions Number array.
        method_visitor.visit_label(end_if_label);
        method_visitor.visit_insn(ARETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();

    let class_file = class_writer.to_bytes().unwrap();

    // Check that the merged frame type is correctly computed.
    assert!(
        ClassFile::new(&class_file)
            .dump_lossy()
            .unwrap()
            .contains("[[[[[[[[Ljava/lang/Number;")
    );
}

/// A class `A` with a method whose frames have 24 uninitialized values whose NEW instructions
/// are visited after the frames (ASM's `testToByteArray_manyFramesWithForwardLabelReferences`).
fn many_frames_with_forward_label_references_class() -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        V1_7,
        ACC_PUBLIC,
        jstr!("A"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    {
        let mut constructor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .unwrap();
        constructor.visit_code();
        constructor.visit_var_insn(ALOAD, 0);
        constructor.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        constructor.visit_insn(RETURN);
        constructor.visit_maxs(1, 1);
        constructor.visit_end();
    }
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_STATIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        let label0 = Label::new();
        method_visitor.visit_jump_insn(GOTO, label0);
        let label1 = Label::new();
        method_visitor.visit_label(label1);
        let new_labels: Vec<Label> = (0..24).map(|_| Label::new()).collect();
        let uninitialized_types: Vec<FrameValue<'_>> = new_labels
            .iter()
            .map(|&label| FrameValue::Uninitialized(label))
            .collect();
        method_visitor.visit_frame(FrameKind::New, &uninitialized_types, &[]);
        for i in 0..new_labels.len() {
            method_visitor.visit_var_insn(ALOAD, u16::try_from(i).unwrap());
            method_visitor.visit_method_insn(
                INVOKESPECIAL,
                jstr!("A"),
                jstr!("<init>"),
                jstr!("()V"),
                false,
            );
        }
        let label2 = Label::new();
        method_visitor.visit_jump_insn(GOTO, label2);
        method_visitor.visit_label(label0);
        let top_types = vec![FrameValue::Top; new_labels.len()];
        method_visitor.visit_frame(FrameKind::New, &top_types, &[]);
        for (i, &new_label) in new_labels.iter().enumerate() {
            method_visitor.visit_label(new_label);
            method_visitor.visit_type_insn(NEW, jstr!("A"));
            method_visitor.visit_var_insn(ASTORE, u16::try_from(i).unwrap());
        }
        method_visitor.visit_jump_insn(GOTO, label1);
        method_visitor.visit_label(label2);
        let new_types = vec![FrameValue::Object(jstr!("A").into()); new_labels.len()];
        method_visitor.visit_frame(FrameKind::New, &new_types, &[]);
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(1, u16::try_from(new_labels.len()).unwrap());
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    class_writer.to_bytes().unwrap()
}

#[test]
fn test_to_byte_array_many_frames_with_forward_label_references() {
    let class_file = many_frames_with_forward_label_references_class();
    assert!(ClassFile::new(&class_file).dump().is_ok());
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_to_byte_array_many_frames_with_forward_label_references_new_instance() {
    let class_file = many_frames_with_forward_label_references_class();
    oxasm_testkit::jvm::verify_class(&class_file).unwrap();
}

/// ASM's `getCommonSuperClass` loads classes; a class hierarchy replaces it (ADR-07). This is the
/// same test over a hierarchy holding what class loading would tell.
#[test]
fn test_get_common_super_class() {
    let mut hierarchy = MapHierarchy::new();
    let object = || Some(JavaString::from("java/lang/Object"));
    hierarchy.insert("java/lang/Object", None, false);
    hierarchy.insert("java/lang/Number", object(), false);
    hierarchy.insert("java/lang/Integer", Some("java/lang/Number".into()), false);
    hierarchy.insert("java/lang/Runnable", object(), true);
    hierarchy.insert("java/lang/Throwable", object(), false);
    hierarchy.insert(
        "java/lang/Exception",
        Some("java/lang/Throwable".into()),
        false,
    );
    hierarchy.insert("java/lang/Error", Some("java/lang/Throwable".into()), false);
    hierarchy.insert(
        "java/lang/RuntimeException",
        Some("java/lang/Exception".into()),
        false,
    );
    hierarchy.insert(
        "java/lang/IndexOutOfBoundsException",
        Some("java/lang/RuntimeException".into()),
        false,
    );
    hierarchy.insert(
        "java/lang/AssertionError",
        Some("java/lang/Error".into()),
        false,
    );
    let common = |a: &str, b: &str| hierarchy.common_super_class(a.into(), b.into());

    assert_eq!(
        common("java/lang/Object", "java/lang/Integer").unwrap(),
        "java/lang/Object"
    );
    assert_eq!(
        common("java/lang/Integer", "java/lang/Object").unwrap(),
        "java/lang/Object"
    );
    assert_eq!(
        common("java/lang/Integer", "java/lang/Runnable").unwrap(),
        "java/lang/Object"
    );
    assert_eq!(
        common("java/lang/Runnable", "java/lang/Integer").unwrap(),
        "java/lang/Object"
    );
    assert_eq!(
        common(
            "java/lang/IndexOutOfBoundsException",
            "java/lang/AssertionError"
        )
        .unwrap(),
        "java/lang/Throwable"
    );
    let error = common("-", "java/lang/Object").unwrap_err();
    assert_eq!(error, HierarchyError::TypeNotPresent("-".into()));
    assert_eq!(error.to_string(), "Type - not present");
    let error = common("java/lang/Object", "-").unwrap_err();
    assert_eq!(error, HierarchyError::TypeNotPresent("-".into()));
    assert_eq!(error.to_string(), "Type - not present");
}

// -------------------------------------------------------------------------------------------------
// Read and write
// -------------------------------------------------------------------------------------------------

/// Tests that a ClassReader -> ClassWriter transform leaves classes unchanged.
#[test]
fn test_read_and_write() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let new_class_file = read_and_write(
            class_file,
            ClassWriterFlags::empty(),
            false,
            ParsingOptions::empty(),
            &frames_hierarchy(),
        )
        .unwrap();
        assert_eq!(
            ClassFile::new(class_file),
            ClassFile::new(&new_class_file),
            "{class_parameter}"
        );
    }
}

/// Tests that a ClassReader -> ClassWriter transform with the SKIP_CODE option produces a valid
/// class.
#[test]
fn test_read_and_write_skip_code() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        class_reader
            .accept_with_codecs(&mut class_writer, &attributes(), ParsingOptions::SKIP_CODE)
            .unwrap();

        assert!(!class_writer.has_flags(ClassWriterFlags::COMPUTE_MAXS));
        assert!(!class_writer.has_flags(ClassWriterFlags::COMPUTE_FRAMES));
        let new_class_file = class_writer.to_bytes().unwrap();
        assert!(
            ClassFile::new(&new_class_file)
                .dump_lossy()
                .unwrap()
                .contains(&class_parameter.internal_name()),
            "{class_parameter}"
        );
    }
}

/// Tests that a ClassReader -> ClassWriter transform with the copy pool option leaves classes
/// unchanged.
#[test]
fn test_read_and_write_copy_pool() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let new_class_file = read_and_write(
            class_file,
            ClassWriterFlags::empty(),
            true,
            ParsingOptions::empty(),
            &frames_hierarchy(),
        )
        .unwrap();
        assert_eq!(
            ClassFile::new(class_file),
            ClassFile::new(&new_class_file),
            "{class_parameter}"
        );
    }
}

/// Tests that a ClassReader -> ClassWriter transform with the EXPAND_FRAMES option leaves
/// classes unchanged.
#[test]
fn test_read_and_write_expand_frames() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let new_class_file = read_and_write(
            class_file,
            ClassWriterFlags::empty(),
            false,
            ParsingOptions::EXPAND_FRAMES,
            &frames_hierarchy(),
        )
        .unwrap();
        assert_eq!(
            ClassFile::new(class_file),
            ClassFile::new(&new_class_file),
            "{class_parameter}"
        );
    }
}

/// Tests that a ClassReader -> ClassWriter transform with the COMPUTE_MAXS option leaves
/// classes unchanged. This is not true in general (the valid max stack and max locals for a
/// given method are not unique), but this should be the case with our precompiled classes
/// (except jdk3.SubOptimalMaxStackAndLocals, which has non optimal max values on purpose).
#[test]
fn test_read_and_write_compute_maxs() {
    for &class_parameter in PrecompiledClass::ALL {
        if class_parameter == PrecompiledClass::Jdk3SubOptimalMaxStackAndLocals {
            continue;
        }
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
        class_reader
            .accept_with_codecs(&mut class_writer, &attributes(), ParsingOptions::empty())
            .unwrap();

        assert!(class_writer.has_flags(ClassWriterFlags::COMPUTE_MAXS));
        assert!(!class_writer.has_flags(ClassWriterFlags::COMPUTE_FRAMES));
        let new_class_file = class_writer.to_bytes().unwrap();
        assert_eq!(
            ClassFile::new(class_file),
            ClassFile::new(&new_class_file),
            "{class_parameter}"
        );
    }
}

/// Tests that classes going through a ClassReader -> ClassWriter transform with the
/// COMPUTE_MAXS option can be loaded and pass bytecode verification.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_read_and_write_compute_maxs_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = read_and_write(
            class_parameter.bytes(),
            ClassWriterFlags::COMPUTE_MAXS,
            false,
            ParsingOptions::empty(),
            &frames_hierarchy(),
        )
        .unwrap();
        assert_new_instance(class_parameter, &new_class_file);
    }
}

/// Tests that classes going through a ClassReader -> ClassWriter transform with the
/// COMPUTE_FRAMES option can be loaded and pass bytecode verification.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_read_and_write_compute_frames() {
    for &class_parameter in PrecompiledClass::ALL {
        if has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES)
            .with_hierarchy(class_loader_hierarchy());
        class_reader
            .accept_with_codecs(&mut class_writer, &attributes(), ParsingOptions::empty())
            .unwrap();
        let new_class_file = class_writer.to_bytes().unwrap();

        assert!(!class_writer.has_flags(ClassWriterFlags::COMPUTE_MAXS));
        assert!(class_writer.has_flags(ClassWriterFlags::COMPUTE_FRAMES));
        // The computed stack map frames should be equal to the original ones, if any (classes
        // before JDK8 don't have ones). This is not true in general (the valid frames for a
        // given method are not unique), but this should be the case with our precompiled
        // classes (except jdk3.SubOptimalMaxStackAndLocals, which has non optimal max values on
        // purpose). ASM's `isMoreRecentThan(Api.ASM4)`: the classes of JDK 8 and later.
        if class_parameter.jdk_version().major_version() >= 52
            && class_parameter != PrecompiledClass::Jdk3SubOptimalMaxStackAndLocals
        {
            assert_eq!(
                ClassFile::new(class_file),
                ClassFile::new(&new_class_file),
                "{class_parameter}"
            );
        }
        assert_new_instance(class_parameter, &new_class_file);
    }
}

/// Tests that classes with JSR or RET instructions are rejected by a class writer that computes
/// frames.
#[test]
fn test_read_and_write_compute_frames_jsr_instructions() {
    for &class_parameter in PrecompiledClass::ALL {
        if !has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let result = read_and_write(
            class_parameter.bytes(),
            ClassWriterFlags::COMPUTE_FRAMES,
            false,
            ParsingOptions::empty(),
            &frames_hierarchy(),
        );
        assert_eq!(
            result,
            Err(WriteError::InvalidArgument(
                "JSR/RET are not supported with computeFrames option".to_owned()
            )),
            "{class_parameter}"
        );
    }
}

/// Tests that classes going through a ClassReader -> ClassWriter transform with the SKIP_FRAMES
/// and COMPUTE_FRAMES options can be loaded and pass bytecode verification.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_read_and_write_skip_and_compute_frames() {
    for &class_parameter in PrecompiledClass::ALL {
        if has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let class_file = class_parameter.bytes();
        let new_class_file = read_and_write(
            class_file,
            ClassWriterFlags::COMPUTE_FRAMES,
            false,
            ParsingOptions::SKIP_FRAMES,
            class_loader_hierarchy(),
        )
        .unwrap();
        // See test_read_and_write_compute_frames.
        if class_parameter.jdk_version().major_version() >= 52
            && class_parameter != PrecompiledClass::Jdk3SubOptimalMaxStackAndLocals
        {
            assert_eq!(
                ClassFile::new(class_file),
                ClassFile::new(&new_class_file),
                "{class_parameter}"
            );
        }
        assert_new_instance(class_parameter, &new_class_file);
    }
}

/// Tests that classes with dead code going through a ClassWriter with the COMPUTE_FRAMES option
/// can be loaded and pass bytecode verification.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_read_and_write_compute_frames_and_dead_code() {
    for &class_parameter in PrecompiledClass::ALL {
        if has_jsr_or_ret_instructions(class_parameter) {
            continue;
        }
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES)
            .with_hierarchy(class_loader_hierarchy());
        {
            let mut class_visitor = DeadCodeInserter {
                next: &mut class_writer,
                class_name: JavaString::new(),
            };
            class_reader
                .accept_with_codecs(
                    &mut class_visitor,
                    &attributes(),
                    ParsingOptions::SKIP_FRAMES,
                )
                .unwrap();
        }
        let new_class_file = class_writer.to_bytes().unwrap();
        assert_new_instance(class_parameter, &new_class_file);
    }
}

/// The classes `test_read_and_write_large_method` transforms: those of at most 32767 bytes.
fn large_method_classes() -> impl Iterator<Item = PrecompiledClass> {
    PrecompiledClass::ALL
        .iter()
        .copied()
        .filter(|class| class.bytes().len() <= usize::from(i16::MAX.unsigned_abs()))
}

/// Inserts a wide jump in the given class (with [`ForwardJumpNopInserter`], or else with
/// [`WideForwardJumpInserter`]), with a class writer that has no class hierarchy (ASM's
/// `ClassWriterWithoutGetCommonSuperClass`: the frames must not be recomputed from scratch).
fn insert_large_method(class_file: &[u8]) -> Vec<u8> {
    let class_reader = ClassReader::new(class_file).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut forward_jump_nop_inserter = ForwardJumpNopInserter {
        next: &mut class_writer,
        transformed: false,
    };
    class_reader
        .accept_with_codecs(
            &mut forward_jump_nop_inserter,
            &attributes(),
            ParsingOptions::empty(),
        )
        .unwrap();
    if !forward_jump_nop_inserter.transformed {
        class_writer = ClassWriter::new(ClassWriterFlags::empty());
        let mut wide_forward_jump_inserter = WideForwardJumpInserter {
            next: &mut class_writer,
            need_frames: false,
            transformed: false,
        };
        class_reader
            .accept_with_codecs(
                &mut wide_forward_jump_inserter,
                &attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();
    }
    class_writer.to_bytes().unwrap()
}

/// The given class written without its code.
fn without_code(class_file: &[u8]) -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    ClassReader::new(class_file)
        .unwrap()
        .accept(&mut class_writer, ParsingOptions::SKIP_CODE)
        .unwrap();
    class_writer.to_bytes().unwrap()
}

/// Tests that classes with large methods (more than 32k) going through a ClassWriter with no
/// option keep the structure of the original classes (#317792), and that frames are not
/// recomputed from scratch during this process (the class writer has no class hierarchy).
#[test]
fn test_read_and_write_large_method() {
    for class_parameter in large_method_classes() {
        let class_file = class_parameter.bytes();
        let transformed_class = insert_large_method(class_file);

        // The transformed class should have the same structure as the original one (#317792).
        assert_eq!(
            ClassFile::new(&without_code(class_file)),
            ClassFile::new(&without_code(&transformed_class)),
            "{class_parameter}"
        );
    }
}

/// Tests that classes with large methods (more than 32k) going through a ClassWriter with no
/// option can be loaded and pass bytecode verification.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_read_and_write_large_method_new_instance() {
    for class_parameter in large_method_classes() {
        let transformed_class = insert_large_method(class_parameter.bytes());
        assert_new_instance(class_parameter, &transformed_class);
    }
}

/// A class writer whose member counts hint is replaced: by the given counts, or by none.
struct MemberCountsHint {
    counts: Option<(u16, u16)>,
    class_writer: ClassWriter<'static>,
}

impl ClassVisitor for MemberCountsHint {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.class_writer)
    }

    fn visit_member_counts(&mut self, _: u16, _: u16) {
        if let Some((fields_count, methods_count)) = self.counts {
            self.class_writer
                .visit_member_counts(fields_count, methods_count);
        }
    }
}

/// oxasm-specific: the member counts hint only sizes the class writer's member storage: with
/// the reader's hint, none, or wrong ones, every class is written the same.
#[test]
fn test_visit_member_counts_is_only_a_hint() {
    for &class in PrecompiledClass::ALL {
        let write = |counts: Option<(u16, u16)>| {
            let mut hinted = MemberCountsHint {
                counts,
                class_writer: ClassWriter::new(ClassWriterFlags::empty()),
            };
            ClassReader::new(class.bytes())
                .unwrap()
                .accept(&mut hinted, ParsingOptions::empty())
                .unwrap();
            hinted.class_writer.to_bytes().unwrap()
        };
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        ClassReader::new(class.bytes())
            .unwrap()
            .accept(&mut class_writer, ParsingOptions::empty())
            .unwrap();
        let expected = class_writer.to_bytes().unwrap();

        for counts in [None, Some((0, 0)), Some((1, 0)), Some((u16::MAX, u16::MAX))] {
            assert!(write(counts) == expected, "{class}, hint {counts:?}");
        }
    }
}

/// A class writer whose constant pool size hint is replaced: by the given size, or by none.
struct ConstantPoolSizeHint {
    size: Option<(u16, u32)>,
    class_writer: ClassWriter<'static>,
}

impl ClassVisitor for ConstantPoolSizeHint {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.class_writer)
    }

    fn visit_constant_pool_size(&mut self, _: u16, _: u32) {
        if let Some((constant_pool_count, length)) = self.size {
            self.class_writer
                .visit_constant_pool_size(constant_pool_count, length);
        }
    }
}

/// oxasm-specific: the constant pool size hint only sizes the class writer's symbol table: with
/// the reader's hint, none, or wrong ones, every class is written the same (W13 in
/// oxasm-optimization.md).
#[test]
fn test_visit_constant_pool_size_is_only_a_hint() {
    for &class in PrecompiledClass::ALL {
        let write = |size: Option<(u16, u32)>| {
            let mut hinted = ConstantPoolSizeHint {
                size,
                class_writer: ClassWriter::new(ClassWriterFlags::empty()),
            };
            ClassReader::new(class.bytes())
                .unwrap()
                .accept(&mut hinted, ParsingOptions::empty())
                .unwrap();
            hinted.class_writer.to_bytes().unwrap()
        };
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        ClassReader::new(class.bytes())
            .unwrap()
            .accept(&mut class_writer, ParsingOptions::empty())
            .unwrap();
        let expected = class_writer.to_bytes().unwrap();

        for size in [None, Some((0, 0)), Some((1, 0)), Some((u16::MAX, 1 << 16))] {
            assert!(write(size) == expected, "{class}, hint {size:?}");
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Adapters
// -------------------------------------------------------------------------------------------------

/// Java's `String.hashCode()`.
fn java_hash_code(s: &JavaStr) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// Java's `java.util.Random`, for `nextFloat`.
struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        #[allow(clippy::cast_possible_truncation)]
        let next = (self.seed >> (48 - bits)) as i32;
        next
    }

    fn next_float(&mut self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let next = self.next(24) as f32 / (1 << 24) as f32;
        next
    }
}

/// Inserts dead code in each method (ASM's `DeadCodeInserter`).
struct DeadCodeInserter<'w, 'h> {
    next: &'w mut ClassWriter<'h>,
    class_name: JavaString,
}

impl ClassVisitor for DeadCodeInserter<'_, '_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(self.next)
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
        self.class_name = name.to_owned();
        // Set V1_7 version to prevent fallback to old verifier.
        let version = if version & 0xFFFF < V1_7 {
            V1_7
        } else {
            version
        };
        self.next
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
        let mut key = self.class_name.clone();
        key.push('.');
        key.push_java_str(name);
        key.push_java_str(descriptor);
        let seed = java_hash_code(&key);
        let next = self
            .next
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(MethodDeadCodeInserter {
            next,
            random: JavaRandom::new(i64::from(seed)),
            inserted: false,
        }))
    }
}

/// ASM's `MethodDeadCodeInserter`.
struct MethodDeadCodeInserter<'a> {
    next: Box<dyn MethodVisitor + 'a>,
    random: JavaRandom,
    inserted: bool,
}

impl MethodDeadCodeInserter<'_> {
    fn maybe_insert_dead_code(&mut self) {
        // Inserts dead code once every 50 instructions in average.
        if !self.inserted && self.random.next_float() < 1.0 / 50.0 {
            self.insert_dead_code();
        }
    }

    fn insert_dead_code(&mut self) {
        let end = Label::new();
        self.visit_jump_insn(GOTO, end);
        self.visit_ldc_insn(&Constant::String(jstr!("DEAD CODE").into()));
        self.visit_label(end);
        self.inserted = true;
    }
}

impl MethodVisitor for MethodDeadCodeInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.next)
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.next.visit_insn(opcode);
        self.maybe_insert_dead_code();
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.next.visit_int_insn(opcode, operand);
        self.maybe_insert_dead_code();
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.next.visit_var_insn(opcode, var_index);
        self.maybe_insert_dead_code();
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.next.visit_type_insn(opcode, type_);
        self.maybe_insert_dead_code();
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.next.visit_field_insn(opcode, owner, name, descriptor);
        self.maybe_insert_dead_code();
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.next
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
        self.maybe_insert_dead_code();
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.next.visit_invoke_dynamic_insn(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        self.maybe_insert_dead_code();
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.next.visit_jump_insn(opcode, label);
        self.maybe_insert_dead_code();
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        // Every Constant can be passed to visit_ldc_insn (ASM checks the Object's type here).
        self.next.visit_ldc_insn(value);
        self.maybe_insert_dead_code();
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.next.visit_iinc_insn(var_index, increment);
        self.maybe_insert_dead_code();
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.next.visit_table_switch_insn(min, max, dflt, labels);
        self.maybe_insert_dead_code();
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.next.visit_lookup_switch_insn(dflt, keys, labels);
        self.maybe_insert_dead_code();
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.next
            .visit_multi_a_new_array_insn(descriptor, num_dimensions);
        self.maybe_insert_dead_code();
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        if !self.inserted {
            self.insert_dead_code();
        }
        self.next.visit_maxs(max_stack, max_locals);
    }
}

/// Inserts NOP instructions before the first backward jump found, to get a wide jump (ASM's
/// `ForwardJumpNopInserter`).
struct ForwardJumpNopInserter<'w> {
    next: &'w mut ClassWriter<'static>,
    transformed: bool,
}

impl ClassVisitor for ForwardJumpNopInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(self.next)
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let next = self
            .next
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(NopInserter {
            next,
            labels: HashSet::new(),
            transformed: &mut self.transformed,
        }))
    }
}

/// The method visitor of [`ForwardJumpNopInserter`].
struct NopInserter<'a> {
    next: Box<dyn MethodVisitor + 'a>,
    labels: HashSet<Label>,
    transformed: &'a mut bool,
}

impl MethodVisitor for NopInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.next)
    }

    fn visit_label(&mut self, label: Label) {
        self.next.visit_label(label);
        self.labels.insert(label);
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        if !*self.transformed && self.labels.contains(&label) {
            *self.transformed = true;
            for _ in 0..=i16::MAX {
                self.next.visit_insn(NOP);
            }
        }
        self.next.visit_jump_insn(opcode, label);
    }
}

/// Inserts a wide forward jump in the first non-abstract method that is found (ASM's
/// `WideForwardJumpInserter`).
struct WideForwardJumpInserter<'w> {
    next: &'w mut ClassWriter<'static>,
    need_frames: bool,
    transformed: bool,
}

impl ClassVisitor for WideForwardJumpInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(self.next)
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
        self.need_frames = version & 0xFFFF >= V1_7;
        self.next
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
        let next = self
            .next
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(WideJumpInserter {
            next,
            need_frames: self.need_frames,
            transformed: &mut self.transformed,
        }))
    }
}

/// The method visitor of [`WideForwardJumpInserter`].
struct WideJumpInserter<'a> {
    next: Box<dyn MethodVisitor + 'a>,
    need_frames: bool,
    transformed: &'a mut bool,
}

impl MethodVisitor for WideJumpInserter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.next)
    }

    fn visit_code(&mut self) {
        self.next.visit_code();
        if !*self.transformed {
            let start_label = Label::new();
            self.next.visit_jump_insn(GOTO, start_label);
            if self.need_frames {
                self.next.visit_label(Label::new());
                self.next.visit_frame(FrameKind::Same, &[], &[]);
            }
            for _ in 0..=i16::MAX {
                self.next.visit_insn(NOP);
            }
            self.next.visit_label(start_label);
            if self.need_frames {
                self.next.visit_frame(FrameKind::Same, &[], &[]);
                self.next.visit_insn(NOP);
            }
            *self.transformed = true;
        }
    }
}
