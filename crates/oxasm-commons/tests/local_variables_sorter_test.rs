//! Port of ASM's `LocalVariablesSorterTest`.
//!
//! - ASM's `new ClassFile(bytes).newInstance()` checks (the JVM loads and verifies the class) run
//!   the oracle's `verify` (`oxasm_testkit::jvm`): they are in `#[ignore = "needs the JVM
//!   oracle"]` tests, run by CI's oracle job with `--ignored`.
//! - The `Api` dimension of the parameterized tests is gone (ADR-09).
//!
//! Dropped: the second half of `testConstructor` (a subclass calling the public constructor
//! throws an `IllegalStateException`): there is no subclassing (ADR-13).

mod common;

use oxasm::constant::FrameValue;
use oxasm::opcodes::*;
use oxasm::{
    ClassReader, ClassWriter, ClassWriterFlags, FrameKind, MethodVisitor, ParsingOptions, Type,
    jstr,
};
use oxasm_commons::LocalVariablesSorter;
use oxasm_testkit::{PrecompiledClass, panic_message, t0_resource};
use oxasm_tree::MethodNode;

use common::{
    LocalVariablesSorterClassAdapter, assert_new_instance, assert_precompiled_new_instance,
};

#[test]
fn test_constructor() {
    let mut method_node = MethodNode::default();

    let _ = LocalVariablesSorter::new(ACC_PUBLIC, jstr!("()V"), &mut method_node);
}

#[test]
fn test_visit_frame_empty_frame() {
    let mut local_variables_sorter =
        LocalVariablesSorter::new(ACC_PUBLIC, jstr!("()V"), MethodNode::default());

    local_variables_sorter.visit_frame(FrameKind::New, &[], &[]);
}

#[test]
fn test_visit_frame_invalid_frame_type() {
    let mut local_variables_sorter =
        LocalVariablesSorter::new(ACC_PUBLIC, jstr!("()V"), MethodNode::default());

    let message = panic_message(|| local_variables_sorter.visit_frame(FrameKind::Full, &[], &[]));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: \
         LocalVariablesSorter only accepts expanded frames (see ClassReader.EXPAND_FRAMES)"
    );
}

fn new_local_next_local(type_: &Type) -> u32 {
    let mut local_variables_sorter =
        LocalVariablesSorter::new(ACC_STATIC, jstr!("()V"), MethodNode::default());

    local_variables_sorter.new_local(type_);

    local_variables_sorter.next_local()
}

#[test]
fn test_new_local_boolean() {
    assert_eq!(new_local_next_local(&Type::BOOLEAN), 1);
}

#[test]
fn test_new_local_byte() {
    assert_eq!(new_local_next_local(&Type::BYTE), 1);
}

#[test]
fn test_new_local_char() {
    assert_eq!(new_local_next_local(&Type::CHAR), 1);
}

#[test]
fn test_new_local_short() {
    assert_eq!(new_local_next_local(&Type::SHORT), 1);
}

#[test]
fn test_new_local_int() {
    assert_eq!(new_local_next_local(&Type::INT), 1);
}

#[test]
fn test_new_local_float() {
    assert_eq!(new_local_next_local(&Type::FLOAT), 1);
}

#[test]
fn test_new_local_long() {
    assert_eq!(new_local_next_local(&Type::LONG), 2);
}

#[test]
fn test_new_local_double() {
    assert_eq!(new_local_next_local(&Type::DOUBLE), 2);
}

#[test]
fn test_new_local_object() {
    assert_eq!(new_local_next_local(&Type::get_object_type("pkg/Class")), 1);
}

#[test]
fn test_new_local_array() {
    assert_eq!(new_local_next_local(&Type::get_type("[I")), 1);
}

/// Reads `class_file` with `EXPAND_FRAMES` through a [`LocalVariablesSorterClassAdapter`] into a
/// `ClassWriter(0)`.
fn sort_local_variables(class_file: &[u8]) -> Vec<u8> {
    let class_reader = ClassReader::new(class_file).unwrap();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_reader
        .accept(
            &mut LocalVariablesSorterClassAdapter(&mut class_writer),
            ParsingOptions::EXPAND_FRAMES,
        )
        .unwrap();
    class_writer.to_bytes().unwrap()
}

/// The first half of ASM's `testAllMethods_precompiledClass`: the transformation succeeds.
#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        sort_local_variables(class_parameter.bytes());
    }
}

/// The second half of ASM's `testAllMethods_precompiledClass`: the JVM verifies the result.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_precompiled_class_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = sort_local_variables(class_parameter.bytes());
        assert_precompiled_new_instance(class_parameter, &class_file);
    }
}

#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_issue317586_new_instance() {
    let class_file = sort_local_variables(&t0_resource("asm-commons", "Issue317586.class"));

    assert_new_instance(&class_file);
}

/// oxasm-specific: the frames of new locals, and the removal of the TOPs after long and double
/// values and of the trailing TOPs.
#[test]
fn test_visit_frame_remaps_and_compacts_locals() {
    let mut method_node = MethodNode::default();
    {
        let mut sorter = LocalVariablesSorter::new(ACC_STATIC, jstr!("(I)V"), &mut method_node);
        sorter.visit_code();
        let new_local = sorter.new_local(&Type::LONG);
        assert_eq!(new_local, 1);
        sorter.visit_var_insn(ALOAD, 1);
        sorter.visit_frame(
            FrameKind::New,
            &[
                FrameValue::Integer,
                FrameValue::Object(jstr!("A").into()),
                FrameValue::Double,
                FrameValue::Top,
            ],
            &[FrameValue::Null],
        );
    }
    let frame = method_node
        .instructions
        .iter()
        .find_map(|node| match &node.insn {
            oxasm_tree::Insn::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        frame.local,
        [
            FrameValue::Integer,
            FrameValue::Long,
            FrameValue::Object(jstr!("A").into()),
            FrameValue::Double,
        ]
    );
    assert_eq!(frame.stack, [FrameValue::Null]);
}
