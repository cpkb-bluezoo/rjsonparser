//! Streaming JSON parser using the push (`receive`) model (Java `JSONParser` port).

use std::io::Read;

use crate::error::{ParseError, ParseResult};
use crate::handler::ContentHandler;
use crate::keys::KeySymbolTable;
use crate::limits::ParserLimits;
use crate::tokenizer::{NestToken, SharedLocator, Tokenizer};

const DEFAULT_BUFFER_SIZE: usize = 8192;

enum BomCheck {
    Done,
    NeedMore,
}

/// A streaming JSON parser using the push (`receive`) model.
///
/// Parsing events are delivered via [`ContentHandler`] as tokens are recognized.
/// The parser handles BOM detection, then delegates to an internal tokenizer.
pub struct Parser<'a, H: ContentHandler + ?Sized> {
    handler: &'a mut H,
    leftover: Vec<u8>,
    key_symbol_table: KeySymbolTable,
    limits: ParserLimits,
    locator: SharedLocator,
    reject_duplicate_keys: bool,
    checked_bom: bool,
    closed: bool,
    document_length: i64,
    buffer_size: usize,
    underflow: bool,
    tokenizer: Option<Tokenizer>,
}

impl<'a, H: ContentHandler + ?Sized> Parser<'a, H> {
    /// Creates a new JSON parser with a default buffer size of 8KB.
    pub fn new(handler: &'a mut H) -> Self {
        Self::with_buffer_size(handler, DEFAULT_BUFFER_SIZE)
    }

    /// Creates a new JSON parser with the specified buffer size.
    pub fn with_buffer_size(handler: &'a mut H, buffer_size: usize) -> Self {
        assert!(buffer_size > 0, "Buffer size must be positive");
        Self {
            handler,
            leftover: Vec::new(),
            key_symbol_table: KeySymbolTable::new(),
            limits: ParserLimits::default(),
            locator: SharedLocator::new(1, 0),
            reject_duplicate_keys: false,
            checked_bom: false,
            closed: false,
            document_length: 0,
            buffer_size,
            underflow: false,
            tokenizer: None,
        }
    }

    /// Sets whether a repeated key within the same object should be treated as a parse error.
    pub fn set_reject_duplicate_keys(&mut self, reject_duplicate_keys: bool) {
        self.reject_duplicate_keys = reject_duplicate_keys;
    }

    /// Sets the maximum object/array nesting depth.
    pub fn set_max_nesting_depth(&mut self, max_nesting_depth: i32) {
        self.limits.max_nesting_depth = max_nesting_depth;
    }

    /// Sets the maximum number of characters in a single number token.
    pub fn set_max_number_length(&mut self, max_number_length: i32) {
        self.limits.max_number_length = max_number_length;
    }

    /// Sets the maximum number of characters in a single string value.
    pub fn set_max_string_length(&mut self, max_string_length: i32) {
        self.limits.max_string_length = max_string_length;
    }

    /// Sets the maximum number of characters in a single object key.
    pub fn set_max_name_length(&mut self, max_name_length: i32) {
        self.limits.max_name_length = max_name_length;
    }

    /// Sets the maximum total number of bytes in one document.
    pub fn set_max_document_length(&mut self, max_document_length: i64) {
        self.limits.max_document_length = max_document_length;
    }

    /// Sets the maximum total number of tokens in one document.
    pub fn set_max_token_count(&mut self, max_token_count: i64) {
        self.limits.max_token_count = max_token_count;
    }

    /// Disables every configurable limit in one call.
    pub fn disable_all_limits(&mut self) {
        self.limits.disable_all();
    }

    /// Returns true when the previous `receive` left an incomplete trailing token.
    pub fn is_underflow(&self) -> bool {
        self.underflow
    }

    /// Receive bytes into the parser.
    ///
    /// After this method returns, `*data` is advanced to the first unconsumed byte.
    /// Unconsumed bytes are retained internally for the next call and for [`Self::close`].
    pub fn receive(&mut self, data: &mut &[u8]) -> ParseResult<()> {
        if self.closed {
            return Err(ParseError::new("Cannot receive data after close()"));
        }

        if data.is_empty() && self.leftover.is_empty() {
            return Ok(());
        }

        if self.limits.max_document_length > 0 && !data.is_empty() {
            self.document_length += data.len() as i64;
            if self.document_length > self.limits.max_document_length {
                return Err(ParseError::new(format!(
                    "Maximum document length exceeded: {}",
                    self.limits.max_document_length
                )));
            }
        }

        let merged = !self.leftover.is_empty();
        if merged {
            self.leftover.extend_from_slice(data);
            *data = &[];
            let buf = std::mem::take(&mut self.leftover);
            let mut work = buf.as_slice();
            self.process_chunk(&mut work)?;
            self.leftover = work.to_vec();
        } else {
            let mut work = *data;
            self.process_chunk(&mut work)?;
            *data = work;
            self.leftover.clear();
            self.leftover.extend_from_slice(work);
        }

        self.underflow = !self.leftover.is_empty();
        Ok(())
    }

    fn process_chunk(&mut self, work: &mut &[u8]) -> ParseResult<()> {
        if !self.checked_bom {
            match check_bom(work)? {
                BomCheck::NeedMore => return Ok(()),
                BomCheck::Done => self.checked_bom = true,
            }
        }

        if self.tokenizer.is_none() {
            let needs_whitespace = self.handler.needs_whitespace();
            self.handler.set_locator(&self.locator);
            self.tokenizer = Some(Tokenizer::new(
                self.reject_duplicate_keys,
                needs_whitespace,
            ));
        }

        let tok = self.tokenizer.as_mut().expect("tokenizer just created");
        tok.receive(
            self.handler,
            &mut self.key_symbol_table,
            &self.limits,
            &self.locator,
            work,
        )
    }

    /// Close the parser, signaling end of input.
    ///
    /// Validates that the JSON document is complete. After closing, further calls to
    /// [`Self::receive`] will return an error. Use [`Self::reset`] to parse a new document.
    pub fn close(&mut self) -> ParseResult<()> {
        if self.closed {
            return Ok(());
        }

        self.closed = true;

        let Some(tok) = self.tokenizer.as_mut() else {
            return Err(ParseError::new("No data"));
        };

        tok.set_closed(true);

        if !self.leftover.is_empty() {
            let mut work = self.leftover.as_slice();
            tok.receive(
                self.handler,
                &mut self.key_symbol_table,
                &self.limits,
                &self.locator,
                &mut work,
            )?;

            if !work.is_empty() {
                return Err(ParseError::new(
                    "Unclosed string or incomplete token at end of input",
                ));
            }
            self.leftover.clear();
        }

        if !tok.seen_any_token {
            return Err(ParseError::new("No data"));
        }

        if let Some(unclosed) = tok.depth_stack.last() {
            return Err(ParseError::new(match unclosed {
                NestToken::StartObject => "Unclosed object",
                NestToken::StartArray => "Unclosed array",
            }));
        }

        Ok(())
    }

    /// Reset the parser to parse a new document.
    ///
    /// Clears leftover bytes and tokenizer state but keeps the key symbol table warm.
    pub fn reset(&mut self) {
        self.leftover.clear();
        self.tokenizer = None;
        self.locator = SharedLocator::new(1, 0);
        self.checked_bom = false;
        self.closed = false;
        self.document_length = 0;
        self.underflow = false;
    }

    /// Parse a JSON document from a [`Read`] source.
    ///
    /// The parser is automatically reset before parsing. The stream is read until EOF,
    /// then [`Self::close`] is called. The reader is not closed.
    pub fn parse<R: Read>(&mut self, mut reader: R) -> ParseResult<()> {
        self.reset();

        let mut buf = vec![0u8; self.buffer_size];
        loop {
            let bytes_read = reader.read(&mut buf).map_err(|e| {
                ParseError::new(format!("I/O error reading stream: {e}"))
            })?;
            if bytes_read == 0 {
                break;
            }

            let mut slice = &buf[..bytes_read];
            self.receive(&mut slice)?;
        }

        if !self.leftover.is_empty() {
            let mut empty = &[][..];
            self.receive(&mut empty)?;
        }

        self.close()
    }
}

fn check_bom(data: &mut &[u8]) -> ParseResult<BomCheck> {
    let remaining = data.len();

    if remaining == 0 {
        return Ok(BomCheck::NeedMore);
    }

    let b1 = data[0];

    // Fast path: UTF-8 BOM (EF BB BF)
    if b1 == 0xEF {
        if remaining < 3 {
            if remaining >= 2 && data[1] == 0xBB {
                return Ok(BomCheck::NeedMore);
            }
            if remaining == 1 {
                return Ok(BomCheck::NeedMore);
            }
        } else if data[1] == 0xBB && data[2] == 0xBF {
            *data = &data[3..];
            return Ok(BomCheck::Done);
        }
        return Ok(BomCheck::Done);
    }

    // UTF-16 BE: FE FF
    if b1 == 0xFE {
        if remaining < 2 {
            return Ok(BomCheck::NeedMore);
        }
        if data[1] == 0xFF {
            return Err(ParseError::new("UTF-16 BE encoding not supported"));
        }
        return Ok(BomCheck::Done);
    }

    // UTF-16 LE or UTF-32 LE: FF FE ...
    if b1 == 0xFF {
        if remaining < 2 {
            return Ok(BomCheck::NeedMore);
        }
        if data[1] == 0xFE {
            if remaining < 4 {
                return Ok(BomCheck::NeedMore);
            }
            if data[2] == 0x00 && data[3] == 0x00 {
                return Err(ParseError::new("UTF-32 LE encoding not supported"));
            }
            return Err(ParseError::new("UTF-16 LE encoding not supported"));
        }
        return Ok(BomCheck::Done);
    }

    // UTF-32 BE: 00 00 FE FF
    if b1 == 0x00 {
        if remaining < 2 {
            return Ok(BomCheck::NeedMore);
        }
        if data[1] == 0x00 {
            if remaining < 4 {
                return Ok(BomCheck::NeedMore);
            }
            if data[2] == 0xFE && data[3] == 0xFF {
                return Err(ParseError::new("UTF-32 BE encoding not supported"));
            }
        }
        return Ok(BomCheck::Done);
    }

    Ok(BomCheck::Done)
}
