mod common;

use common::{parse_all, RecordingHandler};
use rjsonparser::{ContentHandler, Parser};

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
    parser.disable_all_limits();
    parser.set_reject_duplicate_keys(true);
    let mut data = &br#"{"a":1,"a":2}"#[..];
    let err = parser.receive(&mut data).unwrap_err();
    assert!(err.message().contains("Duplicate key"));
}

#[test]
fn duplicate_keys_scoped_per_object() {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.disable_all_limits();
        parser.set_reject_duplicate_keys(true);
        let mut data = &br#"{"a":{"a":1},"b":2}"#[..];
        parser.receive(&mut data).unwrap();
        parser.close().unwrap();
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
    parser.set_max_nesting_depth(2);
    let mut data = &b"[[[1]]]"[..];
    assert!(parser.receive(&mut data).is_err());
}

#[test]
fn limits_string_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_max_string_length(5);
    let mut data = &br#""abcdef""#[..];
    assert!(parser.receive(&mut data).is_err());
}

#[test]
fn limits_number_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_max_number_length(3);
    let mut data = &b"1234"[..];
    parser.receive(&mut data).unwrap(); // may underflow until close
    assert!(parser.close().is_err());
}

#[test]
fn limits_document_length() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_max_document_length(3);
    let mut data = &b"1234"[..];
    assert!(parser.receive(&mut data).is_err());
}

#[test]
fn limits_token_count() {
    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_max_token_count(2);
    let mut data = &b"[1,2,3]"[..];
    assert!(parser.receive(&mut data).is_err());
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
        parser.disable_all_limits();
        let mut d1 = &b"1"[..];
        parser.receive(&mut d1).unwrap();
        parser.close().unwrap();
        parser.reset();
        let mut d2 = &b"2"[..];
        parser.receive(&mut d2).unwrap();
        parser.close().unwrap();
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
        parser.disable_all_limits();
        parser.parse(&br#"{"hello":"world"}"#[..]).unwrap();
    }
    assert_eq!(handler.keys, vec!["hello"]);
}
