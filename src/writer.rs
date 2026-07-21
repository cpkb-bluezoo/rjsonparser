//! Streaming JSON writer (Java `JSONWriter` port).

use std::io::Write;

use crate::error::{WriteError, WriteResult};
use crate::number::Number;

const DEFAULT_CAPACITY: usize = 4096;
const SEND_THRESHOLD: f32 = 0.75;

/// Configuration for JSON output indentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentConfig {
    indent_char: u8,
    indent_count: usize,
}

impl IndentConfig {
    /// Creates an indent configuration.
    ///
    /// `indent_char` must be space or tab; `indent_count` must be positive.
    pub fn new(indent_char: char, indent_count: usize) -> Self {
        assert!(
            indent_char == ' ' || indent_char == '\t',
            "Indent character must be space or tab"
        );
        assert!(indent_count > 0, "Indent count must be positive");
        Self {
            indent_char: indent_char as u8,
            indent_count,
        }
    }

    pub fn indent_char(&self) -> char {
        self.indent_char as char
    }

    pub fn indent_count(&self) -> usize {
        self.indent_count
    }

    /// Single tab per level.
    pub fn tabs() -> Self {
        Self::new('\t', 1)
    }

    /// Two spaces per level.
    pub fn spaces2() -> Self {
        Self::new(' ', 2)
    }

    /// Four spaces per level.
    pub fn spaces4() -> Self {
        Self::new(' ', 4)
    }

    /// The specified number of spaces per level.
    pub fn spaces(count: usize) -> Self {
        Self::new(' ', count)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Init0,
    Init,
    AfterKey,
    AfterValue,
}

/// Streaming JSON writer with internal buffering.
///
/// This class does not perform well-formedness checking on its input: the caller must
/// supply events in the correct order and close objects and arrays they open.
pub struct Writer<W: Write> {
    inner: W,
    buffer: Vec<u8>,
    send_threshold: usize,
    indent: Option<IndentConfig>,
    state: State,
    depth: usize,
}

impl<W: Write> Writer<W> {
    /// Creates a new JSON writer with default capacity (4KB) and no indentation.
    pub fn new(inner: W) -> Self {
        Self::with_capacity(inner, DEFAULT_CAPACITY, None)
    }

    /// Creates a new JSON writer with optional indentation.
    pub fn with_indent(inner: W, indent: IndentConfig) -> Self {
        Self::with_capacity(inner, DEFAULT_CAPACITY, Some(indent))
    }

    fn with_capacity(inner: W, buffer_capacity: usize, indent: Option<IndentConfig>) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(buffer_capacity),
            send_threshold: (buffer_capacity as f32 * SEND_THRESHOLD) as usize,
            indent,
            state: State::Init0,
            depth: 0,
        }
    }

    /// Returns a reference to the underlying writer.
    pub fn inner(&self) -> &W {
        &self.inner
    }

    /// Returns a mutable reference to the underlying writer.
    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    pub fn write_start_object(&mut self) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        self.ensure_capacity(1);
        self.buffer.push(b'{');
        self.state = State::Init;
        self.depth += 1;
        self.send_if_needed()
    }

    pub fn write_end_object(&mut self) -> WriteResult<()> {
        self.depth -= 1;
        if self.indent.is_some() {
            self.write_indent()?;
        }
        self.ensure_capacity(1);
        self.buffer.push(b'}');
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    pub fn write_start_array(&mut self) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        self.ensure_capacity(1);
        self.buffer.push(b'[');
        self.state = State::Init;
        self.depth += 1;
        self.send_if_needed()
    }

    pub fn write_end_array(&mut self) -> WriteResult<()> {
        self.depth -= 1;
        if self.indent.is_some() {
            self.write_indent()?;
        }
        self.ensure_capacity(1);
        self.buffer.push(b']');
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    pub fn write_key(&mut self, key: &str) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else if self.state == State::AfterValue {
            self.ensure_capacity(1);
            self.buffer.push(b',');
        }
        self.write_quoted_string(key)?;
        self.ensure_capacity(1);
        self.buffer.push(b':');
        self.state = State::AfterKey;
        self.send_if_needed()
    }

    pub fn write_string(&mut self, value: &str) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        self.write_quoted_string(value)?;
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    pub fn write_number(&mut self, value: &Number) -> WriteResult<()> {
        self.write_number_str(&value.to_string())
    }

    pub fn write_number_str(&mut self, value: &str) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        self.ensure_capacity(value.len());
        self.buffer.extend_from_slice(value.as_bytes());
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    pub fn write_i32(&mut self, value: i32) -> WriteResult<()> {
        self.write_number_str(&value.to_string())
    }

    pub fn write_i64(&mut self, value: i64) -> WriteResult<()> {
        self.write_number_str(&value.to_string())
    }

    pub fn write_f64(&mut self, value: f64) -> WriteResult<()> {
        self.write_number_str(&Number::F64(value).to_string())
    }

    pub fn write_boolean(&mut self, value: bool) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        if value {
            self.ensure_capacity(4);
            self.buffer.extend_from_slice(b"true");
        } else {
            self.ensure_capacity(5);
            self.buffer.extend_from_slice(b"false");
        }
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    pub fn write_null(&mut self) -> WriteResult<()> {
        if self.indent.is_some() {
            self.write_indented_value_start()?;
        } else {
            self.write_value_separator_if_needed()?;
        }
        self.ensure_capacity(4);
        self.buffer.extend_from_slice(b"null");
        self.state = State::AfterValue;
        self.send_if_needed()
    }

    /// Flushes any buffered data to the underlying writer.
    pub fn flush(&mut self) -> WriteResult<()> {
        if !self.buffer.is_empty() {
            self.send()?;
        }
        self.inner.flush().map_err(WriteError::new)
    }

    /// Flushes buffered data. Does not close the underlying writer.
    pub fn close(mut self) -> WriteResult<()> {
        self.flush()
    }

    fn write_value_separator_if_needed(&mut self) -> WriteResult<()> {
        if self.state == State::AfterValue {
            self.ensure_capacity(1);
            self.buffer.push(b',');
        }
        Ok(())
    }

    fn write_indented_value_start(&mut self) -> WriteResult<()> {
        match self.state {
            State::AfterKey => {
                self.ensure_capacity(1);
                self.buffer.push(b' ');
            }
            State::AfterValue => {
                self.ensure_capacity(1);
                self.buffer.push(b',');
                self.write_indent()?;
            }
            State::Init => {
                self.write_indent()?;
            }
            State::Init0 => {}
        }
        Ok(())
    }

    fn write_indent(&mut self) -> WriteResult<()> {
        let indent = self.indent.expect("indent set");
        let indent_size = indent.indent_count * self.depth;
        self.ensure_capacity(1 + indent_size);
        self.buffer.push(b'\n');
        for _ in 0..indent_size {
            self.buffer.push(indent.indent_char);
        }
        Ok(())
    }

    fn write_quoted_string(&mut self, s: &str) -> WriteResult<()> {
        let estimated = 2 + s.len() + s.len() / 10;
        self.ensure_capacity(estimated);
        self.buffer.push(b'"');

        let mut i = 0;
        while i < s.len() {
            let ch = s[i..].chars().next().expect("valid utf-8");
            let char_len = ch.len_utf8();

            if self.buffer.capacity() - self.buffer.len() < 12 {
                self.grow_buffer(self.buffer.capacity() * 2);
            }

            match ch {
                '"' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'"');
                }
                '\\' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'\\');
                }
                '\u{8}' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'b');
                }
                '\u{c}' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'f');
                }
                '\n' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'n');
                }
                '\r' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'r');
                }
                '\t' => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b't');
                }
                c if (c as u32) < 0x20 => {
                    self.buffer.push(b'\\');
                    self.buffer.push(b'u');
                    self.buffer.push(b'0');
                    self.buffer.push(b'0');
                    self.buffer.push(hex_char(((c as u32) >> 4) & 0xF));
                    self.buffer.push(hex_char((c as u32) & 0xF));
                }
                c if (c as u32) < 0x80 => {
                    self.buffer.push(c as u8);
                }
                c => {
                    self.write_utf8_code_point(c as u32);
                }
            }

            i += char_len;
        }

        self.buffer.push(b'"');
        Ok(())
    }

    fn write_utf8_code_point(&mut self, code_point: u32) {
        if code_point < 0x80 {
            self.buffer.push(code_point as u8);
        } else if code_point < 0x800 {
            self.buffer
                .push(0xC0 | ((code_point >> 6) as u8));
            self.buffer
                .push(0x80 | ((code_point & 0x3F) as u8));
        } else if code_point < 0x1_0000 {
            self.buffer
                .push(0xE0 | ((code_point >> 12) as u8));
            self.buffer
                .push(0x80 | (((code_point >> 6) & 0x3F) as u8));
            self.buffer
                .push(0x80 | ((code_point & 0x3F) as u8));
        } else {
            self.buffer
                .push(0xF0 | ((code_point >> 18) as u8));
            self.buffer
                .push(0x80 | (((code_point >> 12) & 0x3F) as u8));
            self.buffer
                .push(0x80 | (((code_point >> 6) & 0x3F) as u8));
            self.buffer
                .push(0x80 | ((code_point & 0x3F) as u8));
        }
    }

    fn ensure_capacity(&mut self, needed: usize) {
        if self.buffer.len() + needed > self.buffer.capacity() {
            self.grow_buffer(
                (self.buffer.capacity() * 2).max(self.buffer.len() + needed),
            );
        }
    }

    fn grow_buffer(&mut self, new_capacity: usize) {
        self.buffer.reserve(new_capacity.saturating_sub(self.buffer.capacity()));
    }

    fn send_if_needed(&mut self) -> WriteResult<()> {
        if self.buffer.len() >= self.send_threshold {
            self.send()?;
        }
        Ok(())
    }

    fn send(&mut self) -> WriteResult<()> {
        self.inner
            .write_all(&self.buffer)
            .map_err(WriteError::new)?;
        self.buffer.clear();
        Ok(())
    }
}

impl Writer<Vec<u8>> {
    /// Creates a writer that accumulates into an internal [`Vec`].
    pub fn buffer(capacity: usize) -> Self {
        Writer::with_capacity(Vec::with_capacity(capacity), capacity, None)
    }

    /// Flushes the internal buffer and returns the accumulated bytes.
    pub fn finish(mut self) -> WriteResult<Vec<u8>> {
        self.flush()?;
        Ok(self.inner)
    }
}

fn hex_char(n: u32) -> u8 {
    if n < 10 {
        b'0' + n as u8
    } else {
        b'a' + (n - 10) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_roundtrip_compact() -> WriteResult<()> {
        let mut w = Writer::buffer(64);
        w.write_start_object()?;
        w.write_key("a")?;
        w.write_number_str("1")?;
        w.write_end_object()?;
        let bytes = w.finish()?;
        assert_eq!(bytes, br#"{"a":1}"#);
        Ok(())
    }
}
