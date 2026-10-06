//! Port of ASM's `InvokeDynamicInsnNodeTest`.

use oxasm::constant::Constant;
use oxasm::opcodes::INVOKEDYNAMIC;
use oxasm::{Handle, HandleKind};
use oxasm_tree::{Insn, InsnKind};

#[test]
fn test_constructor() {
    let handle = Handle::new(HandleKind::InvokeStatic, "owner", "name", "()V", false);
    let bootstrap_method_arguments = vec![Constant::from("s").into_owned()];

    let invoke_dynamic_insn_node = Insn::invoke_dynamic_insn(
        "name",
        "()V",
        handle.clone(),
        bootstrap_method_arguments.clone(),
    );

    assert_eq!(invoke_dynamic_insn_node.opcode(), Some(INVOKEDYNAMIC));
    assert_eq!(invoke_dynamic_insn_node.kind(), InsnKind::InvokeDynamicInsn);
    let Insn::InvokeDynamic(insn) = &invoke_dynamic_insn_node else {
        unreachable!()
    };
    assert_eq!(insn.name, "name");
    assert_eq!(insn.desc, "()V");
    assert_eq!(insn.bsm, handle);
    assert_eq!(insn.bsm_args, bootstrap_method_arguments);
}
