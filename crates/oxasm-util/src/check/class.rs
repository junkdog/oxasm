//! [`CheckClassAdapter`]: ASM's `CheckClassAdapter`, and its static `verify` and `main` methods.

use std::collections::HashMap;
use std::io;

use oxasm::constant::Constant;
use oxasm::opcodes::*;
use oxasm::{
    AnnotationVisitor, Attribute, ClassHierarchy, ClassReader, ClassVisitor, FieldVisitor, JavaStr,
    JavaString, Label, MethodVisitor, ModuleVisitor, ParsingOptions, RecordComponentVisitor, Type,
    TypePath, TypeReference,
};
use oxasm_analysis::{Analyzer, SimpleVerifier};
use oxasm_tree::ClassNode;

use crate::check::java_chars::DIGIT;
use crate::check::method::{MethodWriterWrapper, print_analyzer_result};
use crate::check::support::{
    Utf16, check_access, check_class_signature, check_constant, check_descriptor,
    check_field_signature, check_fully_qualified_name, check_identifier, check_internal_name,
    check_method_access, check_method_descriptor, check_method_identifier, check_method_signature,
    check_type_ref, check_type_ref_sort, check_unqualified_name, illegal_argument, illegal_state,
    in_ranges,
};
use crate::check::{
    CheckAnnotationAdapter, CheckFieldAdapter, CheckMethodAdapter, CheckModuleAdapter,
    CheckRecordComponentAdapter,
};
use crate::trace::NoDelegate;

/// The help message shown when command line arguments are incorrect.
const USAGE: &str = "Verifies the given class.\nUsage: CheckClassAdapter <fully qualified class name or class file name>";

/// A [`ClassVisitor`] that checks that its methods are properly used, and forwards them to a
/// downstream visitor `V` ([`NoDelegate`] for none).
///
/// More precisely, it checks each method call individually, based *only* on its arguments, but
/// does *not* check the *sequence* of method calls. For example, the invalid sequence
/// `visit_field(ACC_PUBLIC, "i", "I", None, None)` `visit_field(ACC_PUBLIC, "i", "D", None,
/// None)` is *not* detected.
///
/// The methods are checked with [`CheckMethodAdapter`]s, with basic data flow checks unless
/// disabled ([`with_check_data_flow`](Self::with_check_data_flow)). If the downstream visitor is
/// a [`ClassWriter`](oxasm::ClassWriter), these also check the maximum stack sizes and the stack
/// map frames that it does not compute.
///
/// It can be used to check bytecode transformations. [`verify`] then checks the transformed
/// class with a [`SimpleVerifier`]. These checks are not exactly the JVM's, but provide some
/// basic type checking for each instruction.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckClassAdapter<V> {
    cv: V,
    /// Whether the bytecode must be checked with a `BasicVerifier`.
    check_data_flow: bool,
    /// The class version number.
    version: u32,
    visit_called: bool,
    visit_module_called: bool,
    visit_source_called: bool,
    visit_outer_class_called: bool,
    visit_nest_host_called: bool,
    /// The common package of all the nest members, once `visit_nest_member` has been called.
    nest_member_package_name: Option<JavaString>,
    visit_end_called: bool,
    /// The index of the instruction designated by each visited label so far.
    label_insn_indices: HashMap<Label, usize>,
}

impl<V: ClassVisitor> CheckClassAdapter<V> {
    /// Constructs a new adapter that forwards the events to `class_visitor`, with data flow
    /// checks.
    pub fn new(class_visitor: V) -> Self {
        CheckClassAdapter::with_check_data_flow(class_visitor, true)
    }

    /// Constructs a new adapter that forwards the events to `class_visitor`, with basic data flow
    /// checks of the methods or not (see [`CheckMethodAdapter`]).
    pub fn with_check_data_flow(class_visitor: V, check_data_flow: bool) -> Self {
        CheckClassAdapter {
            cv: class_visitor,
            check_data_flow,
            version: 0,
            visit_called: false,
            visit_module_called: false,
            visit_source_called: false,
            visit_outer_class_called: false,
            visit_nest_host_called: false,
            nest_member_package_name: None,
            visit_end_called: false,
            label_insn_indices: HashMap::new(),
        }
    }

    /// The downstream visitor.
    pub fn class_visitor(&mut self) -> &mut V {
        &mut self.cv
    }

    /// The downstream visitor, taken out of this adapter.
    pub fn into_inner(self) -> V {
        self.cv
    }

    /// Checks that `visit` has been called and that `visit_end` has not.
    fn check_state(&self) {
        if !self.visit_called {
            illegal_state("Cannot visit member before visit has been called.");
        }
        if self.visit_end_called {
            illegal_state("Cannot visit member after visitEnd has been called.");
        }
    }
}

impl<V> std::fmt::Debug for CheckClassAdapter<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckClassAdapter")
            .field("check_data_flow", &self.check_data_flow)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The package name of an internal name, or "" if there is no package.
fn package_name(name: &JavaStr) -> JavaString {
    let units: Vec<u16> = name.encode_utf16().collect();
    match units.iter().rposition(|&c| c == u16::from(b'/')) {
        None => JavaString::new(),
        Some(index) => JavaString::from_utf16(&units[..index]),
    }
}

/// Java's `Character.isDigit(char)`.
fn is_digit(c: u16) -> bool {
    in_ranges(DIGIT, u32::from(c))
}

impl<V: ClassVisitor> ClassVisitor for CheckClassAdapter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.cv)
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
        if self.visit_called {
            illegal_state("visit must be called only once");
        }
        self.visit_called = true;
        self.check_state();
        check_access(
            access,
            ACC_PUBLIC
                | ACC_FINAL
                | ACC_SUPER
                | ACC_INTERFACE
                | ACC_ABSTRACT
                | ACC_SYNTHETIC
                | ACC_ANNOTATION
                | ACC_ENUM
                | ACC_DEPRECATED
                | ACC_RECORD
                | ACC_MODULE,
        );
        if !name.ends_with("package-info") && !name.ends_with("module-info") {
            check_internal_name(version, name, "class name");
        }
        if name == "java/lang/Object" {
            if super_name.is_some() {
                illegal_argument("The super class name of the Object class must be 'null'");
            }
        } else if name.ends_with("module-info") {
            if super_name.is_some() {
                illegal_argument("The super class name of a module-info class must be 'null'");
            }
        } else {
            check_internal_name(version, super_name.unwrap_or_default(), "super class name");
        }
        if let Some(signature) = signature {
            check_class_signature(signature);
        }
        if access & ACC_INTERFACE != 0 && super_name.is_none_or(|s| s != "java/lang/Object") {
            illegal_argument("The super class name of interfaces must be 'java/lang/Object'");
        }
        for (i, interface) in interfaces.iter().enumerate() {
            check_internal_name(version, interface, &format!("interface name at index {i}"));
        }
        self.version = version;
        self.cv
            .visit(version, access, name, signature, super_name, interfaces);
    }

    fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
        self.check_state();
        if self.visit_source_called {
            illegal_state("visitSource can be called only once.");
        }
        self.visit_source_called = true;
        self.cv.visit_source(source, debug);
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        self.check_state();
        if self.visit_module_called {
            illegal_state("visitModule can be called only once.");
        }
        self.visit_module_called = true;
        check_fully_qualified_name(self.version, name, "module name");
        check_access(access, ACC_OPEN | ACC_SYNTHETIC | ACC_MANDATED);
        let class_version = self.version;
        let mut check_module_adapter = CheckModuleAdapter::new(
            self.cv.visit_module(name, access, version),
            access & ACC_OPEN != 0,
        );
        check_module_adapter.class_version = class_version;
        Some(Box::new(check_module_adapter))
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.check_state();
        check_internal_name(self.version, nest_host, "nestHost");
        if self.visit_nest_host_called {
            illegal_state("visitNestHost can be called only once.");
        }
        if self.nest_member_package_name.is_some() {
            illegal_state("visitNestHost and visitNestMember are mutually exclusive.");
        }
        self.visit_nest_host_called = true;
        self.cv.visit_nest_host(nest_host);
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        self.check_state();
        check_internal_name(self.version, nest_member, "nestMember");
        if self.visit_nest_host_called {
            illegal_state("visitMemberOfNest and visitNestHost are mutually exclusive.");
        }
        let package_name = package_name(nest_member);
        match &self.nest_member_package_name {
            None => self.nest_member_package_name = Some(package_name),
            Some(nest_member_package_name) if *nest_member_package_name != package_name => {
                illegal_state(format_args!(
                    "nest member {nest_member} should be in the package {nest_member_package_name}"
                ));
            }
            Some(_) => {}
        }
        self.cv.visit_nest_member(nest_member);
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        self.check_state();
        check_internal_name(self.version, permitted_subclass, "permittedSubclass");
        self.cv.visit_permitted_subclass(permitted_subclass);
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        self.check_state();
        if self.visit_outer_class_called {
            illegal_state("visitOuterClass can be called only once.");
        }
        self.visit_outer_class_called = true;
        if let Some(descriptor) = descriptor {
            check_method_descriptor(self.version, descriptor);
        }
        self.cv.visit_outer_class(owner, name, descriptor);
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        self.check_state();
        check_internal_name(self.version, name, "class name");
        if let Some(outer_name) = outer_name {
            check_internal_name(self.version, outer_name, "outer class name");
        }
        if let Some(inner_name) = inner_name {
            let utf16 = Utf16::new(inner_name);
            let mut start_index = 0;
            while start_index < utf16.len() && is_digit(utf16.char_at(start_index)) {
                start_index += 1;
            }
            if start_index == 0 || start_index < utf16.len() {
                check_identifier(
                    self.version,
                    inner_name,
                    start_index,
                    None,
                    "inner class name",
                );
            }
        }
        check_access(
            access,
            ACC_PUBLIC
                | ACC_PRIVATE
                | ACC_PROTECTED
                | ACC_STATIC
                | ACC_FINAL
                | ACC_INTERFACE
                | ACC_ABSTRACT
                | ACC_SYNTHETIC
                | ACC_ANNOTATION
                | ACC_ENUM,
        );
        self.cv
            .visit_inner_class(name, outer_name, inner_name, access);
    }

    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        self.check_state();
        check_unqualified_name(self.version, name, "record component name");
        check_descriptor(self.version, descriptor, false);
        if let Some(signature) = signature {
            check_field_signature(signature);
        }
        Some(Box::new(CheckRecordComponentAdapter::new(
            self.cv.visit_record_component(name, descriptor, signature),
        )))
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        self.check_state();
        check_access(
            access,
            ACC_PUBLIC
                | ACC_PRIVATE
                | ACC_PROTECTED
                | ACC_STATIC
                | ACC_FINAL
                | ACC_VOLATILE
                | ACC_TRANSIENT
                | ACC_SYNTHETIC
                | ACC_ENUM
                | ACC_MANDATED
                | ACC_DEPRECATED,
        );
        check_unqualified_name(self.version, name, "field name");
        check_descriptor(self.version, descriptor, false);
        if let Some(signature) = signature {
            check_field_signature(signature);
        }
        if let Some(value) = value {
            check_constant(value);
        }
        Some(Box::new(CheckFieldAdapter::new(
            self.cv
                .visit_field(access, name, descriptor, signature, value),
        )))
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.check_state();
        check_method_access(
            self.version,
            access,
            ACC_PUBLIC
                | ACC_PRIVATE
                | ACC_PROTECTED
                | ACC_STATIC
                | ACC_FINAL
                | ACC_SYNCHRONIZED
                | ACC_BRIDGE
                | ACC_VARARGS
                | ACC_NATIVE
                | ACC_ABSTRACT
                | ACC_STRICT
                | ACC_SYNTHETIC
                | ACC_MANDATED
                | ACC_DEPRECATED,
        );
        if name != "<init>" && name != "<clinit>" {
            check_method_identifier(self.version, name, "method name");
        }
        check_method_descriptor(self.version, descriptor);
        if let Some(signature) = signature {
            check_method_signature(signature);
        }
        for (i, exception) in exceptions.iter().enumerate() {
            check_internal_name(
                self.version,
                exception,
                &format!("exception name at index {i}"),
            );
        }
        let version = self.version;
        // ASM's `cv instanceof ClassWriter` (ADR-08).
        let class_writer_flags = self.cv.class_writer_flags();
        let method_visitor = self
            .cv
            .visit_method(access, name, descriptor, signature, exceptions);
        let check_method_adapter = if self.check_data_flow {
            let adapter =
                CheckMethodAdapter::with_data_flow(access, name, descriptor, method_visitor);
            match class_writer_flags {
                Some(flags) => adapter.with_method_writer(MethodWriterWrapper { version, flags }),
                None => adapter,
            }
        } else {
            CheckMethodAdapter::new(method_visitor)
        };
        let mut check_method_adapter =
            check_method_adapter.with_label_insn_indices(&mut self.label_insn_indices);
        check_method_adapter.version = version;
        Some(Box::new(check_method_adapter))
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.check_state();
        check_descriptor(self.version, descriptor, false);
        CheckAnnotationAdapter::boxed(self.cv.visit_annotation(descriptor, visible), true)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.check_state();
        check_type_ref_sort(
            type_ref,
            &[
                TypeReference::CLASS_TYPE_PARAMETER,
                TypeReference::CLASS_TYPE_PARAMETER_BOUND,
                TypeReference::CLASS_EXTENDS,
            ],
        );
        check_type_ref(type_ref);
        check_descriptor(self.version, descriptor, false);
        CheckAnnotationAdapter::boxed(
            self.cv
                .visit_type_annotation(type_ref, type_path, descriptor, visible),
            true,
        )
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.check_state();
        self.cv.visit_attribute(attribute);
    }

    fn visit_end(&mut self) {
        self.check_state();
        self.visit_end_called = true;
        self.cv.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// Static verification methods
// -------------------------------------------------------------------------------------------------

/// Checks the given class with a [`CheckClassAdapter`] (without data flow checks), then the
/// methods with a [`SimpleVerifier`] over `hierarchy` (ASM's `CheckClassAdapter.verify`): the
/// errors are printed to `print_writer`, and, if `print_results`, the frames of each method.
///
/// ASM prints the stack trace of each analysis error; this prints its first line,
/// `org.objectweb.asm.tree.analysis.AnalyzerException: <message>`.
///
/// # Errors
///
/// If the class cannot be read (as [`io::ErrorKind::InvalidData`]), or `print_writer` fails.
///
/// # Panics
///
/// If the [`CheckClassAdapter`] does.
pub fn verify(
    class_reader: &ClassReader<'_>,
    hierarchy: &dyn ClassHierarchy,
    print_results: bool,
    print_writer: &mut dyn io::Write,
) -> io::Result<()> {
    let mut class_node = ClassNode::new();
    class_reader
        .accept(
            &mut CheckClassAdapter::with_check_data_flow(&mut class_node, false),
            ParsingOptions::SKIP_DEBUG,
        )
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let super_type = class_node.super_name.as_deref().map(Type::get_object_type);
    let interfaces: Vec<Type> = class_node
        .interfaces
        .iter()
        .map(|interface| Type::get_object_type(&**interface))
        .collect();
    let is_interface = class_node.access & ACC_INTERFACE != 0;

    let mut text = JavaString::new();
    for method in &class_node.methods {
        let verifier = SimpleVerifier::for_class(
            Type::get_object_type(&*class_node.name),
            super_type.clone(),
            interfaces.clone(),
            is_interface,
            hierarchy,
        );
        let mut analyzer = Analyzer::new(verifier);
        if let Err(e) = analyzer.analyze(&class_node.name, method) {
            text.push_str("org.objectweb.asm.tree.analysis.AnalyzerException: ");
            text.push_java_str(&e.message);
            text.push('\n');
        }
        if print_results {
            print_analyzer_result(
                method,
                analyzer.get_frames(),
                i64::from(method.max_stack),
                i64::from(method.max_locals),
                &mut text,
            );
        }
    }
    print_writer.write_all(text.as_bytes())?;
    print_writer.flush()
}

/// Checks the given class (ASM's `CheckClassAdapter.main`): `args` is a class file name;
/// anything else prints the usage to `logger`. The errors are printed to `logger`.
///
/// ASM also accepts a fully qualified class name, loaded from the class path; there is no class
/// path here, so such an argument is a "Class not found" error. The classes the methods use
/// are known through `hierarchy`, which plays the role of ASM's class loader.
///
/// # Errors
///
/// If the class file cannot be read or parsed, or the output cannot be written.
pub fn main(
    args: &[&str],
    hierarchy: &dyn ClassHierarchy,
    logger: &mut dyn io::Write,
) -> io::Result<()> {
    let [class_name] = args else {
        writeln!(logger, "{USAGE}")?;
        return Ok(());
    };
    if !class_name.ends_with(".class") {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Class not found"));
    }
    let class_file = std::fs::read(class_name)?;
    let class_reader =
        ClassReader::new(&class_file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    verify(&class_reader, hierarchy, false, logger)
}

/// A [`CheckClassAdapter`] without a downstream visitor.
impl Default for CheckClassAdapter<NoDelegate> {
    fn default() -> Self {
        CheckClassAdapter::new(NoDelegate)
    }
}
