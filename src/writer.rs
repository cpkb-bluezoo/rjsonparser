//! Streaming JSON writer encoding straight into a caller-owned ring (Java `JSONWriter` port).

use std::io;
use std::io::Write as _;

use loopbuf::LoopBuf;

use crate::error::{WriteError, WriteResult};
use crate::number::Number;

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

/// What goes before a value or key: the separator and indentation its
/// position in the document calls for.
#[derive(Clone, Copy)]
struct Prefix {
    comma: bool,
    space: bool,
    /// A newline and this many levels of indentation.
    newline_depth: Option<usize>,
}

/// Largest token assembled on the stack before being appended in one go.
const SCRATCH: usize = 64;

/// Somewhere encoded bytes can be put: the ring itself, or a small stack
/// scratch a whole token is assembled in first. A token is mostly a few single
/// bytes (a comma, a quote, a colon) around its content, and appending each of
/// those to the ring separately costs far more than copying them once.
trait Out {
    fn put(&mut self, bytes: &[u8]);
}

impl Out for LoopBuf {
    #[inline]
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

struct Scratch {
    bytes: [u8; SCRATCH],
    len: usize,
}

impl Scratch {
    #[inline]
    fn new() -> Self {
        Self {
            bytes: [0; SCRATCH],
            len: 0,
        }
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Out for Scratch {
    #[inline]
    fn put(&mut self, bytes: &[u8]) {
        self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }
}

/// Runs `body` (which encodes a token of `need` bytes into `out`) against a
/// stack scratch if the token is small, appending it to the ring once, or
/// directly against the ring if it is not. The room has been checked.
macro_rules! emit {
    ($buf:expr, $need:expr, |$out:ident| $body:block) => {{
        if $need <= SCRATCH {
            let mut scratch = Scratch::new();
            {
                let $out = &mut scratch;
                $body
            }
            $buf.extend_from_slice(scratch.as_slice());
        } else {
            let $out = &mut *$buf;
            $body
        }
    }};
}

/// Streaming JSON writer that encodes straight into a caller-owned [`LoopBuf`].
///
/// The writer keeps no buffer of its own, only where it is in the document.
/// Every `write_*` method takes the caller's ring and is **atomic**: it either
/// appends the whole token (with any separator and indentation before it) or
/// appends nothing and returns an error, leaving the writer's state exactly as
/// it was.
///
/// - [`WriteError::is_would_block`] means the ring has no room for it *right
///   now*. Drain the ring (to a socket, say) and repeat the same call.
/// - [`std::io::ErrorKind::InvalidInput`] means it can never succeed as it
///   stands: the token is larger than the ring's whole capacity, or it is a
///   number JSON cannot represent. Write a long string with
///   [`Self::write_start_string`], [`Self::write_string_content`] and
///   [`Self::write_end_string`], which take as much as fits each time.
///
/// ```
/// use loopbuf::LoopBuf;
/// use rjsonparser::Writer;
///
/// let mut ring = LoopBuf::with_capacity(4096);
/// let mut w = Writer::new();
/// w.write_start_object(&mut ring).unwrap();
/// w.write_key(&mut ring, "name").unwrap();
/// w.write_string(&mut ring, "Alice").unwrap();
/// w.write_end_object(&mut ring).unwrap();
/// assert_eq!(ring.spans().to_vec(), br#"{"name":"Alice"}"#);
/// ```
///
/// This does not perform well-formedness checking on its input: the caller
/// must supply events in the correct order and close objects and arrays they
/// open.
#[derive(Debug, Clone)]
pub struct Writer {
    indent: Option<IndentConfig>,
    state: State,
    depth: usize,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    /// Creates a new JSON writer with no indentation.
    pub fn new() -> Self {
        Self {
            indent: None,
            state: State::Init0,
            depth: 0,
        }
    }

    /// Creates a new JSON writer with indentation.
    pub fn with_indent(indent: IndentConfig) -> Self {
        Self {
            indent: Some(indent),
            ..Self::new()
        }
    }

    /// Resets the writer to the start of a new document.
    pub fn reset(&mut self) {
        self.state = State::Init0;
        self.depth = 0;
    }

    pub fn write_start_object(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        self.write_open(buf, b'{')
    }

    pub fn write_start_array(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        self.write_open(buf, b'[')
    }

    pub fn write_end_object(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        self.write_close(buf, b'}')
    }

    pub fn write_end_array(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        self.write_close(buf, b']')
    }

    pub fn write_key(&mut self, buf: &mut LoopBuf, key: &str) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + 2 + escaped_len(key) + 1;
        ensure_room(buf, need, "key")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            put_quoted(out, key);
            out.put(b":");
        });
        self.state = State::AfterKey;
        Ok(())
    }

    pub fn write_string(&mut self, buf: &mut LoopBuf, value: &str) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + 2 + escaped_len(value);
        ensure_room(buf, need, "string")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            put_quoted(out, value);
        });
        self.state = State::AfterValue;
        Ok(())
    }

    /// Starts a string value whose content is written in pieces with
    /// [`Self::write_string_content`], then closed with
    /// [`Self::write_end_string`]. The counterpart of the parser's
    /// `start_string_value`/`string_value_content`/`end_string_value`, so a
    /// parser can feed a writer with neither holding the whole string, and the
    /// way to write a string too big to fit the ring in one go.
    pub fn write_start_string(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + 1;
        ensure_room(buf, need, "string")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            out.put(b"\"");
        });
        Ok(())
    }

    /// Writes as much of `content` as fits, escaped as needed, and returns how
    /// many of its bytes were taken (always whole characters). If that is less
    /// than `content.len()` the ring is full: drain it and call again with the
    /// rest. Taking nothing from non-empty content is an error
    /// ([`WriteError::is_would_block`]) rather than `Ok(0)`.
    pub fn write_string_content(&mut self, buf: &mut LoopBuf, content: &str) -> WriteResult<usize> {
        if content.is_empty() {
            return Ok(0);
        }
        let spare = buf.spare_len();
        // Escaping never makes text shorter, so nothing past `spare` raw bytes
        // can fit: look only at that much, however long `content` is. (Looking
        // at all of it every call would cost quadratic time for a long string.)
        let mut limit = content.len().min(spare);
        while !content.is_char_boundary(limit) {
            limit -= 1;
        }
        let candidate = &content[..limit];
        let taken = if escaped_len(candidate) <= spare {
            limit
        } else {
            // as many whole characters as fit
            let mut used = 0;
            let mut taken = 0;
            for (i, ch) in candidate.char_indices() {
                let size = escaped_char_len(ch);
                if used + size > spare {
                    break;
                }
                used += size;
                taken = i + ch.len_utf8();
            }
            taken
        };
        if taken == 0 {
            // the longest single character is 6 bytes escaped
            ensure_room(buf, 6, "string content")?;
            return Err(would_block());
        }
        put_escaped(buf, &content[..taken]);
        Ok(taken)
    }

    /// Ends a string value started with [`Self::write_start_string`].
    pub fn write_end_string(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        ensure_room(buf, 1, "string")?;
        buf.extend_from_slice(b"\"");
        self.state = State::AfterValue;
        Ok(())
    }

    /// Writes a number.
    ///
    /// JSON has no representation for NaN or the infinities, so a `F64` with
    /// one of those values is rejected ([`std::io::ErrorKind::InvalidInput`])
    /// rather than written as text that no JSON parser would accept. The check
    /// comes before anything is written.
    pub fn write_number(&mut self, buf: &mut LoopBuf, value: &Number) -> WriteResult<()> {
        match value {
            Number::I32(v) => self.write_i64(buf, i64::from(*v)),
            Number::I64(v) => self.write_i64(buf, *v),
            Number::F64(v) => self.write_f64(buf, *v),
            Number::BigInt(digits) => self.write_number_str(buf, digits),
        }
    }

    /// Writes already-formatted number text as it is.
    pub fn write_number_str(&mut self, buf: &mut LoopBuf, value: &str) -> WriteResult<()> {
        self.write_number_bytes(buf, value.as_bytes())
    }

    fn write_number_bytes(&mut self, buf: &mut LoopBuf, value: &[u8]) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + value.len();
        ensure_room(buf, need, "number")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            out.put(value);
        });
        self.state = State::AfterValue;
        Ok(())
    }

    pub fn write_i32(&mut self, buf: &mut LoopBuf, value: i32) -> WriteResult<()> {
        self.write_i64(buf, i64::from(value))
    }

    pub fn write_i64(&mut self, buf: &mut LoopBuf, value: i64) -> WriteResult<()> {
        // digits go into the end of a stack array: no allocation, and none of
        // the formatting machinery
        let mut digits = [0u8; 20];
        let mut at = digits.len();
        let mut rest = value.unsigned_abs();
        loop {
            at -= 1;
            digits[at] = b'0' + (rest % 10) as u8;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        if value < 0 {
            at -= 1;
            digits[at] = b'-';
        }
        self.write_number_bytes(buf, &digits[at..])
    }

    pub fn write_f64(&mut self, buf: &mut LoopBuf, value: f64) -> WriteResult<()> {
        if !value.is_finite() {
            return Err(WriteError::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("JSON cannot represent {value}"),
            )));
        }
        // almost every double fits a small stack buffer; one printed out in full
        // like 1e300 takes the allocating path
        let mut text = [0u8; 64];
        let mut cursor: &mut [u8] = &mut text;
        if write!(cursor, "{value}").is_ok() {
            let len = 64 - cursor.len();
            self.write_number_bytes(buf, &text[..len])
        } else {
            self.write_number_str(buf, &value.to_string())
        }
    }

    pub fn write_boolean(&mut self, buf: &mut LoopBuf, value: bool) -> WriteResult<()> {
        self.write_literal(buf, if value { b"true" } else { b"false" })
    }

    pub fn write_null(&mut self, buf: &mut LoopBuf) -> WriteResult<()> {
        self.write_literal(buf, b"null")
    }

    fn write_literal(&mut self, buf: &mut LoopBuf, text: &[u8]) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + text.len();
        ensure_room(buf, need, "value")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            out.put(text);
        });
        self.state = State::AfterValue;
        Ok(())
    }

    fn write_open(&mut self, buf: &mut LoopBuf, bracket: u8) -> WriteResult<()> {
        let prefix = self.prefix();
        let need = self.prefix_len(&prefix) + 1;
        ensure_room(buf, need, "value")?;
        emit!(buf, need, |out| {
            self.put_prefix(out, &prefix);
            out.put(&[bracket]);
        });
        self.state = State::Init;
        self.depth += 1;
        Ok(())
    }

    fn write_close(&mut self, buf: &mut LoopBuf, bracket: u8) -> WriteResult<()> {
        let depth = self.depth.saturating_sub(1);
        let indent = self.indent.map(|i| 1 + i.indent_count * depth);
        let need = indent.unwrap_or(0) + 1;
        ensure_room(buf, need, "value")?;
        emit!(buf, need, |out| {
            if let Some(indent) = self.indent {
                out.put(b"\n");
                put_repeated(out, indent.indent_char, indent.indent_count * depth);
            }
            out.put(&[bracket]);
        });
        self.state = State::AfterValue;
        self.depth = depth;
        Ok(())
    }

    /// The separator and indentation before a key or value, given where the
    /// writer is.
    fn prefix(&self) -> Prefix {
        let none = Prefix {
            comma: false,
            space: false,
            newline_depth: None,
        };
        match (self.indent, self.state) {
            (None, State::AfterValue) => Prefix { comma: true, ..none },
            (None, _) => none,
            (Some(_), State::AfterKey) => Prefix { space: true, ..none },
            (Some(_), State::AfterValue) => Prefix {
                comma: true,
                newline_depth: Some(self.depth),
                ..none
            },
            (Some(_), State::Init) => Prefix {
                newline_depth: Some(self.depth),
                ..none
            },
            (Some(_), State::Init0) => none,
        }
    }

    fn prefix_len(&self, prefix: &Prefix) -> usize {
        let indent = self.indent.map_or(0, |i| i.indent_count);
        usize::from(prefix.comma)
            + usize::from(prefix.space)
            + prefix.newline_depth.map_or(0, |depth| 1 + indent * depth)
    }

    /// Puts a prefix. The room has been checked.
    #[inline]
    fn put_prefix<O: Out>(&self, out: &mut O, prefix: &Prefix) {
        if prefix.comma {
            out.put(b",");
        }
        if prefix.space {
            out.put(b" ");
        }
        if let (Some(depth), Some(indent)) = (prefix.newline_depth, self.indent) {
            out.put(b"\n");
            put_repeated(out, indent.indent_char, indent.indent_count * depth);
        }
    }
}

fn would_block() -> WriteError {
    WriteError::new(io::Error::new(
        io::ErrorKind::WouldBlock,
        "LoopBuf has no room for the whole write: drain it and repeat the call",
    ))
}

/// Succeeds if `need` bytes can be appended now. Otherwise nothing has been
/// done: `WouldBlock` if draining the ring would make room, `InvalidInput` if
/// the ring's whole capacity is too small and it never will.
fn ensure_room(buf: &LoopBuf, need: usize, what: &str) -> WriteResult<()> {
    if need <= buf.spare_len() {
        return Ok(());
    }
    if need > buf.capacity() {
        let hint = if what == "string" || what == "string content" {
            "; write it in pieces with write_start_string"
        } else {
            ""
        };
        return Err(WriteError::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{what} needs {need} bytes but the LoopBuf's whole capacity is {}{hint}",
                buf.capacity()
            ),
        )));
    }
    Err(would_block())
}

/// Puts `byte` `count` times.
fn put_repeated<O: Out>(out: &mut O, byte: u8, count: usize) {
    const CHUNK: usize = 64;
    let run = [byte; CHUNK];
    let mut left = count;
    while left > 0 {
        let n = left.min(CHUNK);
        out.put(&run[..n]);
        left -= n;
    }
}

/// Length of `s` once escaped (not counting the quotes). Bytes of multi-byte
/// characters pass through unchanged.
fn escaped_len(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .map(|&b| match b {
            b'"' | b'\\' | 0x08 | 0x0c | b'\n' | b'\r' | b'\t' => 2,
            0..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

fn escaped_char_len(ch: char) -> usize {
    match ch {
        '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// Puts `s` in quotes, escaped. The room has been checked.
#[inline]
fn put_quoted<O: Out>(out: &mut O, s: &str) {
    out.put(b"\"");
    put_escaped(out, s);
    out.put(b"\"");
}

/// Puts `s` escaped. The room has been checked. Runs of plain bytes are
/// copied whole.
fn put_escaped<O: Out>(out: &mut O, s: &str) {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b >= 0x20 && b != b'"' && b != b'\\' {
            continue;
        }
        if start < i {
            out.put(&bytes[start..i]);
        }
        start = i + 1;
        match b {
            b'"' => out.put(b"\\\""),
            b'\\' => out.put(b"\\\\"),
            0x08 => out.put(b"\\b"),
            0x0c => out.put(b"\\f"),
            b'\n' => out.put(b"\\n"),
            b'\r' => out.put(b"\\r"),
            b'\t' => out.put(b"\\t"),
            c => out.put(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                hex_char(c >> 4),
                hex_char(c & 0xF),
            ]),
        };
    }
    if start < bytes.len() {
        out.put(&bytes[start..]);
    }
}

fn hex_char(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'a' + (n - 10)
    }
}
