//! [`Textifier`]: ASM's `Textifier`, a [`Printer`] that prints a disassembled view of the classes
//! it visits.
//!
//! The output is byte for byte ASM 9.10.1's, which makes it the oracle of the class reader
//! (oxasm-verification, "Text parity"). Floating point values are printed as Java prints them
//! ([`crate::java_fmt`]), and strings are escaped per UTF-16 code unit.
//!
//! # Divergences on malformed input
//!
//! Where ASM's Textifier throws on events that only a malformed class file produces, this one
//! prints something instead, because visitors cannot fail (ADR-06) and printing untrusted classes
//! must not panic:
//!
//! - a malformed signature prints the `// declaration:` of its valid prefix;
//! - a `NEWARRAY` operand outside of `T_BOOLEAN..=T_LONG` prints as a number (ASM indexes
//!   `TYPES` with it), and so does an unknown opcode;
//! - a class constant whose descriptor is a method descriptor prints its descriptor (ASM's
//!   `Type.getClassName` fails on it).
//!
//! ASM throws `IllegalArgumentException` for a frame of kind `F_INSERT` and for an invalid
//! type reference sort; those cannot come from the class reader, and panic here.

use rustc_hash::FxHashMap;

use oxasm::constant::{AnnotationValue, Constant, FrameValue};
use oxasm::signature::SignatureReader;
use oxasm::{
    Attribute, ConstantDynamic, FrameKind, Handle, HandleKind, JavaStr, JavaString, Label, Type,
    TypePath, TypeReference, TypeSort, opcodes,
};

use crate::java_fmt::{double_to_string, float_to_string};
use crate::printer::{self, OPCODES, Printer, TYPES, TextNode, append_string};
use crate::trace::TraceSignatureVisitor;

/// The usage message of [`main`].
const USAGE: &str = "Prints a disassembled view of the given class.\n\
    Usage: Textifier [-nodebug] <fully qualified class name or class file name>";

/// The type of internal names (see [`Textifier::append_descriptor`]).
pub const INTERNAL_NAME: i32 = 0;
/// The type of field descriptors.
pub const FIELD_DESCRIPTOR: i32 = 1;
/// The type of field signatures.
pub const FIELD_SIGNATURE: i32 = 2;
/// The type of method descriptors.
pub const METHOD_DESCRIPTOR: i32 = 3;
/// The type of method signatures.
pub const METHOD_SIGNATURE: i32 = 4;
/// The type of class signatures.
pub const CLASS_SIGNATURE: i32 = 5;
/// The type of method handle descriptors.
pub const HANDLE_DESCRIPTOR: i32 = 9;

const CLASS_SUFFIX: &str = ".class";
const DEPRECATED: &str = "// DEPRECATED\n";
const RECORD: &str = "// RECORD\n";
const INVISIBLE: &str = " // invisible\n";

/// A [`Printer`] that prints a disassembled view of the classes it visits.
#[derive(Debug)]
pub struct Textifier {
    /// The text constructed by this printer.
    pub text: Vec<TextNode>,
    /// Tab for class members.
    pub tab: String,
    /// Tab for bytecode instructions.
    pub tab2: String,
    /// Tab for table and lookup switch instructions.
    pub tab3: String,
    /// Tab for labels.
    pub ltab: String,
    /// The label names, by first appearance (`L0`, `L1`, ...). Each printer has its own, so
    /// labels are named per method.
    pub label_names: FxHashMap<Label, String>,
    /// The access flags of the visited class.
    access: u32,
    /// The number of annotation values visited so far.
    num_annotation_values: usize,
}

impl Default for Textifier {
    fn default() -> Self {
        Textifier::new()
    }
}

impl Textifier {
    /// Constructs a new Textifier.
    #[must_use]
    pub fn new() -> Textifier {
        Textifier {
            text: Vec::new(),
            tab: "  ".to_owned(),
            tab2: "    ".to_owned(),
            tab3: "      ".to_owned(),
            ltab: "   ".to_owned(),
            label_names: FxHashMap::default(),
            access: 0,
            num_annotation_values: 0,
        }
    }

    /// Prints the given class to `output` (`Textifier.main`): `args` is
    /// `[-nodebug] <class file>`. See [`printer::main`].
    ///
    /// # Errors
    ///
    /// If the class file can't be read or parsed, or the output can't be written.
    pub fn main(
        args: &[&str],
        output: &mut dyn std::io::Write,
        logger: &mut dyn std::io::Write,
    ) -> std::io::Result<()> {
        printer::main(args, USAGE, Textifier::new(), output, logger)
    }

    // ---------------------------------------------------------------------------------------------
    // Common methods
    // ---------------------------------------------------------------------------------------------

    /// Prints a disassembled view of the given annotation (Textifier's public
    /// `visitAnnotation(descriptor, visible)`, which the class, field, method and record
    /// component annotation methods share).
    pub fn visit_member_annotation(
        &mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push('@');
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.text.push(TextNode::Str(sb));
        self.add_new_textifier(Some(
            if visible { ")\n" } else { ") // invisible\n" }.into(),
        ))
    }

    /// Prints a disassembled view of the given type annotation.
    pub fn visit_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push('@');
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.text.push(TextNode::Str(sb));
        let mut sb = JavaString::new();
        sb.push_str(") : ");
        append_type_reference(&mut sb, type_ref);
        sb.push_str(", ");
        append_type_path(&mut sb, type_path);
        sb.push_str(if visible { "\n" } else { INVISIBLE });
        self.add_new_textifier(Some(sb))
    }

    /// Prints a disassembled view of the given attribute.
    pub fn visit_attribute(&mut self, attribute: &Attribute) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("ATTRIBUTE ");
        Self::append_descriptor(&mut sb, -1, Some(attribute.name()));
        let textified = match attribute {
            Attribute::Custom(custom) => {
                let label_names = &mut self.label_names;
                custom.textify(&mut |label| label_name(label_names, label))
            }
            Attribute::Unknown(_) => None,
        };
        match textified {
            Some(text) => sb.push_str(&text),
            None => sb.push_str(" : unknown\n"),
        }
        self.text.push(TextNode::Str(sb));
    }

    // ---------------------------------------------------------------------------------------------
    // Utility methods
    // ---------------------------------------------------------------------------------------------

    /// Appends an internal name, a type descriptor or a type signature to the given string
    /// (`Textifier.appendDescriptor`). Signatures are printed as `// signature <value>` lines,
    /// other values as is, `null` for `None`.
    pub fn append_descriptor(sb: &mut JavaString, type_: i32, value: Option<&JavaStr>) {
        if type_ == CLASS_SIGNATURE || type_ == FIELD_SIGNATURE || type_ == METHOD_SIGNATURE {
            if let Some(value) = value {
                sb.push_str("// signature ");
                sb.push_java_str(value);
                sb.push('\n');
            }
        } else {
            append_nullable(sb, value);
        }
    }

    /// Appends the Java generic type declaration corresponding to the given signature.
    fn append_java_declaration(&self, sb: &mut JavaString, name: &JavaStr, signature: &JavaStr) {
        let mut trace_signature_visitor = TraceSignatureVisitor::new(self.access);
        // A malformed signature prints the declaration of its valid prefix (see the module docs).
        let _ = SignatureReader::new(signature).accept(&mut trace_signature_visitor);
        sb.push_str("// declaration: ");
        if let Some(return_type) = trace_signature_visitor.get_return_type() {
            sb.push_java_str(return_type);
            sb.push(' ');
        }
        sb.push_java_str(name);
        sb.push_java_str(trace_signature_visitor.get_declaration());
        if let Some(exceptions) = trace_signature_visitor.get_exceptions() {
            sb.push_str(" throws ");
            sb.push_java_str(&exceptions);
        }
        sb.push('\n');
    }

    /// Appends the name of the given label, naming it first if needed (`Textifier.appendLabel`).
    pub fn append_label(&mut self, sb: &mut JavaString, label: Label) {
        sb.push_str(&label_name(&mut self.label_names, label));
    }

    /// Appends a string representation of the given handle (`Textifier.appendHandle`).
    pub fn append_handle(&self, sb: &mut JavaString, handle: &Handle, after_comment: &str) {
        let tag = handle.get_tag();
        sb.push_str(&format!("// handle kind 0x{:x} : ", tag.tag()));
        sb.push_str(match tag {
            HandleKind::GetField => "GETFIELD",
            HandleKind::GetStatic => "GETSTATIC",
            HandleKind::PutField => "PUTFIELD",
            HandleKind::PutStatic => "PUTSTATIC",
            HandleKind::InvokeInterface => "INVOKEINTERFACE",
            HandleKind::InvokeSpecial => "INVOKESPECIAL",
            HandleKind::InvokeStatic => "INVOKESTATIC",
            HandleKind::InvokeVirtual => "INVOKEVIRTUAL",
            HandleKind::NewInvokeSpecial => "NEWINVOKESPECIAL",
        });
        let is_method_handle = !tag.is_field();
        sb.push('\n');
        sb.push_str(after_comment);
        Self::append_descriptor(sb, INTERNAL_NAME, Some(handle.get_owner()));
        sb.push('.');
        sb.push_java_str(handle.get_name());
        if !is_method_handle {
            sb.push('(');
        }
        Self::append_descriptor(sb, HANDLE_DESCRIPTOR, Some(handle.get_desc()));
        if !is_method_handle {
            sb.push(')');
        }
        if handle.is_interface() {
            sb.push_str(" itf");
        }
    }

    fn append_constant_dynamic(
        &self,
        sb: &mut JavaString,
        condy: &ConstantDynamic,
        condy_indent: &str,
    ) {
        sb.push_java_str(condy.get_name());
        sb.push_str(" : ");
        sb.push_java_str(condy.get_descriptor());
        sb.push_str(" [\n");
        sb.push_str(condy_indent);
        sb.push_str(&self.tab);
        let indent = format!("{condy_indent}{}", self.tab);
        self.append_handle(sb, condy.get_bootstrap_method(), &indent);
        sb.push('\n');
        sb.push_str(condy_indent);
        sb.push_str(&self.tab);
        self.append_bootstrap_method_args(sb, condy.get_bootstrap_method_arguments(), &indent);
        sb.push('\n');
        sb.push_str(condy_indent);
        sb.push(']');
    }

    fn append_bootstrap_method_args(
        &self,
        sb: &mut JavaString,
        bsm_args: &[Constant<'_>],
        arg_indent: &str,
    ) {
        sb.push_str("// arguments:");
        if bsm_args.is_empty() {
            sb.push_str(" none");
        } else {
            for (i, arg) in bsm_args.iter().enumerate() {
                if i != 0 {
                    sb.push_str(", ");
                }
                sb.push('\n');
                sb.push_str(arg_indent);
                match arg {
                    Constant::Type(type_) => {
                        if type_.get_sort() == TypeSort::Method {
                            Self::append_descriptor(
                                sb,
                                METHOD_DESCRIPTOR,
                                Some(&type_.get_descriptor()),
                            );
                        } else {
                            visit_type(sb, type_);
                        }
                    }
                    Constant::Handle(handle) => self.append_handle(sb, handle, arg_indent),
                    Constant::Dynamic(condy) => {
                        sb.push_str("// constant dynamic: ");
                        sb.push('\n');
                        sb.push_str(arg_indent);
                        self.append_constant_dynamic(sb, condy, arg_indent);
                    }
                    _ => append_constant(sb, arg),
                }
            }
        }
    }

    fn append_frame_types(&mut self, sb: &mut JavaString, frame_types: &[FrameValue<'_>]) {
        for (i, frame_type) in frame_types.iter().enumerate() {
            if i > 0 {
                sb.push(' ');
            }
            match frame_type {
                FrameValue::Object(descriptor) => {
                    if descriptor.starts_with('[') {
                        Self::append_descriptor(sb, FIELD_DESCRIPTOR, Some(descriptor));
                    } else {
                        Self::append_descriptor(sb, INTERNAL_NAME, Some(descriptor));
                    }
                }
                FrameValue::Top => sb.push('T'),
                FrameValue::Integer => sb.push('I'),
                FrameValue::Float => sb.push('F'),
                FrameValue::Double => sb.push('D'),
                FrameValue::Long => sb.push('J'),
                FrameValue::Null => sb.push('N'),
                FrameValue::UninitializedThis => sb.push('U'),
                FrameValue::Uninitialized(label) => self.append_label(sb, *label),
            }
        }
    }

    /// Adds a new child Textifier to the text (followed by `end_text`, if any) and returns it.
    fn add_new_textifier(&mut self, end_text: Option<JavaString>) -> &mut dyn Printer {
        let index = self.text.len();
        self.text
            .push(TextNode::Printer(Box::new(Textifier::new())));
        if let Some(end_text) = end_text {
            self.text.push(TextNode::Str(end_text));
        }
        match &mut self.text[index] {
            TextNode::Printer(textifier) => &mut **textifier,
            TextNode::Str(_) => unreachable!("a child Textifier was just added here"),
        }
    }

    fn start_annotation_value(&mut self, name: Option<&JavaStr>) -> JavaString {
        let mut sb = JavaString::new();
        maybe_append_comma(&mut sb, self.num_annotation_values);
        self.num_annotation_values += 1;
        if let Some(name) = name {
            sb.push_java_str(name);
            sb.push('=');
        }
        sb
    }

    fn push(&mut self, sb: JavaString) {
        self.text.push(TextNode::Str(sb));
    }

    fn visit_export_or_open(
        &mut self,
        method: &str,
        packaze: &JavaStr,
        access: u32,
        modules: &[&JavaStr],
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str(method);
        sb.push_java_str(packaze);
        if modules.is_empty() {
            sb.push(';');
        } else {
            sb.push_str(" to");
        }
        append_raw_access(&mut sb, access);
        for (i, module) in modules.iter().enumerate() {
            sb.push_str(&self.tab2);
            sb.push_java_str(module);
            sb.push_str(if i != modules.len() - 1 { ",\n" } else { ";\n" });
        }
        self.push(sb);
    }
}

impl Printer for Textifier {
    fn get_text(&self) -> &[TextNode] {
        &self.text
    }

    // ---------------------------------------------------------------------------------------------
    // Classes
    // ---------------------------------------------------------------------------------------------

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        if access & opcodes::ACC_MODULE != 0 {
            // Modules are printed in visit_module.
            return;
        }
        self.access = access;
        let major_version = version & 0xFFFF;
        let minor_version = version >> 16;
        let mut sb = JavaString::new();
        // Java prints the version as a (signed) int.
        sb.push_str(&format!(
            "// class version {major_version}.{minor_version} ({})\n",
            version.cast_signed()
        ));
        if access & opcodes::ACC_DEPRECATED != 0 {
            sb.push_str(DEPRECATED);
        }
        if access & opcodes::ACC_RECORD != 0 {
            sb.push_str(RECORD);
        }
        append_raw_access(&mut sb, access);
        Self::append_descriptor(&mut sb, CLASS_SIGNATURE, signature);
        if let Some(signature) = signature {
            self.append_java_declaration(&mut sb, name, signature);
        }
        append_access(
            &mut sb,
            access & !(opcodes::ACC_SUPER | opcodes::ACC_MODULE),
        );
        if access & opcodes::ACC_ANNOTATION != 0 {
            sb.push_str("@interface ");
        } else if access & opcodes::ACC_INTERFACE != 0 {
            sb.push_str("interface ");
        } else if access & opcodes::ACC_ENUM == 0 {
            sb.push_str("class ");
        }
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(name));
        if let Some(super_name) = super_name
            && super_name != "java/lang/Object"
        {
            sb.push_str(" extends ");
            Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(super_name));
        }
        if !interfaces.is_empty() {
            sb.push_str(" implements ");
            for (i, interface) in interfaces.iter().enumerate() {
                Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(interface));
                if i != interfaces.len() - 1 {
                    sb.push(' ');
                }
            }
        }
        sb.push_str(" {\n\n");
        self.push(sb);
    }

    fn visit_source(&mut self, file: Option<&JavaStr>, debug: Option<&JavaStr>) {
        let mut sb = JavaString::new();
        if let Some(file) = file {
            sb.push_str(&self.tab);
            sb.push_str("// compiled from: ");
            sb.push_java_str(file);
            sb.push('\n');
        }
        if let Some(debug) = debug {
            sb.push_str(&self.tab);
            sb.push_str("// debug info: ");
            sb.push_java_str(debug);
            sb.push('\n');
        }
        if !sb.is_empty() {
            self.push(sb);
        }
    }

    fn visit_module(
        &mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        if access & opcodes::ACC_OPEN != 0 {
            sb.push_str("open ");
        }
        sb.push_str("module ");
        sb.push_java_str(name);
        sb.push_str(" { ");
        if let Some(version) = version {
            sb.push_str("// ");
            sb.push_java_str(version);
        }
        sb.push_str("\n\n");
        self.push(sb);
        self.add_new_textifier(None)
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("NESTHOST ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(nest_host));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("OUTERCLASS ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(owner));
        sb.push(' ');
        if let Some(name) = name {
            sb.push_java_str(name);
            sb.push(' ');
        }
        Self::append_descriptor(&mut sb, METHOD_DESCRIPTOR, descriptor);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_class_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer {
        self.push("\n".into());
        self.visit_member_annotation(descriptor, visible)
    }

    fn visit_class_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.push("\n".into());
        self.visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    fn visit_class_attribute(&mut self, attribute: &Attribute) {
        self.push("\n".into());
        self.visit_attribute(attribute);
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("NESTMEMBER ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(nest_member));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("PERMITTEDSUBCLASS ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(permitted_subclass));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        append_raw_access(&mut sb, access);
        sb.push_str(&self.tab);
        append_access(&mut sb, access);
        sb.push_str("INNERCLASS ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(name));
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, outer_name);
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, inner_name);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_record_component(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("RECORDCOMPONENT ");
        if let Some(signature) = signature {
            sb.push_str(&self.tab);
            Self::append_descriptor(&mut sb, FIELD_SIGNATURE, Some(signature));
            sb.push_str(&self.tab);
            self.append_java_declaration(&mut sb, name, signature);
        }
        sb.push_str(&self.tab);
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push(' ');
        sb.push_java_str(name);
        sb.push('\n');
        self.push(sb);
        self.add_new_textifier(None)
    }

    fn visit_field(
        &mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push('\n');
        if access & opcodes::ACC_DEPRECATED != 0 {
            sb.push_str(&self.tab);
            sb.push_str(DEPRECATED);
        }
        sb.push_str(&self.tab);
        append_raw_access(&mut sb, access);
        if let Some(signature) = signature {
            sb.push_str(&self.tab);
            Self::append_descriptor(&mut sb, FIELD_SIGNATURE, Some(signature));
            sb.push_str(&self.tab);
            self.append_java_declaration(&mut sb, name, signature);
        }
        sb.push_str(&self.tab);
        append_access(&mut sb, access);
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push(' ');
        sb.push_java_str(name);
        if let Some(value) = value {
            sb.push_str(" = ");
            append_constant(&mut sb, value);
        }
        sb.push('\n');
        self.push(sb);
        self.add_new_textifier(None)
    }

    fn visit_method(
        &mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push('\n');
        if access & opcodes::ACC_DEPRECATED != 0 {
            sb.push_str(&self.tab);
            sb.push_str(DEPRECATED);
        }
        sb.push_str(&self.tab);
        append_raw_access(&mut sb, access);
        if let Some(signature) = signature {
            sb.push_str(&self.tab);
            Self::append_descriptor(&mut sb, METHOD_SIGNATURE, Some(signature));
            sb.push_str(&self.tab);
            self.append_java_declaration(&mut sb, name, signature);
        }
        sb.push_str(&self.tab);
        append_access(
            &mut sb,
            access & !(opcodes::ACC_VOLATILE | opcodes::ACC_TRANSIENT),
        );
        if access & opcodes::ACC_NATIVE != 0 {
            sb.push_str("native ");
        }
        if access & opcodes::ACC_VARARGS != 0 {
            sb.push_str("varargs ");
        }
        if access & opcodes::ACC_BRIDGE != 0 {
            sb.push_str("bridge ");
        }
        if self.access & opcodes::ACC_INTERFACE != 0
            && access & (opcodes::ACC_ABSTRACT | opcodes::ACC_STATIC) == 0
        {
            sb.push_str("default ");
        }
        sb.push_java_str(name);
        Self::append_descriptor(&mut sb, METHOD_DESCRIPTOR, Some(descriptor));
        if !exceptions.is_empty() {
            sb.push_str(" throws");
            for exception in exceptions {
                sb.push(' ');
                Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(exception));
            }
        }
        sb.push('\n');
        self.push(sb);
        self.add_new_textifier(None)
    }

    fn visit_class_end(&mut self) {
        self.push("}\n".into());
    }

    // ---------------------------------------------------------------------------------------------
    // Modules
    // ---------------------------------------------------------------------------------------------

    fn visit_main_class(&mut self, main_class: &JavaStr) {
        let mut sb = JavaString::from("  // main class ");
        sb.push_java_str(main_class);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        let mut sb = JavaString::from("  // package ");
        sb.push_java_str(packaze);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_require(&mut self, require: &JavaStr, access: u32, version: Option<&JavaStr>) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("requires ");
        if access & opcodes::ACC_TRANSITIVE != 0 {
            sb.push_str("transitive ");
        }
        if access & opcodes::ACC_STATIC_PHASE != 0 {
            sb.push_str("static ");
        }
        sb.push_java_str(require);
        sb.push(';');
        append_raw_access(&mut sb, access);
        if let Some(version) = version {
            sb.push_str("  // version ");
            sb.push_java_str(version);
            sb.push('\n');
        }
        self.push(sb);
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.visit_export_or_open("exports ", packaze, access, modules);
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.visit_export_or_open("opens ", packaze, access, modules);
    }

    fn visit_use(&mut self, use_: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("uses ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(use_));
        sb.push_str(";\n");
        self.push(sb);
    }

    fn visit_provide(&mut self, provide: &JavaStr, providers: &[&JavaStr]) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab);
        sb.push_str("provides ");
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(provide));
        sb.push_str(" with\n");
        for (i, provider) in providers.iter().enumerate() {
            sb.push_str(&self.tab2);
            Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(provider));
            sb.push_str(if i != providers.len() - 1 {
                ",\n"
            } else {
                ";\n"
            });
        }
        self.push(sb);
    }

    fn visit_module_end(&mut self) {
        // Nothing to do.
    }

    // ---------------------------------------------------------------------------------------------
    // Annotations
    // ---------------------------------------------------------------------------------------------

    fn visit_annotation_value(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        let mut sb = self.start_annotation_value(name);
        fn list<T: Copy>(sb: &mut JavaString, values: &[T], visit: fn(&mut JavaString, T)) {
            sb.push('{');
            for (i, &value) in values.iter().enumerate() {
                maybe_append_comma(sb, i);
                visit(sb, value);
            }
            sb.push('}');
        }
        match value {
            AnnotationValue::String(s) => append_string(&mut sb, s),
            AnnotationValue::Type(t) => visit_type(&mut sb, t),
            AnnotationValue::Byte(v) => visit_byte(&mut sb, *v),
            AnnotationValue::Boolean(v) => visit_boolean(&mut sb, *v),
            AnnotationValue::Short(v) => visit_short(&mut sb, *v),
            AnnotationValue::Char(v) => visit_char(&mut sb, *v),
            AnnotationValue::Int(v) => visit_int(&mut sb, *v),
            AnnotationValue::Float(v) => visit_float(&mut sb, *v),
            AnnotationValue::Long(v) => visit_long(&mut sb, *v),
            AnnotationValue::Double(v) => visit_double(&mut sb, *v),
            AnnotationValue::ByteArray(a) => list(&mut sb, a, visit_byte),
            AnnotationValue::BooleanArray(a) => list(&mut sb, a, visit_boolean),
            AnnotationValue::ShortArray(a) => list(&mut sb, a, visit_short),
            AnnotationValue::CharArray(a) => list(&mut sb, a, visit_char),
            AnnotationValue::IntArray(a) => list(&mut sb, a, visit_int),
            AnnotationValue::LongArray(a) => list(&mut sb, a, visit_long),
            AnnotationValue::FloatArray(a) => list(&mut sb, a, visit_float),
            AnnotationValue::DoubleArray(a) => list(&mut sb, a, visit_double),
        }
        self.push(sb);
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        let mut sb = self.start_annotation_value(name);
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('.');
        sb.push_java_str(value);
        self.push(sb);
    }

    fn visit_annotation(
        &mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> &mut dyn Printer {
        let mut sb = self.start_annotation_value(name);
        sb.push('@');
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.push(sb);
        self.add_new_textifier(Some(")".into()))
    }

    fn visit_array(&mut self, name: Option<&JavaStr>) -> &mut dyn Printer {
        let mut sb = self.start_annotation_value(name);
        sb.push('{');
        self.push(sb);
        self.add_new_textifier(Some("}".into()))
    }

    fn visit_annotation_end(&mut self) {
        // Nothing to do.
    }

    // ---------------------------------------------------------------------------------------------
    // Record components
    // ---------------------------------------------------------------------------------------------

    fn visit_record_component_annotation(
        &mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_member_annotation(descriptor, visible)
    }

    fn visit_record_component_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    fn visit_record_component_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_record_component_end(&mut self) {
        // Nothing to do.
    }

    // ---------------------------------------------------------------------------------------------
    // Fields
    // ---------------------------------------------------------------------------------------------

    fn visit_field_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer {
        self.visit_member_annotation(descriptor, visible)
    }

    fn visit_field_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    fn visit_field_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_field_end(&mut self) {
        // Nothing to do.
    }

    // ---------------------------------------------------------------------------------------------
    // Methods
    // ---------------------------------------------------------------------------------------------

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("// parameter ");
        append_access(&mut sb, access);
        sb.push(' ');
        match name {
            Some(name) => sb.push_java_str(name),
            None => sb.push_str("<no name>"),
        }
        sb.push('\n');
        self.push(sb);
    }

    fn visit_annotation_default(&mut self) -> &mut dyn Printer {
        let mut sb = JavaString::from(self.tab2.as_str());
        sb.push_str("default=");
        self.push(sb);
        self.add_new_textifier(Some("\n".into()))
    }

    fn visit_method_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer {
        self.visit_member_annotation(descriptor, visible)
    }

    fn visit_method_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    fn visit_annotable_parameter_count(
        &mut self,
        parameter_count: u8,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("// annotable parameter count: ");
        sb.push_str(&parameter_count.to_string());
        sb.push_str(if visible {
            " (visible)\n"
        } else {
            " (invisible)\n"
        });
        self.push(sb);
        self
    }

    fn visit_parameter_annotation(
        &mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push('@');
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.push(sb);
        let mut sb = JavaString::from(if visible {
            ") // parameter "
        } else {
            ") // invisible, parameter "
        });
        sb.push_str(&parameter.to_string());
        sb.push('\n');
        self.add_new_textifier(Some(sb))
    }

    fn visit_method_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_code(&mut self) {
        // Nothing to do.
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        let mut sb = JavaString::new();
        sb.push_str(&self.ltab);
        sb.push_str("FRAME ");
        match kind {
            FrameKind::New | FrameKind::Full => {
                sb.push_str("FULL [");
                self.append_frame_types(&mut sb, local);
                sb.push_str("] [");
                self.append_frame_types(&mut sb, stack);
                sb.push(']');
            }
            FrameKind::Append => {
                sb.push_str("APPEND [");
                self.append_frame_types(&mut sb, local);
                sb.push(']');
            }
            FrameKind::Chop(num_local) => {
                sb.push_str("CHOP ");
                sb.push_str(&num_local.to_string());
            }
            FrameKind::Same => sb.push_str("SAME"),
            FrameKind::Same1 => {
                sb.push_str("SAME1 ");
                self.append_frame_types(&mut sb, &stack[..stack.len().min(1)]);
            }
            // ASM throws an IllegalArgumentException, but a visitor cannot fail: the reader
            // visits this internal kind for the ASM specific opcodes of a malformed class file,
            // with EXPAND_FRAMES.
            FrameKind::Insert => sb.push_str("INSERT"),
        }
        sb.push('\n');
        self.push(sb);
    }

    fn visit_insn(&mut self, opcode: u8) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        let type_name = usize::try_from(operand).ok().and_then(|i| TYPES.get(i));
        match type_name {
            Some(type_name) if opcode == opcodes::NEWARRAY => sb.push_str(type_name),
            _ => sb.push_str(&operand.to_string()),
        }
        sb.push('\n');
        self.push(sb);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        sb.push_str(&var_index.to_string());
        sb.push('\n');
        self.push(sb);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(type_));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(owner));
        sb.push('.');
        sb.push_java_str(name);
        sb.push_str(" : ");
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('\n');
        self.push(sb);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, Some(owner));
        sb.push('.');
        sb.push_java_str(name);
        sb.push(' ');
        Self::append_descriptor(&mut sb, METHOD_DESCRIPTOR, Some(descriptor));
        if is_interface {
            sb.push_str(" (itf)");
        }
        sb.push('\n');
        self.push(sb);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("INVOKEDYNAMIC ");
        sb.push_java_str(name);
        Self::append_descriptor(&mut sb, METHOD_DESCRIPTOR, Some(descriptor));
        sb.push_str(" [\n");
        sb.push_str(&self.tab3);
        self.append_handle(&mut sb, bootstrap_method_handle, &self.tab3);
        sb.push('\n');
        sb.push_str(&self.tab3);
        self.append_bootstrap_method_args(&mut sb, bootstrap_method_arguments, &self.tab3);
        sb.push('\n');
        sb.push_str(&self.tab2);
        sb.push_str("]\n");
        self.push(sb);
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&opcode_name(opcode));
        sb.push(' ');
        self.append_label(&mut sb, label);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_label(&mut self, label: Label) {
        let mut sb = JavaString::new();
        sb.push_str(&self.ltab);
        self.append_label(&mut sb, label);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        let mut sb = JavaString::new();
        match value {
            Constant::Dynamic(condy) => {
                sb.push_str(&self.tab2);
                sb.push_str("LDC ");
                self.append_constant_dynamic(&mut sb, condy, &self.tab2);
            }
            Constant::Handle(handle) => {
                sb.push_str(&self.tab2);
                let after_comment = format!("{}LDC ", self.tab2);
                self.append_handle(&mut sb, handle, &after_comment);
            }
            _ => {
                sb.push_str(&self.tab2);
                sb.push_str("LDC ");
                append_constant(&mut sb, value);
            }
        }
        sb.push('\n');
        self.push(sb);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&format!("IINC {var_index} {increment}\n"));
        self.push(sb);
    }

    fn visit_table_switch_insn(&mut self, min: i32, _max: i32, dflt: Label, labels: &[Label]) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("TABLESWITCH\n");
        for (i, &label) in labels.iter().enumerate() {
            sb.push_str(&self.tab3);
            // Java's int arithmetic (min + i) wraps.
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let key = min.wrapping_add(i as i32);
            sb.push_str(&key.to_string());
            sb.push_str(": ");
            self.append_label(&mut sb, label);
            sb.push('\n');
        }
        sb.push_str(&self.tab3);
        sb.push_str("default: ");
        self.append_label(&mut sb, dflt);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("LOOKUPSWITCH\n");
        for (key, &label) in keys.iter().zip(labels) {
            sb.push_str(&self.tab3);
            sb.push_str(&key.to_string());
            sb.push_str(": ");
            self.append_label(&mut sb, label);
            sb.push('\n');
        }
        sb.push_str(&self.tab3);
        sb.push_str("default: ");
        self.append_label(&mut sb, dflt);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("MULTIANEWARRAY ");
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push(' ');
        sb.push_str(&num_dimensions.to_string());
        sb.push('\n');
        self.push(sb);
    }

    fn visit_insn_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("TRYCATCHBLOCK ");
        self.append_label(&mut sb, start);
        sb.push(' ');
        self.append_label(&mut sb, end);
        sb.push(' ');
        self.append_label(&mut sb, handler);
        sb.push(' ');
        Self::append_descriptor(&mut sb, INTERNAL_NAME, type_);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_try_catch_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("TRYCATCHBLOCK @");
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.push(sb);
        let mut sb = JavaString::from(") : ");
        append_type_reference(&mut sb, type_ref);
        sb.push_str(", ");
        append_type_path(&mut sb, type_path);
        sb.push_str(if visible { "\n" } else { INVISIBLE });
        self.add_new_textifier(Some(sb))
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
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("LOCALVARIABLE ");
        sb.push_java_str(name);
        sb.push(' ');
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push(' ');
        self.append_label(&mut sb, start);
        sb.push(' ');
        self.append_label(&mut sb, end);
        sb.push(' ');
        sb.push_str(&index.to_string());
        sb.push('\n');
        if let Some(signature) = signature {
            sb.push_str(&self.tab2);
            Self::append_descriptor(&mut sb, FIELD_SIGNATURE, Some(signature));
            sb.push_str(&self.tab2);
            self.append_java_declaration(&mut sb, name, signature);
        }
        self.push(sb);
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
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("LOCALVARIABLE @");
        Self::append_descriptor(&mut sb, FIELD_DESCRIPTOR, Some(descriptor));
        sb.push('(');
        self.push(sb);
        let mut sb = JavaString::from(") : ");
        append_type_reference(&mut sb, type_ref);
        sb.push_str(", ");
        append_type_path(&mut sb, type_path);
        for ((&start, &end), index) in start.iter().zip(end).zip(index) {
            sb.push_str(" [ ");
            self.append_label(&mut sb, start);
            sb.push_str(" - ");
            self.append_label(&mut sb, end);
            sb.push_str(" - ");
            sb.push_str(&index.to_string());
            sb.push_str(" ]");
        }
        sb.push_str(if visible { "\n" } else { INVISIBLE });
        self.add_new_textifier(Some(sb))
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str("LINENUMBER ");
        sb.push_str(&line.to_string());
        sb.push(' ');
        self.append_label(&mut sb, start);
        sb.push('\n');
        self.push(sb);
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&format!("MAXSTACK = {max_stack}\n"));
        self.push(sb);
        let mut sb = JavaString::new();
        sb.push_str(&self.tab2);
        sb.push_str(&format!("MAXLOCALS = {max_locals}\n"));
        self.push(sb);
    }

    fn visit_method_end(&mut self) {
        // Nothing to do.
    }
}

// -------------------------------------------------------------------------------------------------
// Utility functions
// -------------------------------------------------------------------------------------------------

/// The name of `label` in `label_names`, naming it `L<n>` first if it has none.
fn label_name(label_names: &mut FxHashMap<Label, String>, label: Label) -> String {
    let next = label_names.len();
    label_names
        .entry(label)
        .or_insert_with(|| format!("L{next}"))
        .clone()
}

fn opcode_name(opcode: u8) -> std::borrow::Cow<'static, str> {
    match OPCODES.get(usize::from(opcode)) {
        Some(name) => (*name).into(),
        None => opcode.to_string().into(),
    }
}

/// Appends the value, or `null` (Java's `StringBuilder.append((String) null)`).
fn append_nullable(sb: &mut JavaString, value: Option<&JavaStr>) {
    match value {
        Some(value) => sb.push_java_str(value),
        None => sb.push_str("null"),
    }
}

fn append_type_path(sb: &mut JavaString, type_path: Option<&TypePath>) {
    match type_path {
        Some(type_path) => sb.push_str(&type_path.to_string()),
        None => sb.push_str("null"),
    }
}

fn maybe_append_comma(sb: &mut JavaString, num_values: usize) {
    if num_values > 0 {
        sb.push_str(", ");
    }
}

fn append_access(sb: &mut JavaString, access_flags: u32) {
    const FLAGS: [(u32, &str); 13] = [
        (opcodes::ACC_PUBLIC, "public "),
        (opcodes::ACC_PRIVATE, "private "),
        (opcodes::ACC_PROTECTED, "protected "),
        (opcodes::ACC_FINAL, "final "),
        (opcodes::ACC_STATIC, "static "),
        (opcodes::ACC_SYNCHRONIZED, "synchronized "),
        (opcodes::ACC_VOLATILE, "volatile "),
        (opcodes::ACC_TRANSIENT, "transient "),
        (opcodes::ACC_ABSTRACT, "abstract "),
        (opcodes::ACC_STRICT, "strictfp "),
        (opcodes::ACC_SYNTHETIC, "synthetic "),
        (opcodes::ACC_MANDATED, "mandated "),
        (opcodes::ACC_ENUM, "enum "),
    ];
    for (flag, name) in FLAGS {
        if access_flags & flag != 0 {
            sb.push_str(name);
        }
    }
}

fn append_raw_access(sb: &mut JavaString, access_flags: u32) {
    sb.push_str(&format!("// access flags 0x{access_flags:X}\n"));
}

/// Appends a constant (`Textifier.appendConstant`): numbers with their Java suffix, a type as
/// its descriptor followed by `.class`, a string quoted and escaped.
fn append_constant(sb: &mut JavaString, constant: &Constant<'_>) {
    match constant {
        Constant::Double(v) => {
            sb.push_str(&double_to_string(*v));
            sb.push('D');
        }
        Constant::Float(v) => {
            sb.push_str(&float_to_string(*v));
            sb.push('F');
        }
        Constant::Long(v) => {
            sb.push_str(&v.to_string());
            sb.push('L');
        }
        Constant::Integer(v) => sb.push_str(&v.to_string()),
        Constant::Type(t) => {
            sb.push_java_str(&t.get_descriptor());
            sb.push_str(CLASS_SUFFIX);
        }
        Constant::String(s) => append_string(sb, s),
        // Java appends the constant's toString().
        Constant::Handle(h) => sb.push_str(&h.to_string()),
        Constant::Dynamic(d) => sb.push_str(&d.to_string()),
    }
}

fn visit_int(sb: &mut JavaString, value: i32) {
    sb.push_str(&value.to_string());
}

fn visit_long(sb: &mut JavaString, value: i64) {
    sb.push_str(&value.to_string());
    sb.push('L');
}

fn visit_float(sb: &mut JavaString, value: f32) {
    sb.push_str(&float_to_string(value));
    sb.push('F');
}

fn visit_double(sb: &mut JavaString, value: f64) {
    sb.push_str(&double_to_string(value));
    sb.push('D');
}

fn visit_char(sb: &mut JavaString, value: u16) {
    sb.push_str("(char)");
    sb.push_str(&value.to_string());
}

fn visit_short(sb: &mut JavaString, value: i16) {
    sb.push_str("(short)");
    sb.push_str(&value.to_string());
}

fn visit_byte(sb: &mut JavaString, value: i8) {
    sb.push_str("(byte)");
    sb.push_str(&value.to_string());
}

fn visit_boolean(sb: &mut JavaString, value: bool) {
    sb.push_str(if value { "true" } else { "false" });
}

/// Appends the class name of the type followed by `.class` (`Textifier.visitType`).
fn visit_type(sb: &mut JavaString, value: &Type) {
    sb.push_java_str(&class_name(value));
    sb.push_str(CLASS_SUFFIX);
}

/// `Type.getClassName`, without its failures on method types and malformed array types (where it
/// returns the descriptor instead; see the module docs).
fn class_name(type_: &Type) -> JavaString {
    match type_.get_sort() {
        TypeSort::Method => type_.get_descriptor().into_owned(),
        TypeSort::Array => match type_.try_get_element_type() {
            Ok(element_type) if element_type.get_sort() != TypeSort::Method => {
                let mut name = class_name(&element_type);
                for _ in 0..type_.get_dimensions() {
                    name.push_str("[]");
                }
                name
            }
            _ => type_.get_descriptor().into_owned(),
        },
        _ => type_.get_class_name(),
    }
}

fn append_type_reference(sb: &mut JavaString, type_ref: TypeReference) {
    let s = match type_ref.get_sort() {
        TypeReference::CLASS_TYPE_PARAMETER => {
            format!(
                "CLASS_TYPE_PARAMETER {}",
                type_ref.get_type_parameter_index()
            )
        }
        TypeReference::METHOD_TYPE_PARAMETER => {
            format!(
                "METHOD_TYPE_PARAMETER {}",
                type_ref.get_type_parameter_index()
            )
        }
        TypeReference::CLASS_EXTENDS => {
            format!("CLASS_EXTENDS {}", type_ref.get_super_type_index())
        }
        TypeReference::CLASS_TYPE_PARAMETER_BOUND => format!(
            "CLASS_TYPE_PARAMETER_BOUND {}, {}",
            type_ref.get_type_parameter_index(),
            type_ref.get_type_parameter_bound_index()
        ),
        TypeReference::METHOD_TYPE_PARAMETER_BOUND => format!(
            "METHOD_TYPE_PARAMETER_BOUND {}, {}",
            type_ref.get_type_parameter_index(),
            type_ref.get_type_parameter_bound_index()
        ),
        TypeReference::FIELD => "FIELD".to_owned(),
        TypeReference::METHOD_RETURN => "METHOD_RETURN".to_owned(),
        TypeReference::METHOD_RECEIVER => "METHOD_RECEIVER".to_owned(),
        TypeReference::METHOD_FORMAL_PARAMETER => {
            format!(
                "METHOD_FORMAL_PARAMETER {}",
                type_ref.get_formal_parameter_index()
            )
        }
        TypeReference::THROWS => format!("THROWS {}", type_ref.get_exception_index()),
        TypeReference::LOCAL_VARIABLE => "LOCAL_VARIABLE".to_owned(),
        TypeReference::RESOURCE_VARIABLE => "RESOURCE_VARIABLE".to_owned(),
        TypeReference::EXCEPTION_PARAMETER => {
            format!(
                "EXCEPTION_PARAMETER {}",
                type_ref.get_try_catch_block_index()
            )
        }
        TypeReference::INSTANCEOF => "INSTANCEOF".to_owned(),
        TypeReference::NEW => "NEW".to_owned(),
        TypeReference::CONSTRUCTOR_REFERENCE => "CONSTRUCTOR_REFERENCE".to_owned(),
        TypeReference::METHOD_REFERENCE => "METHOD_REFERENCE".to_owned(),
        TypeReference::CAST => format!("CAST {}", type_ref.get_type_argument_index()),
        TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT => format!(
            "CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT {}",
            type_ref.get_type_argument_index()
        ),
        TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT => format!(
            "METHOD_INVOCATION_TYPE_ARGUMENT {}",
            type_ref.get_type_argument_index()
        ),
        TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT => format!(
            "CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT {}",
            type_ref.get_type_argument_index()
        ),
        TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT => format!(
            "METHOD_REFERENCE_TYPE_ARGUMENT {}",
            type_ref.get_type_argument_index()
        ),
        sort => panic!("invalid type reference sort: {sort:#x}"),
    };
    sb.push_str(&s);
}
