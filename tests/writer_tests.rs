//! The ring-native writer: output goes straight into a caller-owned `LoopBuf`.
//! Every call is atomic, so a ring with no room is `WouldBlock` with nothing
//! written, and the caller drains it and repeats the same call.

use std::io::{ErrorKind, Read};

use loopbuf::LoopBuf;
use rjsonparser::{IndentConfig, Number, WriteResult, Writer};

/// Runs `f` against a ring big enough for anything here and returns what it wrote.
fn render(f: impl FnOnce(&mut Writer, &mut LoopBuf) -> WriteResult<()>) -> String {
    render_with(Writer::new(), f)
}

fn render_with(
    mut w: Writer,
    f: impl FnOnce(&mut Writer, &mut LoopBuf) -> WriteResult<()>,
) -> String {
    let mut ring = LoopBuf::with_capacity(64 * 1024);
    f(&mut w, &mut ring).unwrap();
    String::from_utf8(ring.spans().to_vec()).unwrap()
}

/// Repeats `f` until it succeeds, draining the ring into `out` whenever it
/// reports that it has no room.
fn retry<T>(
    ring: &mut LoopBuf,
    out: &mut Vec<u8>,
    mut f: impl FnMut(&mut LoopBuf) -> WriteResult<T>,
) -> T {
    loop {
        match f(ring) {
            Ok(value) => return value,
            Err(e) if e.is_would_block() => {
                ring.read_to_end(out).unwrap();
            }
            Err(e) => panic!("{e}"),
        }
    }
}

#[test]
fn simple_object() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "name")?;
        w.write_string(b, "Alice")?;
        w.write_key(b, "age")?;
        w.write_number(b, &Number::I32(30))?;
        w.write_end_object(b)
    });
    assert_eq!(json, r#"{"name":"Alice","age":30}"#);
}

#[test]
fn simple_array() {
    let json = render(|w, b| {
        w.write_start_array(b)?;
        for n in 1..=3 {
            w.write_number(b, &Number::I32(n))?;
        }
        w.write_end_array(b)
    });
    assert_eq!(json, "[1,2,3]");
}

#[test]
fn nested_structures() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "users")?;
        w.write_start_array(b)?;
        for (id, name) in [(1, "Alice"), (2, "Bob")] {
            w.write_start_object(b)?;
            w.write_key(b, "id")?;
            w.write_number(b, &Number::I32(id))?;
            w.write_key(b, "name")?;
            w.write_string(b, name)?;
            w.write_end_object(b)?;
        }
        w.write_end_array(b)?;
        w.write_end_object(b)
    });
    assert_eq!(
        json,
        r#"{"users":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#
    );
}

#[test]
fn string_escaping() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "text")?;
        w.write_string(b, "Hello\n\"World\"\t\r\u{0008}\u{000c}\\")?;
        w.write_end_object(b)
    });
    assert_eq!(json, r#"{"text":"Hello\n\"World\"\t\r\b\f\\"}"#);
}

#[test]
fn control_character_escaping() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "ctrl")?;
        w.write_string(b, "test\u{0001}\u{001f}end")?;
        w.write_end_object(b)
    });
    assert_eq!(json, r#"{"ctrl":"test\u0001\u001fend"}"#);
}

#[test]
fn utf8_characters() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "emoji")?;
        w.write_string(b, "Hello 👋 World 🌍")?;
        w.write_key(b, "chinese")?;
        w.write_string(b, "你好世界")?;
        w.write_end_object(b)
    });
    assert!(json.contains("Hello 👋 World 🌍"));
    assert!(json.contains("你好世界"));
}

#[test]
fn primitive_values() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "bool_true")?;
        w.write_boolean(b, true)?;
        w.write_key(b, "bool_false")?;
        w.write_boolean(b, false)?;
        w.write_key(b, "null")?;
        w.write_null(b)?;
        w.write_key(b, "int")?;
        w.write_number(b, &Number::I32(42))?;
        w.write_key(b, "float")?;
        w.write_number(b, &Number::F64(3.25))?;
        w.write_end_object(b)
    });
    assert_eq!(
        json,
        r#"{"bool_true":true,"bool_false":false,"null":null,"int":42,"float":3.25}"#
    );
}

#[test]
fn numbers_of_every_kind() {
    let json = render(|w, b| {
        w.write_start_array(b)?;
        w.write_i32(b, i32::MIN)?;
        w.write_i64(b, i64::MAX)?;
        w.write_f64(b, -0.5)?;
        w.write_number(b, &Number::BigInt("123456789.1234567901234567890".into()))?;
        w.write_end_array(b)
    });
    assert_eq!(
        json,
        "[-2147483648,9223372036854775807,-0.5,123456789.1234567901234567890]"
    );
}

#[test]
fn integers_at_their_edges() {
    let json = render(|w, b| {
        w.write_start_array(b)?;
        for v in [
            0,
            -1,
            1,
            9,
            10,
            -10,
            99,
            100,
            i64::MIN,
            i64::MAX,
            i64::from(i32::MIN),
        ] {
            w.write_i64(b, v)?;
        }
        w.write_end_array(b)
    });
    assert_eq!(
        json,
        "[0,-1,1,9,10,-10,99,100,-9223372036854775808,9223372036854775807,-2147483648]"
    );
}

#[test]
fn doubles_round_trip_including_ones_too_long_for_the_stack_buffer() {
    let values = [
        0.0,
        -0.0,
        1.5,
        -1.5e-10,
        123456789.12345679,
        1e21,
        1e300,
        -1e300,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
    ];
    let json = render(|w, b| {
        w.write_start_array(b)?;
        for v in values {
            w.write_f64(b, v)?;
        }
        w.write_end_array(b)
    });
    let parsed: Vec<f64> = json[1..json.len() - 1]
        .split(',')
        .map(|t| t.parse().unwrap())
        .collect();
    assert_eq!(parsed.len(), values.len());
    for (got, want) in parsed.iter().zip(values) {
        assert_eq!(got.to_bits(), want.to_bits(), "{want} written as {json}");
    }
}

#[test]
fn indent_spaces2_output() {
    let json = render_with(Writer::with_indent(IndentConfig::spaces2()), |w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "a")?;
        w.write_number(b, &Number::I32(1))?;
        w.write_end_object(b)
    });
    assert_eq!(json, "{\n  \"a\": 1\n}");
}

#[test]
fn indent_tabs_output() {
    let json = render_with(Writer::with_indent(IndentConfig::tabs()), |w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "name")?;
        w.write_string(b, "Alice")?;
        w.write_key(b, "age")?;
        w.write_i32(b, 30)?;
        w.write_key(b, "hobbies")?;
        w.write_start_array(b)?;
        w.write_string(b, "reading")?;
        w.write_string(b, "coding")?;
        w.write_end_array(b)?;
        w.write_end_object(b)
    });
    assert_eq!(
        json,
        "{\n\t\"name\": \"Alice\",\n\t\"age\": 30,\n\t\"hobbies\": [\n\t\t\"reading\",\n\t\t\"coding\"\n\t]\n}"
    );
}

#[test]
fn empty_structures() {
    let json = render(|w, b| {
        w.write_start_object(b)?;
        w.write_key(b, "empty_array")?;
        w.write_start_array(b)?;
        w.write_end_array(b)?;
        w.write_key(b, "empty_object")?;
        w.write_start_object(b)?;
        w.write_end_object(b)?;
        w.write_end_object(b)
    });
    assert_eq!(json, r#"{"empty_array":[],"empty_object":{}}"#);
}

#[test]
fn closing_more_than_was_opened_does_not_panic() {
    let json = render(|w, b| {
        w.write_end_object(b)?;
        w.write_end_array(b)
    });
    assert_eq!(json, "}]");
}

// -- backpressure ------------------------------------------------------------

#[test]
fn a_ring_with_no_room_is_would_block_and_nothing_is_written() {
    let mut ring = LoopBuf::with_capacity(16);
    let mut w = Writer::new();
    w.write_start_array(&mut ring).unwrap();
    w.write_string(&mut ring, "twelve bytes").unwrap(); // [ + 14 = 15 of 16
    let before = ring.spans().to_vec();

    let err = w.write_string(&mut ring, "more").unwrap_err();
    assert!(err.is_would_block(), "{err}");
    assert_eq!(
        ring.spans().to_vec(),
        before,
        "not a byte of it was written"
    );

    // the writer is exactly where it was: drain, repeat the same call
    ring.advance(ring.len());
    w.write_string(&mut ring, "more").unwrap();
    assert_eq!(ring.spans().to_vec(), br#","more""#);
}

#[test]
fn a_separator_and_its_value_go_in_together_or_not_at_all() {
    // the comma belongs to the value after it: a ring with room for the comma
    // but not the number must not be left holding just the comma
    let mut ring = LoopBuf::with_capacity(8);
    let mut w = Writer::new();
    w.write_start_array(&mut ring).unwrap();
    w.write_i32(&mut ring, 1).unwrap();
    w.write_i32(&mut ring, 2).unwrap();
    w.write_i32(&mut ring, 3).unwrap(); // "[1,2,3" = 6 of 8
    let err = w.write_i32(&mut ring, 456).unwrap_err(); // needs 4, has 2
    assert!(err.is_would_block());
    assert_eq!(ring.spans().to_vec(), b"[1,2,3");
}

#[test]
fn a_token_larger_than_the_whole_ring_can_never_succeed() {
    let mut ring = LoopBuf::with_capacity(16);
    let mut w = Writer::new();
    let err = w.write_key(&mut ring, &"k".repeat(100)).unwrap_err();
    assert!(!err.is_would_block());
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(ring.is_empty());

    let err = w.write_string(&mut ring, &"s".repeat(100)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("write_start_string"), "{err}");
}

#[test]
fn a_document_far_bigger_than_the_ring_streams_through_it() {
    fn emit(w: &mut Writer, ring: &mut LoopBuf, out: &mut Vec<u8>) {
        retry(ring, out, |r| w.write_start_array(r));
        for i in 0..500 {
            retry(ring, out, |r| w.write_start_object(r));
            retry(ring, out, |r| w.write_key(r, "id"));
            retry(ring, out, |r| w.write_i32(r, i));
            retry(ring, out, |r| w.write_key(r, "name"));
            retry(ring, out, |r| {
                w.write_string(r, &format!("caf\u{e9} \"{i}\"\n"))
            });
            retry(ring, out, |r| w.write_end_object(r));
        }
        retry(ring, out, |r| w.write_end_array(r));
        ring.read_to_end(out).unwrap();
    }

    for indent in [
        None,
        Some(IndentConfig::spaces2()),
        Some(IndentConfig::tabs()),
    ] {
        let new = || indent.map_or_else(Writer::new, Writer::with_indent);
        let mut big_out = Vec::new();
        emit(
            &mut new(),
            &mut LoopBuf::with_capacity(1 << 20),
            &mut big_out,
        );
        for capacity in [24, 25, 40, 100] {
            let mut out = Vec::new();
            emit(&mut new(), &mut LoopBuf::with_capacity(capacity), &mut out);
            assert_eq!(out, big_out, "capacity {capacity} indent {indent:?}");
        }
    }
}

// -- NaN and the infinities --------------------------------------------------

#[test]
fn nan_and_infinity_are_rejected_before_anything_is_written() {
    let mut ring = LoopBuf::with_capacity(64);
    let mut w = Writer::new();
    w.write_start_array(&mut ring).unwrap();
    w.write_i32(&mut ring, 1).unwrap();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = w.write_f64(&mut ring, bad).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
        assert!(err.to_string().contains("cannot represent"), "{err}");
        let err = w.write_number(&mut ring, &Number::F64(bad)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
    }
    // the writer carries on as if the calls had not been made
    w.write_f64(&mut ring, 2.5).unwrap();
    w.write_end_array(&mut ring).unwrap();
    assert_eq!(ring.spans().to_vec(), b"[1,2.5]");
}

// -- streamed strings --------------------------------------------------------

#[test]
fn a_string_written_in_pieces_is_the_same_as_one_written_whole() {
    let tricky = "quote\" slash\\ nl\n tab\t ctl\u{1} caf\u{e9} \u{1f600} done";
    let whole = render(|w, b| {
        w.write_start_array(b)?;
        w.write_string(b, tricky)?;
        w.write_string(b, tricky)?;
        w.write_end_array(b)
    });
    let pieces = render(|w, b| {
        w.write_start_array(b)?;
        w.write_string(b, tricky)?;
        w.write_start_string(b)?;
        // split on character boundaries, as a parser's content calls are
        let mut rest = tricky;
        while !rest.is_empty() {
            let cut = (rest.chars().next().map_or(0, char::len_utf8) * 3).min(rest.len());
            let cut = (cut..=rest.len())
                .find(|&i| rest.is_char_boundary(i))
                .unwrap();
            let (head, tail) = rest.split_at(cut);
            assert_eq!(w.write_string_content(b, head)?, head.len());
            rest = tail;
        }
        w.write_end_string(b)?;
        w.write_end_array(b)
    });
    assert_eq!(pieces, whole);
}

#[test]
fn a_string_bigger_than_the_ring_streams_through_a_small_one() {
    let big = "x\n\u{e9}\u{1f600}\"".repeat(2_000_000 / 11 + 1); // ~2 MB, 11 bytes escaped
    let mut ring = LoopBuf::with_capacity(64);
    let mut out = Vec::new();
    let mut w = Writer::new();

    retry(&mut ring, &mut out, |r| w.write_start_string(r));
    let mut rest = big.as_str();
    let mut calls = 0;
    while !rest.is_empty() {
        let taken = retry(&mut ring, &mut out, |r| w.write_string_content(r, rest));
        assert!(taken > 0 && taken <= rest.len());
        assert!(rest.is_char_boundary(taken), "whole characters only");
        rest = &rest[taken..];
        calls += 1;
    }
    retry(&mut ring, &mut out, |r| w.write_end_string(r));
    ring.read_to_end(&mut out).unwrap();

    assert!(calls > 1000, "it really did go through in pieces: {calls}");
    let json = String::from_utf8(out).unwrap();
    assert!(json.starts_with("\"x\\n\u{e9}\u{1f600}\\\"x"));
    assert!(json.ends_with('"'));
    assert_eq!(json.matches("\\n").count(), big.matches('\n').count());
}

#[test]
fn content_with_no_room_at_all_is_would_block_not_zero() {
    let mut ring = LoopBuf::with_capacity(8);
    let mut w = Writer::new();
    w.write_start_string(&mut ring).unwrap();
    assert_eq!(w.write_string_content(&mut ring, "1234567").unwrap(), 7);
    assert!(ring.is_full());
    let err = w.write_string_content(&mut ring, "more").unwrap_err();
    assert!(err.is_would_block());
    assert_eq!(w.write_string_content(&mut ring, "").unwrap(), 0);
}

#[test]
fn a_parser_can_feed_the_writer_through_a_small_ring() {
    use rjsonparser::{ContentHandler, ParseResult, Parser};

    /// Re-emits every event it is given, draining the output ring when full.
    struct Echo {
        w: Writer,
        out_ring: LoopBuf,
        out: Vec<u8>,
    }
    impl Echo {
        fn go<T>(
            &mut self,
            mut f: impl FnMut(&mut Writer, &mut LoopBuf) -> WriteResult<T>,
        ) -> ParseResult<T> {
            let Echo { w, out_ring, out } = self;
            Ok(retry(out_ring, out, |r| f(w, r)))
        }
    }
    impl ContentHandler for Echo {
        fn start_object(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_start_object(r))
        }
        fn end_object(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_end_object(r))
        }
        fn start_array(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_start_array(r))
        }
        fn end_array(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_end_array(r))
        }
        fn key(&mut self, key: &str) -> ParseResult<()> {
            self.go(|w, r| w.write_key(r, key))
        }
        fn start_string_value(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_start_string(r))
        }
        fn string_value_content(&mut self, chunk: &str) -> ParseResult<()> {
            let mut rest = chunk;
            while !rest.is_empty() {
                let taken = self.go(|w, r| w.write_string_content(r, rest))?;
                rest = &rest[taken..];
            }
            Ok(())
        }
        fn end_string_value(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_end_string(r))
        }
        fn number_value(&mut self, n: &Number) -> ParseResult<()> {
            self.go(|w, r| w.write_number(r, n))
        }
        fn boolean_value(&mut self, v: bool) -> ParseResult<()> {
            self.go(|w, r| w.write_boolean(r, v))
        }
        fn null_value(&mut self) -> ParseResult<()> {
            self.go(|w, r| w.write_null(r))
        }
    }

    let long = "word \u{e9} ".repeat(5000);
    let json = format!(r#"{{"k":[1,-2.5,true,false,null,"a\tb","{long}"],"café":{{"x":[]}}}}"#);
    // the output ring (32 bytes) is far smaller than the 40 KB string
    let echo = Echo {
        w: Writer::new(),
        out_ring: LoopBuf::with_capacity(32),
        out: Vec::new(),
    };
    let mut parser = Parser::with_buffer_size(echo, 256);
    parser.parse(json.as_bytes()).unwrap();
    let mut echo = parser.into_handler();
    echo.out_ring.read_to_end(&mut echo.out).unwrap();

    let expected = json.replace("\\u00e9", "\u{e9}");
    assert_eq!(String::from_utf8(echo.out).unwrap(), expected);
}

#[test]
fn round_trip_through_parser() {
    use rjsonparser::{ContentHandler, ParseResult, Parser};
    #[derive(Default)]
    struct Echo(Vec<String>, String);
    impl ContentHandler for Echo {
        fn key(&mut self, key: &str) -> ParseResult<()> {
            self.0.push(format!("k:{key}"));
            Ok(())
        }
        fn string_value_content(&mut self, chunk: &str) -> ParseResult<()> {
            self.1.push_str(chunk);
            Ok(())
        }
        fn end_string_value(&mut self) -> ParseResult<()> {
            self.0.push(format!("s:{}", std::mem::take(&mut self.1)));
            Ok(())
        }
    }
    let tricky = "quote\" slash\\ nl\n tab\t ctl\u{1} caf\u{e9} \u{1f600}";
    let mut ring = LoopBuf::with_capacity(256);
    let mut w = Writer::new();
    w.write_start_object(&mut ring).unwrap();
    w.write_key(&mut ring, tricky).unwrap();
    w.write_string(&mut ring, tricky).unwrap();
    w.write_end_object(&mut ring).unwrap();

    // the same ring is the parser's input: nothing is copied between them
    let mut p = Parser::new(Echo::default());
    p.receive(&mut ring).unwrap();
    p.close(&mut ring).unwrap();
    assert_eq!(
        p.handler().0,
        [format!("k:{tricky}"), format!("s:{tricky}")]
    );
}
