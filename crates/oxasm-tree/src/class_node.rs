//! [`ClassNode`]: ASM's `ClassNode`.

use oxasm::constant::Constant;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FieldVisitor, JavaArcStr, JavaStr, MethodVisitor,
    ModuleVisitor, RecordComponentVisitor, TypePath, TypeReference,
};

use crate::annotation_node::{AnnotationNode, TypeAnnotationNode};
use crate::field_node::{
    FieldNode, RecordComponentNode, accept_annotations, add_annotation, add_type_annotation,
};
use crate::method_node::MethodNode;
use crate::method_node::PooledMethodNode;
use crate::module_node::ModuleNode;
use crate::nodes::InnerClassNode;
use crate::strings::{StringPool, Strings};

/// A node that represents a class.
///
/// A class reader fills it by visiting it, and [`accept`](Self::accept) replays it into any
/// class visitor. While it is visited, equal strings are shared: the nodes of a class hold one
/// [`JavaArcStr`] per distinct name, descriptor or signature of the class and of its fields and
/// methods, and of the instructions, local variables and try catch blocks of its methods (much
/// as ASM's nodes share the strings of its class reader).
///
/// ```
/// # fn main() -> Result<(), oxasm::error::ReadError> {
/// # let class_file = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),
/// #     "/../../corpus/t0/jdk3/AllInstructions.class")).unwrap();
/// use oxasm::{ClassReader, ParsingOptions};
/// use oxasm_tree::ClassNode;
///
/// let mut class_node = ClassNode::default();
/// ClassReader::new(&class_file)?.accept(&mut class_node, ParsingOptions::empty())?;
/// assert_eq!(class_node.name, "jdk3/AllInstructions");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct ClassNode {
    /// The class version. The minor version is stored in the 16 most significant bits, and the
    /// major version in the 16 least significant bits.
    pub version: u32,
    /// The class's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// class is deprecated (`ACC_DEPRECATED`) or a record (`ACC_RECORD`).
    pub access: u32,
    /// The internal name of this class.
    pub name: JavaArcStr,
    /// The signature of this class, or `None`.
    pub signature: Option<JavaArcStr>,
    /// The internal name of the super class. For interfaces, the super class is
    /// `java/lang/Object`. `None` only for the `java/lang/Object` class.
    pub super_name: Option<JavaArcStr>,
    /// The internal names of the interfaces directly implemented by this class.
    pub interfaces: Vec<JavaArcStr>,
    /// The name of the source file from which this class was compiled, or `None`.
    pub source_file: Option<JavaArcStr>,
    /// The correspondence between source and compiled elements of this class, or `None`.
    pub source_debug: Option<JavaArcStr>,
    /// The module stored in this class, or `None`.
    pub module: Option<ModuleNode>,
    /// The internal name of the enclosing class of this class (see `visit_outer_class`), or
    /// `None`.
    pub outer_class: Option<JavaArcStr>,
    /// The name of the method that contains this class, or `None` if this class is not enclosed
    /// in a method.
    pub outer_method: Option<JavaArcStr>,
    /// The descriptor of the method that contains this class, or `None` if this class is not
    /// enclosed in a method.
    pub outer_method_desc: Option<JavaArcStr>,
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
    pub nest_host_class: Option<JavaArcStr>,
    /// The internal names of the nest members of this class.
    pub nest_members: Vec<JavaArcStr>,
    /// The internal names of the permitted subclasses of this class.
    pub permitted_subclasses: Vec<JavaArcStr>,
    /// The record components of this class.
    pub record_components: Vec<RecordComponentNode>,
    /// The fields of this class.
    pub fields: Vec<FieldNode>,
    /// The methods of this class.
    pub methods: Vec<MethodNode>,
    /// The strings visited so far, which the nodes of this class share (emptied by
    /// `visit_end`).
    strings: StringPool,
}

impl ClassNode {
    /// Constructs an empty class node (ASM's `ClassNode()`); the same as `ClassNode::default()`.
    pub fn new() -> ClassNode {
        ClassNode::default()
    }

    /// Makes the given class visitor visit this class.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        // Visit the header.
        let interfaces: Vec<&JavaStr> = self.interfaces.iter().map(|i| &**i).collect();
        class_visitor.visit(
            self.version,
            self.access,
            &self.name,
            self.signature.as_deref(),
            self.super_name.as_deref(),
            &interfaces,
        );
        // Visit the source.
        if self.source_file.is_some() || self.source_debug.is_some() {
            class_visitor.visit_source(self.source_file.as_deref(), self.source_debug.as_deref());
        }
        // Visit the module.
        if let Some(module) = &self.module {
            module.accept(class_visitor);
        }
        // Visit the nest host class.
        if let Some(nest_host_class) = &self.nest_host_class {
            class_visitor.visit_nest_host(nest_host_class);
        }
        // Visit the outer class.
        if let Some(outer_class) = &self.outer_class {
            class_visitor.visit_outer_class(
                outer_class,
                self.outer_method.as_deref(),
                self.outer_method_desc.as_deref(),
            );
        }
        // Visit the annotations.
        accept_annotations!(self, &mut *class_visitor);
        // Visit the non standard attributes.
        for attribute in &self.attrs {
            class_visitor.visit_attribute(attribute);
        }
        // Visit the nest members.
        for nest_member in &self.nest_members {
            class_visitor.visit_nest_member(nest_member);
        }
        // Visit the permitted subclasses.
        for permitted_subclass in &self.permitted_subclasses {
            class_visitor.visit_permitted_subclass(permitted_subclass);
        }
        // Visit the inner classes.
        for inner_class in &self.inner_classes {
            inner_class.accept(class_visitor);
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
        self.name = strings.get(name);
        self.signature = strings.get_opt(signature);
        self.super_name = strings.get_opt(super_name);
        self.interfaces = interfaces.iter().map(|&i| strings.get(i)).collect();
    }

    fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
        self.source_file = self.strings.get_opt(source);
        self.source_debug = self.strings.get_opt(debug);
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        Some(Box::new(self.module.insert(ModuleNode::new(
            name,
            access,
            self.strings.get_opt(version),
        ))))
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.nest_host_class = Some(self.strings.get(nest_host));
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        self.outer_class = Some(self.strings.get(owner));
        self.outer_method = self.strings.get_opt(name);
        self.outer_method_desc = self.strings.get_opt(descriptor);
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        add_annotation(
            &mut self.visible_annotations,
            &mut self.invisible_annotations,
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
        self.nest_members.push(self.strings.get(nest_member));
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        self.permitted_subclasses
            .push(self.strings.get(permitted_subclass));
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
            strings.get(name),
            strings.get_opt(outer_name),
            strings.get_opt(inner_name),
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
            strings.get(name),
            strings.get(descriptor),
            strings.get_opt(signature),
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
        self.fields.push(FieldNode::new(
            access,
            strings.get(name),
            strings.get(descriptor),
            strings.get_opt(signature),
            value.map(Constant::to_owned),
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
            access,
            strings.get(name),
            strings.get(descriptor),
            strings.get_opt(signature),
            exceptions.iter().map(|&e| strings.get(e)).collect(),
        ));
        Some(Box::new(PooledMethodNode {
            node: self.methods.last_mut().expect("just added"),
            strings,
        }))
    }

    fn visit_end(&mut self) {
        // The nodes keep their strings; the pool is only needed while visiting.
        self.strings = StringPool::default();
    }
}
