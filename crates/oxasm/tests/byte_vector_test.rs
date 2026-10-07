//! Port of ASM's `ByteVectorTest`. `put11`, `put12`, `put112` and `put122` are crate-private
//! (package-private in ASM): their tests are unit tests of `writer::byte_vector`.

use oxasm::JavaString;
use oxasm::error::WriteError;
use oxasm::writer::ByteVector;

#[test]
fn test_put_byte() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_byte(1);
    assert_eq!(byte_vector.data(), [1]);
    assert_eq!(byte_vector.size(), 1);
}

// testPut11: a unit test (crate-private method).

#[test]
fn test_put_short() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_short(0x0102);
    assert_eq!(byte_vector.data(), [1, 2]);
    assert_eq!(byte_vector.size(), 2);
}

// testPut12 and testPut112: unit tests (crate-private methods).

#[test]
fn test_put_int() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_int(0x0102_0304);
    assert_eq!(byte_vector.data(), [1, 2, 3, 4]);
    assert_eq!(byte_vector.size(), 4);
}

// testPut122: a unit test (crate-private method).

#[test]
fn test_put_long() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_long(0x0102_0304_0506_0708);
    assert_eq!(byte_vector.data(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(byte_vector.size(), 8);
}

#[test]
fn test_put_utf8_ascii() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_utf8("abc".into()).unwrap();
    assert_eq!(byte_vector.data(), [0, 3, b'a', b'b', b'c']);
    assert_eq!(byte_vector.size(), 5);
}

#[test]
fn test_put_utf8_ascii_too_large() {
    for size in [65535, 65536] {
        let mut byte_vector = ByteVector::with_capacity(0);
        let string = JavaString::from("A".repeat(size));
        let result = byte_vector.put_utf8(&string).map(|_| ());
        if size > 65535 {
            let error = result.unwrap_err();
            assert_eq!(error, WriteError::Utf8TooLarge);
            assert_eq!(error.to_string(), "UTF8 string too large");
        } else {
            assert_eq!(result, Ok(()));
        }
    }
}

#[test]
fn test_put_utf8_unicode() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector
        .put_utf8(&JavaString::from_utf16(&[
            u16::from(b'a'),
            0x0000,
            0x0080,
            0x0800,
        ]))
        .unwrap();
    assert_eq!(
        byte_vector.data(),
        [0, 8, b'a', 0xC0, 0x80, 0xC2, 0x80, 0xE0, 0xA0, 0x80]
    );
}

#[test]
fn test_put_utf8_unicode_too_large() {
    let mut byte_vector = ByteVector::with_capacity(0);
    let string = JavaString::from_utf16(&[0x07FF; 32768]);
    let error = byte_vector.put_utf8(&string).map(|_| ()).unwrap_err();
    assert_eq!(error.to_string(), "UTF8 string too large");
}

#[test]
fn test_put_byte_array() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_byte_array(&[0, 1, 2, 3, 4, 5][1..4]);
    assert_eq!(byte_vector.data(), [1, 2, 3]);
    assert_eq!(byte_vector.size(), 3);
}

/// oxasm: a supplementary character is a surrogate pair in modified UTF-8 (6 bytes), and the
/// 65535 bytes limit counts them.
#[test]
fn test_put_utf8_supplementary() {
    let mut byte_vector = ByteVector::with_capacity(0);
    byte_vector.put_utf8("\u{1F600}".into()).unwrap();
    assert_eq!(
        byte_vector.data(),
        [0, 6, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]
    );
    let string = JavaString::from("\u{1F600}".repeat(10923));
    assert_eq!(
        ByteVector::new().put_utf8(&string).map(|_| ()),
        Err(WriteError::Utf8TooLarge)
    );
}
