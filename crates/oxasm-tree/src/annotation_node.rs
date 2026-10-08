//! [`AnnotationNode`], [`TypeAnnotationNode`] and [`LocalVariableAnnotationNode`]: ASM's
//! `AnnotationNode`, `TypeAnnotationNode` and `LocalVariableAnnotationNode`.

use std::borrow::Cow;

use oxasm::constant::AnnotationValue;
use oxasm::{
    AnnotationVisitor, Interner, JavaStr, Label, MethodVisitor, Sym, TypePath, TypeReference,
};

use crate::strings::Strings;
use crate::value::resolve_type;

/// A value of an annotation element, as an [`AnnotationNode`] stores it (an element of ASM's
/// `AnnotationNode.values`, which is a `List<Object>`).
///
/// Primitive arrays never appear here when the value was visited: like ASM, which converts them
/// to `List`s, [`AnnotationNode`] stores them as an [`Array`](Self::Array) of values, which
/// [`AnnotationNode::accept`] visits with `visit_array`.
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationNodeValue {
    Byte(i8),
    Char(u16),
    Short(i16),
    Boolean(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(Sym),
    /// A class value, as its descriptor (a [`Type`](oxasm::Type) of sort `OBJECT` or `ARRAY`, or
    /// a primitive type, for `int.class` and the like).
    Type(Sym),
    /// An enumeration value (ASM's two-element `String[]`).
    Enum {
        /// The class descriptor of the enumeration class.
        descriptor: Sym,
        /// The actual enumeration value.
        value: Sym,
    },
    /// A nested annotation.
    Annotation(AnnotationNode),
    /// An array of values (ASM's `List`).
    Array(Vec<AnnotationNodeValue>),
}

impl AnnotationNodeValue {
    /// The value ASM stores for `AnnotationVisitor.visit(name, value)`: primitive arrays become
    /// arrays of values (`Util.asArrayList`), everything else is stored as is.
    fn from_visited(value: &AnnotationValue<'_>, strings: &mut Strings<'_>) -> AnnotationNodeValue {
        fn array<T: Copy>(
            values: &[T],
            f: impl Fn(T) -> AnnotationNodeValue,
        ) -> AnnotationNodeValue {
            AnnotationNodeValue::Array(values.iter().map(|&v| f(v)).collect())
        }
        match value {
            AnnotationValue::Byte(v) => AnnotationNodeValue::Byte(*v),
            AnnotationValue::Char(v) => AnnotationNodeValue::Char(*v),
            AnnotationValue::Short(v) => AnnotationNodeValue::Short(*v),
            AnnotationValue::Boolean(v) => AnnotationNodeValue::Boolean(*v),
            AnnotationValue::Int(v) => AnnotationNodeValue::Int(*v),
            AnnotationValue::Long(v) => AnnotationNodeValue::Long(*v),
            AnnotationValue::Float(v) => AnnotationNodeValue::Float(*v),
            AnnotationValue::Double(v) => AnnotationNodeValue::Double(*v),
            AnnotationValue::String(s) => AnnotationNodeValue::String(strings.get(s)),
            AnnotationValue::Type(t) => AnnotationNodeValue::Type(strings.get(&t.get_descriptor())),
            AnnotationValue::ByteArray(a) => array(a, AnnotationNodeValue::Byte),
            AnnotationValue::BooleanArray(a) => array(a, AnnotationNodeValue::Boolean),
            AnnotationValue::ShortArray(a) => array(a, AnnotationNodeValue::Short),
            AnnotationValue::CharArray(a) => array(a, AnnotationNodeValue::Char),
            AnnotationValue::IntArray(a) => array(a, AnnotationNodeValue::Int),
            AnnotationValue::LongArray(a) => array(a, AnnotationNodeValue::Long),
            AnnotationValue::FloatArray(a) => array(a, AnnotationNodeValue::Float),
            AnnotationValue::DoubleArray(a) => array(a, AnnotationNodeValue::Double),
        }
    }

    /// The visited value of a primitive, string or class value, or `None` for an enumeration
    /// value, a nested annotation or an array.
    pub fn resolve<'i>(&self, interner: &'i Interner) -> Option<AnnotationValue<'i>> {
        Some(match self {
            AnnotationNodeValue::Byte(v) => AnnotationValue::Byte(*v),
            AnnotationNodeValue::Char(v) => AnnotationValue::Char(*v),
            AnnotationNodeValue::Short(v) => AnnotationValue::Short(*v),
            AnnotationNodeValue::Boolean(v) => AnnotationValue::Boolean(*v),
            AnnotationNodeValue::Int(v) => AnnotationValue::Int(*v),
            AnnotationNodeValue::Long(v) => AnnotationValue::Long(*v),
            AnnotationNodeValue::Float(v) => AnnotationValue::Float(*v),
            AnnotationNodeValue::Double(v) => AnnotationValue::Double(*v),
            AnnotationNodeValue::String(s) => {
                AnnotationValue::String(Cow::Borrowed(interner.resolve(*s)))
            }
            AnnotationNodeValue::Type(t) => AnnotationValue::Type(resolve_type(interner, *t)),
            AnnotationNodeValue::Enum { .. }
            | AnnotationNodeValue::Annotation(_)
            | AnnotationNodeValue::Array(_) => return None,
        })
    }

    /// Makes the given visitor visit this value, under the given element name (ASM's static
    /// `AnnotationNode.accept(annotationVisitor, name, value)`).
    pub fn accept(
        &self,
        interner: &Interner,
        annotation_visitor: &mut dyn AnnotationVisitor,
        name: Option<&JavaStr>,
    ) {
        match self {
            AnnotationNodeValue::Enum { descriptor, value } => {
                annotation_visitor.visit_enum(
                    name,
                    interner.resolve(*descriptor),
                    interner.resolve(*value),
                );
            }
            AnnotationNodeValue::Annotation(annotation) => {
                if let Some(mut av) =
                    annotation_visitor.visit_annotation(name, interner.resolve(annotation.desc))
                {
                    annotation.accept(interner, &mut *av);
                }
            }
            AnnotationNodeValue::Array(values) => {
                if let Some(mut av) = annotation_visitor.visit_array(name) {
                    for value in values {
                        value.accept(interner, &mut *av, None);
                    }
                    av.visit_end();
                }
            }
            value => {
                let value = value
                    .resolve(interner)
                    .expect("a primitive, string or class value");
                annotation_visitor.visit(name, &value);
            }
        }
    }
}

/// A node that represents an annotation.
///
/// Its depth is that of the annotation it was visited from, which a class reader bounds (ADR-06),
/// and [`accept`](Self::accept) recurses over it. Its strings are [`Sym`]s of the interner of the
/// node that holds it; [`visitor`](Self::visitor) visits it on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationNode {
    /// The class descriptor of the annotation class.
    pub desc: Sym,
    /// The name value pairs of this annotation, in visit order. Each name is `None` only if a
    /// visitor passed none (a class reader always passes the element name).
    pub values: Vec<(Option<Sym>, AnnotationNodeValue)>,
}

impl AnnotationNode {
    /// Constructs a new annotation node with no values.
    pub fn new(descriptor: Sym) -> AnnotationNode {
        AnnotationNode {
            desc: descriptor,
            values: Vec::new(),
        }
    }

    /// An annotation visitor that adds the visited values to this node, interning their strings
    /// in the given interner (ASM's `AnnotationNode` is itself the visitor).
    pub fn visitor<'a>(&'a mut self, interner: &'a Interner) -> AnnotationNodeVisitor<'a> {
        AnnotationNodeVisitor {
            node: self,
            strings: Strings::Direct(interner),
        }
    }

    /// Makes the given visitor visit this annotation's values, then `visit_end`.
    pub fn accept(&self, interner: &Interner, annotation_visitor: &mut dyn AnnotationVisitor) {
        for (name, value) in &self.values {
            value.accept(
                interner,
                annotation_visitor,
                name.map(|n| interner.resolve(n)),
            );
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
    interner: &Interner,
    annotation: &AnnotationNode,
    annotation_visitor: Option<Box<dyn AnnotationVisitor + '_>>,
) {
    if let Some(mut av) = annotation_visitor {
        annotation.accept(interner, &mut *av);
    }
}

/// The annotation visitor of an [`AnnotationNode`] (see [`AnnotationNode::visitor`]).
pub struct AnnotationNodeVisitor<'a> {
    node: &'a mut AnnotationNode,
    strings: Strings<'a>,
}

impl<'a> AnnotationNodeVisitor<'a> {
    pub(crate) fn new(node: &'a mut AnnotationNode, strings: Strings<'a>) -> Self {
        AnnotationNodeVisitor { node, strings }
    }
}

impl std::fmt::Debug for AnnotationNodeVisitor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnnotationNodeVisitor")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl AnnotationVisitor for AnnotationNodeVisitor<'_> {
    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        let name = self.strings.get_opt(name);
        let value = AnnotationNodeValue::from_visited(value, &mut self.strings);
        self.node.values.push((name, value));
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        let name = self.strings.get_opt(name);
        let value = AnnotationNodeValue::Enum {
            descriptor: self.strings.get(descriptor),
            value: self.strings.get(value),
        };
        self.node.values.push((name, value));
    }

    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let name = self.strings.get_opt(name);
        let annotation = AnnotationNode::new(self.strings.get(descriptor));
        self.node
            .values
            .push((name, AnnotationNodeValue::Annotation(annotation)));
        let AnnotationNodeValue::Annotation(annotation) = self.node.last_value_mut() else {
            unreachable!("an annotation was just added")
        };
        Some(Box::new(AnnotationNodeVisitor::new(
            annotation,
            self.strings.reborrow(),
        )))
    }

    fn visit_array<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let name = self.strings.get_opt(name);
        self.node
            .values
            .push((name, AnnotationNodeValue::Array(Vec::new())));
        let AnnotationNodeValue::Array(values) = self.node.last_value_mut() else {
            unreachable!("an array was just added")
        };
        Some(Box::new(ValuesVisitor {
            values: Values::Array(values),
            strings: self.strings.reborrow(),
        }))
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// Where a [`ValuesVisitor`] stores its values.
#[derive(Debug)]
pub(crate) enum Values<'a> {
    /// The values of an array.
    Array(&'a mut Vec<AnnotationNodeValue>),
    /// A method's annotation default value: each visited value replaces it (ASM's
    /// `MethodNode.visitAnnotationDefault`, whose list sets `annotationDefault` on each `add`).
    Default(&'a mut Option<AnnotationNodeValue>),
}

/// The visitor of an annotation array, or of an annotation default value: ASM's `AnnotationNode`
/// without a descriptor, whose values have no names.
pub(crate) struct ValuesVisitor<'a> {
    pub(crate) values: Values<'a>,
    pub(crate) strings: Strings<'a>,
}

impl ValuesVisitor<'_> {
    /// Adds a value, and returns it.
    fn add(&mut self, value: AnnotationNodeValue) -> &mut AnnotationNodeValue {
        match &mut self.values {
            Values::Array(values) => {
                values.push(value);
                values.last_mut().expect("a value was just added")
            }
            Values::Default(slot) => slot.insert(value),
        }
    }
}

impl AnnotationVisitor for ValuesVisitor<'_> {
    fn visit(&mut self, _name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        let value = AnnotationNodeValue::from_visited(value, &mut self.strings);
        self.add(value);
    }

    fn visit_enum(&mut self, _name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        let value = AnnotationNodeValue::Enum {
            descriptor: self.strings.get(descriptor),
            value: self.strings.get(value),
        };
        self.add(value);
    }

    fn visit_annotation<'b>(
        &'b mut self,
        _name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let annotation = AnnotationNode::new(self.strings.get(descriptor));
        let ValuesVisitor { values, strings } = self;
        let value = match values {
            Values::Array(values) => {
                values.push(AnnotationNodeValue::Annotation(annotation));
                values.last_mut().expect("a value was just added")
            }
            Values::Default(slot) => slot.insert(AnnotationNodeValue::Annotation(annotation)),
        };
        let AnnotationNodeValue::Annotation(annotation) = value else {
            unreachable!("an annotation was just added")
        };
        Some(Box::new(AnnotationNodeVisitor::new(
            annotation,
            strings.reborrow(),
        )))
    }

    fn visit_array<'b>(
        &'b mut self,
        _name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let ValuesVisitor { values, strings } = self;
        let value = match values {
            Values::Array(values) => {
                values.push(AnnotationNodeValue::Array(Vec::new()));
                values.last_mut().expect("a value was just added")
            }
            Values::Default(slot) => slot.insert(AnnotationNodeValue::Array(Vec::new())),
        };
        let AnnotationNodeValue::Array(values) = value else {
            unreachable!("an array was just added")
        };
        Some(Box::new(ValuesVisitor {
            values: Values::Array(values),
            strings: strings.reborrow(),
        }))
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// A node that represents a type annotation.
///
/// ASM's `TypeAnnotationNode` extends `AnnotationNode`; here the annotation is a field.
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
        descriptor: Sym,
    ) -> TypeAnnotationNode {
        TypeAnnotationNode {
            type_ref,
            type_path,
            annotation: AnnotationNode::new(descriptor),
        }
    }

    /// The class descriptor of the annotation class.
    pub fn desc(&self) -> Sym {
        self.annotation.desc
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
        descriptor: Sym,
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
    pub fn accept(
        &self,
        interner: &Interner,
        method_visitor: &mut dyn MethodVisitor,
        visible: bool,
    ) {
        // ASM iterates over `start`, and reads `end` and `index` at the same positions.
        let n = self.start.len();
        let av = method_visitor.visit_local_variable_annotation(
            self.type_ref,
            self.type_path.as_ref(),
            &self.start,
            &self.end[..n],
            &self.index[..n],
            interner.resolve(self.annotation.desc),
            visible,
        );
        accept_annotation(interner, &self.annotation, av);
    }
}
