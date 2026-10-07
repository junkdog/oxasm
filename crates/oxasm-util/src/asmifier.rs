//! [`Asmifier`]: ASM's `ASMifier`, a [`Printer`] that prints the ASM (Java) code that generates
//! the classes it visits.
//!
//! The output is byte for byte ASM 9.10.1's (oxasm-verification, "Text parity"): it is *Java*
//! code calling ASM's API, not Rust code calling oxasm's. Floating point values are printed as
//! Java prints them ([`crate::java_fmt`]), and strings are escaped per UTF-16 code unit.
//!
//! # Divergences on malformed input
//!
//! Where ASM's ASMifier throws on events that only a malformed class file produces, this one
//! prints something instead, because visitors cannot fail (ADR-06) and printing untrusted classes
//! must not panic:
//!
//! - a `NEWARRAY` operand outside of `0..=T_LONG` prints as a number (ASM indexes `TYPES` with
//!   it), and so does an unknown opcode;
//! - a frame of ASM's internal kind `F_INSERT` prints as `visitFrame(256, ...)` with its content
//!   (ASM throws an `IllegalArgumentException`);
//! - a label that was never declared prints as `null` (ASM throws a `NullPointerException` when
//!   the method declared no label at all);
//! - a type path with an invalid step prints it as `?` (ASM's `TypePath.toString` throws an
//!   `AssertionError`), and one of 128 steps or more prints all of them (ASM reads the length as
//!   a signed byte, see oxasm-verification);
//! - the local and stack counts of a frame are those of its slices: an expanded frame that a
//!   malformed class makes ASM visit with a negative count prints 0 instead.

use rustc_hash::FxHashMap;

use oxasm::constant::{AnnotationValue, Constant, FrameValue};
use oxasm::{
    Attribute, ConstantDynamic, FrameKind, Handle, JavaStr, JavaString, Label, TypePath,
    TypeReference, opcodes,
};

use crate::java_fmt::{double_to_string, float_to_string};
use crate::printer::{self, HANDLE_TAG, OPCODES, Printer, TYPES, TextNode, append_string};

/// The usage message of [`Asmifier::main`].
const USAGE: &str = "Prints the ASM code to generate the given class.\n\
    Usage: ASMifier [-nodebug] <fully qualified class name or class file name>";

/// A pseudo access flag to distinguish class access flags.
const ACCESS_CLASS: u32 = 0x40000;
/// A pseudo access flag to distinguish field access flags.
const ACCESS_FIELD: u32 = 0x80000;
/// A pseudo access flag to distinguish inner class flags.
const ACCESS_INNER: u32 = 0x10_0000;
/// A pseudo access flag to distinguish module requires/exports flags.
const ACCESS_MODULE: u32 = 0x20_0000;

const ANNOTATION_VISITOR: &str = "annotationVisitor";
const ANNOTATION_VISITOR0: &str = "annotationVisitor0 = ";
const COMMA: &str = "\", \"";
const END_ARRAY: &str = " });\n";
const END_PARAMETERS: &str = ");\n\n";
const NEW_OBJECT_ARRAY: &str = ", new Object[] {";
const VISIT_END: &str = ".visitEnd();\n";

/// The names of the primitive frame types, by value (`Opcodes.TOP` is 0).
const FRAME_TYPES: [&str; 7] = [
    "Opcodes.TOP",
    "Opcodes.INTEGER",
    "Opcodes.FLOAT",
    "Opcodes.DOUBLE",
    "Opcodes.LONG",
    "Opcodes.NULL",
    "Opcodes.UNINITIALIZED_THIS",
];

/// A [`Printer`] that prints the ASM code to generate the classes it visits.
#[derive(Debug)]
pub struct Asmifier {
    /// The text constructed by this printer.
    pub text: Vec<TextNode>,
    /// The name of the visitor variable in the produced code.
    pub name: String,
    /// The identifier of the annotation visitor variable in the produced code.
    pub id: u32,
    /// The label names, by first appearance (`label0`, `label1`, ...). Each printer has its own,
    /// so labels are named per method.
    pub label_names: FxHashMap<Label, String>,
}

impl Default for Asmifier {
    fn default() -> Self {
        Asmifier::new()
    }
}

impl Asmifier {
    /// Constructs a new Asmifier, whose class writer variable is `classWriter`.
    #[must_use]
    pub fn new() -> Asmifier {
        Asmifier::with_names("classWriter", 0)
    }

    /// Constructs a new Asmifier (ASM's protected constructor).
    ///
    /// - `visitor_variable_name`: the name of the visitor variable in the produced code.
    /// - `annotation_visitor_id`: the identifier of the annotation visitor variable in the
    ///   produced code.
    #[must_use]
    pub fn with_names(visitor_variable_name: &str, annotation_visitor_id: u32) -> Asmifier {
        Asmifier {
            text: Vec::new(),
            name: visitor_variable_name.to_owned(),
            id: annotation_visitor_id,
            label_names: FxHashMap::default(),
        }
    }

    /// Prints the ASM code to generate the given class to `output` (`ASMifier.main`): `args` is
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
        printer::main(args, USAGE, Asmifier::new(), output, logger)
    }

    // ---------------------------------------------------------------------------------------------
    // Common methods
    // ---------------------------------------------------------------------------------------------

    /// Visits a class, field, method or record component annotation (ASMifier's public
    /// `visitAnnotation(descriptor, visible)`).
    pub fn visit_member_annotation(
        &mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\n");
        sb.push_str(ANNOTATION_VISITOR0);
        sb.push_str(&self.name);
        sb.push_str(".visitAnnotation(");
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {visible});\n"));
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, 0)
    }

    /// Visits a type annotation with the given visit method name (`visitTypeAnnotation`,
    /// `visitInsnAnnotation` or `visitTryCatchAnnotation`).
    pub fn visit_type_annotation(
        &mut self,
        method: &str,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\n");
        sb.push_str(ANNOTATION_VISITOR0);
        sb.push_str(&self.name);
        sb.push('.');
        sb.push_str(method);
        sb.push('(');
        append_type_ref_and_path(&mut sb, type_ref, type_path);
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {visible});\n"));
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, 0)
    }

    /// Visits a class, field, method or record component attribute. An attribute that has an
    /// [`asmify`](oxasm::CustomAttribute::asmify) representation is created in a variable named
    /// `attribute` and visited; the others only print a comment.
    pub fn visit_attribute(&mut self, attribute: &Attribute) {
        let mut sb = JavaString::new();
        sb.push_str("// ATTRIBUTE ");
        sb.push_java_str(attribute.name());
        sb.push('\n');
        if let Attribute::Custom(custom) = attribute {
            let label_names = &mut self.label_names;
            if let Some(code) = custom.asmify("attribute", &mut |label| {
                let next = label_names.len();
                label_names
                    .entry(label)
                    .or_insert_with(|| format!("label{next}"))
                    .clone()
            }) {
                sb.push_str("{\n");
                sb.push_str(&code);
                sb.push_str(&self.name);
                sb.push_str(".visitAttribute(attribute);\n}\n");
            }
        }
        self.push(sb);
    }

    fn visit_member_end(&mut self) {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push_str(VISIT_END);
        self.push(sb);
    }

    // ---------------------------------------------------------------------------------------------
    // Utility methods
    // ---------------------------------------------------------------------------------------------

    /// Adds a new child Asmifier to the text, followed by `}\n`, and returns it
    /// (`createASMifier`).
    fn add_new_asmifier(
        &mut self,
        visitor_variable_name: &str,
        annotation_visitor_id: u32,
    ) -> &mut dyn Printer {
        let index = self.text.len();
        self.text
            .push(TextNode::Printer(Box::new(Asmifier::with_names(
                visitor_variable_name,
                annotation_visitor_id,
            ))));
        self.text.push("}\n".into());
        match &mut self.text[index] {
            TextNode::Printer(asmifier) => &mut **asmifier,
            TextNode::Str(_) => unreachable!("a child Asmifier was just added here"),
        }
    }

    fn push(&mut self, sb: JavaString) {
        self.text.push(TextNode::Str(sb));
    }

    /// Appends a declaration of the given label, if it has none yet (`declareLabel`).
    pub fn declare_label(&mut self, sb: &mut JavaString, label: Label) {
        let next = self.label_names.len();
        if let std::collections::hash_map::Entry::Vacant(entry) = self.label_names.entry(label) {
            let label_name = format!("label{next}");
            sb.push_str(&format!("Label {label_name} = new Label();\n"));
            entry.insert(label_name);
        }
    }

    /// Appends the name of the given label, which must have been declared (`appendLabel`).
    pub fn append_label(&self, sb: &mut JavaString, label: Label) {
        // ASM appends `null` (or throws, see the module docs) for an undeclared label.
        sb.push_str(self.label_names.get(&label).map_or("null", String::as_str));
    }

    fn declare_frame_types(&mut self, sb: &mut JavaString, frame_types: &[FrameValue<'_>]) {
        for frame_type in frame_types {
            if let FrameValue::Uninitialized(label) = frame_type {
                self.declare_label(sb, *label);
            }
        }
    }

    fn append_frame_types(&self, sb: &mut JavaString, frame_types: &[FrameValue<'_>]) {
        for (i, frame_type) in frame_types.iter().enumerate() {
            if i > 0 {
                sb.push_str(", ");
            }
            match frame_type {
                FrameValue::Object(descriptor) => append_string(sb, descriptor),
                FrameValue::Top => sb.push_str(FRAME_TYPES[0]),
                FrameValue::Integer => sb.push_str(FRAME_TYPES[1]),
                FrameValue::Float => sb.push_str(FRAME_TYPES[2]),
                FrameValue::Double => sb.push_str(FRAME_TYPES[3]),
                FrameValue::Long => sb.push_str(FRAME_TYPES[4]),
                FrameValue::Null => sb.push_str(FRAME_TYPES[5]),
                FrameValue::UninitializedThis => sb.push_str(FRAME_TYPES[6]),
                FrameValue::Uninitialized(label) => self.append_label(sb, *label),
            }
        }
    }

    fn visit_export_or_open(
        &mut self,
        visit_method: &str,
        packaze: &JavaStr,
        access: u32,
        modules: &[&JavaStr],
    ) {
        let mut sb = JavaString::new();
        sb.push_str(visit_method);
        append_string(&mut sb, packaze);
        sb.push_str(", ");
        append_access_flags(&mut sb, access | ACCESS_MODULE);
        if !modules.is_empty() {
            sb.push_str(", new String[] {");
            append_string_array_elements(&mut sb, modules);
            sb.push_str(" }");
        }
        sb.push_str(");\n");
        self.push(sb);
    }

    /// Starts a line `<name>.<method>(<opcode>, `.
    fn start_insn(&self, method: &str, opcode: u8) -> JavaString {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push('.');
        sb.push_str(method);
        sb.push('(');
        sb.push_str(&opcode_name(opcode));
        sb.push_str(", ");
        sb
    }
}

impl Printer for Asmifier {
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
        let simple_name = match name.rfind('/') {
            None => name.to_owned(),
            Some(last_slash_index) => {
                let mut package = JavaString::new();
                package.push_str("package asm.");
                package.push_java_str(&name[..last_slash_index].replace('/', '.'));
                package.push_str(";\n");
                self.push(package);
                // Java's replaceAll("[-\\(\\)]", "_").
                let simple_name = name[last_slash_index + 1..].replace('-', '_');
                let simple_name = simple_name.replace('(', '_');
                simple_name.replace(')', '_')
            }
        };
        for import in [
            "AnnotationVisitor",
            "Attribute",
            "ClassReader",
            "ClassWriter",
            "ConstantDynamic",
            "FieldVisitor",
            "Handle",
            "Label",
            "MethodVisitor",
            "Opcodes",
            "RecordComponentVisitor",
            "Type",
            "TypePath",
        ] {
            self.push(
                format!("import org.objectweb.asm.{import};\n")
                    .as_str()
                    .into(),
            );
        }
        let mut class = JavaString::new();
        class.push_str("public class ");
        class.push_java_str(&simple_name);
        class.push_str("Dump implements Opcodes {\n\n");
        self.push(class);
        self.push("public static byte[] dump () throws Exception {\n\n".into());
        self.push("ClassWriter classWriter = new ClassWriter(0);\n".into());
        self.push("FieldVisitor fieldVisitor;\n".into());
        self.push("RecordComponentVisitor recordComponentVisitor;\n".into());
        self.push("MethodVisitor methodVisitor;\n".into());
        self.push("AnnotationVisitor annotationVisitor0;\n\n".into());

        let mut sb = JavaString::new();
        sb.push_str("classWriter.visit(");
        match class_version_name(version) {
            Some(version_name) => sb.push_str(&version_name),
            // Java prints the version as a (signed) int.
            None => sb.push_str(&version.cast_signed().to_string()),
        }
        sb.push_str(", ");
        append_access_flags(&mut sb, access | ACCESS_CLASS);
        sb.push_str(", ");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_nullable_string(&mut sb, signature);
        sb.push_str(", ");
        append_nullable_string(&mut sb, super_name);
        sb.push_str(", ");
        append_string_array(&mut sb, interfaces);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_source(&mut self, file: Option<&JavaStr>, debug: Option<&JavaStr>) {
        let mut sb = JavaString::new();
        sb.push_str("classWriter.visitSource(");
        append_nullable_string(&mut sb, file);
        sb.push_str(", ");
        append_nullable_string(&mut sb, debug);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_module(
        &mut self,
        name: &JavaStr,
        flags: u32,
        version: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\nModuleVisitor moduleVisitor = classWriter.visitModule(");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_access_flags(&mut sb, flags | ACCESS_MODULE);
        sb.push_str(", ");
        append_nullable_string(&mut sb, version);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
        self.add_new_asmifier("moduleVisitor", 0)
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("classWriter.visitNestHost(");
        append_string(&mut sb, nest_host);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        let mut sb = JavaString::new();
        sb.push_str("classWriter.visitOuterClass(");
        append_string(&mut sb, owner);
        sb.push_str(", ");
        append_nullable_string(&mut sb, name);
        sb.push_str(", ");
        append_nullable_string(&mut sb, descriptor);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_class_annotation(&mut self, descriptor: &JavaStr, visible: bool) -> &mut dyn Printer {
        self.visit_member_annotation(descriptor, visible)
    }

    fn visit_class_type_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(
            "visitTypeAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_class_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("classWriter.visitNestMember(");
        append_string(&mut sb, nest_member);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("classWriter.visitPermittedSubclass(");
        append_string(&mut sb, permitted_subclass);
        sb.push_str(END_PARAMETERS);
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
        sb.push_str("classWriter.visitInnerClass(");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_nullable_string(&mut sb, outer_name);
        sb.push_str(", ");
        append_nullable_string(&mut sb, inner_name);
        sb.push_str(", ");
        append_access_flags(&mut sb, access | ACCESS_INNER);
        sb.push_str(END_PARAMETERS);
        self.push(sb);
    }

    fn visit_record_component(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\nrecordComponentVisitor = classWriter.visitRecordComponent(");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_nullable_string(&mut sb, signature);
        sb.push_str(");\n");
        self.push(sb);
        self.add_new_asmifier("recordComponentVisitor", 0)
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
        sb.push_str("{\nfieldVisitor = classWriter.visitField(");
        append_access_flags(&mut sb, access | ACCESS_FIELD);
        sb.push_str(", ");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_nullable_string(&mut sb, signature);
        sb.push_str(", ");
        match value {
            Some(value) => append_constant(&mut sb, value),
            None => sb.push_str("null"),
        }
        sb.push_str(");\n");
        self.push(sb);
        self.add_new_asmifier("fieldVisitor", 0)
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
        sb.push_str("{\nmethodVisitor = classWriter.visitMethod(");
        append_access_flags(&mut sb, access);
        sb.push_str(", ");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_nullable_string(&mut sb, signature);
        sb.push_str(", ");
        append_string_array(&mut sb, exceptions);
        sb.push_str(");\n");
        self.push(sb);
        self.add_new_asmifier("methodVisitor", 0)
    }

    fn visit_class_end(&mut self) {
        self.push("classWriter.visitEnd();\n\n".into());
        self.push("return classWriter.toByteArray();\n".into());
        self.push("}\n".into());
        self.push("}\n".into());
    }

    // ---------------------------------------------------------------------------------------------
    // Modules
    // ---------------------------------------------------------------------------------------------

    fn visit_main_class(&mut self, main_class: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("moduleVisitor.visitMainClass(");
        append_string(&mut sb, main_class);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("moduleVisitor.visitPackage(");
        append_string(&mut sb, packaze);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        let mut sb = JavaString::new();
        sb.push_str("moduleVisitor.visitRequire(");
        append_string(&mut sb, module);
        sb.push_str(", ");
        append_access_flags(&mut sb, access | ACCESS_MODULE);
        sb.push_str(", ");
        append_nullable_string(&mut sb, version);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.visit_export_or_open("moduleVisitor.visitExport(", packaze, access, modules);
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.visit_export_or_open("moduleVisitor.visitOpen(", packaze, access, modules);
    }

    fn visit_use(&mut self, service: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str("moduleVisitor.visitUse(");
        append_string(&mut sb, service);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        let mut sb = JavaString::new();
        sb.push_str("moduleVisitor.visitProvide(");
        append_string(&mut sb, service);
        sb.push_str(",  new String[] {");
        append_string_array_elements(&mut sb, providers);
        sb.push_str(END_ARRAY);
        self.push(sb);
    }

    fn visit_module_end(&mut self) {
        self.push("moduleVisitor.visitEnd();\n".into());
    }

    // ---------------------------------------------------------------------------------------------
    // Annotations
    // ---------------------------------------------------------------------------------------------

    fn visit_annotation_value(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        let mut sb = JavaString::new();
        sb.push_str(&format!("{ANNOTATION_VISITOR}{}.visit(", self.id));
        append_nullable_string(&mut sb, name);
        sb.push_str(", ");
        append_annotation_value(&mut sb, value);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        let mut sb = JavaString::new();
        sb.push_str(&format!("{ANNOTATION_VISITOR}{}.visitEnum(", self.id));
        append_nullable_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_string(&mut sb, value);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_annotation(
        &mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> &mut dyn Printer {
        let id = self.id;
        let mut sb = JavaString::new();
        sb.push_str(&format!(
            "{{\nAnnotationVisitor annotationVisitor{} = annotationVisitor{id}.visitAnnotation(",
            id + 1
        ));
        append_nullable_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(");\n");
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, id + 1)
    }

    fn visit_array(&mut self, name: Option<&JavaStr>) -> &mut dyn Printer {
        let id = self.id;
        let mut sb = JavaString::new();
        sb.push_str(&format!(
            "{{\nAnnotationVisitor annotationVisitor{} = annotationVisitor{id}.visitArray(",
            id + 1
        ));
        append_nullable_string(&mut sb, name);
        sb.push_str(");\n");
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, id + 1)
    }

    fn visit_annotation_end(&mut self) {
        self.push(
            format!("{ANNOTATION_VISITOR}{}{VISIT_END}", self.id)
                .as_str()
                .into(),
        );
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
        self.visit_type_annotation(
            "visitTypeAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_record_component_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_record_component_end(&mut self) {
        self.visit_member_end();
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
        self.visit_type_annotation(
            "visitTypeAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_field_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_field_end(&mut self) {
        self.visit_member_end();
    }

    // ---------------------------------------------------------------------------------------------
    // Methods
    // ---------------------------------------------------------------------------------------------

    fn visit_parameter(&mut self, parameter_name: Option<&JavaStr>, access: u32) {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push_str(".visitParameter(");
        append_nullable_string(&mut sb, parameter_name);
        sb.push_str(", ");
        append_access_flags(&mut sb, access);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_annotation_default(&mut self) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\n");
        sb.push_str(ANNOTATION_VISITOR0);
        sb.push_str(&self.name);
        sb.push_str(".visitAnnotationDefault();\n");
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, 0)
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
        self.visit_type_annotation(
            "visitTypeAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_annotable_parameter_count(
        &mut self,
        parameter_count: u8,
        visible: bool,
    ) -> &mut dyn Printer {
        let line = format!(
            "{}.visitAnnotableParameterCount({parameter_count}, {visible});\n",
            self.name
        );
        self.push(line.as_str().into());
        self
    }

    fn visit_parameter_annotation(
        &mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        let mut sb = JavaString::new();
        sb.push_str("{\n");
        sb.push_str(ANNOTATION_VISITOR0);
        sb.push_str(&self.name);
        sb.push_str(&format!(".visitParameterAnnotation({parameter}, "));
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {visible});\n"));
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, 0)
    }

    fn visit_method_attribute(&mut self, attribute: &Attribute) {
        self.visit_attribute(attribute);
    }

    fn visit_code(&mut self) {
        let line = format!("{}.visitCode();\n", self.name);
        self.push(line.as_str().into());
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        let mut sb = JavaString::new();
        match kind {
            FrameKind::New | FrameKind::Full | FrameKind::Insert => {
                self.declare_frame_types(&mut sb, local);
                self.declare_frame_types(&mut sb, stack);
                sb.push_str(&self.name);
                sb.push_str(match kind {
                    FrameKind::New => ".visitFrame(Opcodes.F_NEW, ",
                    FrameKind::Full => ".visitFrame(Opcodes.F_FULL, ",
                    // ASM throws (see the module docs); F_INSERT is 256.
                    _ => ".visitFrame(256, ",
                });
                sb.push_str(&local.len().to_string());
                sb.push_str(NEW_OBJECT_ARRAY);
                self.append_frame_types(&mut sb, local);
                sb.push_str("}, ");
                sb.push_str(&stack.len().to_string());
                sb.push_str(NEW_OBJECT_ARRAY);
                self.append_frame_types(&mut sb, stack);
                sb.push('}');
            }
            FrameKind::Append => {
                self.declare_frame_types(&mut sb, local);
                sb.push_str(&self.name);
                sb.push_str(".visitFrame(Opcodes.F_APPEND,");
                sb.push_str(&local.len().to_string());
                sb.push_str(NEW_OBJECT_ARRAY);
                self.append_frame_types(&mut sb, local);
                sb.push_str("}, 0, null");
            }
            FrameKind::Chop(num_local) => {
                sb.push_str(&self.name);
                sb.push_str(&format!(
                    ".visitFrame(Opcodes.F_CHOP,{num_local}, null, 0, null"
                ));
            }
            FrameKind::Same => {
                sb.push_str(&self.name);
                sb.push_str(".visitFrame(Opcodes.F_SAME, 0, null, 0, null");
            }
            FrameKind::Same1 => {
                let stack = &stack[..stack.len().min(1)];
                self.declare_frame_types(&mut sb, stack);
                sb.push_str(&self.name);
                sb.push_str(".visitFrame(Opcodes.F_SAME1, 0, null, 1, new Object[] {");
                self.append_frame_types(&mut sb, stack);
                sb.push('}');
            }
        }
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_insn(&mut self, opcode: u8) {
        let line = format!("{}.visitInsn({});\n", self.name, opcode_name(opcode));
        self.push(line.as_str().into());
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        let mut sb = self.start_insn("visitIntInsn", opcode);
        let type_name = usize::try_from(operand).ok().and_then(|i| TYPES.get(i));
        match type_name {
            Some(type_name) if opcode == opcodes::NEWARRAY => sb.push_str(type_name),
            _ => sb.push_str(&operand.to_string()),
        }
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        let mut sb = self.start_insn("visitVarInsn", opcode);
        sb.push_str(&format!("{var_index});\n"));
        self.push(sb);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        let mut sb = self.start_insn("visitTypeInsn", opcode);
        append_string(&mut sb, type_);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        let mut sb = self.start_insn("visitFieldInsn", opcode);
        append_string(&mut sb, owner);
        sb.push_str(", ");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(");\n");
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
        let mut sb = self.start_insn("visitMethodInsn", opcode);
        append_string(&mut sb, owner);
        sb.push_str(", ");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {is_interface});\n"));
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
        sb.push_str(&self.name);
        sb.push_str(".visitInvokeDynamicInsn(");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_handle(&mut sb, bootstrap_method_handle);
        sb.push_str(", new Object[]{");
        append_constants(&mut sb, bootstrap_method_arguments);
        sb.push_str("});\n");
        self.push(sb);
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        let mut sb = JavaString::new();
        self.declare_label(&mut sb, label);
        sb.push_str(&self.name);
        sb.push_str(".visitJumpInsn(");
        sb.push_str(&opcode_name(opcode));
        sb.push_str(", ");
        self.append_label(&mut sb, label);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_label(&mut self, label: Label) {
        let mut sb = JavaString::new();
        self.declare_label(&mut sb, label);
        sb.push_str(&self.name);
        sb.push_str(".visitLabel(");
        self.append_label(&mut sb, label);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push_str(".visitLdcInsn(");
        append_constant(&mut sb, value);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        let line = format!("{}.visitIincInsn({var_index}, {increment});\n", self.name);
        self.push(line.as_str().into());
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        let mut sb = JavaString::new();
        for &label in labels {
            self.declare_label(&mut sb, label);
        }
        self.declare_label(&mut sb, dflt);
        sb.push_str(&self.name);
        sb.push_str(&format!(".visitTableSwitchInsn({min}, {max}, "));
        self.append_label(&mut sb, dflt);
        sb.push_str(", new Label[] {");
        for (i, &label) in labels.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            self.append_label(&mut sb, label);
        }
        sb.push_str(END_ARRAY);
        self.push(sb);
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        let mut sb = JavaString::new();
        for &label in labels {
            self.declare_label(&mut sb, label);
        }
        self.declare_label(&mut sb, dflt);
        sb.push_str(&self.name);
        sb.push_str(".visitLookupSwitchInsn(");
        self.append_label(&mut sb, dflt);
        sb.push_str(", new int[] {");
        for (i, key) in keys.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            sb.push_str(&key.to_string());
        }
        sb.push_str(" }, new Label[] {");
        for (i, &label) in labels.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            self.append_label(&mut sb, label);
        }
        sb.push_str(END_ARRAY);
        self.push(sb);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push_str(".visitMultiANewArrayInsn(");
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {num_dimensions});\n"));
        self.push(sb);
    }

    fn visit_insn_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(
            "visitInsnAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        let mut sb = JavaString::new();
        self.declare_label(&mut sb, start);
        self.declare_label(&mut sb, end);
        self.declare_label(&mut sb, handler);
        sb.push_str(&self.name);
        sb.push_str(".visitTryCatchBlock(");
        self.append_label(&mut sb, start);
        sb.push_str(", ");
        self.append_label(&mut sb, end);
        sb.push_str(", ");
        self.append_label(&mut sb, handler);
        sb.push_str(", ");
        append_nullable_string(&mut sb, type_);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_try_catch_annotation(
        &mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> &mut dyn Printer {
        self.visit_type_annotation(
            "visitTryCatchAnnotation",
            type_ref,
            type_path,
            descriptor,
            visible,
        )
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
        sb.push_str(&self.name);
        sb.push_str(".visitLocalVariable(");
        append_string(&mut sb, name);
        sb.push_str(", ");
        append_string(&mut sb, descriptor);
        sb.push_str(", ");
        append_nullable_string(&mut sb, signature);
        sb.push_str(", ");
        self.append_label(&mut sb, start);
        sb.push_str(", ");
        self.append_label(&mut sb, end);
        sb.push_str(&format!(", {index});\n"));
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
        sb.push_str("{\n");
        sb.push_str(ANNOTATION_VISITOR0);
        sb.push_str(&self.name);
        sb.push_str(".visitLocalVariableAnnotation(");
        append_type_ref_and_path(&mut sb, type_ref, type_path);
        sb.push_str("new Label[] {");
        for (i, &label) in start.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            self.append_label(&mut sb, label);
        }
        sb.push_str(" }, new Label[] {");
        for (i, &label) in end.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            self.append_label(&mut sb, label);
        }
        sb.push_str(" }, new int[] {");
        for (i, index) in index.iter().enumerate() {
            sb.push_str(if i == 0 { " " } else { ", " });
            sb.push_str(&index.to_string());
        }
        sb.push_str(" }, ");
        append_string(&mut sb, descriptor);
        sb.push_str(&format!(", {visible});\n"));
        self.push(sb);
        self.add_new_asmifier(ANNOTATION_VISITOR, 0)
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        let mut sb = JavaString::new();
        sb.push_str(&self.name);
        sb.push_str(&format!(".visitLineNumber({line}, "));
        self.append_label(&mut sb, start);
        sb.push_str(");\n");
        self.push(sb);
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        let line = format!("{}.visitMaxs({max_stack}, {max_locals});\n", self.name);
        self.push(line.as_str().into());
    }

    fn visit_method_end(&mut self) {
        self.visit_member_end();
    }
}

// -------------------------------------------------------------------------------------------------
// Utility functions
// -------------------------------------------------------------------------------------------------

/// The name of the `Opcodes` constant of a class version, if it has one (`CLASS_VERSIONS`).
fn class_version_name(version: u32) -> Option<String> {
    match version {
        0x0003_002D => Some("V1_1".to_owned()),
        46..=52 => Some(format!("V1_{}", version - 44)),
        53..=71 => Some(format!("V{}", version - 44)),
        _ => None,
    }
}

fn opcode_name(opcode: u8) -> std::borrow::Cow<'static, str> {
    match OPCODES.get(usize::from(opcode)) {
        Some(name) => (*name).into(),
        None => opcode.to_string().into(),
    }
}

/// Appends a quoted and escaped string, or `null` (`appendConstant` of a `String` or `null`).
fn append_nullable_string(sb: &mut JavaString, value: Option<&JavaStr>) {
    match value {
        Some(value) => append_string(sb, value),
        None => sb.push_str("null"),
    }
}

/// Appends `new String[] { "a", "b" }`, or `null` for an empty array.
fn append_string_array(sb: &mut JavaString, values: &[&JavaStr]) {
    if values.is_empty() {
        sb.push_str("null");
    } else {
        sb.push_str("new String[] {");
        append_string_array_elements(sb, values);
        sb.push_str(" }");
    }
}

fn append_string_array_elements(sb: &mut JavaString, values: &[&JavaStr]) {
    for (i, value) in values.iter().enumerate() {
        sb.push_str(if i == 0 { " " } else { ", " });
        append_string(sb, value);
    }
}

/// Appends `<type_ref>, null, ` or `<type_ref>, TypePath.fromString("<type_path>"), `.
fn append_type_ref_and_path(
    sb: &mut JavaString,
    type_ref: TypeReference,
    type_path: Option<&TypePath>,
) {
    // Java prints the type reference as a (signed) int.
    sb.push_str(&type_ref.get_value().cast_signed().to_string());
    match type_path {
        None => sb.push_str(", null, "),
        Some(type_path) => sb.push_str(&format!(", TypePath.fromString(\"{type_path}\"), ")),
    }
}

/// Appends a string representation of the given access flags (`appendAccessFlags`). The pseudo
/// flags `ACCESS_*` tell which kind of structure the flags belong to.
fn append_access_flags(sb: &mut JavaString, access_flags: u32) {
    let mut is_empty = true;
    // ASM appends the first three without a separator.
    for (flag, name) in [
        (opcodes::ACC_PUBLIC, "ACC_PUBLIC"),
        (opcodes::ACC_PRIVATE, "ACC_PRIVATE"),
        (opcodes::ACC_PROTECTED, "ACC_PROTECTED"),
    ] {
        if access_flags & flag != 0 {
            sb.push_str(name);
            is_empty = false;
        }
    }
    let mut append = |sb: &mut JavaString, name: &str| {
        if !is_empty {
            sb.push_str(" | ");
        }
        sb.push_str(name);
        is_empty = false;
    };
    if access_flags & opcodes::ACC_FINAL != 0 {
        append(sb, "ACC_FINAL");
    }
    if access_flags & opcodes::ACC_STATIC != 0 {
        append(sb, "ACC_STATIC");
    }
    if access_flags & (opcodes::ACC_SYNCHRONIZED | opcodes::ACC_SUPER | opcodes::ACC_TRANSITIVE)
        != 0
    {
        append(
            sb,
            if access_flags & ACCESS_CLASS != 0 {
                "ACC_SUPER"
            } else if access_flags & ACCESS_MODULE != 0 {
                "ACC_TRANSITIVE"
            } else {
                "ACC_SYNCHRONIZED"
            },
        );
    }
    if access_flags & (opcodes::ACC_VOLATILE | opcodes::ACC_BRIDGE | opcodes::ACC_STATIC_PHASE) != 0
    {
        append(
            sb,
            if access_flags & ACCESS_FIELD != 0 {
                "ACC_VOLATILE"
            } else if access_flags & ACCESS_MODULE != 0 {
                "ACC_STATIC_PHASE"
            } else {
                "ACC_BRIDGE"
            },
        );
    }
    if access_flags & opcodes::ACC_VARARGS != 0 && access_flags & (ACCESS_CLASS | ACCESS_FIELD) == 0
    {
        append(sb, "ACC_VARARGS");
    }
    if access_flags & opcodes::ACC_TRANSIENT != 0 && access_flags & ACCESS_FIELD != 0 {
        append(sb, "ACC_TRANSIENT");
    }
    if access_flags & opcodes::ACC_NATIVE != 0 && access_flags & (ACCESS_CLASS | ACCESS_FIELD) == 0
    {
        append(sb, "ACC_NATIVE");
    }
    if access_flags & opcodes::ACC_ENUM != 0
        && access_flags & (ACCESS_CLASS | ACCESS_FIELD | ACCESS_INNER) != 0
    {
        append(sb, "ACC_ENUM");
    }
    if access_flags & opcodes::ACC_ANNOTATION != 0
        && access_flags & (ACCESS_CLASS | ACCESS_INNER) != 0
    {
        append(sb, "ACC_ANNOTATION");
    }
    for (flag, name) in [
        (opcodes::ACC_ABSTRACT, "ACC_ABSTRACT"),
        (opcodes::ACC_INTERFACE, "ACC_INTERFACE"),
        (opcodes::ACC_STRICT, "ACC_STRICT"),
        (opcodes::ACC_SYNTHETIC, "ACC_SYNTHETIC"),
        (opcodes::ACC_DEPRECATED, "ACC_DEPRECATED"),
        (opcodes::ACC_RECORD, "ACC_RECORD"),
    ] {
        if access_flags & flag != 0 {
            append(sb, name);
        }
    }
    if access_flags & (opcodes::ACC_MANDATED | opcodes::ACC_MODULE) != 0 {
        append(
            sb,
            if access_flags & ACCESS_CLASS == 0 {
                "ACC_MANDATED"
            } else {
                "ACC_MODULE"
            },
        );
    }
    if is_empty {
        sb.push('0');
    }
}

/// Appends the Java expression of a constant (`appendConstant` of an `Integer`, `Float`, `Long`,
/// `Double`, `String`, `Type`, `Handle` or `ConstantDynamic`).
fn append_constant(sb: &mut JavaString, value: &Constant<'_>) {
    match value {
        Constant::Integer(v) => sb.push_str(&format!("Integer.valueOf({v})")),
        Constant::Float(v) => sb.push_str(&format!("Float.valueOf(\"{}\")", float_to_string(*v))),
        Constant::Long(v) => sb.push_str(&format!("Long.valueOf({v}L)")),
        Constant::Double(v) => {
            sb.push_str(&format!("Double.valueOf(\"{}\")", double_to_string(*v)));
        }
        Constant::String(s) => append_string(sb, s),
        Constant::Type(t) => {
            sb.push_str("Type.getType(\"");
            sb.push_java_str(&t.get_descriptor());
            sb.push_str("\")");
        }
        Constant::Handle(handle) => append_handle(sb, handle),
        Constant::Dynamic(condy) => append_constant_dynamic(sb, condy),
    }
}

/// Appends the given constants, separated with `, `.
fn append_constants(sb: &mut JavaString, values: &[Constant<'_>]) {
    for (i, value) in values.iter().enumerate() {
        if i != 0 {
            sb.push_str(", ");
        }
        append_constant(sb, value);
    }
}

/// Appends `new Handle(Opcodes.<tag>, "<owner>", "<name>", "<descriptor>", <is_interface>)`. The
/// strings are not escaped, as in ASM.
fn append_handle(sb: &mut JavaString, handle: &Handle) {
    sb.push_str("new Handle(Opcodes.");
    sb.push_str(HANDLE_TAG[usize::from(handle.get_tag().tag())]);
    sb.push_str(", \"");
    sb.push_java_str(handle.get_owner());
    sb.push_str(COMMA);
    sb.push_java_str(handle.get_name());
    sb.push_str(COMMA);
    sb.push_java_str(handle.get_desc());
    sb.push_str(&format!("\", {})", handle.is_interface()));
}

/// Appends `new ConstantDynamic("<name>", "<descriptor>", <handle>, new Object[] {<args>})`.
/// Its depth is bounded by the reader (ADR-06).
fn append_constant_dynamic(sb: &mut JavaString, condy: &ConstantDynamic) {
    sb.push_str("new ConstantDynamic(\"");
    sb.push_java_str(condy.get_name());
    sb.push_str(COMMA);
    sb.push_java_str(condy.get_descriptor());
    sb.push_str("\", ");
    append_handle(sb, condy.get_bootstrap_method());
    sb.push_str(NEW_OBJECT_ARRAY);
    append_constants(sb, condy.get_bootstrap_method_arguments());
    sb.push_str("})");
}

/// Appends the Java expression of an annotation value (`appendConstant` of a boxed primitive,
/// a `String`, a `Type` or a primitive array).
fn append_annotation_value(sb: &mut JavaString, value: &AnnotationValue<'_>) {
    fn array<T: Copy>(
        sb: &mut JavaString,
        java_type: &str,
        values: &[T],
        append: impl Fn(&mut JavaString, T),
    ) {
        sb.push_str(&format!("new {java_type}[] {{"));
        for (i, &value) in values.iter().enumerate() {
            if i != 0 {
                sb.push(',');
            }
            append(sb, value);
        }
        sb.push('}');
    }
    match value {
        AnnotationValue::Byte(v) => sb.push_str(&format!("Byte.valueOf((byte){v})")),
        AnnotationValue::Boolean(v) => {
            sb.push_str(if *v { "Boolean.TRUE" } else { "Boolean.FALSE" });
        }
        AnnotationValue::Short(v) => sb.push_str(&format!("Short.valueOf((short){v})")),
        AnnotationValue::Char(v) => sb.push_str(&format!("Character.valueOf((char){v})")),
        AnnotationValue::Int(v) => sb.push_str(&format!("Integer.valueOf({v})")),
        AnnotationValue::Float(v) => {
            sb.push_str(&format!("Float.valueOf(\"{}\")", float_to_string(*v)));
        }
        AnnotationValue::Long(v) => sb.push_str(&format!("Long.valueOf({v}L)")),
        AnnotationValue::Double(v) => {
            sb.push_str(&format!("Double.valueOf(\"{}\")", double_to_string(*v)));
        }
        AnnotationValue::String(s) => append_string(sb, s),
        AnnotationValue::Type(t) => {
            sb.push_str("Type.getType(\"");
            sb.push_java_str(&t.get_descriptor());
            sb.push_str("\")");
        }
        AnnotationValue::ByteArray(values) => {
            array(sb, "byte", values, |sb, v| sb.push_str(&v.to_string()));
        }
        AnnotationValue::BooleanArray(values) => {
            array(sb, "boolean", values, |sb, v| sb.push_str(&v.to_string()));
        }
        AnnotationValue::ShortArray(values) => {
            array(sb, "short", values, |sb, v| {
                sb.push_str(&format!("(short){v}"))
            });
        }
        AnnotationValue::CharArray(values) => {
            array(sb, "char", values, |sb, v| {
                sb.push_str(&format!("(char){v}"))
            });
        }
        AnnotationValue::IntArray(values) => {
            array(sb, "int", values, |sb, v| sb.push_str(&v.to_string()));
        }
        AnnotationValue::LongArray(values) => {
            array(sb, "long", values, |sb, v| sb.push_str(&format!("{v}L")));
        }
        AnnotationValue::FloatArray(values) => {
            array(sb, "float", values, |sb, v| {
                sb.push_str(&float_to_string(v));
                sb.push('f');
            });
        }
        AnnotationValue::DoubleArray(values) => {
            array(sb, "double", values, |sb, v| {
                sb.push_str(&double_to_string(v));
                sb.push('d');
            });
        }
    }
}
