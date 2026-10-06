//! Port of `asm-test`'s `ClassFileTest` (ASM 9.10.1).

use oxasm_testkit::{ClassFile, InvalidClass, PrecompiledClass};

#[test]
fn test_get_constant_pool_dump() {
    let class_file = ClassFile::new(PrecompiledClass::Jdk3AllInstructions.bytes());

    let constant_pool_dump = class_file.constant_pool_dump().unwrap();

    let constant_pool_dump = String::from_utf8(constant_pool_dump).unwrap();
    assert!(constant_pool_dump.contains("constant_pool: ConstantClassInfo jdk3/AllInstructions"));
}

// testNewInstance_validClass and testStaticNewInstance_invalidClass are dropped: they load
// classes in a JVM (`newInstance` is not ported; the oracle's `verify` command replaces it).

/// `testNewInstance_invalidClass`: Java's `newInstance` computes the dump first, and it is that
/// which throws the `ClassFormatException` for every invalid class.
#[test]
fn test_new_instance_invalid_class() {
    for invalid_class in InvalidClass::ALL {
        let class_file = ClassFile::new(invalid_class.bytes());

        let error = class_file.dump().unwrap_err();

        assert_eq!(
            error.java_exception(),
            "org.objectweb.asm.test.ClassFormatException",
            "{invalid_class}"
        );
    }
}

#[test]
fn test_equals() {
    let class_file1 = ClassFile::new(PrecompiledClass::Jdk3AllInstructions.bytes());
    let class_file2 = ClassFile::new(PrecompiledClass::Jdk5AllInstructions.bytes());

    let equals_this = class_file1 == class_file1;
    let equals_different_class = class_file1 == class_file2;
    let equals_invalid_class = class_file1 == ClassFile::new(&[]);

    assert!(equals_this);
    assert!(!equals_different_class);
    assert!(!equals_invalid_class);
}

#[test]
fn test_hashcode_valid_class() {
    let precompiled_class = PrecompiledClass::Jdk3AllInstructions;
    let class_file = ClassFile::new(precompiled_class.bytes());

    let hash_code = class_file.hash_code().unwrap();

    assert_ne!(0, hash_code);
}

#[test]
fn test_hashcode_invalid_class() {
    let invalid_class = InvalidClass::InvalidClassVersion;
    let class_file = ClassFile::new(invalid_class.bytes());

    let error = class_file.hash_code().unwrap_err();

    assert_eq!(
        error.java_exception(),
        "org.objectweb.asm.test.ClassFormatException"
    );
    assert_eq!(
        error.message().as_deref(),
        Some("Unsupported class version")
    );
}

#[test]
fn test_to_string_valid_class() {
    let precompiled_class = PrecompiledClass::Jdk3AllInstructions;
    let class_file = ClassFile::new(precompiled_class.bytes());

    let class_string = class_file.to_string();

    assert!(class_string.contains(&precompiled_class.internal_name()));
}

#[test]
fn test_to_string_invalid_class() {
    let invalid_class = InvalidClass::InvalidClassVersion;
    let class_file = ClassFile::new(invalid_class.bytes());

    let error = class_file.dump().unwrap_err();

    assert_eq!(
        error.java_exception(),
        "org.objectweb.asm.test.ClassFormatException"
    );
    assert_eq!(
        error.message().as_deref(),
        Some("Unsupported class version")
    );
}
