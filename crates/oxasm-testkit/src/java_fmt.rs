//! Java's `Float.toString` and `Double.toString` (JDK 19 and later), with which the Textifier and
//! the `ClassFile` dump of oxasm-testkit print floating point constants (oxasm-testkit has a
//! byte-identical copy of this file, since it cannot depend on the oxasm crates).
//!
//! Since JDK 19 (Raffaello Giulietti's algorithm), Java renders the decimal `d` that rounds to the
//! value, has the fewest digits and, among those, is closest to the value (ties to an even last
//! digit). One Java specific rule: when the shortest such decimal has a single digit, the two
//! digit decimals are candidates too, so `Double.MIN_VALUE` is `4.9E-324` (not `5.0E-324`).
//! The layout is plain notation for decimals in `[10^-3, 10^7)` and "computerized scientific
//! notation" (`1.0E10`) otherwise, always with at least one fractional digit.
//!
//! Rust's own shortest formatting (`{:e}`) selects the same shortest, closest decimal (it uses
//! the same round-to-nearest-even interval), so this module takes its digits, applies the two
//! digit rule, and lays them out the Java way.

use std::fmt::LowerExp;
use std::str::FromStr;

/// Java's `Double.toString(value)`.
#[must_use]
pub fn double_to_string(value: f64) -> String {
    to_java_string(value, value.is_nan(), value.is_infinite(), value == 0.0)
}

/// Java's `Float.toString(value)`.
#[must_use]
pub fn float_to_string(value: f32) -> String {
    to_java_string(value, value.is_nan(), value.is_infinite(), value == 0.0)
}

fn to_java_string<F>(value: F, is_nan: bool, is_infinite: bool, is_zero: bool) -> String
where
    F: LowerExp + FromStr + PartialEq + Copy + std::ops::Neg<Output = F> + PartialOrd + Default,
{
    if is_nan {
        return "NaN".to_owned();
    }
    let negative = value < F::default() || (is_zero && format!("{value:e}").starts_with('-'));
    let abs = if negative { -value } else { value };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if is_infinite {
        out.push_str("Infinity");
        return out;
    }
    if is_zero {
        out.push_str("0.0");
        return out;
    }
    let (digits, exponent) = java_digits(abs);
    layout(&mut out, &digits, exponent);
    out
}

/// The decimal Java renders for the finite, positive `value`: its digits (without trailing
/// zeros, except for a single `0`) and the exponent `e` of its first digit, i.e. the value is
/// `d.ddd × 10^e`.
fn java_digits<F>(value: F) -> (Vec<u8>, i32)
where
    F: LowerExp + FromStr + PartialEq + Copy,
{
    let (digits, exponent) = split_exp(&format!("{value:e}"));
    if digits.len() != 1 {
        return tie_to_even(value, digits, exponent);
    }
    // The shortest decimal has one digit: the closest two digit decimal wins if it also rounds to
    // the value (it is at least as close). Rust's `{:.1e}` rounds the exact value correctly.
    let (closest, closest_exponent) = split_exp(&format!("{value:.1e}"));
    let as_value = |digits: &[u8], exponent: i32| -> Option<F> {
        let mut s = String::new();
        for (i, d) in digits.iter().enumerate() {
            if i == 1 {
                s.push('.');
            }
            s.push(char::from(b'0' + d));
        }
        s.push('e');
        s.push_str(&exponent.to_string());
        s.parse::<F>().ok()
    };
    let trimmed = |mut digits: Vec<u8>| {
        while digits.len() > 1 && digits.last() == Some(&0) {
            digits.pop();
        }
        digits
    };
    if as_value(&closest, closest_exponent) == Some(value) {
        return (trimmed(closest), closest_exponent);
    }
    // The closest two digit decimal is on the other side of the value than the one digit
    // decimal, and outside of the rounding interval: its neighbor towards the one digit decimal
    // is between the value and that decimal, so it rounds to the value, and is at least as close.
    let shortest = u32::from(digits[0]) * pow10(exponent - closest_exponent + 1);
    let candidate = u32::from(closest[0]) * 10 + u32::from(closest.get(1).copied().unwrap_or(0));
    let neighbor = if shortest > candidate {
        candidate + 1
    } else {
        candidate - 1
    };
    let (neighbor_digits, neighbor_exponent) = match neighbor {
        100 => (vec![1, 0], closest_exponent + 1),
        9 => (vec![9, 9], closest_exponent - 1),
        n => (vec![digit(n / 10), digit(n % 10)], closest_exponent),
    };
    if as_value(&neighbor_digits, neighbor_exponent) == Some(value) {
        (trimmed(neighbor_digits), neighbor_exponent)
    } else {
        (digits, exponent)
    }
}

/// Rust breaks an exact tie between the two closest shortest decimals upwards, Java towards an
/// even last digit: when the value is exactly halfway between `digits` and its neighbor, picks
/// the even one (if it also rounds to the value).
fn tie_to_even<F>(value: F, digits: Vec<u8>, exponent: i32) -> (Vec<u8>, i32)
where
    F: LowerExp + FromStr + PartialEq + Copy,
{
    let n = digits.len();
    if digits[n - 1].is_multiple_of(2) {
        return (digits, exponent);
    }
    // The exact decimal expansion of a double has at most 767 significant digits.
    let (exact, exact_exponent) = split_exp(&format!("{value:.800e}"));
    let is_tie = exact_exponent == exponent
        && exact.len() > n
        && exact[n] == 5
        && exact[n + 1..].iter().all(|&d| d == 0);
    if !is_tie {
        return (digits, exponent);
    }
    let lower = exact[..n].to_vec();
    let alternative = if lower == digits {
        // The upper neighbor, with carry; an all-nines lower cannot be odd-tied here.
        let mut upper = lower;
        let mut i = n;
        while i > 0 {
            i -= 1;
            if upper[i] == 9 {
                upper[i] = 0;
            } else {
                upper[i] += 1;
                break;
            }
        }
        if upper[0] == 0 {
            return (digits, exponent);
        }
        upper
    } else {
        lower
    };
    if !alternative[n - 1].is_multiple_of(2) {
        return (digits, exponent);
    }
    let mut s = String::new();
    for (i, d) in alternative.iter().enumerate() {
        if i == 1 {
            s.push('.');
        }
        s.push(char::from(b'0' + d));
    }
    s.push('e');
    s.push_str(&exponent.to_string());
    if s.parse::<F>().ok() == Some(value) {
        let mut alternative = alternative;
        while alternative.len() > 1 && alternative.last() == Some(&0) {
            alternative.pop();
        }
        (alternative, exponent)
    } else {
        (digits, exponent)
    }
}

/// The decimal digit `d` (less than 10).
fn digit(d: u32) -> u8 {
    u8::try_from(d).expect("a decimal digit")
}

fn pow10(exponent: i32) -> u32 {
    match exponent {
        0 => 1,
        1 => 10,
        2 => 100,
        _ => 0,
    }
}

/// Splits Rust's `d.ddde±x` notation into digits and exponent.
fn split_exp(s: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = s.split_once('e').expect("exponent notation");
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exponent.parse().expect("exponent"))
}

/// Lays out `d.ddd × 10^exponent` as Java does.
fn layout(out: &mut String, digits: &[u8], exponent: i32) {
    let digit = |d: u8| char::from(b'0' + d);
    if (0..7).contains(&exponent) {
        // Plain notation, with an integer part of exponent + 1 digits.
        let integer_digits = exponent as usize + 1;
        for i in 0..integer_digits {
            out.push(digits.get(i).map_or('0', |&d| digit(d)));
        }
        out.push('.');
        if digits.len() > integer_digits {
            out.extend(digits[integer_digits..].iter().map(|&d| digit(d)));
        } else {
            out.push('0');
        }
    } else if (-3..0).contains(&exponent) {
        out.push_str("0.");
        for _ in 0..(-exponent - 1) {
            out.push('0');
        }
        out.extend(digits.iter().map(|&d| digit(d)));
    } else {
        out.push(digit(digits[0]));
        out.push('.');
        if digits.len() > 1 {
            out.extend(digits[1..].iter().map(|&d| digit(d)));
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exponent.to_string());
    }
}
