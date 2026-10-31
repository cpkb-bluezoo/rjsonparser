//! Common test helpers.
#![allow(dead_code)]

use loopbuf::LoopBuf;
use rjsonparser::ParserLimits;
use rjsonparser::{ContentHandler, Number, ParseResult, Parser};

#[derive(Debug, Default)]
pub struct RecordingHandler {
    pub events: Vec<String>,
    pub strings: Vec<String>,
    pub keys: Vec<String>,
    pub numbers: Vec<Number>,
    pub booleans: Vec<bool>,
    pub nulls: usize,
    pub needs_ws: bool,
    /// Accumulates chunks of the current string value until
    /// `end_string_value`; these tests check whole values, so this
    /// adapts the streaming `ContentHandler` contract back to that for
    /// convenience.
    string_scratch: String,
}

impl RecordingHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_whitespace() -> Self {
        Self {
            needs_ws: true,
            ..Self::default()
        }
    }
}

impl ContentHandler for RecordingHandler {
    fn start_object(&mut self) -> ParseResult<()> {
        self.events.push("startObject".into());
        Ok(())
    }
    fn end_object(&mut self) -> ParseResult<()> {
        self.events.push("endObject".into());
        Ok(())
    }
    fn start_array(&mut self) -> ParseResult<()> {
        self.events.push("startArray".into());
        Ok(())
    }
    fn end_array(&mut self) -> ParseResult<()> {
        self.events.push("endArray".into());
        Ok(())
    }
    fn key(&mut self, key: &str) -> ParseResult<()> {
        self.events.push(format!("key:{key}"));
        self.keys.push(key.to_owned());
        Ok(())
    }
    fn string_value_content(&mut self, chunk: &str) -> ParseResult<()> {
        self.string_scratch.push_str(chunk);
        Ok(())
    }
    fn end_string_value(&mut self) -> ParseResult<()> {
        let value = std::mem::take(&mut self.string_scratch);
        self.events.push(format!("string:{value}"));
        self.strings.push(value);
        Ok(())
    }
    fn number_value(&mut self, value: &Number) -> ParseResult<()> {
        self.numbers.push(value.clone());
        self.events.push(format!("number:{value}"));
        Ok(())
    }
    fn boolean_value(&mut self, value: bool) -> ParseResult<()> {
        self.booleans.push(value);
        self.events.push(format!("boolean:{value}"));
        Ok(())
    }
    fn null_value(&mut self) -> ParseResult<()> {
        self.nulls += 1;
        self.events.push("null".into());
        Ok(())
    }
    fn whitespace(&mut self, ws: &str) -> ParseResult<()> {
        self.events.push(format!("ws:{ws:?}"));
        Ok(())
    }
    fn needs_whitespace(&self) -> bool {
        self.needs_ws
    }
}

pub fn parse_all(input: &[u8]) -> ParseResult<RecordingHandler> {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        let mut buf = LoopBuf::with_capacity(input.len().max(1));
        buf.extend_from_slice(input);
        parser.receive(&mut buf)?;
        parser.close(&mut buf)?;
    }
    Ok(handler)
}

pub fn parse_string_value(json: &str) -> String {
    let handler = parse_all(json.as_bytes()).expect("parse failed");
    handler.strings.into_iter().next().expect("no string")
}

pub fn parse_number_value(json: &str) -> Number {
    let handler = parse_all(json.as_bytes()).expect("parse failed");
    handler.numbers.into_iter().next().expect("no number")
}

/// Feed `input` in chunks of `chunk_size`, writing each chunk into the
/// same ring (sized generously so capacity itself is never the limiting
/// factor here -- see `ring_buffering.rs` for capacity-sweep coverage)
/// and calling `receive` once per chunk, matching the real caller-owned
/// usage pattern.
pub fn parse_chunked(input: &[u8], chunk_size: usize) -> ParseResult<RecordingHandler> {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.set_limits(ParserLimits::unlimited());
        let mut buf = LoopBuf::with_capacity(input.len().max(1));
        let mut offset = 0;
        while offset < input.len() {
            let end = (offset + chunk_size).min(input.len());
            buf.extend_from_slice(&input[offset..end]);
            parser.receive(&mut buf)?;
            offset = end;
        }
        parser.close(&mut buf)?;
    }
    Ok(handler)
}
