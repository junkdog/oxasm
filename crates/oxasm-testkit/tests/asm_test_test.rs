//! Port of `org.objectweb.asm.test.AsmTestTest`.

use std::collections::HashSet;

use oxasm_testkit::{InvalidClass, JdkVersion, PrecompiledClass};

#[test]
fn test_precompiled_class_all_methods() {
    assert_eq!(
        PrecompiledClass::Jdk3AllInstructions.name(),
        "jdk3.AllInstructions"
    );
    assert_eq!(
        PrecompiledClass::Jdk8AllInstructions.internal_name(),
        "jdk8/AllInstructions"
    );
    assert_eq!(PrecompiledClass::Jdk9Module.internal_name(), "module-info");
    // The isMoreRecentThan(Api) assertions are dropped (ADR-09: no api versions); the JDK
    // versions behind them are kept.
    assert_eq!(
        PrecompiledClass::Jdk3AllInstructions.jdk_version(),
        JdkVersion::Jdk7
    );
    assert_eq!(
        PrecompiledClass::Jdk8AllInstructions.jdk_version(),
        JdkVersion::Jdk8
    );
    assert_eq!(PrecompiledClass::Jdk9Module.jdk_version(), JdkVersion::Jdk9);
    assert_eq!(
        PrecompiledClass::Jdk11AllInstructions.jdk_version(),
        JdkVersion::Jdk11
    );
    assert!(!PrecompiledClass::Jdk11AllInstructions.bytes().is_empty());
    assert_eq!(
        PrecompiledClass::Jdk11AllInstructions.to_string(),
        "jdk11.AllInstructions"
    );
    assert_eq!(
        PrecompiledClass::Jdk14AllStructuresRecord.jdk_version(),
        JdkVersion::Jdk14
    );
    assert_eq!(
        PrecompiledClass::Jdk14AllStructuresEmptyRecord.jdk_version(),
        JdkVersion::Jdk14
    );
    assert_eq!(
        PrecompiledClass::Jdk15AllStructures.jdk_version(),
        JdkVersion::Jdk15
    );
    assert!(PrecompiledClass::Jdk15AllStructures.is_preview());
    assert!(!PrecompiledClass::Jdk11AllStructures.is_preview());
}

#[test]
fn test_invalid_class_all_methods() {
    let invalid_bytecode_offset = InvalidClass::InvalidBytecodeOffset;

    assert!(!invalid_bytecode_offset.bytes().is_empty());
    assert_eq!(
        invalid_bytecode_offset.to_string(),
        "invalid.InvalidBytecodeOffset"
    );
}

// testApi_allMethods: dropped (ADR-09: no api versions).

/// testGetAllClassesAndAllApis and testGetAllClassesAndLatestApi, without the api axis.
#[test]
fn test_get_all_classes() {
    let names: HashSet<_> = PrecompiledClass::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(names.len(), PrecompiledClass::ALL.len());
    assert_eq!(PrecompiledClass::ALL.len(), 28);

    let names: HashSet<_> = InvalidClass::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(names.len(), InvalidClass::ALL.len());
    assert_eq!(InvalidClass::ALL.len(), 14);
}

#[test]
fn precompiled_classes_are_class_files() {
    for class in PrecompiledClass::ALL {
        assert_eq!(&class.bytes()[..4], b"\xCA\xFE\xBA\xBE", "{class}");
        let major = u16::from_be_bytes([class.bytes()[6], class.bytes()[7]]);
        let minor = u16::from_be_bytes([class.bytes()[4], class.bytes()[5]]);
        assert_eq!(
            minor == 0xFFFF,
            class.is_preview(),
            "{class}: minor version {minor}"
        );
        assert!(
            u32::from(major) <= class.jdk_version().major_version() + 44,
            "{class}: major version {major}"
        );
    }
}
