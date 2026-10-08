//! [`ClassNode`]: ASM's `ClassNode`.

use std::sync::Arc;

use oxasm::constant::Constant;
use oxasm::sym::InternMemo;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FieldVisitor, Interner, JavaStr, MethodVisitor,
    ModuleVisitor, RecordComponentVisitor, Sym, TypePath, TypeReference,
};

use crate::annotation_node::{AnnotationNode, TypeAnnotationNode};
use crate::field_node::{
    FieldNode, RecordComponentNode, accept_annotations, add_annotation, add_type_annotation,
};
use crate::method_node::MethodNode;
use crate::method_node::PooledMethodNode;
use crate::module_node::ModuleNode;
use crate::nodes::InnerClassNode;
use crate::strings::{Strings, resolve_all};

/// A node that represents a class.
///
/// A class reader fills it by visiting it, and [`accept`](Self::accept) replays it into any
/// class visitor. Its strings, and those of its fields, methods and their parts, are [`Sym`]s of
/// its [`Interner`] (ADR-04's P9 amendment), which [`resolve`](Self::resolve) turns back into
/// strings. Its fields and methods hold the same interner.
///
/// Share one interner between the class nodes of a classpath ([`with_interner`](Self::with_interner)):
/// each distinct string is then stored once, and two symbols of it are equal. There is no
/// constructor that makes the interner implicitly (nor a `Default`): symbols of two interners
/// compare as integers, so two class nodes that each made their own would compare equal symbols
/// for different strings. A class node on its own takes `Arc::default()`.
///
/// ```
/// # fn main() -> Result<(), oxasm::error::ReadError> {
/// # let class_file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),
/// #     "/../../corpus/t0/jdk3/AllInstructions.class")).unwrap();
/// use std::sync::Arc;
///
/// use oxasm::{ClassReader, Interner, ParsingOptions, sym};
/// use oxasm_tree::ClassNode;
///
/// let interner = Arc::new(Interner::new());
/// let mut class_node = ClassNode::with_interner(interner.clone());
/// ClassReader::new(&class_file)?.accept(&mut class_node, ParsingOptions::empty())?;
/// assert_eq!(interner.resolve(class_node.name), "jdk3/AllInstructions");
/// assert_eq!(class_node.super_name, Some(sym::JAVA_LANG_OBJECT));
/// assert!(class_node.methods.iter().any(|m| m.name == sym::INIT));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct ClassNode {
    /// The class version. The minor version is stored in the 16 most significant bits, and the
    /// major version in the 16 least significant bits.
    pub version: u32,
    /// The class's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// class is deprecated (`ACC_DEPRECATED`) or a record (`ACC_RECORD`).
    pub access: u32,
    /// The internal name of this class.
    pub name: Sym,
    /// The signature of this class, or `None`.
    pub signature: Option<Sym>,
    /// The internal name of the super class. For interfaces, the super class is
    /// `java/lang/Object`. `None` only for the `java/lang/Object` class.
    pub super_name: Option<Sym>,
    /// The internal names of the interfaces directly implemented by this class.
    pub interfaces: Vec<Sym>,
    /// The name of the source file from which this class was compiled, or `None`.
    pub source_file: Option<Sym>,
    /// The correspondence between source and compiled elements of this class, or `None`.
    pub source_debug: Option<Sym>,
    /// The module stored in this class, or `None`.
    pub module: Option<ModuleNode>,
    /// The internal name of the enclosing class of this class (see `visit_outer_class`), or
    /// `None`.
    pub outer_class: Option<Sym>,
    /// The name of the method that contains this class, or `None` if this class is not enclosed
    /// in a method.
    pub outer_method: Option<Sym>,
    /// The descriptor of the method that contains this class, or `None` if this class is not
    /// enclosed in a method.
    pub outer_method_desc: Option<Sym>,
    /// The runtime visible annotations of this class.
    pub visible_annotations: Vec<AnnotationNode>,
    /// The runtime invisible annotations of this class.
    pub invisible_annotations: Vec<AnnotationNode>,
    /// The runtime visible type annotations of this class.
    pub visible_type_annotations: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of this class.
    pub invisible_type_annotations: Vec<TypeAnnotationNode>,
    /// The non standard attributes of this class.
    pub attrs: Vec<Attribute>,
    /// The inner classes of this class.
    pub inner_classes: Vec<InnerClassNode>,
    /// The internal name of the nest host class of this class, or `None`.
    pub nest_host_class: Option<Sym>,
    /// The internal names of the nest members of this class.
    pub nest_members: Vec<Sym>,
    /// The internal names of the permitted subclasses of this class.
    pub permitted_subclasses: Vec<Sym>,
    /// The record components of this class.
    pub record_components: Vec<RecordComponentNode>,
    /// The fields of this class.
    pub fields: Vec<FieldNode>,
    /// The methods of this class.
    pub methods: Vec<MethodNode>,
    /// The interner of this node's strings, with the memo of the strings visited so far
    /// (emptied by `visit_end`).
    strings: InternMemo,
}

impl ClassNode {
    /// Constructs an empty class node (ASM's `ClassNode()`) whose strings are symbols of the given
    /// interner.
    pub fn with_interner(interner: Arc<Interner>) -> ClassNode {
        ClassNode {
            version: 0,
            access: 0,
            name: Sym::default(),
            signature: None,
            super_name: None,
            interfaces: Vec::new(),
            source_file: None,
            source_debug: None,
            module: None,
            outer_class: None,
            outer_method: None,
            outer_method_desc: None,
            visible_annotations: Vec::new(),
            invisible_annotations: Vec::new(),
            visible_type_annotations: Vec::new(),
            invisible_type_annotations: Vec::new(),
            attrs: Vec::new(),
            inner_classes: Vec::new(),
            nest_host_class: None,
            nest_members: Vec::new(),
            permitted_subclasses: Vec::new(),
            record_components: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            strings: InternMemo::new(interner),
        }
    }

    /// The interner of this node's strings.
    pub fn interner(&self) -> &Arc<Interner> {
        self.strings.interner()
    }

    /// The string of the given symbol of this node.
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        self.strings.interner().resolve(sym)
    }

    /// The symbol of the given string in this node's interner.
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        self.strings.interner().intern(s)
    }

    /// Makes the given class visitor visit this class.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let interner = &**self.interner();
        let resolve_opt = |s: Option<Sym>| s.map(|s| interner.resolve(s));
        // Visit the header.
        class_visitor.visit(
            self.version,
            self.access,
            interner.resolve(self.name),
            resolve_opt(self.signature),
            resolve_opt(self.super_name),
            &resolve_all(interner, &self.interfaces),
        );
        // Visit the source.
        if self.source_file.is_some() || self.source_debug.is_some() {
            class_visitor.visit_source(
                resolve_opt(self.source_file),
                resolve_opt(self.source_debug),
            );
        }
        // Visit the module.
        if let Some(module) = &self.module {
            module.accept(class_visitor);
        }
        // Visit the nest host class.
        if let Some(nest_host_class) = self.nest_host_class {
            class_visitor.visit_nest_host(interner.resolve(nest_host_class));
        }
        // Visit the outer class.
        if let Some(outer_class) = self.outer_class {
            class_visitor.visit_outer_class(
                interner.resolve(outer_class),
                resolve_opt(self.outer_method),
                resolve_opt(self.outer_method_desc),
            );
        }
        // Visit the annotations.
        accept_annotations!(interner, self, &mut *class_visitor);
        // Visit the non standard attributes.
        for attribute in &self.attrs {
            class_visitor.visit_attribute(attribute);
        }
        // Visit the nest members.
        for &nest_member in &self.nest_members {
            class_visitor.visit_nest_member(interner.resolve(nest_member));
        }
        // Visit the permitted subclasses.
        for &permitted_subclass in &self.permitted_subclasses {
            class_visitor.visit_permitted_subclass(interner.resolve(permitted_subclass));
        }
        // Visit the inner classes.
        for inner_class in &self.inner_classes {
            inner_class.accept(interner, class_visitor);
        }
        // Visit the record components.
        for record_component in &self.record_components {
            record_component.accept(class_visitor);
        }
        // Visit the fields.
        for field in &self.fields {
            field.accept(class_visitor);
        }
        // Visit the methods.
        for method in &self.methods {
            method.accept(class_visitor);
        }
        class_visitor.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// Implementation of the ClassVisitor trait
// -------------------------------------------------------------------------------------------------

impl ClassVisitor for ClassNode {
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.version = version;
        self.access = access;
        let strings = &mut self.strings;
        self.name = strings.intern(name);
        self.signature = strings.intern_opt(signature);
        self.super_name = strings.intern_opt(super_name);
        self.interfaces = interfaces.iter().map(|&i| strings.intern(i)).collect();
    }

    fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
        self.source_file = self.strings.intern_opt(source);
        self.source_debug = self.strings.intern_opt(debug);
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        let name = self.strings.intern(name);
        let version = self.strings.intern_opt(version);
        let interner = self.strings.interner().clone();
        Some(Box::new(
            self.module
                .insert(ModuleNode::new(interner, name, access, version)),
        ))
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.nest_host_class = Some(self.strings.intern(nest_host));
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        self.outer_class = Some(self.strings.intern(owner));
        self.outer_method = self.strings.intern_opt(name);
        self.outer_method_desc = self.strings.intern_opt(descriptor);
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        add_annotation(
            &mut self.visible_annotations,
            &mut self.invisible_annotations,
            Strings::Memo(&mut self.strings),
            descriptor,
            visible,
        )
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        add_type_annotation(
            &mut self.visible_type_annotations,
            &mut self.invisible_type_annotations,
            Strings::Memo(&mut self.strings),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.attrs.push(attribute.clone());
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        self.nest_members.push(self.strings.intern(nest_member));
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        self.permitted_subclasses
            .push(self.strings.intern(permitted_subclass));
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        let strings = &mut self.strings;
        self.inner_classes.push(InnerClassNode::new(
            strings.intern(name),
            strings.intern_opt(outer_name),
            strings.intern_opt(inner_name),
            access,
        ));
    }

    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        let strings = &mut self.strings;
        self.record_components.push(RecordComponentNode::new(
            strings.interner().clone(),
            strings.intern(name),
            strings.intern(descriptor),
            strings.intern_opt(signature),
        ));
        Some(Box::new(
            self.record_components.last_mut().expect("just added"),
        ))
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        let strings = &mut self.strings;
        let name = strings.intern(name);
        let descriptor = strings.intern(descriptor);
        let signature = strings.intern_opt(signature);
        let value = FieldNode::value_of(value, &mut Strings::Memo(strings));
        self.fields.push(FieldNode::new(
            strings.interner().clone(),
            access,
            name,
            descriptor,
            signature,
            value,
        ));
        Some(Box::new(self.fields.last_mut().expect("just added")))
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let strings = &mut self.strings;
        self.methods.push(MethodNode::new(
            strings.interner().clone(),
            access,
            strings.intern(name),
            strings.intern(descriptor),
            strings.intern_opt(signature),
            exceptions.iter().map(|&e| strings.intern(e)).collect(),
        ));
        Some(Box::new(PooledMethodNode {
            node: self.methods.last_mut().expect("just added"),
            memo: strings,
        }))
    }

    fn visit_end(&mut self) {
        // The memo is only needed while visiting (and the addresses it keeps would not outlive
        // the class reader's strings).
        self.strings.clear();
    }
}
