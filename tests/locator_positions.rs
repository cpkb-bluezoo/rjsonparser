//! Locator positions, ported from the Java `LocatorPositionTest`.
//!
//! Lines and columns start at 1, a column counts UTF-16 units, CR/LF/CRLF
//! are each one line break, and the position reported with an event is where
//! that event ends.

use std::rc::Rc;

use loopbuf::LoopBuf;
use rjsonparser::{ContentHandler, Locator, Number, ParseResult, Parser};

#[derive(Default)]
struct Recorder {
    locator: Option<Rc<dyn Locator>>,
    events: Vec<String>,
}

impl Recorder {
    fn add(&mut self, name: &str) -> ParseResult<()> {
        let l = self.locator.as_ref().expect("locator was not set");
        self.events
            .push(format!("{name}@{}:{}", l.line_number(), l.column_number()));
        Ok(())
    }
}

impl ContentHandler for Recorder {
    fn set_locator(&mut self, locator: Rc<dyn Locator>) {
        self.locator = Some(locator);
    }
    fn needs_whitespace(&self) -> bool {
        true
    }
    fn start_object(&mut self) -> ParseResult<()> {
        self.add("{")
    }
    fn end_object(&mut self) -> ParseResult<()> {
        self.add("}")
    }
    fn start_array(&mut self) -> ParseResult<()> {
        self.add("[")
    }
    fn end_array(&mut self) -> ParseResult<()> {
        self.add("]")
    }
    fn key(&mut self, _key: &str) -> ParseResult<()> {
        self.add("key")
    }
    fn start_string_value(&mut self) -> ParseResult<()> {
        self.add("start")
    }
    fn string_value_content(&mut self, _chunk: &str) -> ParseResult<()> {
        self.add("content")
    }
    fn end_string_value(&mut self) -> ParseResult<()> {
        self.add("end")
    }
    fn number_value(&mut self, _value: &Number) -> ParseResult<()> {
        self.add("number")
    }
    fn boolean_value(&mut self, _value: bool) -> ParseResult<()> {
        self.add("boolean")
    }
    fn null_value(&mut self) -> ParseResult<()> {
        self.add("null")
    }
    fn whitespace(&mut self, _ws: &str) -> ParseResult<()> {
        self.add("ws")
    }
}

/// `chunk == 0` feeds the whole document in one `receive`.
fn positions(json: &str, chunk: usize) -> Vec<String> {
    let bytes = json.as_bytes();
    let size = if chunk == 0 { bytes.len() } else { chunk };
    let mut recorder = Recorder::default();
    {
        let mut parser = Parser::new(&mut recorder);
        let mut buf = LoopBuf::with_capacity(size + 64);
        for piece in bytes.chunks(size) {
            buf.extend_from_slice(piece);
            parser.receive(&mut buf).unwrap();
        }
        parser.close(&mut buf).unwrap();
    }
    recorder.events
}

fn assert_positions(json: &str, expected: &[&str]) {
    assert_eq!(positions(json, 0), expected, "{json:?}");
}

#[test]
fn structural_tokens_end_after_themselves() {
    assert_positions("[1]", &["[@1:2", "number@1:3", "]@1:4"]);
    assert_positions("[]", &["[@1:2", "]@1:3"]);
    assert_positions(
        "{\"k\":true}",
        &["{@1:2", "key@1:5", "boolean@1:10", "}@1:11"],
    );
}

#[test]
fn a_key_ends_after_its_closing_quote_not_after_the_colon() {
    assert_positions(
        "{\"abc\" : 1}",
        &[
            "{@1:2",
            "key@1:7",
            "ws@1:8",
            "ws@1:10",
            "number@1:11",
            "}@1:12",
        ],
    );
}

#[test]
fn literals_end_after_their_last_character() {
    assert_positions("[null]", &["[@1:2", "null@1:6", "]@1:7"]);
    assert_positions("[false]", &["[@1:2", "boolean@1:7", "]@1:8"]);
}

#[test]
fn a_string_value_is_reported_as_its_quotes_and_content_end() {
    assert_positions(
        "[\"ab\"]",
        &["[@1:2", "start@1:3", "content@1:5", "end@1:6", "]@1:7"],
    );
    assert_positions("\"hi\"", &["start@1:2", "content@1:4", "end@1:5"]);
    assert_positions("[\"\"]", &["[@1:2", "start@1:3", "end@1:4", "]@1:5"]);
}

#[test]
fn numbers_end_after_their_last_digit() {
    assert_positions("[1.5e10]", &["[@1:2", "number@1:8", "]@1:9"]);
    assert_positions("[-12 ]", &["[@1:2", "number@1:5", "ws@1:6", "]@1:7"]);
}

#[test]
fn a_number_that_ends_the_document_is_reported_by_close_at_its_end() {
    assert_positions("42", &["number@1:3"]);
}

#[test]
fn columns_count_utf16_units_not_characters_or_bytes() {
    assert_positions(
        "[\"é€😀\"]",
        &["[@1:2", "start@1:3", "content@1:7", "end@1:8", "]@1:9"],
    );
    assert_positions("{\"é€\":1}", &["{@1:2", "key@1:6", "number@1:8", "}@1:9"]);
}

#[test]
fn an_escape_counts_as_the_characters_written_in_the_source() {
    assert_positions(
        "[\"\\n\\u00e9\"]",
        &["[@1:2", "start@1:3", "content@1:11", "end@1:12", "]@1:13"],
    );
}

#[test]
fn whitespace_ends_after_its_last_character_and_newlines_start_a_new_line() {
    assert_positions(
        "[ 1,\n  2 ]",
        &[
            "[@1:2",
            "ws@1:3",
            "number@1:4",
            "ws@2:3",
            "number@2:4",
            "ws@2:5",
            "]@2:6",
        ],
    );
    assert_positions("\n\n[]", &["ws@3:1", "[@3:2", "]@3:3"]);
}

#[test]
fn crlf_is_one_line_break() {
    let expected = ["[@1:2", "number@1:3", "ws@2:1", "number@2:2", "]@2:3"];
    assert_positions("[1,\r\n2]", &expected);
    assert_positions("[1,\r2]", &expected);
    assert_positions("[1,\n2]", &expected);
}

fn without_pieces(events: Vec<String>) -> Vec<String> {
    events
        .into_iter()
        .filter(|e| !e.starts_with("content@") && !e.starts_with("ws@"))
        .collect()
}

#[test]
fn positions_do_not_depend_on_how_the_input_is_chunked() {
    let documents = [
        "{\"k\":[1,2.5e3,true,null,\"str\"],\"é\":{\"x\":\"\\u00e9\\n\"}}",
        "[ 1,\r\n  \"a\\u0041\" ,\n\t{ \"k\" : -0.5 } ]",
        "[\"😀😀\",12345]",
    ];
    for doc in documents {
        let whole = without_pieces(positions(doc, 0));
        for chunk in 1..=9 {
            assert_eq!(
                without_pieces(positions(doc, chunk)),
                whole,
                "{doc:?} chunk={chunk}"
            );
        }
    }
}

#[test]
fn a_string_content_piece_ends_after_its_own_text() {
    // one byte at a time: each piece is reported at the end of that character
    assert_eq!(
        positions("[\"ab\"]", 1),
        [
            "[@1:2",
            "start@1:3",
            "content@1:4",
            "content@1:5",
            "end@1:6",
            "]@1:7"
        ]
    );
}
