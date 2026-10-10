//! Port of ASM's `CheckClassAdapterTest`.
//!
//! - The panics are ASM's exceptions: their messages are the exceptions' `toString()`.
//! - `testConstructor`'s subclass half is dropped (no subclassing); `testVisit_illegalClassName`,
//!   `testVisitOuterClass_illegalOuterClassName` and `testVisitAttribute_illegalAttribute` are
//!   dropped: their `null` arguments cannot be passed.
//! - ASM's tests over all the APIs run with the latest one only (no `api`, ADR-09);
//!   `testVisitMethods_noDelegate_precompiledClass` and
//!   `testVisitMethods_noMemberDelegate_precompiledClass` are the same test here (a
//!   `NoDelegate` downstream).
//! - `verify` and `main` take a class hierarchy where ASM uses class loading: the tests use the
//!   hierarchy of T0 and the JDK, from the oracle, and the stack traces ASM prints are reduced to
//!   their first line, as oxasm's `verify` prints them.
//! - `testMain_className`: Rust has no class path, so a class *name* is a "Class not found"
//!   error; `testMain_classFile` runs on a T0 class file. `testVerify_validClass` verifies a
//!   class built like the methods of ASM's test class (the test class itself is not in T0), and
//!   the two `verify` tests compare the whole output with ASM's.

mod common;

use std::io;

use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, MethodVisitor,
    ObjectHierarchy, ParsingOptions, TypeReference, jstr,
};
use oxasm_testkit::{PrecompiledClass, panic_message, panics};
use oxasm_util::check::{self, CheckClassAdapter};
use oxasm_util::trace::NoDelegate;

use common::{assert_same_class_file, attributes, class_loader_hierarchy};

const EXPECTED_USAGE: &str = "Verifies the given class.\nUsage: CheckClassAdapter <fully qualified class name or class file name>";

const OBJECT: Option<&JavaStr> = Some(jstr!("java/lang/Object"));

/// A check adapter without a downstream visitor (ASM's `new CheckClassAdapter(null)`).
fn new_adapter() -> CheckClassAdapter<NoDelegate> {
    CheckClassAdapter::new(NoDelegate)
}

/// A check adapter that has visited the header of a class `C` of the given version.
fn visited_adapter(version: u32) -> CheckClassAdapter<NoDelegate> {
    let mut check_class_adapter = new_adapter();
    check_class_adapter.visit(version, ACC_PUBLIC, jstr!("C"), None, OBJECT, &[]);
    check_class_adapter
}

#[test]
fn test_constructor() {
    // The subclass half of ASM's test is dropped: there is no subclassing.
    assert!(!panics(new_adapter));
}

#[test]
fn test_visit_illegal_class_access_flag() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(V1_1, 1 << 20, jstr!("C"), None, OBJECT, &[]);
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid access flags: 1048576"
    );
}

// testVisit_illegalClassName: dropped, a class name cannot be null.

#[test]
fn test_visit_non_java_identifier_class_name_pre15() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(V1_4, 0, jstr!("class name"), None, OBJECT, &[]);
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid class name (must be an internal class \
         name): class name"
    );
}

#[test]
fn test_visit_non_java_identifier_class_name_post15() {
    let mut check_class_adapter = new_adapter();

    let visit_panics = panics(|| {
        check_class_adapter.visit(V1_5, 0, jstr!("class name"), None, OBJECT, &[]);
    });

    assert!(!visit_panics);
}

#[test]
fn test_visit_illegal_super_class() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(
            V1_1,
            ACC_PUBLIC,
            jstr!("java/lang/Object"),
            None,
            OBJECT,
            &[],
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: The super class name of the Object class must be \
         'null'"
    );
}

#[test]
fn test_visit_module_info_super_class() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(V1_1, ACC_PUBLIC, jstr!("module-info"), None, OBJECT, &[]);
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: The super class name of a module-info class must \
         be 'null'"
    );
}

#[test]
fn test_visit_illegal_interface_super_class() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(V1_1, ACC_INTERFACE, jstr!("I"), None, Some(jstr!("C")), &[]);
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: The super class name of interfaces must be \
         'java/lang/Object'"
    );
}

#[test]
fn test_visit_illegal_signature() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(
            V1_1,
            ACC_PUBLIC,
            jstr!("C"),
            Some(jstr!("LC;I")),
            OBJECT,
            &[],
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: LC;I: error at index 3"
    );
}

#[test]
fn test_visit_illegal_access_flag_set() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| {
        check_class_adapter.visit(
            V1_1,
            ACC_FINAL + ACC_ABSTRACT,
            jstr!("C"),
            None,
            OBJECT,
            &[],
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: final and abstract are mutually exclusive: 1040"
    );
}

#[test]
fn test_visit_illegal_multiple_calls() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        check_class_adapter.visit(V1_1, ACC_PUBLIC, jstr!("C"), None, OBJECT, &[]);
    });

    assert_eq!(
        message,
        "java.lang.IllegalStateException: visit must be called only once"
    );
}

#[test]
fn test_visit_module_illegal_module_name() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_module(jstr!("pkg.invalid=name"), 0, None);
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid module name (must be a fully qualified \
         name): pkg.invalid=name"
    );
}

#[test]
fn test_visit_module_illegal_multiple_calls() {
    let mut check_class_adapter = visited_adapter(V1_1);
    let _ = check_class_adapter.visit_module(jstr!("module1"), ACC_OPEN, None);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_module(jstr!("module2"), 0, None);
    });

    assert_eq!(
        message,
        "java.lang.IllegalStateException: visitModule can be called only once."
    );
}

#[test]
fn test_visit_source_before_start() {
    let mut check_class_adapter = new_adapter();

    let message = panic_message(|| check_class_adapter.visit_source(None, None));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot visit member before visit has been called."
    );
}

#[test]
fn test_visit_source_after_end() {
    let mut check_class_adapter = visited_adapter(V1_1);
    check_class_adapter.visit_end();

    let message = panic_message(|| check_class_adapter.visit_source(None, None));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot visit member after visitEnd has been called."
    );
}

#[test]
fn test_visit_source_illegal_multiple_calls() {
    let mut check_class_adapter = visited_adapter(V1_1);
    check_class_adapter.visit_source(None, None);

    let message = panic_message(|| check_class_adapter.visit_source(None, None));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: visitSource can be called only once."
    );
}

// testVisitOuterClass_illegalOuterClassName: dropped, an outer class owner cannot be null.

#[test]
fn test_visit_outer_class_illegal_multiple_calls() {
    let mut check_class_adapter = visited_adapter(V1_1);
    check_class_adapter.visit_outer_class(jstr!("name"), None, None);

    // ASM passes a null owner, checked after the number of calls.
    let message =
        panic_message(|| check_class_adapter.visit_outer_class(jstr!("name"), None, None));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: visitOuterClass can be called only once."
    );
}

#[test]
fn test_inner_class_illegal_inner_class_name() {
    let mut check_class_adapter = visited_adapter(V1_1);
    check_class_adapter.visit_inner_class(
        jstr!("name"),
        Some(jstr!("outerName")),
        Some(jstr!("0validInnerName")),
        0,
    );

    let message = panic_message(|| {
        check_class_adapter.visit_inner_class(
            jstr!("name"),
            Some(jstr!("outerName")),
            Some(jstr!("0illegalInnerName;")),
            0,
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid inner class name (must be a valid Java \
         identifier): 0illegalInnerName;"
    );
}

/// ASM's `@CsvSource` of the record component and field signature tests.
const ILLEGAL_SIGNATURES: [(&str, &str); 3] = [
    ("L;", "identifier expected at index 1"),
    ("LC+", "';' expected at index 3"),
    ("LC;I", "error at index 3"),
];

#[test]
fn test_visit_record_component_illegal_record_component_signatures() {
    for (invalid_signature, expected_message) in ILLEGAL_SIGNATURES {
        let mut check_class_adapter = visited_adapter(V14);

        let message = panic_message(|| {
            let _ = check_class_adapter.visit_record_component(
                jstr!("i"),
                jstr!("I"),
                Some(JavaStr::from_str(invalid_signature)),
            );
        });

        assert_eq!(
            message,
            format!("java.lang.IllegalArgumentException: {invalid_signature}: {expected_message}")
        );
    }
}

#[test]
fn test_visit_field_illegal_access_flag_set() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_field(
            ACC_PUBLIC + ACC_PRIVATE,
            jstr!("i"),
            jstr!("I"),
            None,
            None,
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: public, protected and private are mutually \
         exclusive: 3"
    );
}

#[test]
fn test_visit_field_illegal_field_signatures() {
    for (invalid_signature, expected_message) in ILLEGAL_SIGNATURES {
        let mut check_class_adapter = visited_adapter(V1_1);

        let message = panic_message(|| {
            let _ = check_class_adapter.visit_field(
                ACC_PUBLIC,
                jstr!("i"),
                jstr!("I"),
                Some(JavaStr::from_str(invalid_signature)),
                None,
            );
        });

        assert_eq!(
            message,
            format!("java.lang.IllegalArgumentException: {invalid_signature}: {expected_message}")
        );
    }
}

#[test]
fn test_visit_method_illegal_access_flag_set() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_method(
            ACC_ABSTRACT | ACC_STRICT,
            jstr!("m"),
            jstr!("()V"),
            None,
            &[],
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: strictfp and abstract are mutually exclusive: 3072"
    );
}

#[test]
fn test_visit_method_legal_access_flag_set_v17() {
    // Java 17 allows to mix ACC_ABSTRACT and ACC_STRICT because ACC_STRICT is ignored.
    let mut check_class_adapter = visited_adapter(V17);

    let visit_method_panics = panics(|| {
        let _ = check_class_adapter.visit_method(
            ACC_ABSTRACT | ACC_STRICT,
            jstr!("m"),
            jstr!("()V"),
            None,
            &[],
        );
    });

    assert!(!visit_method_panics);
}

#[test]
fn test_visit_method_illegal_signature() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_method(
            ACC_PUBLIC,
            jstr!("m"),
            jstr!("()V"),
            Some(jstr!("<T::LI.J<*+LA;>;>()V^LA;X")),
            &[],
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: <T::LI.J<*+LA;>;>()V^LA;X: error at index 24"
    );
}

/// ASM's test checks the start of the message; this checks all of it, as ASM prints it.
#[test]
fn test_visit_method_check_data_flow_by_default() {
    let mut check_class_adapter = visited_adapter(V1_1);
    let mut method_visitor = check_class_adapter
        .visit_method(ACC_PUBLIC, jstr!("m"), jstr!("(I)I"), None, &[])
        .unwrap();
    method_visitor.visit_code();
    method_visitor.visit_var_insn(ILOAD, 1);
    method_visitor.visit_var_insn(ASTORE, 0);
    method_visitor.visit_insn(IRETURN);
    method_visitor.visit_maxs(0, 0);

    let message = panic_message(|| method_visitor.visit_end());

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Error at instruction 1: Expected an object \
         reference or a return address, but found I m(I)I\n\
         00000 R I  :  :     ILOAD 1\n\
         00001 R I  : I  :     ASTORE 0\n\
         00002 ?  :     IRETURN\n\
         \n"
    );
}

/// ASM's test checks the start of the message; this checks all of it, as ASM prints it.
#[test]
fn test_visit_method_check_max_stack_and_locals_if_class_writer_without_compute_maxs() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    let mut check_class_adapter = CheckClassAdapter::new(&mut class_writer);
    check_class_adapter.visit(V1_1, ACC_PUBLIC, jstr!("C"), None, OBJECT, &[]);
    let mut method_visitor = check_class_adapter
        .visit_method(ACC_PUBLIC, jstr!("m"), jstr!("(I)I"), None, &[])
        .unwrap();
    method_visitor.visit_code();
    method_visitor.visit_var_insn(ILOAD, 1);
    method_visitor.visit_insn(IRETURN);
    method_visitor.visit_maxs(0, 2);

    let message = panic_message(|| method_visitor.visit_end());

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Error at instruction 0: Insufficient maximum stack \
         size. m(I)I\n\
         00000 R I  :  :     ILOAD 1\n\
         00001 ?   :     IRETURN\n\
         \n"
    );
}

#[test]
fn test_visit_method_no_data_flow_check_if_disabled() {
    let mut check_class_adapter = CheckClassAdapter::with_check_data_flow(NoDelegate, false);
    check_class_adapter.visit(V1_1, ACC_PUBLIC, jstr!("C"), None, OBJECT, &[]);
    let mut method_visitor = check_class_adapter
        .visit_method(ACC_PUBLIC, jstr!("m"), jstr!("()V"), None, &[])
        .unwrap();
    method_visitor.visit_code();
    method_visitor.visit_var_insn(ILOAD, 1);
    method_visitor.visit_var_insn(ASTORE, 0);
    method_visitor.visit_insn(IRETURN);
    method_visitor.visit_maxs(0, 0);

    let visit_end_panics = panics(|| method_visitor.visit_end());

    assert!(!visit_end_panics);
}

#[test]
fn test_visit_type_annotation_illegal_annotation1() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_type_annotation(
            TypeReference::new(0xFFFF_FFFF),
            None,
            jstr!("LA;"),
            true,
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type reference sort 0xff"
    );
}

#[test]
fn test_visit_type_annotation_illegal_annotation2() {
    let mut check_class_adapter = visited_adapter(V1_1);

    let message = panic_message(|| {
        let _ = check_class_adapter.visit_type_annotation(
            TypeReference::new(0x00FF_FFFF),
            None,
            jstr!("LA;"),
            true,
        );
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid type reference 0xffffff"
    );
}

// testVisitAttribute_illegalAttribute: dropped, an attribute cannot be null.

/// Tests that classes are unchanged with a ClassReader->CheckClassAdapter->ClassWriter
/// transform (with the max stack sizes and frames checked).
#[test]
fn test_visit_methods_class_writer_delegate_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_reader
            .accept_with_codecs(
                &mut CheckClassAdapter::new(&mut class_writer),
                &attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();

        assert_same_class_file(
            class_file,
            &class_writer.to_bytes().unwrap(),
            &class_parameter,
        );
    }
}

/// A class visitor that forwards everything to a class writer (ASM's anonymous
/// `ClassVisitor(api, classWriter) {}`): not a class writer itself.
struct Forwarding<'a, 'h>(&'a mut ClassWriter<'h>);

impl ClassVisitor for Forwarding<'_, '_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.0)
    }
}

/// Tests that classes are unchanged with a ClassReader->CheckClassAdapter->ClassVisitor
/// transform.
#[test]
fn test_visit_methods_non_class_writer_delegate_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_reader
            .accept_with_codecs(
                &mut CheckClassAdapter::new(Forwarding(&mut class_writer)),
                &attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();

        assert_same_class_file(
            class_file,
            &class_writer.to_bytes().unwrap(),
            &class_parameter,
        );
    }
}

/// Also ASM's `testVisitMethods_noMemberDelegate_precompiledClass`: a `ClassVisitor` without a
/// delegate is a `NoDelegate` here too.
#[test]
fn test_visit_methods_no_delegate_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();

        let accept_panics = panics(|| {
            class_reader
                .accept_with_codecs(&mut new_adapter(), &attributes(), ParsingOptions::empty())
                .unwrap();
        });

        assert!(!accept_panics, "{class_parameter}");
    }
}

#[test]
fn test_visit_methods_no_member_delegate_precompiled_class() {
    // Same as test_visit_methods_no_delegate_precompiled_class, through a `&mut NoDelegate`.
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut no_delegate = NoDelegate;

        let accept_panics = panics(|| {
            class_reader
                .accept_with_codecs(
                    &mut CheckClassAdapter::new(&mut no_delegate),
                    &attributes(),
                    ParsingOptions::empty(),
                )
                .unwrap();
        });

        assert!(!accept_panics, "{class_parameter}");
    }
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_verify_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
        let mut logger = Vec::new();

        check::verify(&class_reader, class_loader_hierarchy(), false, &mut logger).unwrap();

        assert_eq!(String::from_utf8_lossy(&logger), "", "{class_parameter}");
    }
}

#[test]
fn test_main_missing_class_name() {
    let mut logger = Vec::new();

    check::main(&[], &ObjectHierarchy, &mut logger).unwrap();

    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_too_many_arguments() {
    let mut logger = Vec::new();

    check::main(
        &[
            "org.objectweb.asm.util.CheckClassAdapterTest",
            "extraArgument",
        ],
        &ObjectHierarchy,
        &mut logger,
    )
    .unwrap();

    assert_eq!(String::from_utf8(logger).unwrap().trim(), EXPECTED_USAGE);
}

#[test]
fn test_main_class_file_not_found() {
    let mut logger = Vec::new();

    let result = check::main(&["DoNotExist.class"], &ObjectHierarchy, &mut logger);

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    assert!(logger.is_empty());
}

#[test]
fn test_main_class_not_found() {
    let mut logger = Vec::new();

    let result = check::main(&["do\\not\\exist"], &ObjectHierarchy, &mut logger);

    assert!(result.is_err());
    assert!(logger.is_empty());
}

/// Adapted: without a class path, a class name is not found.
#[test]
fn test_main_class_name() {
    let mut logger = Vec::new();

    let result = check::main(
        &["org.objectweb.asm.util.CheckClassAdapterTest"],
        &ObjectHierarchy,
        &mut logger,
    );

    assert_eq!(result.unwrap_err().to_string(), "Class not found");
    assert!(logger.is_empty());
}

/// Adapted: on a T0 class file, with the hierarchy of T0 and the JDK.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_main_class_file() {
    let path = oxasm_testkit::corpus_t0().join("jdk8/AllStructures.class");
    let mut logger = Vec::new();

    check::main(
        &[path.to_str().unwrap()],
        class_loader_hierarchy(),
        &mut logger,
    )
    .unwrap();

    assert_eq!(String::from_utf8_lossy(&logger), "");
}

/// A class with the two methods of ASM's `CheckClassAdapterTest` that its test looks at (its
/// constructor, and `methodWithObjectArrayArgument`).
fn check_class_adapter_test_class() -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
    class_writer.visit(
        V1_8,
        ACC_SUPER,
        jstr!("org/objectweb/asm/util/CheckClassAdapterTest"),
        None,
        OBJECT,
        &[],
    );
    {
        let mut method_visitor = class_writer
            .visit_method(0, jstr!("<init>"), jstr!("()V"), None, &[])
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
    {
        let mut method_visitor = class_writer
            .visit_method(
                0,
                jstr!("methodWithObjectArrayArgument"),
                jstr!("([Ljava/lang/Object;)Ljava/lang/Object;"),
                None,
                &[],
            )
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ALOAD, 1);
        method_visitor.visit_insn(ARETURN);
        method_visitor.visit_maxs(0, 0);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    class_writer.to_bytes().unwrap()
}

/// Adapted: on a class with the methods of ASM's test class that its test checks, comparing the
/// whole output with ASM's.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_verify_valid_class() {
    let class_file = check_class_adapter_test_class();
    let class_reader = ClassReader::new(&class_file).unwrap();
    let mut logger = Vec::new();

    check::verify(&class_reader, class_loader_hierarchy(), true, &mut logger).unwrap();

    let log = String::from_utf8(logger).unwrap();
    assert!(
        !log.starts_with("org.objectweb.asm.tree.analysis.AnalyzerException: Error at instruction")
    );
    assert!(log.contains("00000 CheckClassAdapterTest  :  :     ALOAD 0"));
    assert!(log.contains("00001 CheckClassAdapterTest [Object  : [Object  :     ARETURN"));
    assert_eq!(
        log,
        "<init>()V\n\
         00000 CheckClassAdapterTest  :  :     ALOAD 0\n\
         00001 CheckClassAdapterTest  : CheckClassAdapterTest  :     INVOKESPECIAL \
         java/lang/Object.<init> ()V\n\
         00002 CheckClassAdapterTest  :  :     RETURN\n\
         \n\
         methodWithObjectArrayArgument([Ljava/lang/Object;)Ljava/lang/Object;\n\
         00000 CheckClassAdapterTest [Object  :  :     ALOAD 1\n\
         00001 CheckClassAdapterTest [Object  : [Object  :     ARETURN\n\
         \n"
    );
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_verify_invalid_class() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(V1_1, ACC_PUBLIC, jstr!("C"), None, OBJECT, &[]);
    {
        let mut method_visitor = class_writer
            .visit_method(ACC_PUBLIC, jstr!("m"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();
        method_visitor.visit_var_insn(ALOAD, 0);
        method_visitor.visit_var_insn(ISTORE, 30);
        method_visitor.visit_insn(RETURN);
        method_visitor.visit_maxs(1, 31);
        method_visitor.visit_end();
    }
    class_writer.visit_end();
    let class_file = class_writer.to_bytes().unwrap();
    let class_reader = ClassReader::new(&class_file).unwrap();
    let mut logger = Vec::new();

    check::verify(&class_reader, class_loader_hierarchy(), true, &mut logger).unwrap();

    let log = String::from_utf8(logger).unwrap();
    assert!(log.starts_with(
        "org.objectweb.asm.tree.analysis.AnalyzerException: Error at instruction 1: Expected I, \
         but found LC;"
    ));
    assert_eq!(
        log,
        "org.objectweb.asm.tree.analysis.AnalyzerException: Error at instruction 1: Expected I, \
         but found LC;\n\
         m()V\n\
         00000 LC; . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .  :  :     ALOAD 0\n\
         00001 LC; . . . . . . . . . . . . . . . . . . . . . . . . . . . . . .  : LC;  :     \
         ISTORE 30\n\
         00002 ?                                 :     RETURN\n\
         \n"
    );
}
