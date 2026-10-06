//! Port of ASM's `TypePathTest` (asm/src/test/java/org/objectweb/asm/TypePathTest.java).

use oxasm::TypePath;

fn path(s: &str) -> TypePath {
    TypePath::from_string(s).unwrap().unwrap()
}

/// Tests that `get_length` returns correct values.
#[test]
fn test_get_length() {
    assert_eq!(5, path("[.[*0").get_length());
    assert_eq!(5, path("[*0;*[").get_length());
    assert_eq!(1, path("10;").get_length());
    assert_eq!(2, path("1;0;").get_length());
}

/// Tests that `get_step` returns correct values.
#[test]
fn test_get_step() {
    let type_path = path("[.[*7");
    assert_eq!(TypePath::ARRAY_ELEMENT, type_path.get_step(0));
    assert_eq!(TypePath::INNER_TYPE, type_path.get_step(1));
    assert_eq!(TypePath::WILDCARD_BOUND, type_path.get_step(3));
    assert_eq!(TypePath::TYPE_ARGUMENT, type_path.get_step(4));
    assert_eq!(7, type_path.get_step_argument(4));
}

/// Tests that type paths are unchanged via a from_string -> to_string transform.
#[test]
fn test_from_and_to_string() {
    // Java's `fromString(null)` has no Rust analogue; the empty string gives Java's null.
    assert_eq!(None, TypePath::from_string("").unwrap());
    assert_eq!("[.[*0;", path("[.[*0").to_string());
    assert_eq!("[*0;*[", path("[*0;*[").to_string());
    assert_eq!("10;", path("10;").to_string());
    assert_eq!("1;0;", path("1;0;").to_string());
    assert!(TypePath::from_string("-").is_err());
    assert!(TypePath::from_string("=").is_err());
    assert!(TypePath::from_string("1-").is_err());
}

// oxasm-specific: the structure round trip used by the reader and writer.
#[test]
fn structure_round_trip() {
    let p = path("[.[*7");
    assert_eq!(p.as_structure(), [5, 0, 0, 1, 0, 0, 0, 2, 0, 3, 7]);
    assert_eq!(TypePath::from_structure(p.as_structure()), Some(p.clone()));
    // Extra bytes after the structure are ignored; a truncated one is rejected.
    let mut longer = p.as_structure().to_vec();
    longer.push(0xff);
    assert_eq!(TypePath::from_structure(&longer), Some(p));
    assert_eq!(TypePath::from_structure(&[2, 0, 0]), None);
    assert_eq!(TypePath::from_structure(&[]), None);
}
