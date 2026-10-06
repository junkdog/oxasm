use std::borrow::Cow;
use std::cmp::Ordering;

use proptest::prelude::*;

use super::*;

fn utf16(s: &JavaStr) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn from_str_is_free_and_round_trips() {
    let s = jstr!("java/lang/Object");
    assert_eq!(s.as_bytes(), b"java/lang/Object");
    assert_eq!(s.as_str(), Ok("java/lang/Object"));
    assert_eq!(s, "java/lang/Object");
    assert_eq!(s.to_owned(), "java/lang/Object");
}

#[test]
fn lone_surrogates_survive() {
    let s = JavaString::from_utf16(&[0x61, 0xd800, 0x62, 0xdfff]);
    assert_eq!(s.as_bytes(), b"a\xed\xa0\x80b\xed\xbf\xbf");
    assert_eq!(utf16(&s), [0x61, 0xd800, 0x62, 0xdfff]);
    assert!(s.as_str().is_err());
    assert_eq!(s.to_string_lossy(), "a\u{fffd}b\u{fffd}");
    assert_eq!(format!("{s}"), "a\u{fffd}b\u{fffd}");
    assert_eq!(format!("{s:?}"), r#""a\u{d800}b\u{dfff}""#);
    assert_eq!(s.len_utf16(), 4);
}

#[test]
fn surrogate_pairs_are_canonical() {
    // A valid pair is stored as the 4-byte UTF-8 form, so it equals the `str` literal.
    let s = JavaString::from_utf16(&[0xd83d, 0xdca3]);
    assert_eq!(s, "💣");
    assert_eq!(s.len_utf16(), 2);
    // The split form is rejected by validation...
    let split = b"\xed\xa0\xbd\xed\xb2\xa3";
    let err = JavaStr::from_semi_utf8(split).unwrap_err();
    assert_eq!((err.valid_up_to(), err.error_len()), (0, Some(6)));
    // ...but a low surrogate *before* a high one is two lone surrogates.
    assert!(JavaStr::from_semi_utf8(b"\xed\xb2\xa3\xed\xa0\xbd").is_ok());
    // Pushing a low surrogate after a trailing high one joins them.
    let mut joined = JavaString::new();
    joined.push_java(JavaCodePoint::from_u32(0xd83d).unwrap());
    assert_eq!(joined.as_bytes(), b"\xed\xa0\xbd");
    joined.push_java(JavaCodePoint::from_u32(0xdca3).unwrap());
    assert_eq!(joined, "💣");
    // So does concatenation.
    let high = JavaString::from_utf16(&[0x61, 0xd83d]);
    let low = JavaString::from_utf16(&[0xdca3, 0x62]);
    assert_eq!(high + &*low, "a💣b");
}

#[test]
fn modified_utf8_codec() {
    // NUL, a BMP character, a supplementary character (surrogate pair), a lone high surrogate.
    let mutf8: &[u8] = &[
        0x61, 0xc0, 0x80, 0xe2, 0x84, 0x9d, 0xed, 0xa0, 0xbd, 0xed, 0xb2, 0xa3, 0xed, 0xa0, 0x80,
    ];
    let decoded = JavaStr::from_modified_utf8(mutf8).unwrap();
    assert!(matches!(decoded, Cow::Owned(_)));
    assert_eq!(utf16(&decoded), [0x61, 0, 0x211d, 0xd83d, 0xdca3, 0xd800]);
    assert_eq!(&*decoded.to_modified_utf8(), mutf8);
    assert_eq!(decoded.modified_utf8_len(), mutf8.len());

    let plain = JavaStr::from_modified_utf8(b"java/lang/String").unwrap();
    assert!(matches!(plain, Cow::Borrowed(_)));
    assert!(matches!(plain.to_modified_utf8(), Cow::Borrowed(_)));

    for bad in [
        &b"\x00"[..],
        b"\xc0",
        b"\xc0\x81",
        b"\xed",
        b"\xf0\x9f\x92\xa3",
        b"\x80",
        b"\xe0\x80\x80",
    ] {
        assert!(JavaStr::from_modified_utf8(bad).is_err(), "{bad:x?}");
    }
}

#[test]
fn validation_rejects_malformed() {
    for bad in [
        &b"\xff"[..],
        b"\xc3",
        b"\xc3\x28",
        b"\xe0\x80\x80",
        b"\xf5\x80\x80\x80",
        b"\xf4\x90\x80\x80",
    ] {
        assert!(JavaStr::from_semi_utf8(bad).is_err(), "{bad:x?}");
    }
    let e = JavaStr::from_semi_utf8(b"ab\xe2\x84").unwrap_err();
    assert_eq!((e.valid_up_to(), e.error_len()), (2, None));
}

#[test]
fn searching_and_slicing() {
    let s = jstr!("java/util/Map$Entry");
    assert_eq!(s.find('/'), Some(4));
    assert_eq!(s.rfind('/'), Some(9));
    assert_eq!(s.find("Map"), Some(10));
    assert_eq!(s.find(jstr!("Entry")), Some(14));
    assert_eq!(s.find('x'), None);
    assert!(s.starts_with("java/") && s.ends_with('y') && s.contains('$'));
    assert_eq!(&s[10..13], "Map");
    assert_eq!(s.split_at(4), (jstr!("java"), jstr!("/util/Map$Entry")));
    assert_eq!(s.strip_prefix("java/"), Some(jstr!("util/Map$Entry")));
    assert_eq!(s.strip_suffix("$Entry"), Some(jstr!("java/util/Map")));
    assert_eq!(s.replace('/', '.'), "java.util.Map$Entry");
    assert_eq!(jstr!("aéa").replace('é', 'ü'), "aüa");
    assert_eq!(jstr!("é").get(0..1), None);
}

#[test]
#[should_panic(expected = "not on code point boundaries")]
fn slicing_inside_a_code_point_panics() {
    let _ = &jstr!("é")[..1];
}

#[test]
fn cmp_utf16_differs_from_bytewise() {
    let supplementary = jstr!("\u{10000}");
    let bmp_high = jstr!("\u{ffff}");
    assert_eq!(supplementary.cmp(bmp_high), Ordering::Greater);
    assert_eq!(supplementary.cmp_utf16(bmp_high), Ordering::Less);
}

#[test]
fn chars_both_ways() {
    let s = JavaString::from_utf16(&[0x61, 0xd800, 0xd83d, 0xdca3, 0xe9]);
    let fwd: Vec<u32> = s.chars().map(JavaCodePoint::as_u32).collect();
    assert_eq!(fwd, [0x61, 0xd800, 0x1f4a3, 0xe9]);
    let mut back: Vec<u32> = s.chars().rev().map(JavaCodePoint::as_u32).collect();
    back.reverse();
    assert_eq!(back, fwd);
    let idx: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    assert_eq!(idx, [0, 1, 4, 8]);
    let ridx: Vec<usize> = s.char_indices().rev().map(|(i, _)| i).collect();
    assert_eq!(ridx, [8, 4, 1, 0]);
}

#[test]
fn shared_and_boxed_forms() {
    let s = JavaString::from_utf16(&[0x41, 0xdc00]);
    let arc: std::sync::Arc<JavaStr> = std::sync::Arc::from(&*s);
    let rc: std::rc::Rc<JavaStr> = std::rc::Rc::from(&*s);
    let boxed = s.clone().into_boxed_java_str();
    assert_eq!(&*arc, &*s);
    assert_eq!(&*rc, &*s);
    assert_eq!(&*boxed.clone(), &*s);
    assert_eq!(JavaString::from(boxed), s);
}

#[test]
fn hash_agrees_between_owned_and_borrowed() {
    let mut set = std::collections::HashSet::new();
    set.insert(JavaString::from("a/B"));
    assert!(set.contains(jstr!("a/B")));
}

// -------------------------------------------------------------------------------------------------
// Properties against a `Vec<u16>` model (Java's `char[]`)
// -------------------------------------------------------------------------------------------------

/// UTF-16 sequences biased towards the interesting units: surrogates of both kinds, NUL, ASCII.
fn units() -> impl Strategy<Value = Vec<u16>> {
    prop::collection::vec(
        prop_oneof![
            0x0000u16..0x0080,
            Just(0u16),
            0xd800u16..0xdc00,
            0xdc00u16..0xe000,
            any::<u16>(),
        ],
        0..24,
    )
}

proptest! {
    #[test]
    fn utf16_round_trip(u in units()) {
        let s = JavaString::from_utf16(&u);
        prop_assert_eq!(utf16(&s), u.clone());
        prop_assert_eq!(s.len_utf16(), u.len());
        // The bytes are canonical, and re-validate.
        prop_assert!(JavaStr::from_semi_utf8(s.as_bytes()).is_ok());
        // Collecting units one by one gives the same string.
        prop_assert_eq!(u.iter().copied().collect::<JavaString>(), s);
    }

    #[test]
    fn modified_utf8_round_trip(u in units()) {
        let s = JavaString::from_utf16(&u);
        let encoded = s.to_modified_utf8();
        prop_assert!(!encoded.contains(&0));
        prop_assert!(!encoded.iter().any(|&b| b >= 0xf0));
        prop_assert_eq!(encoded.len(), s.modified_utf8_len());
        prop_assert_eq!(JavaString::from_modified_utf8(&encoded).unwrap(), s);
    }

    #[test]
    fn equality_and_order_follow_the_model(a in units(), b in units()) {
        let (sa, sb) = (JavaString::from_utf16(&a), JavaString::from_utf16(&b));
        prop_assert_eq!(sa == sb, a == b);
        prop_assert_eq!(sa.cmp_utf16(&sb), a.cmp(&b));
    }

    #[test]
    fn concatenation_follows_the_model(a in units(), b in units()) {
        let joined = JavaString::from_utf16(&a) + &*JavaString::from_utf16(&b);
        let model: Vec<u16> = a.iter().chain(&b).copied().collect();
        prop_assert_eq!(joined, JavaString::from_utf16(&model));
    }

    #[test]
    fn validation_agrees_with_std_on_utf8(bytes in prop::collection::vec(any::<u8>(), 0..16)) {
        // Without any 0xED byte there can be no surrogate, so semi-UTF-8 is exactly UTF-8.
        if !bytes.contains(&0xed) {
            prop_assert_eq!(JavaStr::from_semi_utf8(&bytes).is_ok(), std::str::from_utf8(&bytes).is_ok());
        }
    }
}
