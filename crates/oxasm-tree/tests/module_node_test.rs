//! Port of ASM's `ModuleNodeTest`. `testConstructor_illegalState` is dropped: there is no
//! subclassing, nor `api` (ADR-09).

use std::sync::Arc;

use oxasm::{ClassVisitor, Interner, JavaStr, ModuleVisitor};
use oxasm_tree::ModuleNode;

#[test]
fn test_constructor() {
    let interner = Arc::new(Interner::new());
    let i = |s: &str| interner.intern(s);
    let module_node1 = ModuleNode::new(interner.clone(), i("module1"), 123, Some(i("1.0")));
    // ASM's constructor with lists (here, null ones): `new`, whose lists are empty.
    let module_node2 = ModuleNode::new(interner.clone(), i("module2"), 456, Some(i("2.0")));

    assert_eq!(module_node1.resolve(module_node1.name), "module1");
    assert_eq!(module_node1.access, 123);
    assert_eq!(module_node1.resolve(module_node1.version.unwrap()), "1.0");
    assert_eq!(module_node2.resolve(module_node2.name), "module2");
    assert_eq!(module_node2.access, 456);
    assert_eq!(module_node2.resolve(module_node2.version.unwrap()), "2.0");
}

#[test]
fn test_accept() {
    struct CopyModuleVisitor<'a>(&'a mut ModuleNode);
    impl ClassVisitor for CopyModuleVisitor<'_> {
        fn visit_module<'a>(
            &'a mut self,
            name: &JavaStr,
            access: u32,
            version: Option<&JavaStr>,
        ) -> Option<Box<dyn ModuleVisitor + 'a>> {
            self.0.name = self.0.intern(name);
            self.0.access = access;
            self.0.version = version.map(|v| self.0.intern(v));
            Some(Box::new(&mut *self.0))
        }
    }
    let interner = Arc::new(Interner::new());
    let i = |s: &str| interner.intern(s);
    let module_node = ModuleNode::new(interner.clone(), i("module"), 123, Some(i("1.0")));
    let mut dst_module_node = ModuleNode::new(interner.clone(), i(""), 0, Some(i("")));

    module_node.accept(&mut CopyModuleVisitor(&mut dst_module_node));

    assert_eq!(dst_module_node.resolve(dst_module_node.name), "module");
    assert_eq!(dst_module_node.access, 123);
    assert_eq!(
        dst_module_node.resolve(dst_module_node.version.unwrap()),
        "1.0"
    );
}

/// oxasm-specific: every directive survives a visit and an accept.
#[test]
fn accept_replays_every_directive() {
    let interner = Arc::new(Interner::new());
    let mut module_node = ModuleNode::new(interner.clone(), interner.intern("m"), 1, None);
    let s = |s: &'static str| oxasm::jstr!(s);
    module_node.visit_main_class(s("p/Main"));
    module_node.visit_package(s("p"));
    module_node.visit_require(s("java.base"), 0x8000, Some(s("25")));
    module_node.visit_export(s("p"), 0, &[s("a"), s("b")]);
    module_node.visit_open(s("q"), 0, &[]);
    module_node.visit_use(s("p/S"));
    module_node.visit_provide(s("p/S"), &[s("p/Impl")]);
    let mut copy = ModuleNode::new(interner.clone(), oxasm::sym::EMPTY, 0, None);

    module_node.accept(&mut CopyModuleVisitor(&mut copy));

    assert_eq!(copy, module_node);

    struct CopyModuleVisitor<'a>(&'a mut ModuleNode);
    impl ClassVisitor for CopyModuleVisitor<'_> {
        fn visit_module<'a>(
            &'a mut self,
            name: &JavaStr,
            access: u32,
            version: Option<&JavaStr>,
        ) -> Option<Box<dyn ModuleVisitor + 'a>> {
            let interner = self.0.interner().clone();
            let name = interner.intern(name);
            let version = version.map(|v| interner.intern(v));
            *self.0 = ModuleNode::new(interner, name, access, version);
            Some(Box::new(&mut *self.0))
        }
    }
}
