//! Port of ASM's `PrinterTest`, plus tests of the `Printer` helpers.
//!
//! Dropped: `testVisitMethodInsn_asm4_unsupportedByDefault` (ADR-09: no `api` versions).

use oxasm::constant::{AnnotationValue, Constant, FrameValue};
use oxasm::opcodes::INVOKESPECIAL;
use oxasm::{Attribute, FrameKind, Handle, JavaStr, JavaString, Label, TypeReference, jstr};
use oxasm_util::printer::{
    HANDLE_TAG, OPCODES, Printer, TYPES, TextNode, UNSUPPORTED_OPERATION, append_string,
    print_list, text_to_java_string,
};

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_module_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_module(jstr!(""), 0, None);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_nest_host_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_nest_host(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_class_type_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_class_type_annotation(TypeReference::new(0), None, jstr!(""), false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_nest_member_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_nest_member(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_permitted_subclass_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_permitted_subclass(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_main_class_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_main_class(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_package_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_package(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_require_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_require(jstr!(""), 0, None);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_export_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_export(jstr!(""), 0, &[]);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_open_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_open(jstr!(""), 0, &[]);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_use_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_use(jstr!(""));
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_provide_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_provide(jstr!(""), &[]);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_module_end_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_module_end();
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_field_type_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_field_type_annotation(TypeReference::new(0), None, jstr!(""), false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_parameter_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_parameter(None, 0);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_method_type_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_method_type_annotation(TypeReference::new(0), None, jstr!(""), false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_annotable_parameter_count_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_annotable_parameter_count(0, false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_method_insn_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_method_insn(
        INVOKESPECIAL,
        jstr!("owner"),
        jstr!("name"),
        jstr!("()V"),
        false,
    );
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_method_insn_if_itf_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_method_insn(
        INVOKESPECIAL,
        jstr!("owner"),
        jstr!("name"),
        jstr!("()V"),
        true,
    );
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_insn_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_insn_annotation(TypeReference::new(0), None, jstr!(""), false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_try_catch_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_try_catch_annotation(TypeReference::new(0), None, jstr!(""), false);
}

#[test]
#[should_panic(expected = "Must be overridden")]
fn test_visit_local_variable_annotation_unsupported_by_default() {
    let mut printer = EmptyPrinter::default();
    printer.visit_local_variable_annotation(
        TypeReference::new(0),
        None,
        &[],
        &[],
        &[],
        jstr!(""),
        false,
    );
}

#[test]
fn test_unsupported_operation_message() {
    assert_eq!(UNSUPPORTED_OPERATION, "Must be overridden");
}

#[test]
fn test_tables() {
    assert_eq!(OPCODES[0], "NOP");
    assert_eq!(OPCODES[0xb6], "INVOKEVIRTUAL");
    assert_eq!(OPCODES[199], "IFNONNULL");
    assert_eq!(TYPES[oxasm::opcodes::T_LONG as usize], "T_LONG");
    assert_eq!(HANDLE_TAG[9], "H_INVOKEINTERFACE");
}

#[test]
fn test_append_string() {
    let mut sb = JavaString::new();
    append_string(&mut sb, jstr!("a\"b\\c\n\r\t\u{7f}\u{80}\u{1234}\u{1F600}"));
    assert_eq!(
        sb,
        "\"a\\\"b\\\\c\\n\\r\\u0009\u{7f}\\u0080\\u1234\\ud83d\\ude00\""
    );
}

#[test]
fn test_append_string_lone_surrogate() {
    let mut sb = JavaString::new();
    append_string(&mut sb, &JavaString::from_utf16(&[0x41, 0xd800, 0x42]));
    assert_eq!(sb, "\"A\\ud800B\"");
}

/// The text tree is printed in order, children at their position. (Printing is iterative; the
/// depth of the tree is that of nested annotations, which the class reader bounds.)
#[test]
fn test_print_list_nested() {
    let mut root = EmptyPrinter::default();
    let mut current: &mut dyn Printer = &mut root;
    for _ in 0..1000 {
        current = current.visit_annotation(None, jstr!("LA;"));
    }
    let text = text_to_java_string(root.get_text());
    assert_eq!(text.len(), 2000);
    let mut out = Vec::new();
    print_list(&mut out, root.get_text()).unwrap();
    assert_eq!(out, [b"(".repeat(1000), b")".repeat(1000)].concat());
}

#[test]
fn test_text_node_debug() {
    let mut printer = EmptyPrinter::default();
    printer.visit_annotation(None, jstr!("LA;"));
    assert_eq!(format!("{:?}", printer.get_text()), r#"["(", [], ")"]"#);
}

/// A printer implementing only the methods that `Printer` requires. Its child printers are
/// nested `EmptyPrinter`s (Java's returns `null`), enclosed in parentheses.
#[derive(Debug, Default)]
struct EmptyPrinter {
    text: Vec<TextNode>,
}

impl EmptyPrinter {
    fn child(&mut self) -> &mut dyn Printer {
        self.text.push("(".into());
        self.text
            .push(TextNode::Printer(Box::<EmptyPrinter>::default()));
        self.text.push(")".into());
        let index = self.text.len() - 2;
        match &mut self.text[index] {
            TextNode::Printer(p) => &mut **p,
            TextNode::Str(_) => unreachable!(),
        }
    }
}

impl Printer for EmptyPrinter {
    fn get_text(&self) -> &[TextNode] {
        &self.text
    }
    fn visit(
        &mut self,
        _version: u32,
        _access: u32,
        _name: &JavaStr,
        _signature: Option<&JavaStr>,
        _super_name: Option<&JavaStr>,
        _interfaces: &[&JavaStr],
    ) {
    }
    fn visit_source(&mut self, _source: Option<&JavaStr>, _debug: Option<&JavaStr>) {}
    fn visit_outer_class(
        &mut self,
        _owner: &JavaStr,
        _name: Option<&JavaStr>,
        _descriptor: Option<&JavaStr>,
    ) {
    }
    fn visit_class_annotation(
        &mut self,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_class_attribute(&mut self, _attribute: &Attribute) {}
    fn visit_inner_class(
        &mut self,
        _name: &JavaStr,
        _outer_name: Option<&JavaStr>,
        _inner_name: Option<&JavaStr>,
        _access: u32,
    ) {
    }
    fn visit_field(
        &mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _value: Option<&Constant<'_>>,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_method(
        &mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_class_end(&mut self) {}
    fn visit_annotation_value(&mut self, _name: Option<&JavaStr>, _value: &AnnotationValue<'_>) {}
    fn visit_enum(&mut self, _name: Option<&JavaStr>, _descriptor: &JavaStr, _value: &JavaStr) {}
    fn visit_annotation(
        &mut self,
        _name: Option<&JavaStr>,
        _descriptor: &JavaStr,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_array(&mut self, _name: Option<&JavaStr>) -> &mut dyn Printer {
        self.child()
    }
    fn visit_annotation_end(&mut self) {}
    fn visit_field_annotation(
        &mut self,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_field_attribute(&mut self, _attribute: &Attribute) {}
    fn visit_field_end(&mut self) {}
    fn visit_annotation_default(&mut self) -> &mut dyn Printer {
        self.child()
    }
    fn visit_method_annotation(
        &mut self,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_parameter_annotation(
        &mut self,
        _parameter: u8,
        _descriptor: &JavaStr,
        _visible: bool,
    ) -> &mut dyn Printer {
        self.child()
    }
    fn visit_method_attribute(&mut self, _attribute: &Attribute) {}
    fn visit_code(&mut self) {}
    fn visit_frame(
        &mut self,
        _kind: FrameKind,
        _local: &[FrameValue<'_>],
        _stack: &[FrameValue<'_>],
    ) {
    }
    fn visit_insn(&mut self, _opcode: u8) {}
    fn visit_int_insn(&mut self, _opcode: u8, _operand: i32) {}
    fn visit_var_insn(&mut self, _opcode: u8, _var_index: u16) {}
    fn visit_type_insn(&mut self, _opcode: u8, _type: &JavaStr) {}
    fn visit_field_insn(
        &mut self,
        _opcode: u8,
        _owner: &JavaStr,
        _name: &JavaStr,
        _descriptor: &JavaStr,
    ) {
    }
    fn visit_invoke_dynamic_insn(
        &mut self,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _bootstrap_method_handle: &Handle,
        _bootstrap_method_arguments: &[Constant<'_>],
    ) {
    }
    fn visit_jump_insn(&mut self, _opcode: u8, _label: Label) {}
    fn visit_label(&mut self, _label: Label) {}
    fn visit_ldc_insn(&mut self, _value: &Constant<'_>) {}
    fn visit_iinc_insn(&mut self, _var_index: u16, _increment: i16) {}
    fn visit_table_switch_insn(&mut self, _min: i32, _max: i32, _dflt: Label, _labels: &[Label]) {}
    fn visit_lookup_switch_insn(&mut self, _dflt: Label, _keys: &[i32], _labels: &[Label]) {}
    fn visit_multi_a_new_array_insn(&mut self, _descriptor: &JavaStr, _num_dimensions: u8) {}
    fn visit_try_catch_block(
        &mut self,
        _start: Label,
        _end: Label,
        _handler: Label,
        _type: Option<&JavaStr>,
    ) {
    }
    fn visit_local_variable(
        &mut self,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _start: Label,
        _end: Label,
        _index: u16,
    ) {
    }
    fn visit_line_number(&mut self, _line: u16, _start: Label) {}
    fn visit_maxs(&mut self, _max_stack: u16, _max_locals: u16) {}
    fn visit_method_end(&mut self) {}
}

/// oxasm-specific: the text is one string, so a surrogate pair split across two nodes is joined,
/// as Java's string concatenation does (the oracle encodes the whole output at once).
#[test]
fn test_print_list_joins_surrogate_pairs_across_nodes() {
    use oxasm::JavaString;
    use oxasm_util::TextNode;

    let high = JavaString::from_utf16(&[u16::from(b'a'), 0xD83D]);
    let low = JavaString::from_utf16(&[0xDE00, u16::from(b'b')]);
    let list = vec![TextNode::Str(high), TextNode::Str(low)];

    let mut output = Vec::new();
    oxasm_util::printer::print_list(&mut output, &list).unwrap();

    assert_eq!(output, "a\u{1F600}b".as_bytes());
}
