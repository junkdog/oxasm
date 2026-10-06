//! Port of ASM's `ModuleNodeTest`. `testConstructor_illegalState` is dropped: there is no
//! subclassing, nor `api` (ADR-09).

use oxasm::{ClassVisitor, JavaStr, ModuleVisitor};
use oxasm_tree::ModuleNode;

#[test]
fn test_constructor() {
    let module_node1 = ModuleNode::new("module1", 123, Some("1.0".into()));
    // ASM's constructor with lists (here, null ones) is a struct literal.
    let module_node2 = ModuleNode {
        name: "module2".into(),
        access: 456,
        version: Some("2.0".into()),
        ..ModuleNode::default()
    };

    assert_eq!(module_node1.name, "module1");
    assert_eq!(module_node1.access, 123);
    assert_eq!(module_node1.version.as_deref().unwrap(), "1.0");
    assert_eq!(module_node2.name, "module2");
    assert_eq!(module_node2.access, 456);
    assert_eq!(module_node2.version.as_deref().unwrap(), "2.0");
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
            self.0.name = name.to_owned();
            self.0.access = access;
            self.0.version = version.map(JavaStr::to_owned);
            Some(Box::new(&mut *self.0))
        }
    }
    let module_node = ModuleNode::new("module", 123, Some("1.0".into()));
    let mut dst_module_node = ModuleNode::new("", 0, Some("".into()));

    module_node.accept(&mut CopyModuleVisitor(&mut dst_module_node));

    assert_eq!(dst_module_node.name, "module");
    assert_eq!(dst_module_node.access, 123);
    assert_eq!(dst_module_node.version.as_deref().unwrap(), "1.0");
}

/// oxasm-specific: every directive survives a visit and an accept.
#[test]
fn accept_replays_every_directive() {
    let mut module_node = ModuleNode::new("m", 1, None);
    let s = |s: &'static str| oxasm::jstr!(s);
    module_node.visit_main_class(s("p/Main"));
    module_node.visit_package(s("p"));
    module_node.visit_require(s("java.base"), 0x8000, Some(s("25")));
    module_node.visit_export(s("p"), 0, &[s("a"), s("b")]);
    module_node.visit_open(s("q"), 0, &[]);
    module_node.visit_use(s("p/S"));
    module_node.visit_provide(s("p/S"), &[s("p/Impl")]);
    let mut copy = ModuleNode::default();

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
            *self.0 = ModuleNode::new(name, access, version.map(JavaStr::to_owned));
            Some(Box::new(&mut *self.0))
        }
    }
}
