//! Port of ASM's `InvokeDynamicInsnNodeTest`.

use oxasm::constant::Constant;
use oxasm::opcodes::INVOKEDYNAMIC;
use oxasm::{Handle, HandleKind, Interner};
use oxasm_tree::{Insn, InsnKind, SymConstant, SymHandle};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let handle = SymHandle::intern(
        &Handle::new(HandleKind::InvokeStatic, "owner", "name", "()V", false),
        &interner,
    );
    let bootstrap_method_arguments = vec![SymConstant::intern(&Constant::from("s"), &interner)];

    let invoke_dynamic_insn_node = Insn::invoke_dynamic_insn(
        interner.intern("name"),
        interner.intern("()V"),
        handle,
        bootstrap_method_arguments.clone(),
    );

    assert_eq!(invoke_dynamic_insn_node.opcode(), Some(INVOKEDYNAMIC));
    assert_eq!(invoke_dynamic_insn_node.kind(), InsnKind::InvokeDynamicInsn);
    let Insn::InvokeDynamic(insn) = &invoke_dynamic_insn_node else {
        unreachable!()
    };
    assert_eq!(interner.resolve(insn.name), "name");
    assert_eq!(interner.resolve(insn.desc), "()V");
    assert_eq!(insn.bsm, handle);
    assert_eq!(insn.bsm_args, bootstrap_method_arguments);
}
