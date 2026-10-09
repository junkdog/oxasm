//! [`InsnNode`], [`Insn`] and [`InsnKind`]: ASM's `AbstractInsnNode` and its 16 subclasses
//! (ADR-11).
//!
//! ASM's node classes (`InsnNode`, `IntInsnNode`, ..., `LineNumberNode`) are the variants of
//! [`Insn`]; what they share, the type annotations of an instruction, is in [`InsnNode`], which
//! an [`InsnList`](crate::InsnList) holds. A `LabelNode` is an [`Insn::Label`]: the label itself is
//! the node's identity, and jump targets, frames, line numbers and the like refer to the label.

use std::collections::HashMap;
use std::hash::BuildHasher;

use oxasm::opcodes::{
    IINC, INVOKEDYNAMIC, INVOKEINTERFACE, LDC, LOOKUPSWITCH, MULTIANEWARRAY, TABLESWITCH,
};
use oxasm::{Interner, Label, MethodVisitor, Sym};

use crate::annotation_node::{TypeAnnotationNode, accept_annotation};
use crate::frame_node::FrameNode;
use crate::value::{SymConstant, SymFrameValue, SymHandle};

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
    pub name: Sym,
    /// The method's descriptor.
    pub desc: Sym,
    /// The bootstrap method.
    pub bsm: SymHandle,
    /// The bootstrap method constant arguments.
    pub bsm_args: Vec<SymConstant>,
}

/// A `TABLESWITCH` instruction (ASM's `TableSwitchInsnNode`), boxed in [`Insn::TableSwitch`] to
/// keep [`Insn`] small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSwitchInsn {
    /// The minimum key value.
    pub min: i32,
    /// The maximum key value.
    pub max: i32,
    /// The beginning of the default handler block.
    pub dflt: Label,
    /// The beginnings of the handler blocks: `labels[i]` is that of the `min + i` key.
    pub labels: Vec<Label>,
}

/// A `LOOKUPSWITCH` instruction (ASM's `LookupSwitchInsnNode`), boxed in
/// [`Insn::LookupSwitch`] to keep [`Insn`] small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupSwitchInsn {
    /// The beginning of the default handler block.
    pub dflt: Label,
    /// The values of the keys.
    pub keys: Vec<i32>,
    /// The beginnings of the handler blocks: `labels[i]` is that of `keys[i]`.
    pub labels: Vec<Label>,
}

/// A bytecode instruction, or one of the pseudo instructions an instruction list also holds
/// (labels, stack map frames and line numbers).
///
/// Its strings are [`Sym`]s of the interner of the method that holds it. The variants with
/// vectors are boxed, which keeps an `Insn` at 24 bytes.
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
        desc: Sym,
    },
    /// A field instruction (ASM's `FieldInsnNode`): `GETSTATIC`, `PUTSTATIC`, `GETFIELD` or
    /// `PUTFIELD`.
    Field {
        /// The opcode.
        opcode: u8,
        /// The internal name of the field's owner class.
        owner: Sym,
        /// The field's name.
        name: Sym,
        /// The field's descriptor.
        desc: Sym,
    },
    /// A method instruction (ASM's `MethodInsnNode`): `INVOKEVIRTUAL`, `INVOKESPECIAL`,
    /// `INVOKESTATIC` or `INVOKEINTERFACE`.
    Method {
        /// The opcode.
        opcode: u8,
        /// The internal name of the method's owner class.
        owner: Sym,
        /// The method's name.
        name: Sym,
        /// The method's descriptor.
        desc: Sym,
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
    Ldc(SymConstant),
    /// An `IINC` instruction (ASM's `IincInsnNode`).
    Iinc {
        /// The index of the local variable to be incremented.
        var: u16,
        /// The amount to increment the local variable by.
        incr: i16,
    },
    /// A `TABLESWITCH` instruction (ASM's `TableSwitchInsnNode`).
    TableSwitch(Box<TableSwitchInsn>),
    /// A `LOOKUPSWITCH` instruction (ASM's `LookupSwitchInsnNode`).
    LookupSwitch(Box<LookupSwitchInsn>),
    /// A `MULTIANEWARRAY` instruction (ASM's `MultiANewArrayInsnNode`).
    MultiANewArray {
        /// An array type descriptor.
        desc: Sym,
        /// The number of dimensions of the array to allocate.
        dims: u8,
    },
    /// A stack map frame (ASM's `FrameNode`).
    Frame(Box<FrameNode>),
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
    pub fn type_insn(opcode: u8, desc: Sym) -> Insn {
        Insn::Type { opcode, desc }
    }

    /// A field instruction.
    pub fn field_insn(opcode: u8, owner: Sym, name: Sym, desc: Sym) -> Insn {
        Insn::Field {
            opcode,
            owner,
            name,
            desc,
        }
    }

    /// A method instruction.
    pub fn method_insn(opcode: u8, owner: Sym, name: Sym, desc: Sym, is_interface: bool) -> Insn {
        Insn::Method {
            opcode,
            owner,
            name,
            desc,
            is_interface,
        }
    }

    /// A method instruction whose owner is an interface iff the opcode is `INVOKEINTERFACE`
    /// (ASM's deprecated 4-argument `MethodInsnNode` constructor).
    pub fn method_insn_of(opcode: u8, owner: Sym, name: Sym, desc: Sym) -> Insn {
        Insn::method_insn(opcode, owner, name, desc, opcode == INVOKEINTERFACE)
    }

    /// An `INVOKEDYNAMIC` instruction.
    pub fn invoke_dynamic_insn(
        name: Sym,
        desc: Sym,
        bsm: SymHandle,
        bsm_args: Vec<SymConstant>,
    ) -> Insn {
        Insn::InvokeDynamic(Box::new(InvokeDynamicInsn {
            name,
            desc,
            bsm,
            bsm_args,
        }))
    }

    /// A `TABLESWITCH` instruction.
    pub fn table_switch_insn(min: i32, max: i32, dflt: Label, labels: Vec<Label>) -> Insn {
        Insn::TableSwitch(Box::new(TableSwitchInsn {
            min,
            max,
            dflt,
            labels,
        }))
    }

    /// A `LOOKUPSWITCH` instruction.
    pub fn lookup_switch_insn(dflt: Label, keys: Vec<i32>, labels: Vec<Label>) -> Insn {
        Insn::LookupSwitch(Box::new(LookupSwitchInsn { dflt, keys, labels }))
    }

    /// A stack map frame.
    pub fn frame(frame: FrameNode) -> Insn {
        Insn::Frame(Box::new(frame))
    }

    /// A `MULTIANEWARRAY` instruction.
    pub fn multi_a_new_array_insn(desc: Sym, dims: u8) -> Insn {
        Insn::MultiANewArray { desc, dims }
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
            Insn::TableSwitch(_) => InsnKind::TableSwitchInsn,
            Insn::LookupSwitch(_) => InsnKind::LookupSwitchInsn,
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
            Insn::TableSwitch(_) => Some(TABLESWITCH),
            Insn::LookupSwitch(_) => Some(LOOKUPSWITCH),
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

    /// The default label and the case labels of this instruction, if it is a `TABLESWITCH` or a
    /// `LOOKUPSWITCH`.
    pub fn switch_labels(&self) -> Option<(Label, &[Label])> {
        match self {
            Insn::TableSwitch(insn) => Some((insn.dflt, &insn.labels)),
            Insn::LookupSwitch(insn) => Some((insn.dflt, &insn.labels)),
            _ => None,
        }
    }

    /// Whether this instruction owns heap memory (a box, or a [`SymConstant::Dynamic`]): the
    /// others have no drop glue, which lets an [`InsnList`](crate::InsnList) drop only these.
    pub(crate) fn owns_memory(&self) -> bool {
        match self {
            Insn::Simple { .. }
            | Insn::Int { .. }
            | Insn::Var { .. }
            | Insn::Type { .. }
            | Insn::Field { .. }
            | Insn::Method { .. }
            | Insn::Jump { .. }
            | Insn::Label(_)
            | Insn::Iinc { .. }
            | Insn::MultiANewArray { .. }
            | Insn::Line { .. } => false,
            Insn::Ldc(constant) => match constant {
                SymConstant::Integer(_)
                | SymConstant::Float(_)
                | SymConstant::Long(_)
                | SymConstant::Double(_)
                | SymConstant::String(_)
                | SymConstant::Type(_)
                | SymConstant::MethodType(_)
                | SymConstant::Handle(_) => false,
                SymConstant::Dynamic(_) => true,
            },
            Insn::InvokeDynamic(_)
            | Insn::TableSwitch(_)
            | Insn::LookupSwitch(_)
            | Insn::Frame(_) => true,
        }
    }

    /// The label of this instruction, if it is a label node.
    pub fn as_label(&self) -> Option<Label> {
        match self {
            Insn::Label(label) => Some(*label),
            _ => None,
        }
    }

    /// Makes the given visitor visit this instruction (without its type annotations), with
    /// the strings of the given interner.
    pub fn accept(&self, interner: &Interner, method_visitor: &mut dyn MethodVisitor) {
        let resolve = |sym: &Sym| interner.resolve(*sym);
        match self {
            Insn::Simple { opcode } => method_visitor.visit_insn(*opcode),
            Insn::Int { opcode, operand } => method_visitor.visit_int_insn(*opcode, *operand),
            Insn::Var { opcode, var } => method_visitor.visit_var_insn(*opcode, *var),
            Insn::Type { opcode, desc } => method_visitor.visit_type_insn(*opcode, resolve(desc)),
            Insn::Field {
                opcode,
                owner,
                name,
                desc,
            } => method_visitor.visit_field_insn(
                *opcode,
                resolve(owner),
                resolve(name),
                resolve(desc),
            ),
            Insn::Method {
                opcode,
                owner,
                name,
                desc,
                is_interface,
            } => method_visitor.visit_method_insn(
                *opcode,
                resolve(owner),
                resolve(name),
                resolve(desc),
                *is_interface,
            ),
            Insn::InvokeDynamic(insn) => {
                let bsm_args: Vec<_> = insn
                    .bsm_args
                    .iter()
                    .map(|arg| arg.resolve(interner))
                    .collect();
                method_visitor.visit_invoke_dynamic_insn(
                    resolve(&insn.name),
                    resolve(&insn.desc),
                    &insn.bsm.resolve(interner),
                    &bsm_args,
                );
            }
            Insn::Jump { opcode, label } => method_visitor.visit_jump_insn(*opcode, *label),
            Insn::Label(label) => method_visitor.visit_label(*label),
            Insn::Ldc(value) => method_visitor.visit_ldc_insn(&value.resolve(interner)),
            Insn::Iinc { var, incr } => method_visitor.visit_iinc_insn(*var, *incr),
            Insn::TableSwitch(insn) => {
                method_visitor.visit_table_switch_insn(insn.min, insn.max, insn.dflt, &insn.labels);
            }
            Insn::LookupSwitch(insn) => {
                method_visitor.visit_lookup_switch_insn(insn.dflt, &insn.keys, &insn.labels);
            }
            Insn::MultiANewArray { desc, dims } => {
                method_visitor.visit_multi_a_new_array_insn(resolve(desc), *dims);
            }
            Insn::Frame(frame) => frame.accept(interner, method_visitor),
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
            Insn::TableSwitch(insn) => {
                Insn::table_switch_insn(insn.min, insn.max, clone(&insn.dflt), clones(&insn.labels))
            }
            Insn::LookupSwitch(insn) => {
                Insn::lookup_switch_insn(clone(&insn.dflt), insn.keys.clone(), clones(&insn.labels))
            }
            Insn::Frame(frame) => {
                let clone_values = |values: &[SymFrameValue]| {
                    values
                        .iter()
                        .map(|value| match value {
                            SymFrameValue::Uninitialized(label) => {
                                SymFrameValue::Uninitialized(clone(label))
                            }
                            value => *value,
                        })
                        .collect()
                };
                Insn::frame(FrameNode {
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

// The fields of the instructions that `owns_memory` says own none.
const _: () = assert!(
    !std::mem::needs_drop::<Sym>()
        && !std::mem::needs_drop::<Label>()
        && !std::mem::needs_drop::<SymHandle>()
);

/// An instruction of an [`InsnList`](crate::InsnList): an [`Insn`] and the type annotations of
/// the instruction (ASM's `AbstractInsnNode`).
///
/// The type annotations of an instruction are not in its node: the list holds them (see
/// [`InsnList::type_annotations`](crate::InsnList::type_annotations)), since instructions rarely
/// have any. This keeps a node at the size of its [`Insn`], with no drop glue of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct InsnNode {
    /// The instruction.
    pub insn: Insn,
}

/// The type annotations of an instruction (ASM's `visibleTypeAnnotations` and
/// `invisibleTypeAnnotations`), held by its [`InsnList`](crate::InsnList). Only real
/// instructions have some (not labels, frames, or line number nodes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InsnTypeAnnotations {
    /// The runtime visible type annotations of the instruction.
    pub visible: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of the instruction.
    pub invisible: Vec<TypeAnnotationNode>,
}

impl InsnTypeAnnotations {
    /// Makes the given visitor visit these type annotations, with the strings of the given
    /// interner.
    pub fn accept(&self, interner: &Interner, method_visitor: &mut dyn MethodVisitor) {
        for (annotations, visible) in [(&self.visible, true), (&self.invisible, false)] {
            for type_annotation in annotations {
                let av = method_visitor.visit_insn_annotation(
                    type_annotation.type_ref,
                    type_annotation.type_path.as_ref(),
                    interner.resolve(type_annotation.desc()),
                    visible,
                );
                accept_annotation(interner, &type_annotation.annotation, av);
            }
        }
    }
}

impl InsnNode {
    /// An instruction node.
    pub fn new(insn: Insn) -> InsnNode {
        InsnNode { insn }
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

    /// A copy of this instruction, whose labels are replaced with their image in the given map
    /// (ASM's `clone(Map<LabelNode, LabelNode>)`); see [`Insn::clone_with`]. ASM's clone also
    /// copies the type annotations of real instructions, which here are in the list: see
    /// [`InsnList::type_annotations`](crate::InsnList::type_annotations).
    pub fn clone_with<S: BuildHasher>(&self, cloned_labels: &HashMap<Label, Label, S>) -> InsnNode {
        InsnNode::new(self.insn.clone_with(cloned_labels))
    }
}

impl From<Insn> for InsnNode {
    fn from(insn: Insn) -> InsnNode {
        InsnNode::new(insn)
    }
}

impl From<FrameNode> for InsnNode {
    fn from(frame: FrameNode) -> InsnNode {
        InsnNode::new(Insn::frame(frame))
    }
}
