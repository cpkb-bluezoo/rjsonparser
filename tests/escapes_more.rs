//! Escape-sequence tests ported from the Java EscapeSequenceTest.

mod common;

use common::{parse_all, parse_string_value};

fn s(json: &str) -> String {
    parse_string_value(json)
}

fn err_msg(json: &str) -> String {
    parse_all(json.as_bytes()).err().expect("should fail").to_string().to_lowercase()
}

#[test]
fn single_escapes_in_context() {
    assert_eq!(s(r#""Hello\"World""#), "Hello\"World");
    assert_eq!(s(r#""Hello\\World""#), "Hello\\World");
    assert_eq!(s(r#""Hello\/World""#), "Hello/World");
    assert_eq!(s(r#""Hello\bWorld""#), "Hello\u{8}World");
    assert_eq!(s(r#""Hello\fWorld""#), "Hello\u{c}World");
    assert_eq!(s(r#""Hello\nWorld""#), "Hello\nWorld");
    assert_eq!(s(r#""Hello\rWorld""#), "Hello\rWorld");
    assert_eq!(s(r#""Hello\tWorld""#), "Hello\tWorld");
    assert_eq!(s(r#""\nstart""#), "\nstart");
    assert_eq!(s(r#""end\n""#), "end\n");
}

#[test]
fn consecutive_and_mixed_escapes() {
    assert_eq!(s(r#""\"\n\t""#), "\"\n\t");
    assert_eq!(s(r#""\n\n\n""#), "\n\n\n");
    assert_eq!(s(r#""\"a\\b\nc""#), "\"a\\b\nc");
    assert_eq!(s(r#""""#), "");
}

#[test]
fn unicode_escapes() {
    assert_eq!(s(r#""\u0041""#), "A");
    assert_eq!(s(r#""\u00e9\u00E9""#), "\u{e9}\u{e9}");
    assert_eq!(s(r#""x\u0041y\u0042z""#), "xAyBz");
    assert_eq!(s(r#""\u0000""#), "\u{0}");
    assert_eq!(s(r#""\u0001\u001F""#), "\u{1}\u{1f}");
    assert_eq!(s(r#""\uFFFF""#), "\u{ffff}");
    assert_eq!(s(r#""\u4e2d\u6587""#), "\u{4e2d}\u{6587}");
}

#[test]
fn hex_digits_use_correct_shifts() {
    // Regression for the Java bug (7632f4d): d3 << 0x10 etc. instead of 4/8/12.
    assert_eq!(s(r#""\u1234""#), "\u{1234}");
    assert_eq!(s(r#""\uABCD""#), "\u{abcd}");
    assert_eq!(s(r#""\u0100""#), "\u{100}");
    assert_eq!(s(r#""\u0010""#), "\u{10}");
}

#[test]
fn surrogate_pairs() {
    assert_eq!(s(r#""\uD83D\uDE00""#), "\u{1f600}");
    assert_eq!(s(r#""a\uD83D\uDE00b""#), "a\u{1f600}b");
    assert_eq!(s(r#""\uD83D\uDE00\uD834\uDD1E""#), "\u{1f600}\u{1d11e}");
}

#[test]
fn real_world_strings() {
    assert_eq!(s(r#""C:\\Users\\test\\file.txt""#), "C:\\Users\\test\\file.txt");
    assert_eq!(s(r#""{\"key\": \"value\"}""#), "{\"key\": \"value\"}");
    assert_eq!(s(r#""Line 1\nLine 2\nLine 3""#), "Line 1\nLine 2\nLine 3");
    assert_eq!(
        s(r#""https:\/\/example.com\/path""#),
        "https://example.com/path"
    );
}

#[test]
fn invalid_escapes_rejected_with_useful_messages() {
    assert!(err_msg(r#""\x""#).contains("escape"));
    assert!(err_msg(r#""\u00XY""#).contains("hex"));
    assert!(err_msg("\"\u{1}\"").contains("control"));
    assert!(parse_all(br#""\u004""#).is_err());
    assert!(parse_all(br#""Hello\""#).is_err());
}
