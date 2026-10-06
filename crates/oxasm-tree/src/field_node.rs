//! [`FieldNode`] and [`RecordComponentNode`]: ASM's `FieldNode` and `RecordComponentNode`.

use oxasm::constant::Constant;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FieldVisitor, JavaArcStr, JavaStr,
    RecordComponentVisitor, TypePath, TypeReference,
};

use crate::annotation_node::{AnnotationNode, TypeAnnotationNode};

/// Adds an annotation node to the visible or invisible list, and returns it (the
/// `visitAnnotation` of the member nodes).
pub(crate) fn add_annotation<'a>(
    visible_annotations: &'a mut Vec<AnnotationNode>,
    invisible_annotations: &'a mut Vec<AnnotationNode>,
    descriptor: &JavaStr,
    visible: bool,
) -> Option<Box<dyn AnnotationVisitor + 'a>> {
    let annotations = if visible {
        visible_annotations
    } else {
        invisible_annotations
    };
    annotations.push(AnnotationNode::new(descriptor));
    Some(Box::new(annotations.last_mut().expect("just added")))
}

/// Adds a type annotation node to the visible or invisible list, and returns it (the
/// `visitTypeAnnotation` of the member nodes).
pub(crate) fn add_type_annotation<'a>(
    visible_type_annotations: &'a mut Vec<TypeAnnotationNode>,
    invisible_type_annotations: &'a mut Vec<TypeAnnotationNode>,
    type_ref: TypeReference,
    type_path: Option<&TypePath>,
    descriptor: &JavaStr,
    visible: bool,
) -> Option<Box<dyn AnnotationVisitor + 'a>> {
    let type_annotations = if visible {
        visible_type_annotations
    } else {
        invisible_type_annotations
    };
    type_annotations.push(TypeAnnotationNode::new(
        type_ref,
        type_path.cloned(),
        descriptor,
    ));
    let type_annotation = type_annotations.last_mut().expect("just added");
    Some(Box::new(&mut type_annotation.annotation))
}

/// Makes the given member visitor visit the annotations and type annotations of a field, record
/// component, class or method node (the part of their `accept` methods common to all of them).
macro_rules! accept_annotations {
    ($node:expr, $visitor:expr) => {{
        let node = $node;
        let visitor = $visitor;
        for annotation in &node.visible_annotations {
            let av = visitor.visit_annotation(&annotation.desc, true);
            $crate::annotation_node::accept_annotation(annotation, av);
        }
        for annotation in &node.invisible_annotations {
            let av = visitor.visit_annotation(&annotation.desc, false);
            $crate::annotation_node::accept_annotation(annotation, av);
        }
        for type_annotation in &node.visible_type_annotations {
            let av = visitor.visit_type_annotation(
                type_annotation.type_ref,
                type_annotation.type_path.as_ref(),
                type_annotation.desc(),
                true,
            );
            $crate::annotation_node::accept_annotation(&type_annotation.annotation, av);
        }
        for type_annotation in &node.invisible_type_annotations {
            let av = visitor.visit_type_annotation(
                type_annotation.type_ref,
                type_annotation.type_path.as_ref(),
                type_annotation.desc(),
                false,
            );
            $crate::annotation_node::accept_annotation(&type_annotation.annotation, av);
        }
    }};
}
pub(crate) use accept_annotations;

/// A node that represents a field.
#[derive(Clone, Debug, Default)]
pub struct FieldNode {
    /// The field's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// field is synthetic and/or deprecated.
    pub access: u32,
    /// The field's name.
    pub name: JavaArcStr,
    /// The field's descriptor.
    pub desc: JavaArcStr,
    /// The field's signature, or `None`.
    pub signature: Option<JavaArcStr>,
    /// The field's initial value: an `Integer`, a `Float`, a `Long`, a `Double` or a `String`
    /// constant, or `None`.
    pub value: Option<Constant<'static>>,
    /// The runtime visible annotations of this field.
    pub visible_annotations: Vec<AnnotationNode>,
    /// The runtime invisible annotations of this field.
    pub invisible_annotations: Vec<AnnotationNode>,
    /// The runtime visible type annotations of this field.
    pub visible_type_annotations: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of this field.
    pub invisible_type_annotations: Vec<TypeAnnotationNode>,
    /// The non standard attributes of this field.
    pub attrs: Vec<Attribute>,
}

impl FieldNode {
    /// Constructs a new field node, with no annotations nor attributes.
    pub fn new(
        access: u32,
        name: impl Into<JavaArcStr>,
        descriptor: impl Into<JavaArcStr>,
        signature: Option<JavaArcStr>,
        value: Option<Constant<'static>>,
    ) -> FieldNode {
        FieldNode {
            access,
            name: name.into(),
            desc: descriptor.into(),
            signature,
            value,
            ..FieldNode::default()
        }
    }

    /// Makes the given class visitor visit this field.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let Some(mut field_visitor) = class_visitor.visit_field(
            self.access,
            &self.name,
            &self.desc,
            self.signature.as_deref(),
            self.value.as_ref(),
        ) else {
            return;
        };
        accept_annotations!(self, &mut *field_visitor);
        for attribute in &self.attrs {
            field_visitor.visit_attribute(attribute);
        }
        field_visitor.visit_end();
    }
}

impl FieldVisitor for FieldNode {
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

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// A node that represents a record component.
#[derive(Clone, Debug, Default)]
pub struct RecordComponentNode {
    /// The record component name.
    pub name: JavaArcStr,
    /// The record component descriptor.
    pub descriptor: JavaArcStr,
    /// The record component signature, or `None`.
    pub signature: Option<JavaArcStr>,
    /// The runtime visible annotations of this record component.
    pub visible_annotations: Vec<AnnotationNode>,
    /// The runtime invisible annotations of this record component.
    pub invisible_annotations: Vec<AnnotationNode>,
    /// The runtime visible type annotations of this record component.
    pub visible_type_annotations: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of this record component.
    pub invisible_type_annotations: Vec<TypeAnnotationNode>,
    /// The non standard attributes of this record component.
    pub attrs: Vec<Attribute>,
}

impl RecordComponentNode {
    /// Constructs a new record component node, with no annotations nor attributes.
    pub fn new(
        name: impl Into<JavaArcStr>,
        descriptor: impl Into<JavaArcStr>,
        signature: Option<JavaArcStr>,
    ) -> RecordComponentNode {
        RecordComponentNode {
            name: name.into(),
            descriptor: descriptor.into(),
            signature,
            ..RecordComponentNode::default()
        }
    }

    /// Makes the given class visitor visit this record component.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let Some(mut record_component_visitor) = class_visitor.visit_record_component(
            &self.name,
            &self.descriptor,
            self.signature.as_deref(),
        ) else {
            return;
        };
        accept_annotations!(self, &mut *record_component_visitor);
        for attribute in &self.attrs {
            record_component_visitor.visit_attribute(attribute);
        }
        record_component_visitor.visit_end();
    }
}

impl RecordComponentVisitor for RecordComponentNode {
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

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}
