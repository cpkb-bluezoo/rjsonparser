mod common;

use common::{parse_all, parse_number_value};
use rjsonparser::{Number, Parser};

#[test]
fn plain_integers() {
    assert_eq!(parse_number_value("0"), Number::I32(0));
    assert_eq!(parse_number_value("42"), Number::I32(42));
    assert_eq!(parse_number_value("-42"), Number::I32(-42));
    assert_eq!(
        parse_number_value(&i32::MAX.to_string()),
        Number::I32(i32::MAX)
    );
    let just_over = (i32::MAX as i64) + 1;
    assert_eq!(
        parse_number_value(&just_over.to_string()),
        Number::I64(just_over)
    );
    assert_eq!(
        parse_number_value(&i64::MAX.to_string()),
        Number::I64(i64::MAX)
    );
}

#[test]
fn huge_integer_bigint() {
    let big = "123456789012345678901234567890";
    match parse_number_value(big) {
        Number::BigInt(s) => assert_eq!(s, big),
        other => panic!("expected BigInt, got {other:?}"),
    }
}

#[test]
fn simple_decimals() {
    for s in ["3.14", "0.5", "-42.0", "0.1", "-0.001"] {
        let n = parse_number_value(s);
        let expected: f64 = s.parse().unwrap();
        let got = n.as_f64().unwrap();
        let tol = (expected.ulp() * 2.0).max(0.0);
        assert!((got - expected).abs() <= tol, "{s}: {got} vs {expected}");
    }
}

#[test]
fn exponents() {
    for s in ["1e10", "1.5e-5", "6.022e23", "-2.5E+10", "1E0"] {
        let n = parse_number_value(s);
        let expected: f64 = s.parse().unwrap();
        let got = n.as_f64().unwrap();
        if expected.is_infinite() || expected == 0.0 {
            assert_eq!(got, expected, "{s}");
        } else {
            let rel = ((got - expected) / expected).abs();
            assert!(rel < 1e-9, "{s}: {got} vs {expected} rel={rel}");
        }
    }
}

#[test]
fn leading_zeros_rejected() {
    assert!(parse_all(b"01").is_err());
    assert!(parse_all(b"-01").is_err());
}

#[test]
fn incomplete_number_forms_rejected() {
    assert!(parse_all(b"-").is_err());
    assert!(parse_all(b"1.").is_err());
    assert!(parse_all(b"1e").is_err());
    assert!(parse_all(b"1e+").is_err());
}

#[test]
fn bom_utf8_skipped() {
    let mut data = vec![0xEF, 0xBB, 0xBF];
    data.extend_from_slice(br#"{"a":1}"#);
    let h = parse_all(&data).unwrap();
    assert_eq!(h.keys, vec!["a"]);
    assert_eq!(h.numbers[0].as_i32(), Some(1));
}

#[test]
fn bom_utf8_split() {
    let mut handler = common::RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.disable_all_limits();
        let mut a = &[0xEFu8, 0xBB][..];
        parser.receive(&mut a).unwrap();
        let mut b = &[0xBFu8, b'[', b'4', b'2', b']'][..];
        parser.receive(&mut b).unwrap();
        parser.close().unwrap();
    }
    assert_eq!(handler.numbers[0].as_i32(), Some(42));
}

#[test]
fn bom_utf16_be_rejected() {
    let data = [0xFEu8, 0xFF, b'{', b'}'];
    assert!(parse_all(&data).is_err());
}

#[test]
fn bom_utf16_le_rejected() {
    let data = [0xFFu8, 0xFE, 0x00, 0x00];
    // May need 4 bytes; FF FE with more non-zero is UTF-16 LE
    let data2 = [0xFFu8, 0xFE, 0x7B, 0x00];
    assert!(parse_all(&data2).is_err());
    let _ = data;
}

trait Ulp {
    fn ulp(self) -> Self;
}
impl Ulp for f64 {
    fn ulp(self) -> Self {
        if self.is_nan() || self.is_infinite() {
            return f64::NAN;
        }
        let bits = self.to_bits();
        let next = f64::from_bits(bits + 1);
        (next - self).abs()
    }
}
