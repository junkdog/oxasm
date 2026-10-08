//! Port of ASM's `LdcInsnNodeTest`.

use oxasm::constant::Constant;
use oxasm::opcodes::LDC;
use oxasm::{Interner, Type};
use oxasm_tree::{Insn, InsnKind, SymConstant};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let ldc_insn_node = Insn::Ldc(SymConstant::intern(&Constant::from("s"), &interner));

    assert_eq!(ldc_insn_node.kind(), InsnKind::LdcInsn);
    assert_eq!(ldc_insn_node.opcode(), Some(LDC));
    assert!(
        matches!(&ldc_insn_node, Insn::Ldc(cst) if cst.resolve(&interner) == Constant::from("s"))
    );
}

/// Not in ASM: the tree's constants give back the types they were made from, even a method type
/// whose descriptor isn't one (`Type::get_method_type` keeps it as is), which a malformed
/// `CONSTANT_MethodType` gives.
#[test]
fn test_types_round_trip() {
    let interner = Interner::new();
    for ty in [
        Type::get_method_type(""),
        Type::get_method_type("V"),
        Type::get_method_type("(I)V"),
        Type::get_object_type("a;b"),
        Type::get_object_type(""),
        Type::get_object_type("[I"),
        Type::get_type("Ljava/lang/String;"),
    ] {
        let constant = SymConstant::intern(&Constant::Type(ty.clone()), &interner);
        let Constant::Type(resolved) = constant.resolve(&interner) else {
            panic!("a type constant");
        };
        assert_eq!(resolved.get_sort(), ty.get_sort());
        assert_eq!(resolved.get_descriptor(), ty.get_descriptor());
        assert_eq!(resolved, ty);
    }
}
