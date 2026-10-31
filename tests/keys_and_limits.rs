mod common;

use rjsonparser::ParserLimits;
use loopbuf::LoopBuf;

use common::{parse_all, RecordingHandler};
use rjsonparser::{ContentHandler, Parser};

/// Feeds `data` to `parser` through a freshly allocated ring sized to fit
/// it all, returning that ring so the caller can pass the same one to
/// `close()` afterward if needed.
fn feed<H: ContentHandler>(parser: &mut Parser<H>, data: &[u8]) -> (rjsonparser::ParseResult<()>, LoopBuf) {
    let mut buf = LoopBuf::with_capacity(data.len().max(1));
    buf.extend_from_slice(data);
    let result = parser.receive(&mut buf);
    (result, buf)
}

#[test]
fn duplicate_keys_allowed_by_default() {
    let h = parse_all(br#"{"a":1,"a":2}"#).unwrap();
    assert_eq!(h.keys, vec!["a", "a"]);
    assert_eq!(h.numbers.len(), 2);
}

#[test]
fn duplicate_keys_rejected_when_enabled() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_limits(ParserLimits::unlimited());
    parser.set_reject_duplicate_keys(true);
    let (result, _buf) = feed(&mut parser, br#"{"a":1,"a":2}"#);
    let err = result.unwrap_err();
    assert!(err.message().contains("Duplicate key"));
}

#[test]
fn duplicate_keys_rejected_when_one_key_uses_an_escape() {
    // "a" and "\u0061" both decode to the same key -- the duplicate
    // check must still catch this on the escape-decoding path, not only
    // the plain/interning fast path.
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_limits(ParserLimits::unlimited());
    parser.set_reject_duplicate_keys(true);
    let (result, _buf) = feed(&mut parser, br#"{"a":1,"\u0061":2}"#);
    let err = result.unwrap_err();
    assert!(err.message().contains("Duplicate key"));
}

#[test]
fn duplicate_keys_scoped_per_object() {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        parser.set_reject_duplicate_keys(true);
        let (result, mut buf) = feed(&mut parser, br#"{"a":{"a":1},"b":2}"#);
        result.unwrap();
        parser.close(&mut buf).unwrap();
    }
    assert_eq!(handler.keys, vec!["a", "a", "b"]);
}

#[test]
fn key_interning_repeated_keys() {
    // Many objects with the same key — should parse correctly (interning is opaque).
    let mut json = String::from("[");
    for i in 0..100 {
        if i > 0 {
            json.push(',');
        }
        json.push_str(r#"{"name":"x"}"#);
    }
    json.push(']');
    let h = parse_all(json.as_bytes()).unwrap();
    assert_eq!(h.keys.len(), 100);
    assert!(h.keys.iter().all(|k| k == "name"));
}

#[test]
fn limits_nesting_depth() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.limits_mut().max_nesting_depth = Some(2);
    let (result, _buf) = feed(&mut parser, b"[[[1]]]");
    assert!(result.is_err());
}

#[test]
fn limits_string_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.limits_mut().max_string_length = Some(5);
    let (result, _buf) = feed(&mut parser, br#""abcdef""#);
    assert!(result.is_err());
}

#[test]
fn limits_number_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.limits_mut().max_number_length = Some(3);
    // a number is read a byte at a time, so it is rejected as soon as it is too long
    let (result, _buf) = feed(&mut parser, b"1234");
    assert!(result.is_err());
}

#[test]
fn limits_document_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.limits_mut().max_document_length = Some(3);
    let (result, _buf) = feed(&mut parser, b"1234");
    assert!(result.is_err());
}

#[test]
fn limits_token_count() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.limits_mut().max_token_count = Some(2);
    let (result, _buf) = feed(&mut parser, b"[1,2,3]");
    assert!(result.is_err());
}

#[test]
fn disable_all_limits_allows_deep() {
    let nested = "[".repeat(50) + "1" + &"]".repeat(50);
    let h = parse_all(nested.as_bytes()).unwrap();
    assert_eq!(h.numbers[0].as_i32(), Some(1));
}

#[test]
fn reset_reuses_parser() {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        let (result, mut buf1) = feed(&mut parser, b"1");
        result.unwrap();
        parser.close(&mut buf1).unwrap();
        parser.reset();
        let (result, mut buf2) = feed(&mut parser, b"2");
        result.unwrap();
        parser.close(&mut buf2).unwrap();
    }
    assert_eq!(handler.numbers.len(), 2);
    assert_eq!(handler.numbers[0].as_i32(), Some(1));
    assert_eq!(handler.numbers[1].as_i32(), Some(2));
}

struct CountingHandler {
    keys: Vec<String>,
}
impl ContentHandler for CountingHandler {
    fn key(&mut self, key: &str) -> rjsonparser::ParseResult<()> {
        self.keys.push(key.to_owned());
        Ok(())
    }
}

#[test]
fn parse_from_read() {
    let mut handler = CountingHandler { keys: Vec::new() };
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        parser.parse(&br#"{"hello":"world"}"#[..]).unwrap();
    }
    assert_eq!(handler.keys, vec!["hello"]);
}

#[test]
fn many_distinct_keys_exceeding_intern_cap_still_parse() {
    // Well past KeySymbolTable's MAX_ENTRIES: interning degrades to a no-op,
    // but every key must still be delivered correctly.
    let n = 15_000;
    let body: Vec<String> = (0..n).map(|i| format!("\"key{i}\":{i}")).collect();
    let json = format!("{{{}}}", body.join(","));
    let h = parse_all(json.as_bytes()).unwrap();
    assert_eq!(h.keys.len(), n);
    assert_eq!(h.keys[0], "key0");
    assert_eq!(h.keys[n - 1], format!("key{}", n - 1));
}

#[test]
fn similar_keys_are_not_conflated() {
    let h = parse_all(br#"{"ab":1,"ac":2,"bb":3}"#).unwrap();
    assert_eq!(h.keys, ["ab", "ac", "bb"]);
}

#[test]
fn interning_table_stays_correct_across_reset() {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        for doc in [&br#"{"widget":1}"#[..], &br#"{"widget":2,"gadget":3}"#[..]] {
            let (result, mut buf) = feed(&mut parser, doc);
            result.unwrap();
            parser.close(&mut buf).unwrap();
            parser.reset();
        }
    }
    assert_eq!(handler.keys, ["widget", "widget", "gadget"]);
}

#[test]
fn duplicate_check_pops_after_nested_object_closes() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_reject_duplicate_keys(true);
    // "a" inside the nested object is not a duplicate of the outer "a",
    // but the second outer "a" after it is.
    let (result, mut buf) = feed(&mut parser, br#"{"a":{"a":1},"b":2}"#);
    result.unwrap();
    parser.close(&mut buf).unwrap();
    parser.reset();
    let (result, _buf) = feed(&mut parser, br#"{"a":{"a":1},"a":2}"#);
    assert!(result.is_err());
}
