//! Tokens split across receive() calls (ported from Java TokenFragmentationTest).

mod common;

use common::parse_chunked;

fn number_events(json: &str, chunk: usize) -> Vec<String> {
    parse_chunked(json.as_bytes(), chunk)
        .unwrap_or_else(|e| panic!("{json} @ {chunk}: {e}"))
        .events
        .into_iter()
        .filter(|e| e.starts_with("number:"))
        .collect()
}

/// Every chunk size must produce the same events as one-shot parsing.
fn assert_chunk_invariant(json: &str) {
    let whole = parse_chunked(json.as_bytes(), json.len()).unwrap().events;
    for chunk in 1..=json.len() {
        let got = parse_chunked(json.as_bytes(), chunk)
            .unwrap_or_else(|e| panic!("{json} @ {chunk}: {e}"))
            .events;
        assert_eq!(got, whole, "{json} @ chunk {chunk}");
    }
}

#[test]
fn numbers_split_everywhere() {
    for json in [
        "[12345]", "[123.456]", "[123.456789]", "[123e45]", "[123e+45]", "[123e456]",
        "[123.456e789]", "[-123.456]", "[123.456e-78]", "[-0]", "[0.5]", "[1E5]",
    ] {
        assert_chunk_invariant(json);
    }
}

#[test]
fn split_number_values() {
    assert_eq!(number_events("[12345]", 3), ["number:12345"]);
    assert_eq!(number_events("[123.456]", 4), ["number:123.456"]);
    assert_eq!(number_events("[123.456]", 5), ["number:123.456"]);
    assert_eq!(number_events("[-123.456]", 2), ["number:-123.456"]);
    assert_eq!(number_events("[123e45]", 5), number_events("[123e45]", 8));
}

#[test]
fn strings_keys_literals_split_everywhere() {
    for json in [
        r#"{"key":"value","k2":[true,false,null]}"#,
        r#"["a\nb","\u00e9\ud83d\ude00","\\\"/"]"#,
        "[\"caf\u{e9} \u{1f600} \u{4e2d}\u{6587}\"]",
        r#"{"a":{"b":{"c":[1,2,{"d":null}]}}}"#,
    ] {
        assert_chunk_invariant(json);
    }
}

#[test]
fn whitespace_split_everywhere() {
    assert_chunk_invariant("  [ 1 ,\n\t2 , { \"a\" : 3 } ]\r\n");
}

#[test]
fn utf8_bom_then_document_split_everywhere() {
    let mut json = vec![0xEF, 0xBB, 0xBF];
    json.extend_from_slice(r#"{"k":"v\u00e9"}"#.as_bytes());
    let whole = parse_chunked(&json, json.len()).unwrap().events;
    for chunk in 1..=json.len() {
        assert_eq!(parse_chunked(&json, chunk).unwrap().events, whole, "chunk {chunk}");
    }
}

#[test]
fn truncated_documents_fail_at_every_prefix() {
    let json = br#"{"a":[1,2.5e3,"x\n",true,null]}"#;
    for end in 1..json.len() {
        assert!(
            parse_chunked(&json[..end], 3).is_err(),
            "prefix of {end} bytes should be incomplete"
        );
    }
    assert!(parse_chunked(json, 3).is_ok());
}
