//! The `loopbuf`-backed ring in `Parser::receive`: parsing must be correct
//! regardless of ring capacity, including when a token's bytes straddle the
//! ring's wraparound point. The parser never buffers for itself, so a ring
//! smaller than the next whole token (a key, a literal, an escape) is an
//! error, not something it works around.

mod common;

use loopbuf::LoopBuf;
use rjsonparser::ParserLimits;

use common::RecordingHandler;
use rjsonparser::{ParseResult, Parser};

fn parse_with_capacity(json: &[u8], capacity: usize) -> ParseResult<RecordingHandler> {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::with_buffer_size(&mut handler, capacity);
        parser.set_limits(ParserLimits::unlimited());
        parser.parse(json)?;
    }
    Ok(handler)
}

/// Every ring capacity from `min_capacity` (the longest token that must be
/// present whole) up to the document's length must produce identical events:
/// the ring's wraparound must be invisible to parsing results.
fn assert_capacity_invariant(json: &str, min_capacity: usize) {
    let whole = parse_with_capacity(json.as_bytes(), json.len() + 64)
        .unwrap()
        .events;
    for capacity in min_capacity..=json.len() {
        let got = parse_with_capacity(json.as_bytes(), capacity)
            .unwrap_or_else(|e| panic!("{json} @ capacity {capacity}: {e}"))
            .events;
        assert_eq!(got, whole, "{json} @ capacity {capacity}");
    }
}

#[test]
fn numbers_and_structure_survive_every_ring_capacity() {
    // numbers and strings stream, so even a one byte ring will do
    assert_capacity_invariant("[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]", 1);
}

#[test]
fn strings_survive_every_ring_capacity_including_mid_token_wrap() {
    assert_capacity_invariant(r#"["one","two","three","four","five","six","seven"]"#, 1);
}

#[test]
fn escapes_and_keys_survive_every_ring_capacity() {
    // the longest whole tokens: the key "ccc" and the literal false, 5 bytes
    assert_capacity_invariant(r#"{"a":"x\ny","bb":"zé","ccc":[true,false,null]}"#, 5);
}

#[test]
fn a_string_larger_than_the_ring_streams_through_it() {
    // Ring capacity (8) is smaller than the string value itself; the string
    // is delivered as it arrives, so it never has to fit.
    let json = r#"[1,"a longer string than the ring capacity",2]"#;
    let h = parse_with_capacity(json.as_bytes(), 8).unwrap();
    assert_eq!(h.numbers.len(), 2);
    assert_eq!(h.strings, vec!["a longer string than the ring capacity"]);
}

#[test]
fn a_key_larger_than_the_ring_is_an_error_not_a_reason_to_buffer() {
    let json = r#"{"a key that is longer than the ring capacity":1}"#;
    let err = parse_with_capacity(json.as_bytes(), 8).unwrap_err();
    assert!(err.message().to_lowercase().contains("buffer"), "{err}");
}

#[test]
fn an_escape_larger_than_the_ring_is_an_error() {
    let err = parse_with_capacity(br#"["a\u00e9b"]"#, 4).unwrap_err();
    assert!(err.message().to_lowercase().contains("buffer"), "{err}");
}

#[test]
fn many_one_byte_receives_wrap_a_tiny_ring_many_times() {
    // Feeding one byte at a time through a 4-byte ring forces many
    // write/drain/advance cycles, wrapping repeatedly over the course of
    // one document -- the push-parsing-codecs 1-byte-chunk convention,
    // applied against ring capacity instead of caller chunk size.
    let json = br#"{"a":[1,2,"bcd",3],"e":"fg","h":null}"#;

    let whole_events = {
        let mut reference_handler = RecordingHandler::new();
        let mut parser = Parser::new(&mut reference_handler);
        parser.set_limits(ParserLimits::unlimited());
        let mut buf = LoopBuf::with_capacity(json.len());
        buf.extend_from_slice(json);
        parser.receive(&mut buf).unwrap();
        parser.close(&mut buf).unwrap();
        reference_handler.events
    };

    let mut handler = RecordingHandler::new();
    let mut parser = Parser::new(&mut handler);
    parser.set_limits(ParserLimits::unlimited());
    let mut buf = LoopBuf::with_capacity(4);
    for &byte in json.iter() {
        // The caller is responsible for making room before writing more --
        // keep draining via receive() until the ring has spare capacity.
        while buf.is_full() {
            parser.receive(&mut buf).unwrap();
        }
        buf.extend_from_slice(&[byte]);
        parser.receive(&mut buf).unwrap();
    }
    parser.close(&mut buf).unwrap();
    assert_eq!(handler.events, whole_events);
}

/// Hands out its bytes in small, uneven reads, so the ring's write cursor
/// lands at a different offset every time and tokens of every kind end up
/// straddling the physical end of the ring.
struct Dribble<'a> {
    data: &'a [u8],
    step: usize,
}

impl std::io::Read for Dribble<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.step = self.step % 5 + 1;
        let n = self.step.min(out.len()).min(self.data.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

#[test]
fn tokens_straddling_the_wrap_parse_identically_at_every_capacity() {
    // multi-byte characters, escapes, an escaped surrogate pair, keys,
    // literals and numbers, all of which can land across the wrap
    let json = concat!(
        r#"{"key":"plain","é€":"café 😀 😀 end","n":[-12.5e3,0,123456789012,true,false,null],"#,
        r#""esc\n\t\"q\"":"a\\b\/c","deep":{"x":[{"y":"zé"},[],{}]},"last":"€€€€€€€€"}"#
    );
    let whole = parse_with_capacity(json.as_bytes(), json.len() + 64)
        .unwrap()
        .events;
    // the longest token that must be present whole is the 12 byte surrogate
    // pair escape; a ring of at least 16 always has room for it
    for capacity in 16..=json.len() {
        for start_step in 0..5 {
            let mut handler = RecordingHandler::new();
            {
                let mut parser = Parser::with_buffer_size(&mut handler, capacity);
                parser.set_limits(ParserLimits::unlimited());
                parser
                    .parse(Dribble {
                        data: json.as_bytes(),
                        step: start_step,
                    })
                    .unwrap_or_else(|e| panic!("capacity {capacity} step {start_step}: {e}"));
            }
            assert_eq!(
                handler.events, whole,
                "capacity {capacity} step {start_step}"
            );
        }
    }
}
