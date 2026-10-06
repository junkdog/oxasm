//! `java_fmt` against `Float.toString`/`Double.toString` of Java 25 (oxasm-specific: ASM has no
//! such class). `data/java_fmt.txt` holds Java's output for edge cases (subnormals, powers of two
//! and ten and their neighbors, exact ties, `Double.MIN_VALUE`) and random bit patterns, as
//! `<D|F> <raw bits in hex> <Java's string>` lines.
//!
//! The module was also checked offline against Java on 10.5 million random and boundary values
//! and on every 61st float bit pattern (70 million values), without a mismatch.

use oxasm_util::java_fmt::{double_to_string, float_to_string};

#[test]
fn test_against_java() {
    let data = include_str!("data/java_fmt.txt");
    let mut checked = 0;
    for line in data.lines() {
        let mut fields = line.splitn(3, ' ');
        let (kind, bits, expected) = (
            fields.next().unwrap(),
            fields.next().unwrap(),
            fields.next().unwrap(),
        );
        let actual = match kind {
            "D" => double_to_string(f64::from_bits(u64::from_str_radix(bits, 16).unwrap())),
            "F" => float_to_string(f32::from_bits(u32::from_str_radix(bits, 16).unwrap())),
            _ => panic!("bad line: {line}"),
        };
        assert_eq!(actual, expected, "{kind} {bits}");
        checked += 1;
    }
    assert!(checked > 2000);
}

#[test]
fn test_special_values() {
    assert_eq!(double_to_string(f64::NAN), "NaN");
    assert_eq!(double_to_string(f64::INFINITY), "Infinity");
    assert_eq!(double_to_string(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(double_to_string(0.0), "0.0");
    assert_eq!(double_to_string(-0.0), "-0.0");
    assert_eq!(float_to_string(f32::NAN), "NaN");
    assert_eq!(float_to_string(-0.0), "-0.0");
}

#[test]
fn test_layout() {
    assert_eq!(double_to_string(1.0), "1.0");
    assert_eq!(double_to_string(100.0), "100.0");
    assert_eq!(double_to_string(1e7), "1.0E7");
    assert_eq!(double_to_string(9_999_999.0), "9999999.0");
    assert_eq!(double_to_string(0.001), "0.001");
    assert_eq!(double_to_string(0.0001), "1.0E-4");
    assert_eq!(double_to_string(-123.456), "-123.456");
    assert_eq!(double_to_string(1.5e300), "1.5E300");
    assert_eq!(float_to_string(3.4028235e38), "3.4028235E38");
}

#[test]
fn test_two_digit_rule() {
    // The shortest decimals have one digit (5E-324, 1E-45), but Java prints the closer two
    // digit decimal.
    assert_eq!(double_to_string(f64::from_bits(1)), "4.9E-324");
    assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
}

#[test]
fn test_ties_to_even() {
    // 2^-25 is exactly 2.98023223876953125E-8: Java picks the even 17-digit neighbor.
    assert_eq!(double_to_string(2f64.powi(-25)), "2.9802322387695312E-8");
    assert_eq!(float_to_string(2f32.powi(-12)), "2.4414062E-4");
}

/// oxasm-testkit's copy of this module (for the `ClassFile` dump) is byte-identical.
#[test]
fn testkit_copy_is_identical() {
    assert_eq!(
        include_str!("../src/java_fmt.rs"),
        include_str!("../../oxasm-testkit/src/java_fmt.rs"),
        "update crates/oxasm-testkit/src/java_fmt.rs from crates/oxasm-util/src/java_fmt.rs"
    );
}
