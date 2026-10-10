//! [`FieldNode`] and [`RecordComponentNode`]: ASM's `FieldNode` and `RecordComponentNode`.

use std::sync::Arc;

use oxasm::constant::Constant;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FieldVisitor, Interner, JavaStr,
    RecordComponentVisitor, Sym, TypePath, TypeReference,
};

use crate::annotation_node::{AnnotationNode, AnnotationNodeVisitor, TypeAnnotationNode};
use crate::strings::Strings;
use crate::value::SymConstant;

/// Adds an annotation node to the visible or invisible list, and returns it (the
/// `visitAnnotation` of the member nodes).
pub(crate) fn add_annotation<'a>(
    visible_annotations: &'a mut Vec<AnnotationNode>,
    invisible_annotations: &'a mut Vec<AnnotationNode>,
    mut strings: Strings<'a>,
    descriptor: &JavaStr,
    visible: bool,
) -> Option<Box<dyn AnnotationVisitor + 'a>> {
    let annotations = if visible {
        visible_annotations
    } else {
        invisible_annotations
    };
    annotations.push(AnnotationNode::new(strings.get(descriptor)));
    let annotation = annotations.last_mut().expect("just added");
    Some(Box::new(AnnotationNodeVisitor::new(annotation, strings)))
}

/// Adds a type annotation node to the visible or invisible list, and returns it (the
/// `visitTypeAnnotation` of the member nodes).
#[allow(clippy::too_many_arguments)] // ASM's, and where to store the annotation
pub(crate) fn add_type_annotation<'a>(
    visible_type_annotations: &'a mut Vec<TypeAnnotationNode>,
    invisible_type_annotations: &'a mut Vec<TypeAnnotationNode>,
    mut strings: Strings<'a>,
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
        strings.get(descriptor),
    ));
    let type_annotation = type_annotations.last_mut().expect("just added");
    Some(Box::new(AnnotationNodeVisitor::new(
        &mut type_annotation.annotation,
        strings,
    )))
}

/// Makes the given member visitor visit the annotations and type annotations of a field, record
/// component, class or method node (the part of their `accept` methods common to all of them).
macro_rules! accept_annotations {
    ($interner:expr, $node:expr, $visitor:expr) => {{
        let interner: &oxasm::Interner = $interner;
        let node = $node;
        let visitor = $visitor;
        for (annotations, visible) in [
            (&node.visible_annotations, true),
            (&node.invisible_annotations, false),
        ] {
            for annotation in annotations {
                let av = visitor.visit_annotation(interner.resolve(annotation.desc), visible);
                $crate::annotation_node::accept_annotation(interner, annotation, av);
            }
        }
        for (type_annotations, visible) in [
            (&node.visible_type_annotations, true),
            (&node.invisible_type_annotations, false),
        ] {
            for type_annotation in type_annotations {
                let av = visitor.visit_type_annotation(
                    type_annotation.type_ref,
                    type_annotation.type_path.as_ref(),
                    interner.resolve(type_annotation.desc()),
                    visible,
                );
                $crate::annotation_node::accept_annotation(
                    interner,
                    &type_annotation.annotation,
                    av,
                );
            }
        }
    }};
}
pub(crate) use accept_annotations;

/// A node that represents a field.
///
/// Its strings are [`Sym`]s of its interner, which [`resolve`](Self::resolve) turns back into
/// strings. `FieldNode::default()` makes a new interner.
#[derive(Clone, Debug, Default)]
pub struct FieldNode {
    /// The interner of this node's strings.
    interner: Arc<Interner>,
    /// The field's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// field is synthetic and/or deprecated.
    pub access: u32,
    /// The field's name.
    pub name: Sym,
    /// The field's descriptor.
    pub desc: Sym,
    /// The field's signature, or `None`.
    pub signature: Option<Sym>,
    /// The field's initial value: an `Integer`, a `Float`, a `Long`, a `Double` or a `String`
    /// constant, or `None`.
    pub value: Option<SymConstant>,
    /// The rarely used parts of this field, or `None` if it has none (see
    /// [`extras`](Self::extras) and [`extras_mut`](Self::extras_mut)).
    pub extras: Option<Box<FieldExtras>>,
}

// The rarely used parts are behind one box (T12).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<FieldNode>() == 48);

/// The rarely used parts of a [`FieldNode`]: its annotations and non standard attributes,
/// which most fields don't have.
///
/// Like [`MethodExtras`](crate::MethodExtras), they are behind one box, which only the fields
/// that have some allocate, so that a field node takes 48 bytes instead of 160.
#[derive(Clone, Debug, Default)]
pub struct FieldExtras {
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

/// The extras of a field node that has none.
static NO_EXTRAS: FieldExtras = FieldExtras {
    visible_annotations: Vec::new(),
    invisible_annotations: Vec::new(),
    visible_type_annotations: Vec::new(),
    invisible_type_annotations: Vec::new(),
    attrs: Vec::new(),
};

impl FieldNode {
    /// Constructs a new field node, with no annotations nor attributes, whose strings are
    /// symbols of the given interner.
    pub fn new(
        interner: Arc<Interner>,
        access: u32,
        name: Sym,
        descriptor: Sym,
        signature: Option<Sym>,
        value: Option<SymConstant>,
    ) -> FieldNode {
        FieldNode {
            interner,
            access,
            name,
            desc: descriptor,
            signature,
            value,
            extras: None,
        }
    }

    /// The rarely used parts of this field: empty ones if it has none.
    #[inline]
    pub fn extras(&self) -> &FieldExtras {
        self.extras.as_deref().unwrap_or(&NO_EXTRAS)
    }

    /// The rarely used parts of this field, which are allocated (empty) if it has none.
    #[inline]
    pub fn extras_mut(&mut self) -> &mut FieldExtras {
        self.extras.get_or_insert_with(Box::default)
    }

    /// The interner of this node's strings.
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// The string of the given symbol of this node.
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        self.interner.resolve(sym)
    }

    /// The symbol of the given string in this node's interner.
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        self.interner.intern(s)
    }

    /// Makes the given class visitor visit this field.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let interner = &*self.interner;
        let value = self.value.as_ref().map(|value| value.resolve(interner));
        let Some(mut field_visitor) = class_visitor.visit_field(
            self.access,
            interner.resolve(self.name),
            interner.resolve(self.desc),
            self.signature.map(|s| interner.resolve(s)),
            value.as_ref(),
        ) else {
            return;
        };
        let extras = self.extras();
        accept_annotations!(interner, extras, &mut *field_visitor);
        for attribute in &extras.attrs {
            field_visitor.visit_attribute(attribute);
        }
        field_visitor.visit_end();
    }

    /// The initial value of a visited field.
    pub(crate) fn value_of(
        value: Option<&Constant<'_>>,
        strings: &mut Strings<'_>,
    ) -> Option<SymConstant> {
        value.map(|value| SymConstant::from_visited(value, strings))
    }
}

impl FieldVisitor for FieldNode {
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        add_annotation(
            &mut extras.visible_annotations,
            &mut extras.invisible_annotations,
            Strings::Direct(&self.interner),
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
        let extras = self.extras.get_or_insert_with(Box::default);
        add_type_annotation(
            &mut extras.visible_type_annotations,
            &mut extras.invisible_type_annotations,
            Strings::Direct(&self.interner),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.extras_mut().attrs.push(attribute.clone());
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// A node that represents a record component.
///
/// Its strings are [`Sym`]s of its interner. `RecordComponentNode::default()` makes a new
/// interner.
#[derive(Clone, Debug, Default)]
pub struct RecordComponentNode {
    /// The interner of this node's strings.
    interner: Arc<Interner>,
    /// The record component name.
    pub name: Sym,
    /// The record component descriptor.
    pub descriptor: Sym,
    /// The record component signature, or `None`.
    pub signature: Option<Sym>,
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
    /// Constructs a new record component node, with no annotations nor attributes, whose strings
    /// are symbols of the given interner.
    pub fn new(
        interner: Arc<Interner>,
        name: Sym,
        descriptor: Sym,
        signature: Option<Sym>,
    ) -> RecordComponentNode {
        RecordComponentNode {
            interner,
            name,
            descriptor,
            signature,
            visible_annotations: Vec::new(),
            invisible_annotations: Vec::new(),
            visible_type_annotations: Vec::new(),
            invisible_type_annotations: Vec::new(),
            attrs: Vec::new(),
        }
    }

    /// The interner of this node's strings.
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// The string of the given symbol of this node.
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        self.interner.resolve(sym)
    }

    /// The symbol of the given string in this node's interner.
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        self.interner.intern(s)
    }

    /// Makes the given class visitor visit this record component.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let interner = &*self.interner;
        let Some(mut record_component_visitor) = class_visitor.visit_record_component(
            interner.resolve(self.name),
            interner.resolve(self.descriptor),
            self.signature.map(|s| interner.resolve(s)),
        ) else {
            return;
        };
        accept_annotations!(interner, self, &mut *record_component_visitor);
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
            Strings::Direct(&self.interner),
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
            Strings::Direct(&self.interner),
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
