//! Common test helpers.
#![allow(dead_code)]

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
        self.keys.push(key.to_owned());
        self.events.push(format!("key:{key}"));
        Ok(())
    }
    fn string_value(&mut self, value: &str) -> ParseResult<()> {
        self.strings.push(value.to_owned());
        self.events.push(format!("string:{value}"));
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
        parser.disable_all_limits();
        let mut data = input;
        parser.receive(&mut data)?;
        parser.close()?;
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

/// Feed `input` in chunks of `chunk_size`, using leftover-aware compact simulation.
pub fn parse_chunked(input: &[u8], chunk_size: usize) -> ParseResult<RecordingHandler> {
    let mut handler = RecordingHandler::new();
    {
        let mut parser = Parser::new(&mut handler);
        parser.disable_all_limits();
        let mut offset = 0;
        while offset < input.len() {
            let end = (offset + chunk_size).min(input.len());
            let mut slice = &input[offset..end];
            parser.receive(&mut slice)?;
            // Unconsumed bytes stay in parser leftover; advance past what we offered.
            offset = end;
        }
        parser.close()?;
    }
    Ok(handler)
}
