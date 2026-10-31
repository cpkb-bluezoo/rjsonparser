//! Streaming JSON parser using the push (`receive`) model (Java `JSONParser` port).

use std::io::Read;
use std::rc::Rc;

use loopbuf::{LoopBuf, Spans};

use crate::error::{ParseError, ParseResult};
use crate::handler::ContentHandler;
use crate::keys::KeySymbolTable;
use crate::limits::ParserLimits;
use crate::locator::SharedLocator;
use crate::tokenizer::{NestToken, Tokenizer, MIN_DECODE_ROOM};

const DEFAULT_BUFFER_SIZE: usize = 8192;
const DEFAULT_STRING_CHUNK_SIZE: usize = 32768;

enum Bom {
    /// More bytes are needed to tell whether there is a byte order mark.
    NeedMore,
    /// Skip this many bytes (3 for a UTF-8 BOM, 0 if there is none).
    Skip(usize),
}

/// A streaming JSON parser using the push (`receive`) model.
///
/// Parsing events are delivered via [`ContentHandler`] as tokens are
/// recognized. The parser owns its handler, so it can live inside another
/// object (a network response handler, say) and be fed chunk by chunk. Pass
/// `&mut h` to keep a handler you own elsewhere, `Box<dyn ContentHandler>` for
/// a handler chosen at run time, or the handler itself; [`Self::handler`] and
/// [`Self::into_handler`] get it back.
///
/// # Buffering
///
/// [`Self::receive`] takes a caller-owned [`LoopBuf`] by reference: the same
/// ring across every call for one document (fill it, call `receive`, fill it
/// some more, repeat). The parser consumes whole tokens, scanned straight out
/// of the ring with no copy, and holds none of the document's bytes itself.
/// Anything it cannot use yet (a key or literal that is not finished, the
/// start of an escape sequence or multi-byte character, a partial byte order
/// mark) is left unconsumed in the ring, which will then hold it together with
/// whatever the caller reads in next, to be presented again. After `receive`
/// returns, the ring's length is exactly how much was left.
///
/// The ring must be larger than the longest key (see [`ParserLimits::max_name_length`])
/// plus its two quotes. If it is full of one unfinished token, `receive`
/// returns an error rather than waiting for data that cannot fit.
///
/// # Memory
///
/// Memory use does not grow with document size or string length. Besides the
/// caller's ring the parser has one buffer, for characters, of a fixed size
/// ([`Self::set_string_chunk_size`], 32768 bytes by default). String content is
/// decoded into it and handed to the handler as a `&str` as it arrives, so a
/// string of any length takes the same memory. Keys are delivered whole, so a
/// key stays in the ring until complete and must also fit that buffer.
pub struct Parser<H: ContentHandler> {
    handler: H,
    key_symbol_table: KeySymbolTable,
    limits: ParserLimits,
    locator: Rc<SharedLocator>,
    reject_duplicate_keys: bool,
    checked_bom: bool,
    closed: bool,
    document_length: u64,
    /// How many bytes the last `receive` left in the ring. They are presented
    /// again, so they were already counted towards `document_length`.
    unconsumed: usize,
    /// Only used to size [`Self::parse`]'s own internal ring; `receive`
    /// itself has no buffer size of its own to configure, since the ring it
    /// scans is whatever capacity the caller chose.
    buffer_size: usize,
    string_chunk_size: usize,
    underflow: bool,
    tokenizer: Option<Tokenizer>,
}

impl<H: ContentHandler> Parser<H> {
    /// Creates a new JSON parser. `buffer_size` only sizes the ring
    /// [`Self::parse`] allocates for itself; [`Self::receive`] scans
    /// whatever ring the caller passes it, of whatever capacity they chose.
    pub fn new(handler: H) -> Self {
        Self::with_buffer_size(handler, DEFAULT_BUFFER_SIZE)
    }

    /// Creates a new JSON parser with the specified buffer size (see [`Self::new`]).
    pub fn with_buffer_size(handler: H, buffer_size: usize) -> Self {
        assert!(buffer_size > 0, "Buffer size must be positive");
        Self {
            handler,
            key_symbol_table: KeySymbolTable::new(),
            limits: ParserLimits::default(),
            locator: Rc::new(SharedLocator::new()),
            reject_duplicate_keys: false,
            checked_bom: false,
            closed: false,
            document_length: 0,
            unconsumed: 0,
            buffer_size,
            string_chunk_size: DEFAULT_STRING_CHUNK_SIZE,
            underflow: false,
            tokenizer: None,
        }
    }

    /// The handler the parser is driving.
    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Mutable access to the handler between chunks.
    pub fn handler_mut(&mut self) -> &mut H {
        &mut self.handler
    }

    /// Take the handler back, dropping the parser.
    pub fn into_handler(self) -> H {
        self.handler
    }

    /// Sets whether a repeated key within the same object should be treated as a parse error.
    pub fn set_reject_duplicate_keys(&mut self, reject_duplicate_keys: bool) {
        self.reject_duplicate_keys = reject_duplicate_keys;
    }

    /// The limits in force.
    pub fn limits(&self) -> &ParserLimits {
        &self.limits
    }

    /// Mutable access to the limits, to change one without restating the rest.
    pub fn limits_mut(&mut self) -> &mut ParserLimits {
        &mut self.limits
    }

    /// Replaces the limits. See [`ParserLimits`] for what each one guards and
    /// its default.
    pub fn set_limits(&mut self, limits: ParserLimits) {
        self.limits = limits;
    }

    /// Sets the size in bytes of the buffer that string content is decoded
    /// into, which is also the most bytes one
    /// [`ContentHandler::string_value_content`] call carries. Default 32768.
    /// However large the ring passed to [`Self::receive`] is, a string is
    /// decoded and delivered in pieces of at most this size, so the parser's
    /// memory for strings is fixed by this setting. A larger value means fewer
    /// calls to the handler; a smaller one, a smaller buffer. A key must also
    /// fit it.
    ///
    /// The value must be at least 16, enough for the longest escape sequence or
    /// multi-byte character. It takes effect from the next string, so it may be
    /// changed at any time.
    ///
    /// # Panics
    ///
    /// If `string_chunk_size` is less than 16.
    pub fn set_string_chunk_size(&mut self, string_chunk_size: usize) {
        assert!(
            string_chunk_size >= MIN_DECODE_ROOM,
            "String chunk size must be at least {MIN_DECODE_ROOM}"
        );
        self.string_chunk_size = string_chunk_size;
        if let Some(tokenizer) = self.tokenizer.as_mut() {
            tokenizer.set_chunk_size(string_chunk_size);
        }
    }

    /// Returns true when the previous `receive` left bytes unconsumed in the ring.
    pub fn is_underflow(&self) -> bool {
        self.underflow
    }

    /// Receive bytes into the parser.
    ///
    /// `buf` is caller-owned: write into its spare capacity
    /// (`spare_capacity_mut`/`commit_write`, or `extend_from_slice`) before
    /// calling, the same ring every time for one document. Tokens are
    /// processed as they complete and their events delivered to the handler.
    /// After this returns, `buf` holds exactly what the parser could not use
    /// yet; read more in after it and call this again. [`Self::close`] takes
    /// the same ring once there is no more data coming.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] if the input is malformed, a limit is exceeded, the
    /// parser is closed, or `buf` is full of a single token that is not yet
    /// complete (it can never fit).
    pub fn receive(&mut self, buf: &mut LoopBuf) -> ParseResult<()> {
        if self.closed {
            return Err(ParseError::new("Cannot receive data after close()"));
        }
        self.feed(buf, true)
    }

    fn feed(&mut self, buf: &mut LoopBuf, check_room: bool) -> ParseResult<()> {
        let available = buf.len();
        if available == 0 {
            return Ok(());
        }

        if let Some(max) = self.limits.max_document_length {
            // The unconsumed bytes presented again were counted when they first arrived
            self.document_length += available.saturating_sub(self.unconsumed) as u64;
            if self.document_length > max {
                return Err(ParseError::new(format!(
                    "Maximum document length exceeded: {max}"
                )));
            }
        }

        let mut consumed = 0;
        let result = self.process(buf.spans(), &mut consumed);
        buf.advance(consumed);
        result?;

        self.unconsumed = buf.len();
        self.underflow = self.unconsumed > 0;
        // If it consumed nothing and the ring has no room for more, the caller
        // can never supply the rest, so say so rather than loop forever
        if check_room && consumed == 0 && buf.is_full() {
            return Err(ParseError::new(format!(
                "Receive buffer too small: the next token needs more than the {} bytes it holds. \
                 A key must fit the buffer (see ParserLimits::max_name_length).",
                buf.len()
            )));
        }
        Ok(())
    }

    /// Checks the BOM on the first call, creates the tokenizer if needed, then
    /// hands it what is in `input`. `consumed` is advanced past whatever was
    /// used, even if an error is returned.
    fn process(&mut self, input: Spans<'_>, consumed: &mut usize) -> ParseResult<()> {
        let mut input = input;
        if !self.checked_bom {
            match check_bom(input)? {
                Bom::NeedMore => return Ok(()),
                Bom::Skip(n) => {
                    self.checked_bom = true;
                    *consumed += n;
                    input = input.skip(n);
                }
            }
        }

        if self.tokenizer.is_none() {
            let needs_whitespace = self.handler.needs_whitespace();
            self.handler.set_locator(self.locator.clone());
            self.tokenizer = Some(Tokenizer::new(
                self.reject_duplicate_keys,
                needs_whitespace,
                self.string_chunk_size,
            ));
        }

        let tokenizer = self.tokenizer.as_mut().expect("tokenizer just created");
        let mut used = 0;
        let result = tokenizer.receive(
            &mut self.handler,
            &mut self.key_symbol_table,
            &self.limits,
            &self.locator,
            input,
            &mut used,
        );
        *consumed += used;
        result
    }

    /// Close the parser, signaling end of input.
    ///
    /// `buf` is the same ring passed to [`Self::receive`] throughout. A number
    /// that ends the document (such as the whole document `42`) has no
    /// delimiter to show where it ends, so it is reported by this call. Bytes
    /// still left unconsumed are an incomplete token and are rejected, as is
    /// anything else that leaves the document unfinished. After closing,
    /// further calls to [`Self::receive`] will return an error. Use
    /// [`Self::reset`] to parse a new document.
    pub fn close(&mut self, buf: &mut LoopBuf) -> ParseResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        if self.tokenizer.is_none() {
            // bytes presented here and not yet received are given their chance
            self.feed(buf, false)?;
            if self.tokenizer.is_none() {
                return Err(ParseError::new("No data"));
            }
        } else if !buf.is_empty() {
            self.feed(buf, false)?;
        }

        // Bytes left unconsumed can only have been the start of a token that
        // now can never be completed
        if !buf.is_empty() {
            return Err(ParseError::new(
                "Unclosed string or incomplete token at end of input",
            ));
        }

        let tokenizer = self.tokenizer.as_mut().expect("checked above");
        // Completes a number that ends the document and rejects an unfinished
        // string or number
        tokenizer.end_of_input(&mut self.handler, &self.locator)?;

        if !tokenizer.seen_any_token {
            return Err(ParseError::new("No data"));
        }

        if let Some(unclosed) = tokenizer.depth_stack.last() {
            return Err(ParseError::new(match unclosed {
                NestToken::StartObject => "Unclosed object",
                NestToken::StartArray => "Unclosed array",
            }));
        }

        Ok(())
    }

    /// Reset the parser to parse a new document.
    ///
    /// Clears internal state but keeps the key symbol table warm. The ring
    /// passed to [`Self::receive`] is caller-owned and not this parser's
    /// concern to reset.
    pub fn reset(&mut self) {
        self.tokenizer = None;
        self.locator = Rc::new(SharedLocator::new());
        self.checked_bom = false;
        self.closed = false;
        self.document_length = 0;
        self.unconsumed = 0;
        self.underflow = false;
    }

    /// Parse a JSON document from a [`Read`] source.
    ///
    /// The parser is automatically reset before parsing. Allocates one ring
    /// (sized by [`Self::with_buffer_size`]) and never grows it: reads from
    /// `reader` into its spare capacity and calls [`Self::receive`] until EOF,
    /// then [`Self::close`]. A key longer than the ring is reported as a
    /// buffer-too-small error. The reader is not closed.
    pub fn parse<R: Read>(&mut self, mut reader: R) -> ParseResult<()> {
        self.reset();

        let mut buf = LoopBuf::with_capacity(self.buffer_size);
        loop {
            let (spare, _) = buf.spare_capacity_mut();
            if spare.is_empty() {
                // `receive` reports a ring full of one unfinished token, so
                // this is not reached; it is here so a bug cannot loop forever
                return Err(ParseError::new("Receive buffer too small"));
            }
            let bytes_read = reader
                .read(spare)
                .map_err(|e| ParseError::new(format!("I/O error reading stream: {e}")))?;
            if bytes_read == 0 {
                break;
            }
            buf.commit_write(bytes_read);
            self.receive(&mut buf)?;
        }

        self.close(&mut buf)
    }
}

/// Checks for a byte order mark. Optimized for UTF-8, and detects UTF-16/32
/// only to reject them.
fn check_bom(input: Spans<'_>) -> ParseResult<Bom> {
    let remaining = input.len();
    if remaining == 0 {
        return Ok(Bom::NeedMore);
    }
    let b1 = input[0];

    match b1 {
        // UTF-8 BOM (EF BB BF)
        0xEF => {
            if remaining >= 3 {
                if input[1] == 0xBB && input[2] == 0xBF {
                    return Ok(Bom::Skip(3));
                }
            } else if remaining == 1 || input[1] == 0xBB {
                return Ok(Bom::NeedMore); // might be a BOM
            }
            Ok(Bom::Skip(0))
        }
        // UTF-16 BE: FE FF
        0xFE => {
            if remaining < 2 {
                return Ok(Bom::NeedMore);
            }
            if input[1] == 0xFF {
                return Err(ParseError::new("UTF-16 BE encoding not supported"));
            }
            Ok(Bom::Skip(0))
        }
        // UTF-16 LE or UTF-32 LE: FF FE (4 bytes to tell them apart)
        0xFF => {
            if remaining < 2 {
                return Ok(Bom::NeedMore);
            }
            if input[1] == 0xFE {
                if remaining < 4 {
                    return Ok(Bom::NeedMore);
                }
                if input[2] == 0x00 && input[3] == 0x00 {
                    return Err(ParseError::new("UTF-32 LE encoding not supported"));
                }
                return Err(ParseError::new("UTF-16 LE encoding not supported"));
            }
            Ok(Bom::Skip(0))
        }
        // UTF-32 BE: 00 00 FE FF
        0x00 => {
            if remaining < 2 {
                return Ok(Bom::NeedMore);
            }
            if input[1] == 0x00 {
                if remaining < 4 {
                    return Ok(Bom::NeedMore);
                }
                if input[2] == 0xFE && input[3] == 0xFF {
                    return Err(ParseError::new("UTF-32 BE encoding not supported"));
                }
            }
            Ok(Bom::Skip(0))
        }
        _ => Ok(Bom::Skip(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::DefaultHandler;

    #[test]
    fn only_short_keys_are_interned() {
        // The same 30 short and 50 long (200 byte) keys in every object, so
        // interning pays off and stays on. Long keys must not be kept: the
        // table lives as long as the parser, and holds up to 10,000 keys.
        let mut doc = String::from("[");
        for object in 0..100 {
            doc.push_str(if object == 0 { "{" } else { ",{" });
            for k in 0..30 {
                if k > 0 {
                    doc.push(',');
                }
                doc.push_str(&format!("\"short{k}\":1"));
            }
            for k in 0..50 {
                doc.push_str(&format!(",\"long-{k}-{}\":1", "x".repeat(200)));
            }
            doc.push('}');
        }
        doc.push(']');

        let mut parser = Parser::with_buffer_size(DefaultHandler, 65536);
        parser.parse(doc.as_bytes()).unwrap();
        assert_eq!(parser.key_symbol_table.len(), 30, "only the short keys are interned");
    }
}
