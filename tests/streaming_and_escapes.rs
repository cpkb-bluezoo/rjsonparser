mod common;

use common::{parse_all, parse_chunked, parse_string_value, RecordingHandler};
use rjsonparser::{ContentHandler, DefaultHandler, ParseResult, Parser};

#[test]
fn escape_basic() {
    assert_eq!(parse_string_value(r#""Hello\"World""#), "Hello\"World");
    assert_eq!(parse_string_value(r#""Hello\\World""#), "Hello\\World");
    assert_eq!(parse_string_value(r#""Hello\/World""#), "Hello/World");
    assert_eq!(parse_string_value(r#""Hello\bWorld""#), "Hello\u{0008}World");
    assert_eq!(parse_string_value(r#""Hello\fWorld""#), "Hello\u{000c}World");
    assert_eq!(parse_string_value(r#""Hello\nWorld""#), "Hello\nWorld");
    assert_eq!(parse_string_value(r#""Hello\rWorld""#), "Hello\rWorld");
    assert_eq!(parse_string_value(r#""Hello\tWorld""#), "Hello\tWorld");
}

#[test]
fn escape_unicode() {
    assert_eq!(parse_string_value(r#""\u0041""#), "A");
    assert_eq!(parse_string_value(r#""\u00e9""#), "é");
    assert_eq!(parse_string_value(r#""\u20ac""#), "€");
    assert_eq!(parse_string_value(r#""\u4e2d""#), "中");
    assert_eq!(parse_string_value(r#""\u0041\u0042\u0043""#), "ABC");
}

#[test]
fn escape_surrogate_pair() {
    // Musical G clef U+1D11E
    assert_eq!(
        parse_string_value(r#""\uD834\uDD1E""#),
        "\u{1D11E}"
    );
}

#[test]
fn escape_all_basic() {
    assert_eq!(
        parse_string_value(r#""\"\\\/\b\f\n\r\t""#),
        "\"\\/\u{0008}\u{000c}\n\r\t"
    );
}

#[test]
fn reject_invalid_escape() {
    assert!(parse_all(br#""\x""#).is_err());
    assert!(parse_all(br#""\u12""#).is_err());
    assert!(parse_all(b"\"\n\"").is_err()); // unescaped control
}

#[test]
fn reject_unclosed_string() {
    assert!(parse_all(br#""hello"#).is_err());
}

#[test]
fn streaming_byte_by_byte() {
    let input = br#"{"a":[true,false,null],"b":"hi"}"#;
    let h = parse_chunked(input, 1).expect("chunked parse");
    assert!(h.keys.contains(&"a".into()));
    assert!(h.keys.contains(&"b".into()));
    assert_eq!(h.booleans, vec![true, false]);
    assert_eq!(h.nulls, 1);
    assert_eq!(h.strings, vec!["hi"]);
}

#[test]
fn streaming_split_string_escape() {
    let input = br#""Hello\nWorld""#;
    for size in 1..=input.len() {
        let h = parse_chunked(input, size).unwrap_or_else(|e| panic!("size {size}: {e}"));
        assert_eq!(h.strings, vec!["Hello\nWorld"], "size {size}");
    }
}

#[test]
fn streaming_split_number() {
    let input = b"12345";
    for size in 1..=input.len() {
        let h = parse_chunked(input, size).unwrap_or_else(|e| panic!("size {size}: {e}"));
        assert_eq!(h.numbers[0].as_i32(), Some(12345), "size {size}");
    }
}

#[test]
fn trailing_comma_rejected() {
    assert!(parse_all(b"[1,]").is_err());
    assert!(parse_all(br#"{"a":1,}"#).is_err());
}

#[test]
fn empty_object_and_array() {
    let h = parse_all(b"{}").unwrap();
    assert_eq!(h.events, vec!["startObject", "endObject"]);
    let h = parse_all(b"[]").unwrap();
    assert_eq!(h.events, vec!["startArray", "endArray"]);
}

#[test]
fn whitespace_opt_in() {
    let mut handler = RecordingHandler::with_whitespace();
    {
        let mut parser = Parser::new(&mut handler);
        parser.disable_all_limits();
        let mut data = &b"  1  "[..];
        parser.receive(&mut data).unwrap();
        parser.close().unwrap();
    }
    assert!(handler.events.iter().any(|e| e.starts_with("ws:")));
    assert_eq!(handler.numbers[0].as_i32(), Some(1));
}

#[test]
fn default_handler_ok() {
    let mut handler = DefaultHandler;
    let mut parser = Parser::new(&mut handler);
    parser.disable_all_limits();
    let mut data = &br#"{"x":1}"#[..];
    parser.receive(&mut data).unwrap();
    parser.close().unwrap();
}

struct AbortHandler;
impl ContentHandler for AbortHandler {
    fn start_object(&mut self) -> ParseResult<()> {
        Err(rjsonparser::ParseError::new("abort"))
    }
}

#[test]
fn handler_error_propagates() {
    let mut handler = AbortHandler;
    let mut parser = Parser::new(&mut handler);
    let mut data = &b"{"[..];
    let err = parser.receive(&mut data).unwrap_err();
    assert_eq!(err.message(), "abort");
}
