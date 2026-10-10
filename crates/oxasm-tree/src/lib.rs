//! The tree (object model) API for [`oxasm`]: a Rust port of ASM's `asm-tree` module
//! (`org.objectweb.asm.tree`).
//!
//! A [`ClassNode`] is a class visitor that stores what it visits, and replays it with
//! [`ClassNode::accept`]; likewise for the other member nodes. The instructions of a method are an
//! [`InsnList`] of [`InsnNode`]s, designated by [`InsnId`] handles (ADR-11):
//!
//! - ASM's sixteen `*InsnNode` classes are the variants of [`Insn`] (`getType()` is
//!   [`Insn::kind`], `getOpcode()` is [`Insn::opcode`]);
//! - a `LabelNode` is an [`Insn::Label`], whose [`Label`](oxasm::Label) is its identity: jumps,
//!   switches, frames, line numbers, try catch blocks and local variables refer to labels, and
//!   [`InsnList::label_position`] finds their node;
//! - ASM's `getNext()`/`getPrevious()` and `indexOf` are methods of the list, which owns the
//!   nodes.
//!
//! The strings of the nodes are interned [`Sym`](oxasm::Sym)s (ADR-04's P9 amendment): four
//! bytes each, compared and hashed as integers. A [`ClassNode`], [`MethodNode`], [`FieldNode`],
//! [`RecordComponentNode`] or [`ModuleNode`] holds its [`Interner`](oxasm::Interner), in an
//! `Arc` (and so do the members of a class node: the class's); the other nodes hold symbols of
//! the interner of the node that holds them, and their `accept` methods take that interner. The
//! constants a tree stores are [`SymConstant`]s, [`SymHandle`]s and the like.
//!
//! Every node is `Send + Sync`.
//!
//! [`oxasm`]: https://docs.rs/oxasm

mod annotation_node;
mod class_node;
mod field_node;
mod frame_node;
mod insn;
mod insn_list;
mod method_node;
mod module_node;
mod nodes;
mod strings;
mod value;

pub use annotation_node::{
    AnnotationNode, AnnotationNodeValue, AnnotationNodeVisitor, LocalVariableAnnotationNode,
    TypeAnnotationNode,
};
pub use class_node::ClassNode;
pub use field_node::{FieldExtras, FieldNode, RecordComponentNode};
pub use frame_node::FrameNode;
pub use insn::{
    Insn, InsnKind, InsnNode, InsnTypeAnnotations, InvokeDynamicInsn, LookupSwitchInsn,
    TableSwitchInsn,
};
pub use insn_list::{Ids, InsnCursor, InsnId, InsnList, Iter};
pub use method_node::{MethodExtras, MethodNode};
pub use module_node::{
    ModuleExportNode, ModuleNode, ModuleOpenNode, ModuleProvideNode, ModuleRequireNode,
};
pub use nodes::{InnerClassNode, LocalVariableNode, ParameterNode, TryCatchBlockNode};
pub use value::{SymConstant, SymConstantDynamic, SymFrameValue, SymHandle};
