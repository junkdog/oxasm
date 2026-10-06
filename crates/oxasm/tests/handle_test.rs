//! Port of ASM's `HandleTest` (asm/src/test/java/org/objectweb/asm/HandleTest.java).
//!
//! Dropped: `testDeprecatedConstructor` (the deprecated 4-argument constructor, which derived
//! `isInterface` from the tag, is not ported).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use oxasm::{Handle, HandleKind};

fn hash(h: &Handle) -> u64 {
    let mut hasher = DefaultHasher::new();
    h.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn test_constructor() {
    let handle = Handle::new(HandleKind::GetField, "owner", "name", "descriptor", false);
    assert_eq!(HandleKind::GetField, handle.get_tag());
    assert_eq!("owner", handle.get_owner());
    assert_eq!("name", handle.get_name());
    assert_eq!("descriptor", handle.get_desc());
    assert!(!handle.is_interface());
    assert_eq!("owner.namedescriptor (1)", handle.to_string());
}

#[test]
fn test_equals() {
    let handle1 = Handle::new(HandleKind::GetField, "owner", "name", "descriptor", false);
    let handle2 = Handle::new(HandleKind::GetField, "owner", "name", "descriptor", false);
    // `equalsNull` has no Rust analogue.
    assert_eq!(handle1, handle1);
    assert_eq!(handle1, handle2);
    assert_ne!(
        handle1,
        Handle::new(HandleKind::PutField, "owner", "name", "descriptor", false)
    );
    assert_ne!(
        handle1,
        Handle::new(HandleKind::GetField, "o", "name", "descriptor", false)
    );
    assert_ne!(
        handle1,
        Handle::new(HandleKind::GetField, "owner", "n", "descriptor", false)
    );
    assert_ne!(
        handle1,
        Handle::new(HandleKind::GetField, "owner", "name", "d", false)
    );
    assert_ne!(
        handle1,
        Handle::new(HandleKind::GetField, "owner", "n", "descriptor", true)
    );
}

#[test]
fn test_hash_code() {
    let handle1 = Handle::new(
        HandleKind::InvokeStatic,
        "owner",
        "name",
        "descriptor",
        false,
    );
    let handle2 = Handle::new(
        HandleKind::InvokeStatic,
        "owner",
        "name",
        "descriptor",
        true,
    );
    // Equal handles hash equally; the interface flag takes part in the hash.
    assert_eq!(hash(&handle1), hash(&handle1.clone()));
    assert_ne!(hash(&handle1), hash(&handle2));
}

// oxasm-specific: the interface marker in `Display`, and the reference kinds.
#[test]
fn display_and_kinds() {
    let handle = Handle::new(
        HandleKind::InvokeInterface,
        "owner",
        "name",
        "descriptor",
        true,
    );
    assert_eq!("owner.namedescriptor (9 itf)", handle.to_string());
    for (i, kind) in HandleKind::ALL.iter().enumerate() {
        assert_eq!(Some(*kind), HandleKind::from_tag(kind.tag()));
        assert_eq!(usize::from(kind.tag()), i + 1);
        assert_eq!(kind.is_field(), i < 4);
    }
    assert_eq!(None, HandleKind::from_tag(0));
    assert_eq!(None, HandleKind::from_tag(10));
}
