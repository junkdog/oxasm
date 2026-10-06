//! The T0 corpus classes, as ASM's `AsmTest.PrecompiledClass` and `AsmTest.InvalidClass` name
//! them. The `Api` dimension of ASM's parameterized tests is gone (ADR-09): tests iterate
//! [`PrecompiledClass::ALL`] instead of `allClassesAndAllApis()`.

use std::fmt;

/// The JDK a precompiled class was compiled with (`AsmTest.JdkVersion`, without its minimum
/// `Api`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JdkVersion {
    Jdk7,
    Jdk8,
    Jdk9,
    Jdk11,
    Jdk14,
    Jdk15,
}

impl JdkVersion {
    /// The Java major version, e.g. `8`.
    pub fn major_version(self) -> u32 {
        match self {
            JdkVersion::Jdk7 => 7,
            JdkVersion::Jdk8 => 8,
            JdkVersion::Jdk9 => 9,
            JdkVersion::Jdk11 => 11,
            JdkVersion::Jdk14 => 14,
            JdkVersion::Jdk15 => 15,
        }
    }
}

macro_rules! precompiled_classes {
    ($($variant:ident = $name:literal, $path:literal, $jdk:ident, $preview:literal;)*) => {
        /// A class compiled ahead of time to cover the class file features of one JDK
        /// (`AsmTest.PrecompiledClass`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum PrecompiledClass {
            $($variant,)*
        }

        impl PrecompiledClass {
            /// Every precompiled class, in `AsmTest` order.
            pub const ALL: &[PrecompiledClass] = &[$(PrecompiledClass::$variant,)*];

            /// The fully qualified name, e.g. `jdk3.AllStructures$1` (also the golden name).
            pub fn name(self) -> &'static str {
                match self {
                    $(PrecompiledClass::$variant => $name,)*
                }
            }

            /// The JDK this class was compiled with.
            pub fn jdk_version(self) -> JdkVersion {
                match self {
                    $(PrecompiledClass::$variant => JdkVersion::$jdk,)*
                }
            }

            /// Whether this class was compiled with `--enable-preview`.
            pub fn is_preview(self) -> bool {
                match self {
                    $(PrecompiledClass::$variant => $preview,)*
                }
            }

            /// The class file content.
            pub fn bytes(self) -> &'static [u8] {
                match self {
                    $(PrecompiledClass::$variant => include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"), "/../../corpus/t0/", $path
                    )),)*
                }
            }
        }
    };
}

precompiled_classes! {
    DefaultPackage = "DefaultPackage", "DefaultPackage.class", Jdk7, false;
    Jdk3AllInstructions = "jdk3.AllInstructions", "jdk3/AllInstructions.class", Jdk7, false;
    Jdk3AllStructures = "jdk3.AllStructures", "jdk3/AllStructures.class", Jdk7, false;
    Jdk3AnonymousInnerClass = "jdk3.AllStructures$1", "jdk3/AllStructures$1.class", Jdk7, false;
    Jdk3ArtificialStructures = "jdk3.ArtificialStructures", "jdk3/ArtificialStructures.class",
        Jdk7, false;
    Jdk3InnerClass = "jdk3.AllStructures$InnerClass", "jdk3/AllStructures$InnerClass.class",
        Jdk7, false;
    Jdk3LargeMethod = "jdk3.LargeMethod", "jdk3/LargeMethod.class", Jdk7, false;
    Jdk3SubOptimalMaxStackAndLocals = "jdk3.SubOptimalMaxStackAndLocals",
        "jdk3/SubOptimalMaxStackAndLocals.class", Jdk7, false;
    Jdk5AllInstructions = "jdk5.AllInstructions", "jdk5/AllInstructions.class", Jdk7, false;
    Jdk5AllStructures = "jdk5.AllStructures", "jdk5/AllStructures.class", Jdk7, false;
    Jdk5Annotation = "jdk5.AllStructures$InvisibleAnnotation",
        "jdk5/AllStructures$InvisibleAnnotation.class", Jdk7, false;
    Jdk5Enum = "jdk5.AllStructures$EnumClass", "jdk5/AllStructures$EnumClass.class", Jdk7, false;
    Jdk5LocalClass = "jdk5.AllStructures$1LocalClass", "jdk5/AllStructures$1LocalClass.class",
        Jdk7, false;
    Jdk8AllFrames = "jdk8.AllFrames", "jdk8/AllFrames.class", Jdk8, false;
    Jdk8AllInstructions = "jdk8.AllInstructions", "jdk8/AllInstructions.class", Jdk8, false;
    Jdk8AllStructures = "jdk8.AllStructures", "jdk8/AllStructures.class", Jdk8, false;
    Jdk8AllLambdas = "jdk8.AllLambdas", "jdk8/AllLambdas.class", Jdk8, false;
    Jdk8AnonymousInnerClass = "jdk8.AllStructures$1", "jdk8/AllStructures$1.class", Jdk8, false;
    Jdk8ArtificialStructures = "jdk8.Artificial$()$Structures",
        "jdk8/Artificial$()$Structures.class", Jdk8, false;
    Jdk8InnerClass = "jdk8.AllStructures$InnerClass", "jdk8/AllStructures$InnerClass.class",
        Jdk8, false;
    Jdk8LargeMethod = "jdk8.LargeMethod", "jdk8/LargeMethod.class", Jdk8, false;
    Jdk9Module = "jdk9.module-info", "jdk9/module-info.class", Jdk9, false;
    Jdk11AllInstructions = "jdk11.AllInstructions", "jdk11/AllInstructions.class", Jdk11, false;
    Jdk11AllStructures = "jdk11.AllStructures", "jdk11/AllStructures.class", Jdk11, false;
    Jdk11AllStructuresNested = "jdk11.AllStructures$Nested", "jdk11/AllStructures$Nested.class",
        Jdk11, false;
    Jdk14AllStructuresRecord = "jdk14.AllStructures$RecordSubType",
        "jdk14/AllStructures$RecordSubType.class", Jdk14, true;
    Jdk14AllStructuresEmptyRecord = "jdk14.AllStructures$EmptyRecord",
        "jdk14/AllStructures$EmptyRecord.class", Jdk14, true;
    Jdk15AllStructures = "jdk15.AllStructures", "jdk15/AllStructures.class", Jdk15, true;
}

impl PrecompiledClass {
    /// The internal name, e.g. `jdk3/AllStructures$1`, or `module-info` for module classes.
    pub fn internal_name(self) -> String {
        let name = self.name();
        if name.ends_with(MODULE_INFO) {
            MODULE_INFO.to_owned()
        } else {
            name.replace('.', "/")
        }
    }
}

impl fmt::Display for PrecompiledClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

const MODULE_INFO: &str = "module-info";

macro_rules! invalid_classes {
    ($($variant:ident = $name:literal, $path:literal;)*) => {
        /// A class hand-edited to contain an invalid class file structure (`AsmTest.InvalidClass`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum InvalidClass {
            $($variant,)*
        }

        impl InvalidClass {
            /// Every invalid class, in `AsmTest` order.
            pub const ALL: &[InvalidClass] = &[$(InvalidClass::$variant,)*];

            /// The fully qualified name, e.g. `invalid.InvalidOpcode` (also the golden name).
            pub fn name(self) -> &'static str {
                match self {
                    $(InvalidClass::$variant => $name,)*
                }
            }

            /// The (invalid) class file content.
            pub fn bytes(self) -> &'static [u8] {
                match self {
                    $(InvalidClass::$variant => include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"), "/../../corpus/t0/", $path
                    )),)*
                }
            }
        }
    };
}

invalid_classes! {
    InvalidBytecodeOffset = "invalid.InvalidBytecodeOffset", "invalid/InvalidBytecodeOffset.clazz";
    InvalidClassVersion = "invalid.InvalidClassVersion", "invalid/InvalidClassVersion.clazz";
    InvalidCodeLength = "invalid.InvalidCodeLength", "invalid/InvalidCodeLength.clazz";
    InvalidConstantPoolIndex = "invalid.InvalidConstantPoolIndex",
        "invalid/InvalidConstantPoolIndex.clazz";
    InvalidConstantPoolReference = "invalid.InvalidConstantPoolReference",
        "invalid/InvalidConstantPoolReference.clazz";
    InvalidCpInfoTag = "invalid.InvalidCpInfoTag", "invalid/InvalidCpInfoTag.clazz";
    InvalidElementValue = "invalid.InvalidElementValue", "invalid/InvalidElementValue.clazz";
    InvalidInsnTypeAnnotationTargetType = "invalid.InvalidInsnTypeAnnotationTargetType",
        "invalid/InvalidInsnTypeAnnotationTargetType.clazz";
    InvalidOpcode = "invalid.InvalidOpcode", "invalid/InvalidOpcode.clazz";
    InvalidSourceDebugExtension = "invalid.InvalidSourceDebugExtension",
        "invalid/InvalidSourceDebugExtension.clazz";
    InvalidStackMapFrameType = "invalid.InvalidStackMapFrameType",
        "invalid/InvalidStackMapFrameType.clazz";
    InvalidTypeAnnotationTargetType = "invalid.InvalidTypeAnnotationTargetType",
        "invalid/InvalidTypeAnnotationTargetType.clazz";
    InvalidVerificationTypeInfo = "invalid.InvalidVerificationTypeInfo",
        "invalid/InvalidVerificationTypeInfo.clazz";
    InvalidWideOpcode = "invalid.InvalidWideOpcode", "invalid/InvalidWideOpcode.clazz";
}

impl fmt::Display for InvalidClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
