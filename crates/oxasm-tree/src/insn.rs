//! [`InsnNode`], [`Insn`] and [`InsnKind`]: ASM's `AbstractInsnNode` and its 16 subclasses
//! (ADR-11).
//!
//! ASM's node classes (`InsnNode`, `IntInsnNode`, ..., `LineNumberNode`) are the variants of
//! [`Insn`]; what they share, the type annotations of an instruction, is in [`InsnNode`], which
//! an [`InsnList`](crate::InsnList) holds. A `LabelNode` is an [`Insn::Label`]: the label itself is
//! the node's identity, and jump targets, frames, line numbers and the like refer to the label.

use std::collections::HashMap;
use std::hash::BuildHasher;

use oxasm::constant::Constant;
use oxasm::constant::FrameValue;
use oxasm::opcodes::{
    IINC, INVOKEDYNAMIC, INVOKEINTERFACE, LDC, LOOKUPSWITCH, MULTIANEWARRAY, TABLESWITCH,
};
use oxasm::{Handle, JavaArcStr, Label, MethodVisitor};

use crate::annotation_node::{TypeAnnotationNode, accept_annotation};
use crate::frame_node::FrameNode;

/// The type of an instruction node (ASM's `AbstractInsnNode.getType()` constants, which
/// [`InsnKind::to_i32`] returns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InsnKind {
    /// A zero operand instruction ([`Insn::Simple`]).
    Insn,
    /// [`Insn::Int`].
    IntInsn,
    /// [`Insn::Var`].
    VarInsn,
    /// [`Insn::Type`].
    TypeInsn,
    /// [`Insn::Field`].
    FieldInsn,
    /// [`Insn::Method`].
    MethodInsn,
    /// [`Insn::InvokeDynamic`].
    InvokeDynamicInsn,
    /// [`Insn::Jump`].
    JumpInsn,
    /// [`Insn::Label`].
    Label,
    /// [`Insn::Ldc`].
    LdcInsn,
    /// [`Insn::Iinc`].
    IincInsn,
    /// [`Insn::TableSwitch`].
    TableSwitchInsn,
    /// [`Insn::LookupSwitch`].
    LookupSwitchInsn,
    /// [`Insn::MultiANewArray`].
    MultiANewArrayInsn,
    /// [`Insn::Frame`].
    Frame,
    /// [`Insn::Line`].
    Line,
}

impl InsnKind {
    /// ASM's value of this type (`AbstractInsnNode.INSN` = 0, ..., `LINE` = 15).
    pub const fn to_i32(self) -> i32 {
        self as i32
    }
}

/// An invokedynamic instruction (ASM's `InvokeDynamicInsnNode`), boxed in
/// [`Insn::InvokeDynamic`] to keep [`Insn`] small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvokeDynamicInsn {
    /// The method's name.
    pub name: JavaArcStr,
    /// The method's descriptor.
    pub desc: JavaArcStr,
    /// The bootstrap method.
    pub bsm: Handle,
    /// The bootstrap method constant arguments.
    pub bsm_args: Vec<Constant<'static>>,
}

/// A bytecode instruction, or one of the pseudo instructions an instruction list also holds
/// (labels, stack map frames and line numbers).
#[derive(Clone, Debug, PartialEq)]
pub enum Insn {
    /// A zero operand instruction (ASM's `InsnNode`): `NOP`, `ACONST_NULL`, ..., `IADD`, ...,
    /// `MONITOREXIT`.
    Simple {
        /// The opcode.
        opcode: u8,
    },
    /// An instruction with a single int operand (ASM's `IntInsnNode`): `BIPUSH`, `SIPUSH` or
    /// `NEWARRAY`.
    Int {
        /// The opcode.
        opcode: u8,
        /// The operand: a value for `BIPUSH` and `SIPUSH`, an array type (`T_BOOLEAN`, ...) for
        /// `NEWARRAY`.
        operand: i32,
    },
    /// A local variable instruction (ASM's `VarInsnNode`): `ILOAD`, ..., `ASTORE` or `RET`.
    Var {
        /// The opcode.
        opcode: u8,
        /// The index of the local variable.
        var: u16,
    },
    /// A type instruction (ASM's `TypeInsnNode`): `NEW`, `ANEWARRAY`, `CHECKCAST` or
    /// `INSTANCEOF`.
    Type {
        /// The opcode.
        opcode: u8,
        /// The operand: an internal name or, for arrays, a type descriptor.
        desc: JavaArcStr,
    },
    /// A field instruction (ASM's `FieldInsnNode`): `GETSTATIC`, `PUTSTATIC`, `GETFIELD` or
    /// `PUTFIELD`.
    Field {
        /// The opcode.
        opcode: u8,
        /// The internal name of the field's owner class.
        owner: JavaArcStr,
        /// The field's name.
        name: JavaArcStr,
        /// The field's descriptor.
        desc: JavaArcStr,
    },
    /// A method instruction (ASM's `MethodInsnNode`): `INVOKEVIRTUAL`, `INVOKESPECIAL`,
    /// `INVOKESTATIC` or `INVOKEINTERFACE`.
    Method {
        /// The opcode.
        opcode: u8,
        /// The internal name of the method's owner class.
        owner: JavaArcStr,
        /// The method's name.
        name: JavaArcStr,
        /// The method's descriptor.
        desc: JavaArcStr,
        /// Whether the method's owner class is an interface (ASM's `itf`).
        is_interface: bool,
    },
    /// An `INVOKEDYNAMIC` instruction (ASM's `InvokeDynamicInsnNode`).
    InvokeDynamic(Box<InvokeDynamicInsn>),
    /// A jump instruction (ASM's `JumpInsnNode`): `IFEQ`, ..., `GOTO`, `JSR`, `IFNULL` or
    /// `IFNONNULL`.
    Jump {
        /// The opcode.
        opcode: u8,
        /// The jump target.
        label: Label,
    },
    /// A label (ASM's `LabelNode`): a position in the instruction list, which jumps, switches,
    /// frames, line numbers, try catch blocks and local variables refer to.
    Label(Label),
    /// An `LDC` instruction (ASM's `LdcInsnNode`), with the constant it loads.
    Ldc(Constant<'static>),
    /// An `IINC` instruction (ASM's `IincInsnNode`).
    Iinc {
        /// The index of the local variable to be incremented.
        var: u16,
        /// The amount to increment the local variable by.
        incr: i16,
    },
    /// A `TABLESWITCH` instruction (ASM's `TableSwitchInsnNode`).
    TableSwitch {
        /// The minimum key value.
        min: i32,
        /// The maximum key value.
        max: i32,
        /// The beginning of the default handler block.
        dflt: Label,
        /// The beginnings of the handler blocks: `labels[i]` is that of the `min + i` key.
        labels: Vec<Label>,
    },
    /// A `LOOKUPSWITCH` instruction (ASM's `LookupSwitchInsnNode`).
    LookupSwitch {
        /// The beginning of the default handler block.
        dflt: Label,
        /// The values of the keys.
        keys: Vec<i32>,
        /// The beginnings of the handler blocks: `labels[i]` is that of `keys[i]`.
        labels: Vec<Label>,
    },
    /// A `MULTIANEWARRAY` instruction (ASM's `MultiANewArrayInsnNode`).
    MultiANewArray {
        /// An array type descriptor.
        desc: JavaArcStr,
        /// The number of dimensions of the array to allocate.
        dims: u8,
    },
    /// A stack map frame (ASM's `FrameNode`).
    Frame(FrameNode),
    /// A line number declaration (ASM's `LineNumberNode`).
    Line {
        /// A line number, which refers to the source file from which the class was compiled.
        line: u16,
        /// The first instruction corresponding to this line number.
        start: Label,
    },
}

impl Insn {
    /// A type instruction (`NEW`, `ANEWARRAY`, `CHECKCAST` or `INSTANCEOF`).
    pub fn type_insn(opcode: u8, desc: impl Into<JavaArcStr>) -> Insn {
        Insn::Type {
            opcode,
            desc: desc.into(),
        }
    }

    /// A field instruction.
    pub fn field_insn(
        opcode: u8,
        owner: impl Into<JavaArcStr>,
        name: impl Into<JavaArcStr>,
        desc: impl Into<JavaArcStr>,
    ) -> Insn {
        Insn::Field {
            opcode,
            owner: owner.into(),
            name: name.into(),
            desc: desc.into(),
        }
    }

    /// A method instruction.
    pub fn method_insn(
        opcode: u8,
        owner: impl Into<JavaArcStr>,
        name: impl Into<JavaArcStr>,
        desc: impl Into<JavaArcStr>,
        is_interface: bool,
    ) -> Insn {
        Insn::Method {
            opcode,
            owner: owner.into(),
            name: name.into(),
            desc: desc.into(),
            is_interface,
        }
    }

    /// A method instruction whose owner is an interface iff the opcode is `INVOKEINTERFACE`
    /// (ASM's deprecated 4-argument `MethodInsnNode` constructor).
    pub fn method_insn_of(
        opcode: u8,
        owner: impl Into<JavaArcStr>,
        name: impl Into<JavaArcStr>,
        desc: impl Into<JavaArcStr>,
    ) -> Insn {
        Insn::method_insn(opcode, owner, name, desc, opcode == INVOKEINTERFACE)
    }

    /// An `INVOKEDYNAMIC` instruction.
    pub fn invoke_dynamic_insn(
        name: impl Into<JavaArcStr>,
        desc: impl Into<JavaArcStr>,
        bsm: Handle,
        bsm_args: Vec<Constant<'static>>,
    ) -> Insn {
        Insn::InvokeDynamic(Box::new(InvokeDynamicInsn {
            name: name.into(),
            desc: desc.into(),
            bsm,
            bsm_args,
        }))
    }

    /// A `MULTIANEWARRAY` instruction.
    pub fn multi_a_new_array_insn(desc: impl Into<JavaArcStr>, dims: u8) -> Insn {
        Insn::MultiANewArray {
            desc: desc.into(),
            dims,
        }
    }

    /// The type of this instruction (ASM's `getType()`).
    pub fn kind(&self) -> InsnKind {
        match self {
            Insn::Simple { .. } => InsnKind::Insn,
            Insn::Int { .. } => InsnKind::IntInsn,
            Insn::Var { .. } => InsnKind::VarInsn,
            Insn::Type { .. } => InsnKind::TypeInsn,
            Insn::Field { .. } => InsnKind::FieldInsn,
            Insn::Method { .. } => InsnKind::MethodInsn,
            Insn::InvokeDynamic(_) => InsnKind::InvokeDynamicInsn,
            Insn::Jump { .. } => InsnKind::JumpInsn,
            Insn::Label(_) => InsnKind::Label,
            Insn::Ldc(_) => InsnKind::LdcInsn,
            Insn::Iinc { .. } => InsnKind::IincInsn,
            Insn::TableSwitch { .. } => InsnKind::TableSwitchInsn,
            Insn::LookupSwitch { .. } => InsnKind::LookupSwitchInsn,
            Insn::MultiANewArray { .. } => InsnKind::MultiANewArrayInsn,
            Insn::Frame(_) => InsnKind::Frame,
            Insn::Line { .. } => InsnKind::Line,
        }
    }

    /// The opcode of this instruction, or `None` if this is not a JVM instruction (a label, a
    /// frame or a line number, for which ASM's `getOpcode()` returns -1).
    pub fn opcode(&self) -> Option<u8> {
        match self {
            Insn::Simple { opcode }
            | Insn::Int { opcode, .. }
            | Insn::Var { opcode, .. }
            | Insn::Type { opcode, .. }
            | Insn::Field { opcode, .. }
            | Insn::Method { opcode, .. }
            | Insn::Jump { opcode, .. } => Some(*opcode),
            Insn::InvokeDynamic(_) => Some(INVOKEDYNAMIC),
            Insn::Ldc(_) => Some(LDC),
            Insn::Iinc { .. } => Some(IINC),
            Insn::TableSwitch { .. } => Some(TABLESWITCH),
            Insn::LookupSwitch { .. } => Some(LOOKUPSWITCH),
            Insn::MultiANewArray { .. } => Some(MULTIANEWARRAY),
            Insn::Label(_) | Insn::Frame(_) | Insn::Line { .. } => None,
        }
    }

    /// Sets the opcode of this instruction (ASM's `setOpcode`, of the nodes whose opcode
    /// varies).
    ///
    /// # Panics
    ///
    /// If this instruction has a fixed opcode, or none (`INVOKEDYNAMIC`, `LDC`, `IINC`,
    /// `TABLESWITCH`, `LOOKUPSWITCH`, `MULTIANEWARRAY`, labels, frames and line numbers).
    #[track_caller]
    pub fn set_opcode(&mut self, new_opcode: u8) {
        match self {
            Insn::Simple { opcode }
            | Insn::Int { opcode, .. }
            | Insn::Var { opcode, .. }
            | Insn::Type { opcode, .. }
            | Insn::Field { opcode, .. }
            | Insn::Method { opcode, .. }
            | Insn::Jump { opcode, .. } => *opcode = new_opcode,
            insn => panic!("a {:?} instruction has no settable opcode", insn.kind()),
        }
    }

    /// The label of this instruction, if it is a label node.
    pub fn as_label(&self) -> Option<Label> {
        match self {
            Insn::Label(label) => Some(*label),
            _ => None,
        }
    }

    /// Makes the given visitor visit this instruction (without its type annotations).
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        match self {
            Insn::Simple { opcode } => method_visitor.visit_insn(*opcode),
            Insn::Int { opcode, operand } => method_visitor.visit_int_insn(*opcode, *operand),
            Insn::Var { opcode, var } => method_visitor.visit_var_insn(*opcode, *var),
            Insn::Type { opcode, desc } => method_visitor.visit_type_insn(*opcode, desc),
            Insn::Field {
                opcode,
                owner,
                name,
                desc,
            } => method_visitor.visit_field_insn(*opcode, owner, name, desc),
            Insn::Method {
                opcode,
                owner,
                name,
                desc,
                is_interface,
            } => method_visitor.visit_method_insn(*opcode, owner, name, desc, *is_interface),
            Insn::InvokeDynamic(insn) => {
                method_visitor.visit_invoke_dynamic_insn(
                    &insn.name,
                    &insn.desc,
                    &insn.bsm,
                    &insn.bsm_args,
                );
            }
            Insn::Jump { opcode, label } => method_visitor.visit_jump_insn(*opcode, *label),
            Insn::Label(label) => method_visitor.visit_label(*label),
            Insn::Ldc(value) => method_visitor.visit_ldc_insn(value),
            Insn::Iinc { var, incr } => method_visitor.visit_iinc_insn(*var, *incr),
            Insn::TableSwitch {
                min,
                max,
                dflt,
                labels,
            } => method_visitor.visit_table_switch_insn(*min, *max, *dflt, labels),
            Insn::LookupSwitch { dflt, keys, labels } => {
                method_visitor.visit_lookup_switch_insn(*dflt, keys, labels);
            }
            Insn::MultiANewArray { desc, dims } => {
                method_visitor.visit_multi_a_new_array_insn(desc, *dims);
            }
            Insn::Frame(frame) => frame.accept(method_visitor),
            Insn::Line { line, start } => method_visitor.visit_line_number(*line, *start),
        }
    }

    /// A copy of this instruction, whose labels are replaced with their image in the given map
    /// (ASM's `clone(Map<LabelNode, LabelNode>)`). Labels the map does not contain are kept
    /// (ASM would use `null`).
    pub fn clone_with<S: BuildHasher>(&self, cloned_labels: &HashMap<Label, Label, S>) -> Insn {
        let clone = |label: &Label| cloned_labels.get(label).copied().unwrap_or(*label);
        let clones = |labels: &[Label]| labels.iter().map(clone).collect();
        match self {
            Insn::Jump { opcode, label } => Insn::Jump {
                opcode: *opcode,
                label: clone(label),
            },
            Insn::Label(label) => Insn::Label(clone(label)),
            Insn::TableSwitch {
                min,
                max,
                dflt,
                labels,
            } => Insn::TableSwitch {
                min: *min,
                max: *max,
                dflt: clone(dflt),
                labels: clones(labels),
            },
            Insn::LookupSwitch { dflt, keys, labels } => Insn::LookupSwitch {
                dflt: clone(dflt),
                keys: keys.clone(),
                labels: clones(labels),
            },
            Insn::Frame(frame) => {
                let clone_values = |values: &[FrameValue<'static>]| {
                    values
                        .iter()
                        .map(|value| match value {
                            FrameValue::Uninitialized(label) => {
                                FrameValue::Uninitialized(clone(label))
                            }
                            value => value.clone(),
                        })
                        .collect()
                };
                Insn::Frame(FrameNode {
                    kind: frame.kind,
                    local: clone_values(&frame.local),
                    stack: clone_values(&frame.stack),
                })
            }
            Insn::Line { line, start } => Insn::Line {
                line: *line,
                start: clone(start),
            },
            insn => insn.clone(),
        }
    }
}

/// An instruction of an [`InsnList`](crate::InsnList): an [`Insn`] and the type annotations of
/// the instruction (ASM's `AbstractInsnNode`).
#[derive(Clone, Debug, PartialEq)]
pub struct InsnNode {
    /// The instruction.
    pub insn: Insn,
    /// The type annotations of this instruction, or `None` if it has none (ASM's
    /// `visibleTypeAnnotations` and `invisibleTypeAnnotations`, both `null`). Only used for real
    /// instructions (not for labels, frames, or line number nodes).
    ///
    /// They are boxed because instructions rarely have any: this keeps an instruction node
    /// small. [`visible_type_annotations`](Self::visible_type_annotations) and
    /// [`invisible_type_annotations`](Self::invisible_type_annotations) read them, and
    /// [`type_annotations_mut`](Self::type_annotations_mut) adds some.
    pub type_annotations: Option<Box<InsnTypeAnnotations>>,
}

/// The type annotations of an instruction (see [`InsnNode::type_annotations`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InsnTypeAnnotations {
    /// The runtime visible type annotations of the instruction.
    pub visible: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of the instruction.
    pub invisible: Vec<TypeAnnotationNode>,
}

impl InsnNode {
    /// An instruction node without type annotations.
    pub fn new(insn: Insn) -> InsnNode {
        InsnNode {
            insn,
            type_annotations: None,
        }
    }

    /// The type of this instruction (ASM's `getType()`).
    pub fn kind(&self) -> InsnKind {
        self.insn.kind()
    }

    /// The opcode of this instruction, or `None` for a label, a frame or a line number (ASM's
    /// `getOpcode()`, which returns -1 for those).
    pub fn opcode(&self) -> Option<u8> {
        self.insn.opcode()
    }

    /// The runtime visible type annotations of this instruction.
    pub fn visible_type_annotations(&self) -> &[TypeAnnotationNode] {
        self.type_annotations
            .as_deref()
            .map_or(&[], |annotations| &annotations.visible)
    }

    /// The runtime invisible type annotations of this instruction.
    pub fn invisible_type_annotations(&self) -> &[TypeAnnotationNode] {
        self.type_annotations
            .as_deref()
            .map_or(&[], |annotations| &annotations.invisible)
    }

    /// The type annotations of this instruction, which are created (empty) if it has none.
    pub fn type_annotations_mut(&mut self) -> &mut InsnTypeAnnotations {
        self.type_annotations.get_or_insert_default()
    }

    /// Makes the given visitor visit this instruction, then its type annotations if it is a
    /// real instruction.
    pub fn accept(&self, method_visitor: &mut dyn MethodVisitor) {
        self.insn.accept(method_visitor);
        if !matches!(
            self.insn,
            Insn::Label(_) | Insn::Frame(_) | Insn::Line { .. }
        ) {
            self.accept_annotations(method_visitor);
        }
    }

    /// Makes the given visitor visit the type annotations of this instruction.
    fn accept_annotations(&self, method_visitor: &mut dyn MethodVisitor) {
        for (annotations, visible) in [
            (self.visible_type_annotations(), true),
            (self.invisible_type_annotations(), false),
        ] {
            for type_annotation in annotations {
                let av = method_visitor.visit_insn_annotation(
                    type_annotation.type_ref,
                    type_annotation.type_path.as_ref(),
                    type_annotation.desc(),
                    visible,
                );
                accept_annotation(&type_annotation.annotation, av);
            }
        }
    }

    /// A copy of this instruction, whose labels are replaced with their image in the given map
    /// (ASM's `clone(Map<LabelNode, LabelNode>)`); see [`Insn::clone_with`]. Like in ASM, the
    /// type annotations of real instructions are copied, and those of labels, frames and line
    /// numbers are not.
    pub fn clone_with<S: BuildHasher>(&self, cloned_labels: &HashMap<Label, Label, S>) -> InsnNode {
        let mut clone = InsnNode::new(self.insn.clone_with(cloned_labels));
        if !matches!(
            self.insn,
            Insn::Label(_) | Insn::Frame(_) | Insn::Line { .. }
        ) {
            clone.type_annotations.clone_from(&self.type_annotations);
        }
        clone
    }
}

impl From<Insn> for InsnNode {
    fn from(insn: Insn) -> InsnNode {
        InsnNode::new(insn)
    }
}

impl From<FrameNode> for InsnNode {
    fn from(frame: FrameNode) -> InsnNode {
        InsnNode::new(Insn::Frame(frame))
    }
}
