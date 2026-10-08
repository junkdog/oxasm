//! Port of ASM's `MethodWriterTest` and `LabelTest` (whose `getOffset` is the method writer's
//! label table here, ADR-03).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::constant::Constant;
use crate::constant_dynamic::ConstantDynamic;
use crate::handle::Handle;
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::opcodes::{self, HandleKind};
use crate::visitor::{ClassVisitor, MethodVisitor};
use crate::writer::method_writer::{Compute, MethodWriter, MethodWriterState};
use crate::writer::symbol_table::SymbolTable;
use crate::writer::{ClassWriter, ClassWriterFlags};

// -------------------------------------------------------------------------------------------------
// MethodWriterTest
// -------------------------------------------------------------------------------------------------

/// Tests that the attribute name constants of `constants` are the expected ones. This test is
/// designed to fail each time new attributes are added to `constants`, and serves as a reminder
/// to update `can_copy_method_attributes`, if needed. (ASM lists the `String` fields of
/// `Constants` by reflection; this reads the `&JavaStr` constants of `constants.rs`.)
#[test]
fn test_can_copy_method_attributes_updated() {
    let actual_attributes: BTreeSet<&str> = include_str!("../../constants.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("pub(crate) const "))
        .filter_map(|rest| rest.split_once(": &JavaStr"))
        .map(|(name, _)| name)
        .collect();
    let expected_attributes: BTreeSet<&str> = [
        "CONSTANT_VALUE",
        "CODE",
        "STACK_MAP_TABLE",
        "EXCEPTIONS",
        "INNER_CLASSES",
        "ENCLOSING_METHOD",
        "SYNTHETIC",
        "SIGNATURE",
        "SOURCE_FILE",
        "SOURCE_DEBUG_EXTENSION",
        "LINE_NUMBER_TABLE",
        "LOCAL_VARIABLE_TABLE",
        "LOCAL_VARIABLE_TYPE_TABLE",
        "DEPRECATED",
        "RUNTIME_VISIBLE_ANNOTATIONS",
        "RUNTIME_INVISIBLE_ANNOTATIONS",
        "RUNTIME_VISIBLE_PARAMETER_ANNOTATIONS",
        "RUNTIME_INVISIBLE_PARAMETER_ANNOTATIONS",
        "RUNTIME_VISIBLE_TYPE_ANNOTATIONS",
        "RUNTIME_INVISIBLE_TYPE_ANNOTATIONS",
        "ANNOTATION_DEFAULT",
        "BOOTSTRAP_METHODS",
        "METHOD_PARAMETERS",
        "MODULE",
        "MODULE_PACKAGES",
        "MODULE_MAIN_CLASS",
        "NEST_HOST",
        "NEST_MEMBERS",
        "PERMITTED_SUBCLASSES",
        "RECORD",
    ]
    .into_iter()
    .collect();
    // IMPORTANT: if this fails, update the list AND update can_copy_method_attributes(), if
    // needed.
    assert_eq!(expected_attributes, actual_attributes);
}

#[test]
fn test_recursive_condy_fast_enough() {
    let bsm = Handle::new(
        HandleKind::InvokeStatic,
        "RT",
        "bsm",
        "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/Class;I)Ljava/lang/invoke/CallSite;",
        false,
    );
    let mut chain = ConstantDynamic::new("condy", "I", bsm.clone(), vec![Constant::Integer(0)]);
    for i in 0..32 {
        chain = ConstantDynamic::new(
            format!("condy{i}"),
            "I",
            bsm.clone(),
            vec![Constant::Dynamic(Box::new(chain))],
        );
    }
    let condy = Constant::Dynamic(Box::new(chain));
    // ASM: assertTimeoutPreemptively(Duration.ofMillis(1_000), ...).
    let start = Instant::now();
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_writer.visit(
        opcodes::V11,
        opcodes::ACC_SUPER,
        JavaStr::from_str("Test"),
        None,
        Some(JavaStr::from_str("java/lang/Object")),
        &[],
    );
    {
        let mut mv = class_writer
            .visit_method(
                opcodes::ACC_STATIC,
                JavaStr::from_str("m"),
                JavaStr::from_str("()V"),
                None,
                &[],
            )
            .expect("a method writer");
        mv.visit_code();
        mv.visit_ldc_insn(&condy);
        mv.visit_maxs(0, 0);
        mv.visit_end();
    }
    class_writer.visit_end();
    class_writer.to_bytes().expect("a class file");
    assert!(start.elapsed() < Duration::from_millis(1_000));
}

/// Tests that the code length hint (an oxasm extension) only sizes the method writer's buffers:
/// with no hint, a wrong one or one larger than any method, the class is the same.
#[test]
fn test_visit_code_length_is_only_a_hint() {
    let write = |code_length: Option<u32>| {
        let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES);
        class_writer.visit(
            opcodes::V11,
            opcodes::ACC_SUPER,
            JavaStr::from_str("Test"),
            None,
            Some(JavaStr::from_str("java/lang/Object")),
            &[],
        );
        {
            let mut mv = class_writer
                .visit_method(
                    opcodes::ACC_STATIC,
                    JavaStr::from_str("m"),
                    JavaStr::from_str("(I)I"),
                    None,
                    &[],
                )
                .expect("a method writer");
            mv.visit_code();
            if let Some(code_length) = code_length {
                mv.visit_code_length(code_length);
            }
            let (zero, end) = (Label::new(), Label::new());
            mv.visit_var_insn(opcodes::ILOAD, 0);
            mv.visit_jump_insn(opcodes::IFEQ, zero);
            mv.visit_insn(opcodes::ICONST_1);
            mv.visit_jump_insn(opcodes::GOTO, end);
            mv.visit_label(zero);
            mv.visit_insn(opcodes::ICONST_0);
            mv.visit_label(end);
            mv.visit_insn(opcodes::IRETURN);
            mv.visit_maxs(0, 0);
            mv.visit_end();
        }
        class_writer.visit_end();
        class_writer.to_bytes().expect("a class file")
    };
    let expected = write(None);
    for code_length in [0, 1, 10, 65_535, u32::MAX] {
        assert_eq!(write(Some(code_length)), expected, "hint {code_length}");
    }
}

// -------------------------------------------------------------------------------------------------
// LabelTest
// -------------------------------------------------------------------------------------------------

/// A method writer state for `m()V`, computing nothing.
fn new_method_writer_state(symbol_table: &mut SymbolTable) -> MethodWriterState {
    MethodWriterState::new(
        symbol_table,
        opcodes::ACC_PUBLIC,
        JavaStr::from_str("m"),
        JavaStr::from_str("()V"),
        None,
        &[],
        Compute::Nothing,
    )
}

/// Tests that the offset of a label is correct after the label is visited (ASM's
/// `Label.getOffset`, here the method writer's `label_offset`).
#[test]
fn test_get_offset() {
    let mut symbol_table = SymbolTable::new();
    let mut state = new_method_writer_state(&mut symbol_table);
    let label = Label::new();
    {
        let mut method_visitor = MethodWriter {
            symbol_table: &mut symbol_table,
            state: &mut state,
            hierarchy: None,
        };
        method_visitor.visit_code();
        method_visitor.visit_var_insn(opcodes::ALOAD, 0);
        method_visitor.visit_label(label);
    }
    assert_eq!(state.label_offset(label), Some(1));
}

/// Tests that a label has no offset before it is visited (ASM's `getOffset` throws "Label offset
/// position has not been resolved yet").
#[test]
fn test_get_offset_illegal_state() {
    let mut symbol_table = SymbolTable::new();
    let state = new_method_writer_state(&mut symbol_table);
    assert_eq!(state.label_offset(Label::new()), None);
}

/// Tests that `Label`'s string form starts with "L".
#[test]
fn test_to_string() {
    let string = Label::new().to_string();
    assert!(string.starts_with('L'));
}
