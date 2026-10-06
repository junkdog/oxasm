//! Port of ASM's `SignatureVisitorTest`
//! (asm/src/test/java/org/objectweb/asm/signature/SignatureVisitorTest.java).
//!
//! Dropped (ADR-09): `testConstructor_invalidApi`, the only test, checks the `api` argument,
//! which oxasm does not have. In its place, the default methods are checked to do what ASM's do.

use oxasm::jstr;
use oxasm::signature::{SignatureReader, SignatureVisitor};

/// Overrides only the leaf events; every nested visitor is `self` by default.
#[derive(Default)]
struct Leaves(Vec<String>);

impl SignatureVisitor for Leaves {
    fn visit_base_type(&mut self, descriptor: char) {
        self.0.push(descriptor.to_string());
    }
    fn visit_class_type(&mut self, name: &oxasm::JavaStr) {
        self.0.push(name.to_string());
    }
}

#[test]
fn defaults_return_self() {
    let mut leaves = Leaves::default();
    SignatureReader::new(jstr!("<T:Ljava/lang/Object;>(I[J)Ljava/util/List<-TT;>;"))
        .accept(&mut leaves)
        .unwrap();
    assert_eq!(leaves.0, ["java/lang/Object", "I", "J", "java/util/List"]);
}
