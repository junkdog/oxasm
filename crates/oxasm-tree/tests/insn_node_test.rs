//! Port of ASM's `InsnNodeTest` (a zero operand instruction, here `Insn::Simple`).

use oxasm::opcodes::ACONST_NULL;
use oxasm_tree::{Insn, InsnKind, InsnNode};

#[test]
fn test_constructor() {
    let insn_node = InsnNode::new(Insn::Simple {
        opcode: ACONST_NULL,
    });

    assert_eq!(insn_node.kind(), InsnKind::Insn);
    assert_eq!(insn_node.opcode(), Some(ACONST_NULL));
}

/// oxasm-specific: `InsnKind` has ASM's `AbstractInsnNode` type values.
#[test]
fn insn_kinds_have_asm_values() {
    let kinds = [
        InsnKind::Insn,
        InsnKind::IntInsn,
        InsnKind::VarInsn,
        InsnKind::TypeInsn,
        InsnKind::FieldInsn,
        InsnKind::MethodInsn,
        InsnKind::InvokeDynamicInsn,
        InsnKind::JumpInsn,
        InsnKind::Label,
        InsnKind::LdcInsn,
        InsnKind::IincInsn,
        InsnKind::TableSwitchInsn,
        InsnKind::LookupSwitchInsn,
        InsnKind::MultiANewArrayInsn,
        InsnKind::Frame,
        InsnKind::Line,
    ];
    for (value, kind) in kinds.into_iter().enumerate() {
        assert_eq!(kind.to_i32(), i32::try_from(value).unwrap());
    }
}

/// oxasm-specific: an instruction stays small, since a method holds thousands of them (the
/// variants with vectors are boxed).
#[test]
fn insn_is_small() {
    assert_eq!(size_of::<Insn>(), 24);
}
