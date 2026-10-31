//! The caller-owned buffer contract, ported from the Java `CallerBufferContractTest`.
//!
//! The parser holds none of the document's bytes: what it cannot use yet is
//! left unconsumed in the caller's ring, which the caller tops up and
//! presents again. A ring too small for the next token is an error, never a
//! reason for the parser to start buffering for itself.

mod common;

use loopbuf::LoopBuf;
use rjsonparser::ParserLimits;
use rjsonparser::{ContentHandler, DefaultHandler, ParseResult, Parser};

fn ring_with(bytes: &[u8]) -> LoopBuf {
    let mut buf = LoopBuf::with_capacity(bytes.len() + 64);
    buf.extend_from_slice(bytes);
    buf
}

fn unconsumed(buf: &LoopBuf) -> Vec<u8> {
    let (a, b) = buf.readable_spans();
    [a, b].concat()
}

fn receive_leaves(input: &[u8]) -> Vec<u8> {
    let mut parser = Parser::new(DefaultHandler);
    let mut buf = ring_with(input);
    parser.receive(&mut buf).unwrap();
    unconsumed(&buf)
}

#[test]
fn everything_is_consumed_when_the_input_ends_on_a_token_boundary() {
    assert_eq!(receive_leaves(b"{\"a\":[1,\"two\",true,null]} "), b"");
}

#[test]
fn incomplete_key_stays_in_the_ring() {
    assert_eq!(receive_leaves(b"{\"abc"), b"\"abc");
}

#[test]
fn incomplete_literal_stays_in_the_ring() {
    assert_eq!(receive_leaves(b"[tr"), b"tr");
}

#[test]
fn incomplete_escape_stays_in_the_ring_from_its_backslash() {
    assert_eq!(receive_leaves(b"[\"ab\\u00"), b"\\u00");
}

#[test]
fn incomplete_utf8_sequence_stays_in_the_ring() {
    assert_eq!(
        receive_leaves(&[b'[', b'"', b'a', 0xE2, 0x82]),
        &[0xE2, 0x82]
    );
}

#[test]
fn incomplete_byte_order_mark_stays_in_the_ring() {
    assert_eq!(receive_leaves(&[0xEF, 0xBB]), &[0xEF, 0xBB]);
}

#[test]
fn escaped_high_surrogate_waits_in_the_ring_for_its_partner() {
    // the pair must reach the handler as one character, so the first half
    // is not consumed until the second can be seen
    assert_eq!(receive_leaves(b"[\"a\\ud83d"), b"\\ud83d");
    assert_eq!(receive_leaves(b"[\"a\\ud83d\\ude"), b"\\ud83d\\ude");
}

#[test]
fn unconsumed_bytes_presented_again_complete_the_token() {
    #[derive(Default)]
    struct Keys(String);
    impl ContentHandler for Keys {
        fn key(&mut self, key: &str) -> ParseResult<()> {
            self.0.push_str(key);
            Ok(())
        }
    }
    let mut keys = Keys::default();
    {
        let mut parser = Parser::new(&mut keys);
        let mut buf = LoopBuf::with_capacity(64);
        buf.extend_from_slice(b"{\"abc");
        parser.receive(&mut buf).unwrap();
        buf.extend_from_slice(b"def\":1}");
        parser.receive(&mut buf).unwrap();
        parser.close(&mut buf).unwrap();
    }
    assert_eq!(keys.0, "abcdef");
}

#[test]
fn close_rejects_bytes_left_unconsumed_and_says_so() {
    for truncated in [&b"{\"abc"[..], b"[tr", b"[\"ab\\u00", b"{\"a\":nu"] {
        let mut parser = Parser::new(DefaultHandler);
        let mut buf = ring_with(truncated);
        parser.receive(&mut buf).unwrap();
        let err = parser.close(&mut buf).unwrap_err();
        assert!(
            err.message().contains("incomplete token"),
            "{:?}: {err}",
            String::from_utf8_lossy(truncated)
        );
    }
}

#[test]
fn a_parser_can_be_reused_after_a_document_that_ended_mid_token() {
    let mut parser = Parser::new(DefaultHandler);
    let mut buf = ring_with(b"{\"abc");
    parser.receive(&mut buf).unwrap();
    assert!(parser.close(&mut buf).is_err());

    parser.reset();
    let mut buf = ring_with(b"[1,2,3]");
    parser.receive(&mut buf).unwrap();
    parser.close(&mut buf).unwrap();
    assert!(buf.is_empty());
}

#[test]
fn a_ring_too_small_for_the_next_token_is_reported() {
    let doc = b"{\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\":1}";
    let mut parser = Parser::new(DefaultHandler);
    parser.set_limits(ParserLimits::unlimited());
    let mut buf = LoopBuf::with_capacity(16);
    let mut offset = 0;
    let err = loop {
        let n = buf.extend_from_slice(&doc[offset..]);
        offset += n;
        assert!(
            n > 0,
            "the ring is full of an unfinished token and the parser did not say so"
        );
        if let Err(e) = parser.receive(&mut buf) {
            break e;
        }
        assert!(
            offset < doc.len(),
            "a 44 byte key cannot fit a 16 byte ring"
        );
    };
    assert!(err.message().to_lowercase().contains("buffer"), "{err}");
}

#[test]
fn parse_from_a_reader_uses_one_fixed_buffer() {
    let mut doc = String::from("{");
    for i in 0..500 {
        if i > 0 {
            doc.push(',');
        }
        doc.push_str(&format!(
            "\"key{i}\":[{},\"value number {i}\",true,null]",
            i as f64 * 1.5
        ));
    }
    doc.push('}');

    #[derive(Default)]
    struct Counter(usize);
    impl ContentHandler for Counter {
        fn key(&mut self, _key: &str) -> ParseResult<()> {
            self.0 += 1;
            Ok(())
        }
    }
    let mut counter = Counter::default();
    Parser::with_buffer_size(&mut counter, 64)
        .parse(doc.as_bytes())
        .unwrap();
    assert_eq!(counter.0, 500);
}

#[test]
fn parse_from_a_reader_reports_a_key_too_big_for_the_buffer() {
    let doc = format!("{{\"{}\":1}}", "k".repeat(100));
    let mut parser = Parser::with_buffer_size(DefaultHandler, 32);
    parser.set_limits(ParserLimits::unlimited());
    let err = parser.parse(doc.as_bytes()).unwrap_err();
    assert!(err.message().to_lowercase().contains("buffer"), "{err}");
}

#[test]
fn max_document_length_counts_each_byte_once() {
    // bytes left unconsumed and presented again must not be counted twice,
    // including a byte order mark split across chunks
    for doc in ["{\"abcdef\":123456}", "\u{feff}{\"abcdef\":123456}"] {
        let bytes = doc.as_bytes();
        for chunk in [1usize, 2, 3, 5, 100] {
            let feed = |limit: u64| -> ParseResult<()> {
                let mut parser = Parser::new(DefaultHandler);
                parser.limits_mut().max_document_length = Some(limit);
                let mut buf = LoopBuf::with_capacity(chunk + 64);
                for piece in bytes.chunks(chunk) {
                    buf.extend_from_slice(piece);
                    parser.receive(&mut buf)?;
                }
                parser.close(&mut buf)
            };
            feed(bytes.len() as u64).unwrap_or_else(|e| panic!("chunk={chunk}: {e}"));
            assert!(
                feed(bytes.len() as u64 - 1).is_err(),
                "chunk={chunk}: the document is one byte over the limit"
            );
        }
    }
}

#[test]
fn the_default_key_limit_is_4096_raw_bytes_and_strings_are_unlimited() {
    let key = |len: usize| format!("{{\"{}\":1}}", "k".repeat(len));
    assert!(common::parse_all(key(4096).as_bytes()).is_ok());
    // parse_all disables the limits, so go through a default parser
    let run = |json: &str| {
        let mut parser = Parser::with_buffer_size(DefaultHandler, 16384);
        parser.parse(json.as_bytes())
    };
    assert!(run(&key(4096)).is_ok());
    assert!(run(&key(4097)).is_err());
    let long_string = format!("[\"{}\"]", "s".repeat(30_000_000));
    assert!(run(&long_string).is_ok());
}

#[test]
fn a_bad_literal_is_rejected_without_waiting_for_the_rest_of_it() {
    // the error must not depend on where the chunk boundary fell
    for bad in [&b"[tx"[..], b"[fa!", b"[nulx"] {
        let mut parser = Parser::new(DefaultHandler);
        let mut buf = ring_with(bad);
        assert!(
            parser.receive(&mut buf).is_err(),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
}
