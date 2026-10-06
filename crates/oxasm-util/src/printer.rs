//! [`Printer`]: ASM's `Printer`, an abstract converter from visit events to text (ADR-13).
//!
//! A printer collects its output in a list of [`TextNode`]s: strings, and the child printers it
//! creates for nested structures (methods, annotations, ...), whose text appears at that point.
//! Java's `List<Object> text`, holding strings and the child printers' `text` lists, becomes
//! this tree: a child printer is *owned* by its parent's text, and the `visit_*` methods that
//! start a nested structure return a mutable borrow of it.

use std::fmt;
use std::io;

use oxasm::constant::{AnnotationValue, Constant, FrameValue};
use oxasm::{Attribute, FrameKind, Handle, JavaStr, JavaString, Label, TypePath, TypeReference};

use crate::trace::TraceClassVisitor;

/// The names of the Java Virtual Machine opcodes (`Printer.OPCODES`).
pub const OPCODES: [&str; 200] = [
    "NOP",
    "ACONST_NULL",
    "ICONST_M1",
    "ICONST_0",
    "ICONST_1",
    "ICONST_2",
    "ICONST_3",
    "ICONST_4",
    "ICONST_5",
    "LCONST_0",
    "LCONST_1",
    "FCONST_0",
    "FCONST_1",
    "FCONST_2",
    "DCONST_0",
    "DCONST_1",
    "BIPUSH",
    "SIPUSH",
    "LDC",
    "LDC_W",
    "LDC2_W",
    "ILOAD",
    "LLOAD",
    "FLOAD",
    "DLOAD",
    "ALOAD",
    "ILOAD_0",
    "ILOAD_1",
    "ILOAD_2",
    "ILOAD_3",
    "LLOAD_0",
    "LLOAD_1",
    "LLOAD_2",
    "LLOAD_3",
    "FLOAD_0",
    "FLOAD_1",
    "FLOAD_2",
    "FLOAD_3",
    "DLOAD_0",
    "DLOAD_1",
    "DLOAD_2",
    "DLOAD_3",
    "ALOAD_0",
    "ALOAD_1",
    "ALOAD_2",
    "ALOAD_3",
    "IALOAD",
    "LALOAD",
    "FALOAD",
    "DALOAD",
    "AALOAD",
    "BALOAD",
    "CALOAD",
    "SALOAD",
    "ISTORE",
    "LSTORE",
    "FSTORE",
    "DSTORE",
    "ASTORE",
    "ISTORE_0",
    "ISTORE_1",
    "ISTORE_2",
    "ISTORE_3",
    "LSTORE_0",
    "LSTORE_1",
    "LSTORE_2",
    "LSTORE_3",
    "FSTORE_0",
    "FSTORE_1",
    "FSTORE_2",
    "FSTORE_3",
    "DSTORE_0",
    "DSTORE_1",
    "DSTORE_2",
    "DSTORE_3",
    "ASTORE_0",
    "ASTORE_1",
    "ASTORE_2",
    "ASTORE_3",
    "IASTORE",
    "LASTORE",
    "FASTORE",
    "DASTORE",
    "AASTORE",
    "BASTORE",
    "CASTORE",
    "SASTORE",
    "POP",
    "POP2",
    "DUP",
    "DUP_X1",
    "DUP_X2",
    "DUP2",
    "DUP2_X1",
    "DUP2_X2",
    "SWAP",
    "IADD",
    "LADD",
    "FADD",
    "DADD",
    "ISUB",
    "LSUB",
    "FSUB",
    "DSUB",
    "IMUL",
    "LMUL",
    "FMUL",
    "DMUL",
    "IDIV",
    "LDIV",
    "FDIV",
    "DDIV",
    "IREM",
    "LREM",
    "FREM",
    "DREM",
    "INEG",
    "LNEG",
    "FNEG",
    "DNEG",
    "ISHL",
    "LSHL",
    "ISHR",
    "LSHR",
    "IUSHR",
    "LUSHR",
    "IAND",
    "LAND",
    "IOR",
    "LOR",
    "IXOR",
    "LXOR",
    "IINC",
    "I2L",
    "I2F",
    "I2D",
    "L2I",
    "L2F",
    "L2D",
    "F2I",
    "F2L",
    "F2D",
    "D2I",
    "D2L",
    "D2F",
    "I2B",
    "I2C",
    "I2S",
    "LCMP",
    "FCMPL",
    "FCMPG",
    "DCMPL",
    "DCMPG",
    "IFEQ",
    "IFNE",
    "IFLT",
    "IFGE",
    "IFGT",
    "IFLE",
    "IF_ICMPEQ",
    "IF_ICMPNE",
    "IF_ICMPLT",
    "IF_ICMPGE",
    "IF_ICMPGT",
    "IF_ICMPLE",
    "IF_ACMPEQ",
    "IF_ACMPNE",
    "GOTO",
    "JSR",
    "RET",
    "TABLESWITCH",
    "LOOKUPSWITCH",
    "IRETURN",
    "LRETURN",
    "FRETURN",
    "DRETURN",
    "ARETURN",
    "RETURN",
    "GETSTATIC",
    "PUTSTATIC",
    "GETFIELD",
    "PUTFIELD",
    "INVOKEVIRTUAL",
    "INVOKESPECIAL",
    "INVOKESTATIC",
    "INVOKEINTERFACE",
    "INVOKEDYNAMIC",
    "NEW",
    "NEWARRAY",
    "ANEWARRAY",
    "ARRAYLENGTH",
    "ATHROW",
    "CHECKCAST",
    "INSTANCEOF",
    "MONITORENTER",
    "MONITOREXIT",
    "WIDE",
    "MULTIANEWARRAY",
    "IFNULL",
    "IFNONNULL",
];

/// The names of the `operand` values of the `NEWARRAY` instruction (`Printer.TYPES`).
pub const TYPES: [&str; 12] = [
    "",
    "",
    "",
    "",
    "T_BOOLEAN",
    "T_CHAR",
    "T_FLOAT",
    "T_DOUBLE",
    "T_BYTE",
    "T_SHORT",
    "T_INT",
    "T_LONG",
];

/// The names of the `tag` field values of method handles (`Printer.HANDLE_TAG`).
pub const HANDLE_TAG: [&str; 10] = [
    "",
    "H_GETFIELD",
    "H_GETSTATIC",
    "H_PUTFIELD",
    "H_PUTSTATIC",
    "H_INVOKEVIRTUAL",
    "H_INVOKESTATIC",
    "H_INVOKESPECIAL",
    "H_NEWINVOKESPECIAL",
    "H_INVOKEINTERFACE",
];

/// The message of the panics of the [`Printer`] methods that have no default behavior (the
/// `UnsupportedOperationException` message of ASM).
pub const UNSUPPORTED_OPERATION: &str = "Must be overridden";

/// An element of the text of a [`Printer`]: a string, or a child printer whose own text is
/// printed at this position.
pub enum TextNode {
    /// A piece of text. It is a [`JavaString`] because names from class files may contain
    /// unpaired surrogates, which are printed as is (as semi-UTF-8).
    Str(JavaString),
    /// A printer created for a nested structure (Java adds the child's `text` list here).
    Printer(Box<dyn Printer>),
}

impl TextNode {
    /// The string of this node, if it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&JavaStr> {
        match self {
            TextNode::Str(s) => Some(s),
            TextNode::Printer(_) => None,
        }
    }

    /// The text of the child printer of this node, if it is one.
    #[must_use]
    pub fn as_list(&self) -> Option<&[TextNode]> {
        match self {
            TextNode::Str(_) => None,
            TextNode::Printer(p) => Some(p.get_text()),
        }
    }
}

impl fmt::Debug for TextNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextNode::Str(s) => fmt::Debug::fmt(s, f),
            TextNode::Printer(p) => f.debug_list().entries(p.get_text()).finish(),
        }
    }
}

impl From<JavaString> for TextNode {
    fn from(s: JavaString) -> Self {
        TextNode::Str(s)
    }
}

impl From<&str> for TextNode {
    fn from(s: &str) -> Self {
        TextNode::Str(JavaString::from(s))
    }
}

/// An abstract converter from visit events to text (ASM's `Printer`).
///
/// The methods that ASM's `Printer` declares `abstract` are required here; the others panic with
/// [`UNSUPPORTED_OPERATION`] by default, as ASM throws an `UnsupportedOperationException`.
/// The methods that start a nested structure return the printer of that structure, which is
/// usually a new printer owned by this one's text (see [`TextNode`]).
///
/// Unlike the visitor traits, the methods have no defaults that forward: a printer is a leaf.
/// The arguments are those of the corresponding visitor methods (see [`oxasm::visitor`]).
#[allow(unused_variables)]
pub trait Printer {
    /// The text constructed by this printer (`Printer.text`/`getText()`).
    fn get_text(&self) -> &[TextNode];

    /// Prints the text constructed by this printer to the given writer.
    ///
    /// # Errors
    ///
    /// If the writer fails.
    fn print(&self, print_writer: &mut dyn io::Write) -> io::Result<()> {
        print_list(print_writer, self.get_text())
    }

    // ---------------------------------------------------------------------------------------------
    // Classes
    // ---------------------------------------------------------------------------------------------

    /// Class header. See [`oxasm::ClassVisitor::visit`].
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    );

    /// Class source. See [`oxasm::ClassVisitor::visit_source`].
    fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>);

    /// Module. See [`oxasm::ClassVisitor::visit_module`].
    fn visit_module(
        &mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Visits the nest host class of the class. See [`oxasm::ClassVisitor::visit_nest_host`].
    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Class outer class. See [`oxasm::ClassVisitor::visit_outer_class`].
    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    );

    /// Class annotation. See [`oxasm::ClassVisitor::visit_annotation`].
    fn visit_class_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer;

    /// Class type annotation. See [`oxasm::ClassVisitor::visit_type_annotation`].
    fn visit_class_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Class attribute. See [`oxasm::ClassVisitor::visit_attribute`].
    fn visit_class_attribute(&mut self, attribute: &Attribute);

    /// Visits a nest member. See [`oxasm::ClassVisitor::visit_nest_member`].
    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Visits a permitted subclass. See [`oxasm::ClassVisitor::visit_permitted_subclass`].
    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Class inner name. See [`oxasm::ClassVisitor::visit_inner_class`].
    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    );

    /// Visits a record component of the class. See
    /// [`oxasm::ClassVisitor::visit_record_component`].
    fn visit_record_component(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Class field. See [`oxasm::ClassVisitor::visit_field`].
    fn visit_field(
        &mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> &mut dyn Printer;

    /// Class method. See [`oxasm::ClassVisitor::visit_method`].
    fn visit_method(
        &mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> &mut dyn Printer;

    /// Class end. See [`oxasm::ClassVisitor::visit_end`].
    fn visit_class_end(&mut self);

    // ---------------------------------------------------------------------------------------------
    // Modules
    // ---------------------------------------------------------------------------------------------

    /// Module main class. See [`oxasm::ModuleVisitor::visit_main_class`].
    fn visit_main_class(&mut self, main_class: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module package. See [`oxasm::ModuleVisitor::visit_package`].
    fn visit_package(&mut self, packaze: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module require. See [`oxasm::ModuleVisitor::visit_require`].
    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module export. See [`oxasm::ModuleVisitor::visit_export`].
    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module open. See [`oxasm::ModuleVisitor::visit_open`].
    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module use. See [`oxasm::ModuleVisitor::visit_use`].
    fn visit_use(&mut self, service: &JavaStr) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module provide. See [`oxasm::ModuleVisitor::visit_provide`].
    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Module end. See [`oxasm::ModuleVisitor::visit_end`].
    fn visit_module_end(&mut self) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    // ---------------------------------------------------------------------------------------------
    // Annotations
    // ---------------------------------------------------------------------------------------------

    /// Annotation value (Java's `Printer.visit(String, Object)`). See
    /// [`oxasm::AnnotationVisitor::visit`].
    fn visit_annotation_value(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>);

    /// Annotation enum value. See [`oxasm::AnnotationVisitor::visit_enum`].
    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr);

    /// Nested annotation value. See [`oxasm::AnnotationVisitor::visit_annotation`].
    fn visit_annotation(
        &mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> &mut dyn Printer;

    /// Annotation array value. See [`oxasm::AnnotationVisitor::visit_array`].
    fn visit_array(&mut self, name: Option<&JavaStr>) -> &mut dyn Printer;

    /// Annotation end. See [`oxasm::AnnotationVisitor::visit_end`].
    fn visit_annotation_end(&mut self);

    // ---------------------------------------------------------------------------------------------
    // Record components
    // ---------------------------------------------------------------------------------------------

    /// Visits an annotation of the record component. See
    /// [`oxasm::RecordComponentVisitor::visit_annotation`].
    fn visit_record_component_annotation(
        &mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Visits an annotation on a type in the record component signature. See
    /// [`oxasm::RecordComponentVisitor::visit_type_annotation`].
    fn visit_record_component_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Visits a non standard attribute of the record component. See
    /// [`oxasm::RecordComponentVisitor::visit_attribute`].
    fn visit_record_component_attribute(&mut self, attribute: &Attribute) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Visits the end of the record component. See
    /// [`oxasm::RecordComponentVisitor::visit_end`].
    fn visit_record_component_end(&mut self) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    // ---------------------------------------------------------------------------------------------
    // Fields
    // ---------------------------------------------------------------------------------------------

    /// Field annotation. See [`oxasm::FieldVisitor::visit_annotation`].
    fn visit_field_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer;

    /// Field type annotation. See [`oxasm::FieldVisitor::visit_type_annotation`].
    fn visit_field_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Field attribute. See [`oxasm::FieldVisitor::visit_attribute`].
    fn visit_field_attribute(&mut self, attribute: &Attribute);

    /// Field end. See [`oxasm::FieldVisitor::visit_end`].
    fn visit_field_end(&mut self);

    // ---------------------------------------------------------------------------------------------
    // Methods
    // ---------------------------------------------------------------------------------------------

    /// Method parameter. See [`oxasm::MethodVisitor::visit_parameter`].
    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method default annotation. See [`oxasm::MethodVisitor::visit_annotation_default`].
    fn visit_annotation_default(&mut self) -> &mut dyn Printer;

    /// Method annotation. See [`oxasm::MethodVisitor::visit_annotation`].
    fn visit_method_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer;

    /// Method type annotation. See [`oxasm::MethodVisitor::visit_type_annotation`].
    fn visit_method_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Number of method parameters that can have annotations. See
    /// [`oxasm::MethodVisitor::visit_annotable_parameter_count`].
    fn visit_annotable_parameter_count(
        &mut self,
        parameter_count: u8,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method parameter annotation. See [`oxasm::MethodVisitor::visit_parameter_annotation`].
    fn visit_parameter_annotation(
        &mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer;

    /// Method attribute. See [`oxasm::MethodVisitor::visit_attribute`].
    fn visit_method_attribute(&mut self, attribute: &Attribute);

    /// Method start. See [`oxasm::MethodVisitor::visit_code`].
    fn visit_code(&mut self);

    /// Method stack frame. See [`oxasm::MethodVisitor::visit_frame`].
    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_insn`].
    fn visit_insn(&mut self, opcode: u8);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_int_insn`].
    fn visit_int_insn(&mut self, opcode: u8, operand: i32);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_var_insn`].
    fn visit_var_insn(&mut self, opcode: u8, var_index: u16);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_type_insn`].
    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_field_insn`].
    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    );

    /// Method instruction. See [`oxasm::MethodVisitor::visit_method_insn`].
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method instruction. See [`oxasm::MethodVisitor::visit_invoke_dynamic_insn`].
    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    );

    /// Method jump instruction. See [`oxasm::MethodVisitor::visit_jump_insn`].
    fn visit_jump_insn(&mut self, opcode: u8, label: Label);

    /// Method label. See [`oxasm::MethodVisitor::visit_label`].
    fn visit_label(&mut self, label: Label);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_ldc_insn`].
    fn visit_ldc_insn(&mut self, value: &Constant<'_>);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_iinc_insn`].
    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_table_switch_insn`].
    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_lookup_switch_insn`].
    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]);

    /// Method instruction. See [`oxasm::MethodVisitor::visit_multi_a_new_array_insn`].
    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8);

    /// Instruction type annotation. See [`oxasm::MethodVisitor::visit_insn_annotation`].
    fn visit_insn_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method exception handler. See [`oxasm::MethodVisitor::visit_try_catch_block`].
    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    );

    /// Try catch block type annotation. See
    /// [`oxasm::MethodVisitor::visit_try_catch_annotation`].
    fn visit_try_catch_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method debug info. See [`oxasm::MethodVisitor::visit_local_variable`].
    fn visit_local_variable(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    );

    /// Local variable type annotation. See
    /// [`oxasm::MethodVisitor::visit_local_variable_annotation`].
    #[allow(clippy::too_many_arguments)]
    fn visit_local_variable_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        panic!("{UNSUPPORTED_OPERATION}")
    }

    /// Method debug info. See [`oxasm::MethodVisitor::visit_line_number`].
    fn visit_line_number(&mut self, line: u16, start: Label);

    /// Method max stack and max locals. See [`oxasm::MethodVisitor::visit_maxs`].
    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16);

    /// Method end. See [`oxasm::MethodVisitor::visit_end`].
    fn visit_method_end(&mut self);
}

/// Prints the given text tree to the given writer (`Printer.printList`).
///
/// The text is written as one string, like Java's `PrintWriter` over the concatenated UTF-16
/// strings: a lone high surrogate at the end of one node and a lone low surrogate at the start of
/// the next one make a supplementary character.
///
/// # Errors
///
/// If the writer fails.
pub fn print_list(print_writer: &mut dyn io::Write, list: &[TextNode]) -> io::Result<()> {
    print_writer.write_all(text_to_java_string(list).as_bytes())
}

/// The text tree of a printer, flattened into one string, iteratively: the depth of the tree is
/// that of the input's nested annotations, which is input controlled (ADR-06).
#[must_use]
pub fn text_to_java_string(list: &[TextNode]) -> JavaString {
    let mut text = JavaString::new();
    let mut stack = vec![list.iter()];
    while let Some(iter) = stack.last_mut() {
        match iter.next() {
            // push_java_str joins a surrogate pair split across two nodes, as Java's string
            // concatenation does.
            Some(TextNode::Str(s)) => text.push_java_str(s),
            Some(TextNode::Printer(p)) => stack.push(p.get_text().iter()),
            None => {
                stack.pop();
            }
        }
    }
    text
}

/// Appends a quoted string to the given buffer, escaping `\n`, `\r`, `\\`, `"` and the UTF-16
/// code units outside of `0x20..=0x7f` as `\uXXXX` (`Printer.appendString`).
pub fn append_string(string_builder: &mut JavaString, string: &JavaStr) {
    string_builder.push('"');
    for c in string.encode_utf16() {
        match c {
            0x0a => string_builder.push_str("\\n"),
            0x0d => string_builder.push_str("\\r"),
            0x5c => string_builder.push_str("\\\\"),
            0x22 => string_builder.push_str("\\\""),
            c if !(0x20..=0x7f).contains(&c) => {
                string_builder.push_str(&format!("\\u{c:04x}"));
            }
            c => string_builder.push(char::from(u8::try_from(c).expect("an ASCII code unit"))),
        }
    }
    string_builder.push('"');
}

/// The `main` method shared by the printers (`Printer.main`): prints the class of the given file
/// with `printer`. `args` is `[-nodebug] <class file>`; anything else prints `usage` to `logger`.
///
/// ASM also accepts a fully qualified class name, loaded from the class path; there is no class
/// path here, so such an argument is a "Class not found" error.
///
/// # Errors
///
/// If the class file can't be read or parsed, or the output can't be written.
pub fn main(
    args: &[&str],
    usage: &str,
    printer: impl Printer,
    output: &mut dyn io::Write,
    logger: &mut dyn io::Write,
) -> io::Result<()> {
    if args.is_empty()
        || args.len() > 2
        || ((args[0] == "-debug" || args[0] == "-nodebug") && args.len() != 2)
    {
        writeln!(logger, "{usage}")?;
        return Ok(());
    }

    let (class_name, parsing_options) = if args[0] == "-nodebug" {
        (args[1], oxasm::ParsingOptions::SKIP_DEBUG)
    } else {
        (args[0], oxasm::ParsingOptions::empty())
    };

    if !(class_name.ends_with(".class") || class_name.contains('\\') || class_name.contains('/')) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Class not found"));
    }
    let class_file = std::fs::read(class_name)?;
    let mut trace_class_visitor = TraceClassVisitor::with_printer(printer, output);
    oxasm::ClassReader::new(&class_file)
        .and_then(|reader| reader.accept(&mut trace_class_visitor, parsing_options))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    trace_class_visitor.take_io_error()
}

// -------------------------------------------------------------------------------------------------
// Forwarding impls
// -------------------------------------------------------------------------------------------------

macro_rules! forward_printer {
    ($($ty:ty),*) => {$(
        impl<T: Printer + ?Sized> Printer for $ty {
            fn get_text(&self) -> &[TextNode] {
                (**self).get_text()
            }
            fn print(&self, print_writer: &mut dyn io::Write) -> io::Result<()> {
                (**self).print(print_writer)
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
                (**self).visit(version, access, name, signature, super_name, interfaces);
            }
            fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
                (**self).visit_source(source, debug);
            }
            fn visit_module(
                &mut self,
                name: &JavaStr,
                access: u32,
                version: Option<&JavaStr>,
            ) -> &mut dyn Printer {
                (**self).visit_module(name, access, version)
            }
            fn visit_nest_host(&mut self, nest_host: &JavaStr) {
                (**self).visit_nest_host(nest_host);
            }
            fn visit_outer_class(
                &mut self,
                owner: &JavaStr,
                name: Option<&JavaStr>,
                descriptor: Option<&JavaStr>,
            ) {
                (**self).visit_outer_class(owner, name, descriptor);
            }
            fn visit_class_annotation(
                &mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_class_annotation(descriptor, visible)
            }
            fn visit_class_type_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_class_type_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_class_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_class_attribute(attribute);
            }
            fn visit_nest_member(&mut self, nest_member: &JavaStr) {
                (**self).visit_nest_member(nest_member);
            }
            fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
                (**self).visit_permitted_subclass(permitted_subclass);
            }
            fn visit_inner_class(
                &mut self,
                name: &JavaStr,
                outer_name: Option<&JavaStr>,
                inner_name: Option<&JavaStr>,
                access: u32,
            ) {
                (**self).visit_inner_class(name, outer_name, inner_name, access);
            }
            fn visit_record_component(
                &mut self,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
            ) -> &mut dyn Printer {
                (**self).visit_record_component(name, descriptor, signature)
            }
            fn visit_field(
                &mut self,
                access: u32,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
                value: Option<&Constant<'_>>,
            ) -> &mut dyn Printer {
                (**self).visit_field(access, name, descriptor, signature, value)
            }
            fn visit_method(
                &mut self,
                access: u32,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
                exceptions: &[&JavaStr],
            ) -> &mut dyn Printer {
                (**self).visit_method(access, name, descriptor, signature, exceptions)
            }
            fn visit_class_end(&mut self) {
                (**self).visit_class_end();
            }
            fn visit_main_class(&mut self, main_class: &JavaStr) {
                (**self).visit_main_class(main_class);
            }
            fn visit_package(&mut self, packaze: &JavaStr) {
                (**self).visit_package(packaze);
            }
            fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
                (**self).visit_require(module, access, version);
            }
            fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
                (**self).visit_export(packaze, access, modules);
            }
            fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
                (**self).visit_open(packaze, access, modules);
            }
            fn visit_use(&mut self, service: &JavaStr) {
                (**self).visit_use(service);
            }
            fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
                (**self).visit_provide(service, providers);
            }
            fn visit_module_end(&mut self) {
                (**self).visit_module_end();
            }
            fn visit_annotation_value(
                &mut self,
                name: Option<&JavaStr>,
                value: &AnnotationValue<'_>,
            ) {
                (**self).visit_annotation_value(name, value);
            }
            fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
                (**self).visit_enum(name, descriptor, value);
            }
            fn visit_annotation(
                &mut self,
                name: Option<&JavaStr>,
                descriptor: &JavaStr,
            ) -> &mut dyn Printer {
                (**self).visit_annotation(name, descriptor)
            }
            fn visit_array(&mut self, name: Option<&JavaStr>) -> &mut dyn Printer {
                (**self).visit_array(name)
            }
            fn visit_annotation_end(&mut self) {
                (**self).visit_annotation_end();
            }
            fn visit_record_component_annotation(
                &mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_record_component_annotation(descriptor, visible)
            }
            fn visit_record_component_type_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_record_component_type_annotation(
                    type_ref, type_path, descriptor, visible,
                )
            }
            fn visit_record_component_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_record_component_attribute(attribute);
            }
            fn visit_record_component_end(&mut self) {
                (**self).visit_record_component_end();
            }
            fn visit_field_annotation(
                &mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_field_annotation(descriptor, visible)
            }
            fn visit_field_type_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_field_type_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_field_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_field_attribute(attribute);
            }
            fn visit_field_end(&mut self) {
                (**self).visit_field_end();
            }
            fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
                (**self).visit_parameter(name, access);
            }
            fn visit_annotation_default(&mut self) -> &mut dyn Printer {
                (**self).visit_annotation_default()
            }
            fn visit_method_annotation(
                &mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_method_annotation(descriptor, visible)
            }
            fn visit_method_type_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_method_type_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_annotable_parameter_count(
                &mut self,
                parameter_count: u8,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_annotable_parameter_count(parameter_count, visible)
            }
            fn visit_parameter_annotation(
                &mut self,
                parameter: u8,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_parameter_annotation(parameter, descriptor, visible)
            }
            fn visit_method_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_method_attribute(attribute);
            }
            fn visit_code(&mut self) {
                (**self).visit_code();
            }
            fn visit_frame(
                &mut self,
                kind: FrameKind,
                local: &[FrameValue<'_>],
                stack: &[FrameValue<'_>],
            ) {
                (**self).visit_frame(kind, local, stack);
            }
            fn visit_insn(&mut self, opcode: u8) {
                (**self).visit_insn(opcode);
            }
            fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
                (**self).visit_int_insn(opcode, operand);
            }
            fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
                (**self).visit_var_insn(opcode, var_index);
            }
            fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
                (**self).visit_type_insn(opcode, type_);
            }
            fn visit_field_insn(
                &mut self,
                opcode: u8,
                owner: &JavaStr,
                name: &JavaStr,
                descriptor: &JavaStr,
            ) {
                (**self).visit_field_insn(opcode, owner, name, descriptor);
            }
            fn visit_method_insn(
                &mut self,
                opcode: u8,
                owner: &JavaStr,
                name: &JavaStr,
                descriptor: &JavaStr,
                is_interface: bool,
            ) {
                (**self).visit_method_insn(opcode, owner, name, descriptor, is_interface);
            }
            fn visit_invoke_dynamic_insn(
                &mut self,
                name: &JavaStr,
                descriptor: &JavaStr,
                bootstrap_method_handle: &Handle,
                bootstrap_method_arguments: &[Constant<'_>],
            ) {
                (**self).visit_invoke_dynamic_insn(
                    name,
                    descriptor,
                    bootstrap_method_handle,
                    bootstrap_method_arguments,
                );
            }
            fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
                (**self).visit_jump_insn(opcode, label);
            }
            fn visit_label(&mut self, label: Label) {
                (**self).visit_label(label);
            }
            fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
                (**self).visit_ldc_insn(value);
            }
            fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
                (**self).visit_iinc_insn(var_index, increment);
            }
            fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
                (**self).visit_table_switch_insn(min, max, dflt, labels);
            }
            fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
                (**self).visit_lookup_switch_insn(dflt, keys, labels);
            }
            fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
                (**self).visit_multi_a_new_array_insn(descriptor, num_dimensions);
            }
            fn visit_insn_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_insn_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_try_catch_block(
                &mut self,
                start: Label,
                end: Label,
                handler: Label,
                type_: Option<&JavaStr>,
            ) {
                (**self).visit_try_catch_block(start, end, handler, type_);
            }
            fn visit_try_catch_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_try_catch_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_local_variable(
                &mut self,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
                start: Label,
                end: Label,
                index: u16,
            ) {
                (**self).visit_local_variable(name, descriptor, signature, start, end, index);
            }
            fn visit_local_variable_annotation(
                &mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                start: &[Label],
                end: &[Label],
                index: &[u16],
                descriptor: &JavaStr,
                visible: bool,
            ) -> &mut dyn Printer {
                (**self).visit_local_variable_annotation(
                    type_ref, type_path, start, end, index, descriptor, visible,
                )
            }
            fn visit_line_number(&mut self, line: u16, start: Label) {
                (**self).visit_line_number(line, start);
            }
            fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
                (**self).visit_maxs(max_stack, max_locals);
            }
            fn visit_method_end(&mut self) {
                (**self).visit_method_end();
            }
        }
    )*};
}

forward_printer!(&mut T, Box<T>);
