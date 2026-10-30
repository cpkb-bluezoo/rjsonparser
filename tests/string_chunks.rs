//! String content goes through one fixed-size character buffer, so no
//! content call carries more than `set_string_chunk_size` characters, however
//! much the ring holds. Ported from the Java `StringStreamingTest`.

use loopbuf::LoopBuf;
use rjsonparser::{ContentHandler, ParseResult, Parser};

#[derive(Default)]
struct Chunks {
    pieces: Vec<String>,
    value: String,
    values: Vec<String>,
}

impl ContentHandler for Chunks {
    fn string_value_content(&mut self, chunk: &str) -> ParseResult<()> {
        assert!(!chunk.is_empty(), "content is never empty");
        self.pieces.push(chunk.to_owned());
        self.value.push_str(chunk);
        Ok(())
    }
    fn end_string_value(&mut self) -> ParseResult<()> {
        self.values.push(std::mem::take(&mut self.value));
        Ok(())
    }
}

fn run(json: &str, chunk_size: usize, ring_capacity: usize, feed: usize) -> Chunks {
    let mut handler = Chunks::default();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_string_chunk_size(chunk_size);
        let mut buf = LoopBuf::with_capacity(ring_capacity);
        for piece in json.as_bytes().chunks(feed) {
            buf.extend_from_slice(piece);
            parser.receive(&mut buf).unwrap();
        }
        parser.close(&mut buf).unwrap();
    }
    handler
}

#[test]
fn no_content_call_carries_more_than_the_chunk_size() {
    let plain = "x".repeat(10_000);
    let mixed = "aé€😀\\n\\u00e9\\ud83d\\ude00".repeat(500);
    for body in [plain.as_str(), mixed.as_str()] {
        let json = format!("[\"{body}\"]");
        let whole = run(&json, 32768, json.len() + 64, json.len());
        for chunk_size in [16, 17, 20, 64, 1000] {
            let h = run(&json, chunk_size, json.len() + 64, json.len());
            assert_eq!(h.values, whole.values, "chunk_size={chunk_size}");
            for piece in &h.pieces {
                assert!(
                    piece.len() <= chunk_size,
                    "chunk_size={chunk_size}: a piece of {} bytes",
                    piece.len()
                );
            }
        }
    }
}

#[test]
fn the_ring_size_does_not_change_the_pieces_beyond_the_chunk_size() {
    let json = format!("[\"{}\"]", "abcdefghij".repeat(2000));
    for (ring, feed) in [(64, 7), (128, 100), (4096, 1000)] {
        let h = run(&json, 40, ring, feed);
        assert_eq!(h.values[0], "abcdefghij".repeat(2000), "ring={ring}");
        assert!(h.pieces.iter().all(|p| p.len() <= 40), "ring={ring}");
    }
}

#[test]
fn an_escaped_surrogate_pair_is_one_character() {
    let h = run("[\"\\ud83d\\ude00\\ud83d\\ude00\"]", 16, 64, 3);
    assert_eq!(h.values[0], "😀😀");
}

#[test]
fn an_escaped_lone_surrogate_becomes_the_replacement_character() {
    // a Rust &str cannot hold an unpaired surrogate
    let h = run("[\"a\\ud800b\\udc00c\"]", 16, 64, 100);
    assert_eq!(h.values[0], "a\u{fffd}b\u{fffd}c");
}

#[test]
#[should_panic(expected = "String chunk size must be at least 16")]
fn a_chunk_size_below_sixteen_is_rejected() {
    Parser::new(rjsonparser::DefaultHandler).set_string_chunk_size(15);
}
