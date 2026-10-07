//! Port of ASM's `CheckMethodAdapterTest`.
//!
//! ASM's `assertThrows(XxxException.class, ...)` + `getMessage()` become
//! [`panic_message`] assertions on the full panic message, which is the `toString()` of ASM's
//! exception (so it also checks the exception class).
//!
//! Java's `null` arrays are empty slices here (ADR-02), and `null` labels, names, attributes and
//! frame values have no counterpart; the tests that only check those are dropped, with their
//! reason. ASM's `MethodWriterWrapper` is internal: the tests that build one directly go through
//! a `CheckClassAdapter` over a `ClassWriter`, which builds it.

use std::collections::HashMap;

use oxasm::constant::{Constant, FrameValue};
use oxasm::opcodes::*;
use oxasm::{
    Attribute, ClassVisitor, ClassWriter, ClassWriterFlags, FrameKind, Handle, HandleKind, JavaStr,
    JavaString, Label, MethodVisitor, Type, TypeReference, UnknownAttribute, jstr,
};
use oxasm_testkit::panic_message;
use oxasm_util::check::{CheckClassAdapter, CheckMethodAdapter};

const IAE: &str = "java.lang.IllegalArgumentException: ";
const ISE: &str = "java.lang.IllegalStateException: ";

/// ASM's `new CheckMethodAdapter(null)`.
fn check_method_adapter() -> CheckMethodAdapter<'static> {
    CheckMethodAdapter::new(None)
}

/// The panic message of `f`, which must be an `IllegalArgumentException`'s; returns ASM's
/// `getMessage()`.
#[track_caller]
fn iae<R>(f: impl FnOnce() -> R) -> String {
    let message = panic_message(f);
    message
        .strip_prefix(IAE)
        .unwrap_or_else(|| panic!("not an IllegalArgumentException: {message}"))
        .to_owned()
}

/// The panic message of `f`, which must be an `IllegalStateException`'s; returns ASM's
/// `getMessage()`.
#[track_caller]
fn ise<R>(f: impl FnOnce() -> R) -> String {
    let message = panic_message(f);
    message
        .strip_prefix(ISE)
        .unwrap_or_else(|| panic!("not an IllegalStateException: {message}"))
        .to_owned()
}

fn j(s: &str) -> &JavaStr {
    jstr!(s)
}

/// ASM's test `Comment` attribute: any attribute will do.
fn comment() -> Attribute {
    Attribute::Unknown(UnknownAttribute {
        name: JavaString::from("Comment"),
        content: Vec::new(),
    })
}

#[test]
fn test_constructor() {
    let _ = check_method_adapter();
    // Dropped: `new CheckMethodAdapter(null) {}` and the data flow variant throw if subclassed;
    // there is no subclassing in Rust.
}

#[test]
fn test_visit_type_annotation_illegal_type_ref() {
    let mut a = check_method_adapter();
    let message = iae(|| {
        drop(a.visit_type_annotation(TypeReference::new(0xFFFF_FFFF), None, j("LA;"), true));
    });
    assert_eq!(message, "Invalid type reference sort 0xff");
}

#[test]
fn test_visit_parameter_annotation_invisible_annotation_illegal_parameter_index() {
    let mut a = check_method_adapter();
    a.visit_annotable_parameter_count(1, false);
    let message = iae(|| drop(a.visit_parameter_annotation(1, j("LA;"), false)));
    assert_eq!(message, "Invalid parameter index");
}

#[test]
fn test_visit_parameter_annotation_visible_annotation_illegal_parameter_index() {
    let mut a = check_method_adapter();
    a.visit_annotable_parameter_count(2, true);
    let message = iae(|| drop(a.visit_parameter_annotation(2, j("LA;"), true)));
    assert_eq!(message, "Invalid parameter index");
}

#[test]
fn test_visit_parameter_annotation_illegal_descriptor() {
    let mut a = check_method_adapter();
    let message = iae(|| drop(a.visit_parameter_annotation(0, j("'"), true)));
    assert_eq!(message, "Invalid descriptor: '");
}

#[test]
fn test_visit_attribute_after_end() {
    let mut a = check_method_adapter();
    a.visit_end();
    let message = ise(|| a.visit_attribute(&comment()));
    assert_eq!(
        message,
        "Cannot visit elements after visitEnd has been called."
    );
}

// test_visit_attribute_null_attribute: dropped, a null attribute is unrepresentable (`&Attribute`).

#[test]
fn test_visit_code_abstract_method() {
    let mut a = CheckMethodAdapter::with_data_flow(ACC_ABSTRACT, j("m"), j("()V"), None);
    let message = panic_message(|| a.visit_code());
    assert_eq!(
        message,
        "java.lang.UnsupportedOperationException: Abstract methods cannot have code"
    );
}

#[test]
fn test_visit_frame_illegal_frame_type() {
    // Adapted: FrameKind can only hold valid frame types, and ASM's internal F_INSERT (256),
    // which the check rejects as Java's 123 is.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_frame(FrameKind::Insert, &[], &[]));
    assert_eq!(message, "Invalid frame type 256");
}

#[test]
fn test_visit_frame_illegal_local_count() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_frame(FrameKind::Same, &[FrameValue::Integer], &[]));
    assert_eq!(message, "Invalid numLocal=1 for frame type 3");
}

#[test]
fn test_visit_frame_illegal_stack_count() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_frame(FrameKind::Same, &[], &[FrameValue::Integer]));
    assert_eq!(message, "Invalid numStack=1 for frame type 3");
}

// test_visit_frame_illegal_local_array: dropped, the slices' lengths are the counts.
// test_visit_frame_illegal_stack_array: dropped, the slices' lengths are the counts.

#[test]
fn test_visit_frame_illegal_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message =
        iae(|| a.visit_frame(FrameKind::Full, &[FrameValue::Object(j("LC;").into())], &[]));
    assert_eq!(
        message,
        "Invalid Invalid stack frame value (must be an internal class name): LC;"
    );
}

// test_visit_frame_illegal_primitive_type: dropped, a frame value is a FrameValue (no
// `new Integer(0)` that is not `Opcodes.INTEGER`).
// test_visit_frame_illegal_value_class: dropped, a frame value is a FrameValue (no Float).

#[test]
fn test_visit_frame_illegal_mixed_frame_types() {
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_frame(FrameKind::New, &[], &[]);
    a.visit_insn(NOP);
    let message = iae(|| a.visit_frame(FrameKind::Full, &[], &[]));
    assert_eq!(message, "Expanded and compressed frames must not be mixed.");
}

#[test]
fn test_visit_insn_before_start() {
    let mut a = check_method_adapter();
    let message = ise(|| a.visit_insn(NOP));
    assert_eq!(
        message,
        "Cannot visit instructions before visitCode has been called."
    );
}

#[test]
fn test_visit_insn_illegal_insn_visit_after_end() {
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_maxs(0, 0);
    let message = ise(|| a.visit_insn(NOP));
    assert_eq!(
        message,
        "Cannot visit instructions after visitMaxs has been called."
    );
}

#[test]
fn test_visit_int_insn_illegal_opcode() {
    // Adapted: an opcode is a u8, so -1 is 255 here.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_int_insn(255, 0));
    assert_eq!(message, "Invalid opcode: 255");
}

#[test]
fn test_visit_int_insn_illegal_operand() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_int_insn(NEWARRAY, 0));
    assert_eq!(
        message,
        "Invalid operand (must be an array type code T_...): 0"
    );
}

#[test]
fn test_visit_int_insn_illegal_byte_operand() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_int_insn(BIPUSH, i32::MAX));
    assert_eq!(
        message,
        "Invalid operand (must be a signed byte): 2147483647"
    );
}

#[test]
fn test_visit_int_insn_illegal_short_operand() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_int_insn(SIPUSH, i32::MAX));
    assert_eq!(
        message,
        "Invalid operand (must be a signed short): 2147483647"
    );
}

// test_visit_var_insn_illegal_operand: dropped, a local variable index is a u16.

#[test]
fn test_visit_type_insn_illegal_operand() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_type_insn(NEW, j("[I")));
    assert_eq!(message, "NEW cannot be used to create arrays: [I");
}

#[test]
fn test_visit_field_insn_null_owner() {
    // Adapted: a null owner is unrepresentable; an empty one fails with the same message.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j(""), j("i"), j("I")));
    assert_eq!(message, "Invalid owner (must not be null or empty)");
}

#[test]
fn test_visit_field_insn_invalid_owner_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("-"), j("i"), j("I")));
    assert_eq!(message, "Invalid owner (must be an internal class name): -");
}

#[test]
fn test_visit_field_insn_null_name() {
    // Adapted: a null name is unrepresentable; an empty one fails with the same message.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j(""), j("I")));
    assert_eq!(message, "Invalid name (must not be null or empty)");
}

#[test]
fn test_visit_field_insn_invalid_field_name() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("-"), j("I")));
    assert_eq!(message, "Invalid name (must be a valid Java identifier): -");
}

#[test]
fn test_visit_field_insn_invalid_field_name2() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("a-"), j("I")));
    assert_eq!(
        message,
        "Invalid name (must be a valid Java identifier): a-"
    );
}

#[test]
fn test_visit_field_insn_null_descriptor() {
    // Adapted: a null descriptor is unrepresentable; an empty one fails with the same message.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("")));
    assert_eq!(
        message,
        "Invalid type descriptor (must not be null or empty)"
    );
}

#[test]
fn test_visit_field_insn_void_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("V")));
    assert_eq!(message, "Invalid descriptor: V");
}

#[test]
fn test_visit_field_insn_invalid_primitive_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("II")));
    assert_eq!(message, "Invalid descriptor: II");
}

#[test]
fn test_visit_field_insn_illegal_array_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("[")));
    assert_eq!(message, "Invalid descriptor: [");
}

#[test]
fn test_visit_field_insn_invalid_reference_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("L")));
    assert_eq!(message, "Invalid descriptor: L");
}

#[test]
fn test_visit_field_insn_invalid_reference_descriptor2() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_field_insn(GETFIELD, j("C"), j("i"), j("L-;")));
    assert_eq!(message, "Invalid descriptor: L-;");
}

// test_deprecated_visit_method_insn_invalid_opcode: dropped, there is no deprecated
// visitMethodInsn without isInterface (ADR-09: no api versions).

#[test]
fn test_visit_method_insn_invalid_opcode() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(42, j("o"), j("m"), j("()V"), false));
    assert_eq!(
        message,
        "Invalid combination of opcode and method: 42, VISIT_METHOD_INSN"
    );
}

#[test]
fn test_visit_method_insn_null_name() {
    // Adapted: a null name is unrepresentable; an empty one fails with the same message.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j(""), j("()V"), false));
    assert_eq!(message, "Invalid name (must not be null or empty)");
}

#[test]
fn test_visit_method_insn_invalid_name() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("-"), j("()V"), false));
    assert_eq!(
        message,
        "Invalid name (must be a '<init>', '<clinit>' or a valid Java identifier): -"
    );
}

#[test]
fn test_visit_method_insn_invalid_name2() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("a-"), j("()V"), false));
    assert_eq!(
        message,
        "Invalid name (must be a '<init>', '<clinit>' or a valid Java identifier): a-"
    );
}

#[test]
fn test_visit_method_insn_null_descriptor() {
    // Adapted: a null descriptor is unrepresentable; an empty one fails with the same message.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("m"), j(""), false));
    assert_eq!(
        message,
        "Invalid method descriptor (must not be null or empty)"
    );
}

#[test]
fn test_visit_method_insn_invalid_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("m"), j("I"), false));
    assert_eq!(message, "Invalid descriptor: I");
}

#[test]
fn test_visit_method_insn_invalid_parameter_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("m"), j("(V)V"), false));
    assert_eq!(message, "Invalid descriptor: (V)V");
}

#[test]
fn test_visit_method_insn_invalid_return_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("m"), j("()VV"), false));
    assert_eq!(message, "Invalid descriptor: ()VV");
}

#[test]
fn test_visit_method_insn_illegal_invoke_interface() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEINTERFACE, j("C"), j("m"), j("()V"), false));
    assert_eq!(message, "INVOKEINTERFACE can't be used with classes");
}

#[test]
fn test_visit_method_insn_illegal_invoke_interface2() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKEVIRTUAL, j("C"), j("m"), j("()V"), true));
    assert_eq!(message, "INVOKEVIRTUAL can't be used with interfaces");
}

#[test]
fn test_visit_method_insn_illegal_invoke_special() {
    let mut a = check_method_adapter();
    a.version = V1_7;
    a.visit_code();
    let message = iae(|| a.visit_method_insn(INVOKESPECIAL, j("C"), j("m"), j("()V"), true));
    assert_eq!(
        message,
        "INVOKESPECIAL can't be used with interfaces prior to Java 8"
    );
}

#[test]
fn test_visit_method_insn_invoke_special_on_interface() {
    let mut a = check_method_adapter();
    a.version = V1_8;
    a.visit_code();
    a.visit_method_insn(INVOKESPECIAL, j("C"), j("m"), j("()V"), true);
}

#[test]
fn test_visit_invoke_dynamic_insn_illegal_handle_tag() {
    // Adapted: Java's tag is the opcode GETFIELD (180), which HandleKind cannot hold; H_GETFIELD
    // is just as invalid for a bootstrap method.
    let mut a = check_method_adapter();
    a.visit_code();
    let handle = Handle::new(HandleKind::GetField, "o", "m", "()V", false);
    let message = iae(|| a.visit_invoke_dynamic_insn(j("m"), j("()V"), &handle, &[]));
    assert_eq!(message, "invalid handle tag 1");
}

#[test]
fn test_visit_label_already_visited_label() {
    let label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(label);
    let message = ise(|| a.visit_label(label));
    assert_eq!(message, "Already visited label");
}

// test_visit_ldc_insn_v11_illegal_operand_type: dropped, an ldc operand is a Constant (no
// `new Object()`).

#[test]
fn test_visit_ldc_insn_v11_primitive_descriptor() {
    let mut a = check_method_adapter();
    a.version = V1_1;
    a.visit_code();
    let message = iae(|| a.visit_ldc_insn(&Constant::Type(Type::get_type("I"))));
    assert_eq!(message, "Illegal LDC constant value");
}

#[test]
fn test_visit_ldc_insn_v11_illegal_constant_class() {
    let mut a = check_method_adapter();
    a.version = V1_1;
    a.visit_code();
    let message = iae(|| a.visit_ldc_insn(&Constant::Type(Type::get_object_type("I"))));
    assert_eq!(
        message,
        "ldc of a constant class requires at least version 1.5"
    );
}

#[test]
fn test_visit_ldc_insn_v11_method_descriptor() {
    let mut a = check_method_adapter();
    a.version = V1_1;
    a.visit_code();
    let message = iae(|| a.visit_ldc_insn(&Constant::Type(Type::get_method_type("()V"))));
    assert_eq!(
        message,
        "ldc of a method type requires at least version 1.7"
    );
}

#[test]
fn test_visit_ldc_insn_v11_handle() {
    // Adapted: Java's tag is the opcode GETFIELD, which HandleKind cannot hold; the version
    // check comes first anyway.
    let mut a = check_method_adapter();
    a.version = V1_1;
    a.visit_code();
    let handle = Handle::new(HandleKind::GetField, "o", "m", "()V", false);
    let message = iae(|| a.visit_ldc_insn(&Constant::Handle(handle)));
    assert_eq!(message, "ldc of a Handle requires at least version 1.7");
}

// test_visit_ldc_insn_v18_invalid_handle_tag: dropped, a handle tag is a HandleKind (no -1).

#[test]
fn test_visit_ldc_insn_v18_handle() {
    let mut a = check_method_adapter();
    a.version = V1_8;
    a.visit_code();
    let handle = Handle::new(HandleKind::NewInvokeSpecial, "o", "<init>", "()V", false);
    a.visit_ldc_insn(&Constant::Handle(handle));
}

#[test]
fn test_visit_ldc_insn_v18_illegal_handle_name() {
    let mut a = check_method_adapter();
    a.version = V1_8;
    a.visit_code();
    let handle = Handle::new(HandleKind::InvokeVirtual, "o", "<init>", "()V", false);
    let message = iae(|| a.visit_ldc_insn(&Constant::Handle(handle)));
    assert_eq!(
        message,
        "Invalid handle name (must be a valid unqualified name): <init>"
    );
}

#[test]
fn test_visit_table_switch_insn_invalid_min_max() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_table_switch_insn(1, 0, Label::new(), &[]));
    assert_eq!(message, "Max = 0 must be greater than or equal to min = 1");
}

// test_visit_table_switch_insn_invalid_default_label: dropped, a null label is unrepresentable
// (Label is Copy).

#[test]
fn test_visit_table_switch_insn_null_key_labels() {
    // Adapted: a null label array is an empty slice (ADR-02).
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_table_switch_insn(0, 1, Label::new(), &[]));
    assert_eq!(message, "There must be max - min + 1 labels");
}

#[test]
fn test_visit_table_switch_insn_invalid_key_label_count() {
    let mut a = check_method_adapter();
    a.visit_code();
    let labels: [Label; 0] = [];
    let message = iae(|| a.visit_table_switch_insn(0, 1, Label::new(), &labels));
    assert_eq!(message, "There must be max - min + 1 labels");
}

// test_visit_lookup_switch_insn_null_key_array_one_label: dropped, a null key array is an empty
// slice (ADR-02), as long as the (empty) label array.
// test_visit_lookup_switch_insn_no_key_null_label_array: dropped, a null label array is an empty
// slice (ADR-02), as long as the (empty) key array.

#[test]
fn test_visit_lookup_switch_insn_no_key_one_null_label() {
    // Adapted: a null label is unrepresentable; any label makes the arrays' lengths differ.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_lookup_switch_insn(Label::new(), &[], &[Label::new()]));
    assert_eq!(message, "There must be the same number of keys and labels");
}

#[test]
fn test_visit_lookup_switch_insn_non_sorted_keys() {
    let label0 = Label::new();
    let label1 = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_lookup_switch_insn(Label::new(), &[2, 1], &[label0, label1]));
    assert_eq!(message, "The keys must be sorted in increasing order");
}

#[test]
fn test_visit_multi_a_new_array_insn_invalid_descriptor() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_multi_a_new_array_insn(j("I"), 1));
    assert_eq!(
        message,
        "Invalid descriptor (must be an array type descriptor): I"
    );
}

#[test]
fn test_visit_multi_a_new_array_insn_not_enough_dimensions() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_multi_a_new_array_insn(j("[[I"), 0));
    assert_eq!(message, "Invalid dimensions (must be greater than 0): 0");
}

#[test]
fn test_visit_multi_a_new_array_insn_too_many_dimensions() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_multi_a_new_array_insn(j("[[I"), 3));
    assert_eq!(
        message,
        "Invalid dimensions (must not be greater than numDimensions(descriptor)): 3"
    );
}

#[test]
fn test_visit_insn_annotation_invalid_type_reference() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| {
        drop(a.visit_insn_annotation(
            TypeReference::new_super_type_reference(0),
            None,
            j("LA;"),
            true,
        ));
    });
    assert_eq!(message, "Invalid type reference sort 0x10");
}

#[test]
fn test_visit_try_catch_block_after_start_label() {
    let label0 = Label::new();
    let label1 = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(label0);
    let message = ise(|| a.visit_try_catch_block(label0, label1, label1, None));
    assert_eq!(
        message,
        "Try catch blocks must be visited before their labels"
    );
}

#[test]
fn test_visit_try_catch_block_after_end_label() {
    let label0 = Label::new();
    let label1 = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(label0);
    let message = ise(|| a.visit_try_catch_block(label1, label0, label1, None));
    assert_eq!(
        message,
        "Try catch blocks must be visited before their labels"
    );
}

#[test]
fn test_visit_try_catch_block_after_handler_label() {
    let label0 = Label::new();
    let label1 = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(label0);
    let message = ise(|| a.visit_try_catch_block(label1, label1, label0, None));
    assert_eq!(
        message,
        "Try catch blocks must be visited before their labels"
    );
}

#[test]
fn test_visit_try_catch_annotation_invalid_type_reference() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| {
        drop(a.visit_try_catch_annotation(
            TypeReference::new_super_type_reference(0),
            None,
            j("LA;"),
            true,
        ));
    });
    assert_eq!(message, "Invalid type reference sort 0x10");
}

#[test]
fn test_visit_local_variable_invalid_range() {
    let start_label = Label::new();
    let end_label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(start_label);
    a.visit_insn(NOP);
    a.visit_label(end_label);
    let message = iae(|| a.visit_local_variable(j("i"), j("I"), None, end_label, start_label, 0));
    assert_eq!(
        message,
        "Invalid start and end labels (end must be greater than start)"
    );
}

#[test]
fn test_visit_local_variable_annotation_invalid_type_reference() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| {
        drop(a.visit_local_variable_annotation(
            TypeReference::new_super_type_reference(0),
            None,
            &[],
            &[],
            &[],
            j("LA;"),
            true,
        ));
    });
    assert_eq!(message, "Invalid type reference sort 0x10");
}

const INVALID_ARRAYS: &str =
    "Invalid start, end and index arrays (must be non null and of identical length";

// test_visit_local_variable_annotation_null_start_no_end_no_index: dropped, a null array is an
// empty slice (ADR-02), as long as the other (empty) arrays.
// test_visit_local_variable_annotation_no_start_null_end_no_index: dropped, likewise.

#[test]
fn test_visit_local_variable_annotation_no_start_one_null_end_no_index() {
    // Adapted: a null label is unrepresentable; any label makes the arrays' lengths differ.
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| {
        drop(a.visit_local_variable_annotation(
            TypeReference::new(u32::from(TypeReference::LOCAL_VARIABLE) << 24),
            None,
            &[],
            &[Label::new()],
            &[],
            j("LA;"),
            true,
        ));
    });
    assert_eq!(message, INVALID_ARRAYS);
}

#[test]
fn test_visit_local_variable_annotation_no_start_no_end_one_index() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| {
        drop(a.visit_local_variable_annotation(
            TypeReference::new(u32::from(TypeReference::RESOURCE_VARIABLE) << 24),
            None,
            &[],
            &[],
            &[0],
            j("LA;"),
            true,
        ));
    });
    assert_eq!(message, INVALID_ARRAYS);
}

#[test]
fn test_visit_local_variable_annotation_invalid_range() {
    let start_label = Label::new();
    let end_label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_label(start_label);
    a.visit_insn(NOP);
    a.visit_label(end_label);
    let message = iae(|| {
        drop(a.visit_local_variable_annotation(
            TypeReference::new(u32::from(TypeReference::RESOURCE_VARIABLE) << 24),
            None,
            &[end_label],
            &[start_label],
            &[0],
            j("LA;"),
            true,
        ));
    });
    assert_eq!(
        message,
        "Invalid start and end labels (end must be greater than start)"
    );
}

#[test]
fn test_visit_line_number_before_label() {
    let mut a = check_method_adapter();
    a.visit_code();
    let message = iae(|| a.visit_line_number(0, Label::new()));
    assert_eq!(message, "Invalid start label (must be visited first)");
}

#[test]
fn test_visit_maxs_unvisited_jump_labels() {
    let label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_jump_insn(IFEQ, label);
    let message = ise(|| a.visit_maxs(0, 0));
    assert_eq!(message, "Undefined label used");
}

#[test]
fn test_visit_maxs_unvisited_try_catch_labels() {
    let start_label = Label::new();
    let end_label = Label::new();
    let handler_label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_try_catch_block(start_label, end_label, handler_label, Some(j("E")));
    a.visit_label(end_label);
    a.visit_label(handler_label);
    let message = ise(|| a.visit_maxs(0, 0));
    assert_eq!(message, "Undefined label used");
}

#[test]
fn test_visit_maxs_invalid_try_catch_range() {
    let start_label = Label::new();
    let end_label = Label::new();
    let handler_label = Label::new();
    let mut a = check_method_adapter();
    a.visit_code();
    a.visit_try_catch_block(end_label, start_label, handler_label, Some(j("E")));
    a.visit_label(start_label);
    a.visit_insn(NOP);
    a.visit_label(end_label);
    a.visit_label(handler_label);
    let message = ise(|| a.visit_maxs(0, 0));
    assert_eq!(message, "Empty try catch block handler range");
}

#[test]
fn test_visit_end_invalid_data_flow() {
    let mut a = CheckMethodAdapter::with_data_flow(ACC_PUBLIC, j("m"), j("(I)I"), None);
    a.visit_code();
    a.visit_var_insn(ILOAD, 1);
    a.visit_var_insn(ASTORE, 0);
    a.visit_insn(IRETURN);
    a.visit_maxs(0, 0);
    let message = iae(|| a.visit_end());
    assert!(
        message.starts_with(
            "Error at instruction 1: Expected an object reference or a return address, but \
             found I m(I)I"
        ),
        "{message}"
    );
}

/// The outcome of a data flow checking method adapter whose downstream visitor is a method
/// writer of a class writer with the given flags, in a class of the given version (ASM's
/// `MethodWriterWrapper` tests, through a `CheckClassAdapter`, which builds the wrapper): `None`
/// if `visit_end` succeeds, or its panic message.
fn visit_end_with_class_writer(
    flags: ClassWriterFlags,
    version: u32,
    visit_code: impl FnOnce(&mut dyn MethodVisitor),
) -> Option<String> {
    let mut class_writer = ClassWriter::new(flags);
    let mut check_class_adapter = CheckClassAdapter::new(&mut class_writer);
    check_class_adapter.visit(
        version,
        ACC_PUBLIC,
        j("C"),
        None,
        Some(j("java/lang/Object")),
        &[],
    );
    let mut method_visitor = check_class_adapter
        .visit_method(ACC_PUBLIC, j("m"), j("(I)I"), None, &[])
        .expect("a check method adapter");
    visit_code(&mut *method_visitor);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| method_visitor.visit_end()))
        .err()
        .map(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "<non string panic payload>".to_owned())
        })
}

/// `ILOAD 1; IRETURN; visitMaxs(0, 2)`: the max stack is wrong.
fn iload_ireturn(mv: &mut dyn MethodVisitor) {
    mv.visit_code();
    mv.visit_var_insn(ILOAD, 1);
    mv.visit_insn(IRETURN);
    mv.visit_maxs(0, 2);
}

/// `ILOAD 1; IFEQ L; L: ICONST_0; IRETURN; visitMaxs(1, 2)`: no frame at L.
fn jump_without_frame(mv: &mut dyn MethodVisitor) {
    let label = Label::new();
    mv.visit_code();
    mv.visit_var_insn(ILOAD, 1);
    mv.visit_jump_insn(IFEQ, label);
    mv.visit_label(label);
    mv.visit_insn(ICONST_0);
    mv.visit_insn(IRETURN);
    mv.visit_maxs(1, 2);
}

#[test]
fn test_visit_end_no_invalid_max_stack_if_class_writer_with_compute_maxs() {
    let outcome = visit_end_with_class_writer(ClassWriterFlags::COMPUTE_MAXS, V1_5, iload_ireturn);
    assert_eq!(outcome, None);
}

#[test]
fn test_visit_end_no_invalid_max_stack_if_class_writer_with_compute_frames() {
    let outcome =
        visit_end_with_class_writer(ClassWriterFlags::COMPUTE_FRAMES, V1_5, iload_ireturn);
    assert_eq!(outcome, None);
}

#[test]
fn test_visit_end_invalid_max_stack_if_class_writer_without_compute_maxs_or_compute_frames() {
    let message = visit_end_with_class_writer(ClassWriterFlags::empty(), V1_5, iload_ireturn)
        .expect("visit_end must panic");
    assert!(
        message.starts_with(&format!(
            "{IAE}Error at instruction 0: Insufficient maximum stack size. m(I)I"
        )),
        "{message}"
    );
}

#[test]
fn test_visit_end_no_missing_frame_if_class_writer_with_compute_frames() {
    let outcome =
        visit_end_with_class_writer(ClassWriterFlags::COMPUTE_FRAMES, V1_7, jump_without_frame);
    assert_eq!(outcome, None);
}

#[test]
fn test_visit_end_missing_frame_if_class_writer_without_compute_frames() {
    let message =
        visit_end_with_class_writer(ClassWriterFlags::COMPUTE_MAXS, V1_7, jump_without_frame)
            .expect("visit_end must panic");
    assert!(
        message.starts_with(&format!(
            "{IAE}Error at instruction 1: Expected stack map frame at instruction 2 m(I)I"
        )),
        "{message}"
    );
}

#[test]
fn test_visit_end_invalid_return_type() {
    let mut a = CheckMethodAdapter::with_data_flow(ACC_PUBLIC, j("m"), j("(I)V"), None);
    a.visit_code();
    a.visit_var_insn(ILOAD, 1);
    a.visit_insn(IRETURN);
    a.visit_maxs(1, 2);
    let message = iae(|| a.visit_end());
    assert!(
        message.starts_with(
            "Error at instruction 1: Incompatible return type: expected null, but found I m(I)V"
        ),
        "{message}"
    );
}

/// A `CheckMethodAdapter` that shares its label indices with another one (ASM's
/// `labelInsnIndices` constructor argument; oxasm-specific).
#[test]
fn shared_label_insn_indices_detect_labels_visited_in_another_method() {
    let label = Label::new();
    let mut label_insn_indices = HashMap::new();
    {
        let mut a = CheckMethodAdapter::new(None).with_label_insn_indices(&mut label_insn_indices);
        a.visit_code();
        a.visit_label(label);
    }
    let mut b = CheckMethodAdapter::new(None).with_label_insn_indices(&mut label_insn_indices);
    b.visit_code();
    let message = ise(|| b.visit_label(label));
    assert_eq!(message, "Already visited label");
}
