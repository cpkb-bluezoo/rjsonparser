//! Floating-point literals must round exactly like `str::parse::<f64>`
//! (ported from the Java NumberParsingTest, but strict: no tolerance).

mod common;

use common::parse_number_value;
use rjsonparser::Number;

fn assert_exact(s: &str) {
    let expected: f64 = s.parse().unwrap();
    match parse_number_value(s) {
        Number::F64(got) => assert_eq!(
            got.to_bits(),
            expected.to_bits(),
            "{s}: got {got:e}, expected {expected:e}"
        ),
        other => panic!("{s}: expected F64, got {other:?}"),
    }
}

#[test]
fn decimals_are_exact() {
    for s in [
        "3.141592653589793",
        "2.718281828459045",
        "1.7976931348623157",
        "123456.789",
        "0.1",
        "0.30000000000000004",
        "-0.0",
        "0.0",
    ] {
        assert_exact(s);
    }
    assert!(parse_number_value("-0.0").as_f64().unwrap().is_sign_negative());
}

#[test]
fn exponents_are_exact() {
    for s in [
        "1e10", "1.5e-5", "6.022e23", "-2.5E+10", "1E0", "5e-1", "1e22", "1e23", "9e22",
        "1e300", "1e-300", "1.7e308", "4.9e-324", "2.2250738585072014e-308", "8.41e21",
        "1e999999", "1e-999999",
    ] {
        assert_exact(s);
    }
}

#[test]
fn long_digit_strings_are_exact() {
    assert_exact("0.12345678901234567890123456789");
    assert_exact("1.123456789012345678901234567890123456789e10");
}

#[test]
fn random_decimals_are_exact() {
    // Small deterministic xorshift so the suite needs no dev-dependency.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..5000 {
        let int_part = next() % 1_000_000;
        let frac_digits = 1 + (next() % 15) as usize;
        let frac = format!("{:0width$}", next() % 10u64.pow(frac_digits as u32), width = frac_digits);
        let s = if next() % 2 == 0 {
            format!("{int_part}.{frac}")
        } else {
            format!("{int_part}.{frac}e{}", (next() % 40) as i64 - 20)
        };
        assert_exact(&s);
    }
}
