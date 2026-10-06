//! [`AnnotationNode`], [`TypeAnnotationNode`] and [`LocalVariableAnnotationNode`]: ASM's
//! `AnnotationNode`, `TypeAnnotationNode` and `LocalVariableAnnotationNode`.

use oxasm::constant::AnnotationValue;
use oxasm::{
    AnnotationVisitor, JavaArcStr, JavaStr, Label, MethodVisitor, TypePath, TypeReference,
};

/// A value of an annotation element, as an [`AnnotationNode`] stores it (an element of ASM's
/// `AnnotationNode.values`, which is a `List<Object>`).
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationNodeValue {
    /// A primitive, `String` or class value (ASM's `Byte`, ..., `String` and `Type` values).
    ///
    /// Primitive arrays never appear here when the value was visited: like ASM, which converts
    /// them to `List`s, [`AnnotationNode`] stores them as an [`Array`](Self::Array) of values,
    /// which [`AnnotationNode::accept`] visits with `visit_array`.
    Value(AnnotationValue<'static>),
    /// An enumeration value (ASM's two-element `String[]`).
    Enum {
        /// The class descriptor of the enumeration class.
        descriptor: JavaArcStr,
        /// The actual enumeration value.
        value: JavaArcStr,
    },
    /// A nested annotation.
    Annotation(AnnotationNode),
    /// An array of values (ASM's `List`).
    Array(Vec<AnnotationNodeValue>),
}

impl AnnotationNodeValue {
    /// The value ASM stores for `AnnotationVisitor.visit(name, value)`: primitive arrays become
    /// arrays of values (`Util.asArrayList`), everything else is stored as is.
    fn from_visited(value: &AnnotationValue<'_>) -> AnnotationNodeValue {
        fn array<T: Copy>(
            values: &[T],
            f: impl Fn(T) -> AnnotationValue<'static>,
        ) -> AnnotationNodeValue {
            AnnotationNodeValue::Array(
                values
                    .iter()
                    .map(|&v| AnnotationNodeValue::Value(f(v)))
                    .collect(),
            )
        }
        match value {
            AnnotationValue::ByteArray(a) => array(a, AnnotationValue::Byte),
            AnnotationValue::BooleanArray(a) => array(a, AnnotationValue::Boolean),
            AnnotationValue::ShortArray(a) => array(a, AnnotationValue::Short),
            AnnotationValue::CharArray(a) => array(a, AnnotationValue::Char),
            AnnotationValue::IntArray(a) => array(a, AnnotationValue::Int),
            AnnotationValue::LongArray(a) => array(a, AnnotationValue::Long),
            AnnotationValue::FloatArray(a) => array(a, AnnotationValue::Float),
            AnnotationValue::DoubleArray(a) => array(a, AnnotationValue::Double),
            value => AnnotationNodeValue::Value(value.to_owned()),
        }
    }

    /// Makes the given visitor visit this value, under the given element name (ASM's static
    /// `AnnotationNode.accept(annotationVisitor, name, value)`).
    pub fn accept(&self, annotation_visitor: &mut dyn AnnotationVisitor, name: Option<&JavaStr>) {
        match self {
            AnnotationNodeValue::Value(value) => annotation_visitor.visit(name, value),
            AnnotationNodeValue::Enum { descriptor, value } => {
                annotation_visitor.visit_enum(name, descriptor, value);
            }
            AnnotationNodeValue::Annotation(annotation) => {
                if let Some(mut av) = annotation_visitor.visit_annotation(name, &annotation.desc) {
                    annotation.accept(&mut *av);
                }
            }
            AnnotationNodeValue::Array(values) => {
                if let Some(mut av) = annotation_visitor.visit_array(name) {
                    for value in values {
                        value.accept(&mut *av, None);
                    }
                    av.visit_end();
                }
            }
        }
    }
}

/// A node that represents an annotation.
///
/// Its depth is that of the annotation it was visited from, which a class reader bounds (ADR-06),
/// and [`accept`](Self::accept) recurses over it.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationNode {
    /// The class descriptor of the annotation class.
    pub desc: JavaArcStr,
    /// The name value pairs of this annotation, in visit order. Each name is `None` only if a
    /// visitor passed none (a class reader always passes the element name).
    pub values: Vec<(Option<JavaArcStr>, AnnotationNodeValue)>,
}

impl AnnotationNode {
    /// Constructs a new annotation node with no values.
    pub fn new(descriptor: impl Into<JavaArcStr>) -> AnnotationNode {
        AnnotationNode {
            desc: descriptor.into(),
            values: Vec::new(),
        }
    }

    /// Makes the given visitor visit this annotation's values, then `visit_end`.
    pub fn accept(&self, annotation_visitor: &mut dyn AnnotationVisitor) {
        for (name, value) in &self.values {
            value.accept(annotation_visitor, name.as_deref());
        }
        annotation_visitor.visit_end();
    }

    /// The last value, which was just added.
    fn last_value_mut(&mut self) -> &mut AnnotationNodeValue {
        &mut self.values.last_mut().expect("a value was just added").1
    }
}

/// Makes the visitor returned by a parent's `visit_*annotation` method, if any, visit the given
/// annotation (ASM's `annotation.accept(visitor.visitAnnotation(...))`).
pub(crate) fn accept_annotation(
    annotation: &AnnotationNode,
    annotation_visitor: Option<Box<dyn AnnotationVisitor + '_>>,
) {
    if let Some(mut av) = annotation_visitor {
        annotation.accept(&mut *av);
    }
}

impl AnnotationVisitor for AnnotationNode {
    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        self.values.push((
            name.map(JavaArcStr::from),
            AnnotationNodeValue::from_visited(value),
        ));
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        self.values.push((
            name.map(JavaArcStr::from),
            AnnotationNodeValue::Enum {
                descriptor: descriptor.into(),
                value: value.into(),
            },
        ));
    }

    fn visit_annotation<'a>(
        &'a mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.values.push((
            name.map(JavaArcStr::from),
            AnnotationNodeValue::Annotation(AnnotationNode::new(descriptor)),
        ));
        let AnnotationNodeValue::Annotation(annotation) = self.last_value_mut() else {
            unreachable!("an annotation was just added")
        };
        Some(Box::new(annotation))
    }

    fn visit_array<'a>(
        &'a mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.values.push((
            name.map(JavaArcStr::from),
            AnnotationNodeValue::Array(Vec::new()),
        ));
        let AnnotationNodeValue::Array(values) = self.last_value_mut() else {
            unreachable!("an array was just added")
        };
        Some(Box::new(ValuesVisitor::Array(values)))
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// The visitor of an annotation array, or of an annotation default value: ASM's `AnnotationNode`
/// without a descriptor, whose values have no names.
#[derive(Debug)]
pub(crate) enum ValuesVisitor<'a> {
    /// The values of an array.
    Array(&'a mut Vec<AnnotationNodeValue>),
    /// A method's annotation default value: each visited value replaces it (ASM's
    /// `MethodNode.visitAnnotationDefault`, whose list sets `annotationDefault` on each `add`).
    Default(&'a mut Option<AnnotationNodeValue>),
}

impl ValuesVisitor<'_> {
    /// Adds a value, and returns it.
    fn add(&mut self, value: AnnotationNodeValue) -> &mut AnnotationNodeValue {
        match self {
            ValuesVisitor::Array(values) => {
                values.push(value);
                values.last_mut().expect("a value was just added")
            }
            ValuesVisitor::Default(slot) => slot.insert(value),
        }
    }
}

impl AnnotationVisitor for ValuesVisitor<'_> {
    fn visit(&mut self, _name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        self.add(AnnotationNodeValue::from_visited(value));
    }

    fn visit_enum(&mut self, _name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        self.add(AnnotationNodeValue::Enum {
            descriptor: descriptor.into(),
            value: value.into(),
        });
    }

    fn visit_annotation<'a>(
        &'a mut self,
        _name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let value = self.add(AnnotationNodeValue::Annotation(AnnotationNode::new(
            descriptor,
        )));
        let AnnotationNodeValue::Annotation(annotation) = value else {
            unreachable!("an annotation was just added")
        };
        Some(Box::new(annotation))
    }

    fn visit_array<'a>(
        &'a mut self,
        _name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let AnnotationNodeValue::Array(values) = self.add(AnnotationNodeValue::Array(Vec::new()))
        else {
            unreachable!("an array was just added")
        };
        Some(Box::new(ValuesVisitor::Array(values)))
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// A node that represents a type annotation.
///
/// ASM's `TypeAnnotationNode` extends `AnnotationNode`; here the annotation is a field, and this
/// node is an [`AnnotationVisitor`] that delegates to it.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeAnnotationNode {
    /// A reference to the annotated type.
    pub type_ref: TypeReference,
    /// The path to the annotated type argument, wildcard bound, array element type, or static
    /// outer type within the referenced type. `None` if the annotation targets the referenced
    /// type as a whole.
    pub type_path: Option<TypePath>,
    /// The annotation: its descriptor and values.
    pub annotation: AnnotationNode,
}

impl TypeAnnotationNode {
    /// Constructs a new type annotation node with no values.
    pub fn new(
        type_ref: TypeReference,
        type_path: Option<TypePath>,
        descriptor: impl Into<JavaArcStr>,
    ) -> TypeAnnotationNode {
        TypeAnnotationNode {
            type_ref,
            type_path,
            annotation: AnnotationNode::new(descriptor),
        }
    }

    /// The class descriptor of the annotation class.
    pub fn desc(&self) -> &JavaStr {
        &self.annotation.desc
    }
}

impl AnnotationVisitor for TypeAnnotationNode {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut self.annotation)
    }
}

/// A node that represents a type annotation on a local or resource variable.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalVariableAnnotationNode {
    /// A reference to the annotated type (of sort `LOCAL_VARIABLE` or `RESOURCE_VARIABLE`).
    pub type_ref: TypeReference,
    /// The path to the annotated type argument, wildcard bound, array element type, or static
    /// outer type within the referenced type, or `None`.
    pub type_path: Option<TypePath>,
    /// The first instructions corresponding to the continuous ranges that make the scope of
    /// this local variable (inclusive).
    pub start: Vec<Label>,
    /// The last instructions corresponding to the continuous ranges that make the scope of this
    /// local variable (exclusive). Must have the same length as `start`.
    pub end: Vec<Label>,
    /// The local variable's index in each range. Must have the same length as `start`.
    pub index: Vec<u16>,
    /// The annotation: its descriptor and values.
    pub annotation: AnnotationNode,
}

impl LocalVariableAnnotationNode {
    /// Constructs a new local variable annotation node with no values.
    pub fn new(
        type_ref: TypeReference,
        type_path: Option<TypePath>,
        start: Vec<Label>,
        end: Vec<Label>,
        index: Vec<u16>,
        descriptor: impl Into<JavaArcStr>,
    ) -> LocalVariableAnnotationNode {
        LocalVariableAnnotationNode {
            type_ref,
            type_path,
            start,
            end,
            index,
            annotation: AnnotationNode::new(descriptor),
        }
    }

    /// Makes the given visitor visit this type annotation.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor, visible: bool) {
        // ASM iterates over `start`, and reads `end` and `index` at the same positions.
        let n = self.start.len();
        let av = method_visitor.visit_local_variable_annotation(
            self.type_ref,
            self.type_path.as_ref(),
            &self.start,
            &self.end[..n],
            &self.index[..n],
            &self.annotation.desc,
            visible,
        );
        accept_annotation(&self.annotation, av);
    }
}

impl AnnotationVisitor for LocalVariableAnnotationNode {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut self.annotation)
    }
}
