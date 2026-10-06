//! The plain member nodes: [`InnerClassNode`], [`ParameterNode`], [`LocalVariableNode`] and
//! [`TryCatchBlockNode`] (ASM's classes of the same names).

use oxasm::{ClassVisitor, JavaArcStr, Label, MethodVisitor, TypeReference};

use crate::annotation_node::{TypeAnnotationNode, accept_annotation};

/// A node that represents an inner class. This inner class is not necessarily a member of the
/// [`ClassNode`](crate::ClassNode) containing this object. More precisely, every class or
/// interface C which is referenced by a class and which is not a package member must be
/// represented with an `InnerClassNode`. The class must reference its nested class or interface
/// members, and its enclosing class, if any. See the JVMS 4.7.6 section for more details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InnerClassNode {
    /// The internal name of an inner class.
    pub name: JavaArcStr,
    /// The internal name of the class to which the inner class belongs. `None` if the inner
    /// class is not the member of a class or interface (e.g. for local or anonymous classes).
    pub outer_name: Option<JavaArcStr>,
    /// The (simple) name of the inner class inside its enclosing class. `None` for anonymous
    /// inner classes.
    pub inner_name: Option<JavaArcStr>,
    /// The access flags of the inner class as originally declared in the enclosing class.
    pub access: u32,
}

impl InnerClassNode {
    /// Constructs a new inner class node.
    pub fn new(
        name: impl Into<JavaArcStr>,
        outer_name: Option<JavaArcStr>,
        inner_name: Option<JavaArcStr>,
        access: u32,
    ) -> InnerClassNode {
        InnerClassNode {
            name: name.into(),
            outer_name,
            inner_name,
            access,
        }
    }

    /// Makes the given class visitor visit this inner class.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        class_visitor.visit_inner_class(
            &self.name,
            self.outer_name.as_deref(),
            self.inner_name.as_deref(),
            self.access,
        );
    }
}

/// A node that represents a parameter of a method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterNode {
    /// The parameter's name, or `None` if none is provided.
    pub name: Option<JavaArcStr>,
    /// The parameter's access flags (see [`oxasm::opcodes`]). Valid values are `ACC_FINAL`,
    /// `ACC_SYNTHETIC` and `ACC_MANDATED`.
    pub access: u32,
}

impl ParameterNode {
    /// Constructs a new parameter node.
    pub fn new(name: Option<JavaArcStr>, access: u32) -> ParameterNode {
        ParameterNode { name, access }
    }

    /// Makes the given visitor visit this parameter declaration.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        method_visitor.visit_parameter(self.name.as_deref(), self.access);
    }
}

/// A node that represents a local variable declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalVariableNode {
    /// The name of a local variable.
    pub name: JavaArcStr,
    /// The type descriptor of this local variable.
    pub desc: JavaArcStr,
    /// The signature of this local variable, or `None`.
    pub signature: Option<JavaArcStr>,
    /// The first instruction corresponding to the scope of this local variable (inclusive).
    pub start: Label,
    /// The last instruction corresponding to the scope of this local variable (exclusive).
    pub end: Label,
    /// The local variable's index.
    pub index: u16,
}

impl LocalVariableNode {
    /// Constructs a new local variable node.
    pub fn new(
        name: impl Into<JavaArcStr>,
        descriptor: impl Into<JavaArcStr>,
        signature: Option<JavaArcStr>,
        start: Label,
        end: Label,
        index: u16,
    ) -> LocalVariableNode {
        LocalVariableNode {
            name: name.into(),
            desc: descriptor.into(),
            signature,
            start,
            end,
            index,
        }
    }

    /// Makes the given visitor visit this local variable declaration.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        method_visitor.visit_local_variable(
            &self.name,
            &self.desc,
            self.signature.as_deref(),
            self.start,
            self.end,
            self.index,
        );
    }
}

/// A node that represents a try catch block.
#[derive(Clone, Debug, PartialEq)]
pub struct TryCatchBlockNode {
    /// The beginning of the exception handler's scope (inclusive).
    pub start: Label,
    /// The end of the exception handler's scope (exclusive).
    pub end: Label,
    /// The beginning of the exception handler's code.
    pub handler: Label,
    /// The internal name of the type of exceptions handled by the handler. `None` to catch any
    /// exceptions (for "finally" blocks).
    pub type_: Option<JavaArcStr>,
    /// The runtime visible type annotations on the exception handler type.
    pub visible_type_annotations: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations on the exception handler type.
    pub invisible_type_annotations: Vec<TypeAnnotationNode>,
}

impl TryCatchBlockNode {
    /// Constructs a new try catch block node, without type annotations.
    pub fn new(
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<JavaArcStr>,
    ) -> TryCatchBlockNode {
        TryCatchBlockNode {
            start,
            end,
            handler,
            type_,
            visible_type_annotations: Vec::new(),
            invisible_type_annotations: Vec::new(),
        }
    }

    /// Updates the index of this try catch block in the method's list of try catch block
    /// nodes. This index may change because of the insertion or removal of try catch blocks,
    /// and this changes the type references of the type annotations.
    pub fn update_index(&mut self, index: usize) {
        let new_type_ref = try_catch_type_ref(index);
        for type_annotation in self
            .visible_type_annotations
            .iter_mut()
            .chain(&mut self.invisible_type_annotations)
        {
            type_annotation.type_ref = new_type_ref;
        }
    }

    /// Makes the given visitor visit this try catch block.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        self.accept_with(method_visitor, None);
    }

    /// Makes the given visitor visit this try catch block, with the type references of its
    /// type annotations set to `type_ref`, if any: what `MethodNode.accept` visits after calling
    /// `updateIndex` (which mutates the node, unlike `MethodNode::accept` here).
    pub(crate) fn accept_with(
        &self,
        method_visitor: &mut dyn MethodVisitor,
        type_ref: Option<TypeReference>,
    ) {
        method_visitor.visit_try_catch_block(
            self.start,
            self.end,
            self.handler,
            self.type_.as_deref(),
        );
        for (annotations, visible) in [
            (&self.visible_type_annotations, true),
            (&self.invisible_type_annotations, false),
        ] {
            for type_annotation in annotations {
                let av = method_visitor.visit_try_catch_annotation(
                    type_ref.unwrap_or(type_annotation.type_ref),
                    type_annotation.type_path.as_ref(),
                    type_annotation.desc(),
                    visible,
                );
                accept_annotation(&type_annotation.annotation, av);
            }
        }
    }
}

/// The type reference of the type annotations of the try catch block of the given index (ASM's
/// `0x42000000 | (index << 8)`, with Java's `int` shift).
pub(crate) fn try_catch_type_ref(index: usize) -> TypeReference {
    let index = u32::try_from(index).expect("fewer than 2^32 try catch blocks");
    TypeReference::new(0x4200_0000 | (index << 8))
}
