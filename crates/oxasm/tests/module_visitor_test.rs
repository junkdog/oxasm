//! Port of ASM's `ModuleVisitorTest`.

use std::ptr;

use oxasm::{ModuleVisitor, jstr};

/// A module visitor with no delegate.
struct NoDelegate;

impl ModuleVisitor for NoDelegate {}

/// A module visitor that delegates to another.
struct Delegating<'a>(&'a mut dyn ModuleVisitor);

impl ModuleVisitor for Delegating<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        Some(&mut *self.0)
    }
}

#[test]
fn test_constructor_valid_api() {
    // A visitor without a delegate accepts every event, and is not interested in any content.
    let mut visitor = NoDelegate;
    assert!(visitor.delegate().is_none());
    visitor.visit_main_class(jstr!("C"));
    visitor.visit_package(jstr!("p"));
    visitor.visit_end();
}

// testConstructor_invalidApi: dropped (ADR-09, there are no api versions).

#[test]
fn test_get_delegate() {
    let mut delegate = NoDelegate;
    let delegate_address = ptr::from_mut(&mut delegate).cast::<()>();
    let mut visitor = Delegating(&mut delegate);
    let address = visitor
        .delegate()
        .map(|d| ptr::from_mut(d).cast::<()>())
        .expect("a delegate");
    assert_eq!(address, delegate_address);
}
