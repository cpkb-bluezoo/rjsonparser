//! JSON tokenizer operating on byte slices for streaming parsing (Java `JSONTokenizer` port).

use std::cell::Cell;
use std::collections::HashSet;

use crate::error::{ParseError, ParseResult};
use crate::handler::ContentHandler;
use crate::keys::KeySymbolTable;
use crate::limits::ParserLimits;
use crate::locator::Locator;
use crate::number::Number;

const KEY_INTERN_DISABLE_THRESHOLD: i32 = 64;

/// Nesting token pushed onto [`Tokenizer::depth_stack`] (Java `Token`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NestToken {
    StartObject,
    StartArray,
}

/// Live line/column storage for handler [`Locator`] callbacks (updated as bytes are consumed).
#[derive(Debug, Default)]
pub(crate) struct SharedLocator {
    line: Cell<usize>,
    column: Cell<usize>,
}

impl SharedLocator {
    pub(crate) fn new(line: usize, column: usize) -> Self {
        Self {
            line: Cell::new(line),
            column: Cell::new(column),
        }
    }

    fn set_line(&self, line: usize) {
        self.line.set(line);
    }

    fn set_column(&self, column: usize) {
        self.column.set(column);
    }

    fn add_column(&self, delta: usize) {
        self.column.set(self.column.get() + delta);
    }
}

impl Locator for SharedLocator {
    fn line_number(&self) -> usize {
        self.line.get()
    }

    fn column_number(&self) -> usize {
        self.column.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    Array,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    ExpectValue,
    ExpectKey,
    ExpectColon,
    AfterValue,
}

/// Streaming JSON tokenizer calling [`ContentHandler`] methods as tokens are recognized.
pub(crate) struct Tokenizer {
    reject_duplicate_keys: bool,
    needs_whitespace: bool,
    closed: bool,

    context_stack: Vec<Context>,
    state: State,

    pub seen_any_token: bool,
    pub depth_stack: Vec<NestToken>,
    after_comma: bool,

    key_intern_attempts: i32,
    key_intern_hits: i32,
    key_intern_disabled: bool,

    duplicate_key_stack: Vec<HashSet<String>>,

    token_count: i64,

    /// Reused buffer for strings containing escape sequences (UTF-16 code units).
    escape_units: Vec<u16>,
}

impl Tokenizer {
    pub fn new(reject_duplicate_keys: bool, needs_whitespace: bool) -> Self {
        Self {
            reject_duplicate_keys,
            needs_whitespace,
            closed: false,
            context_stack: Vec::new(),
            state: State::ExpectValue,
            seen_any_token: false,
            depth_stack: Vec::new(),
            after_comma: false,
            key_intern_attempts: 0,
            key_intern_hits: 0,
            key_intern_disabled: false,
            duplicate_key_stack: Vec::new(),
            token_count: 0,
            escape_units: Vec::new(),
        }
    }

    pub fn set_closed(&mut self, closed: bool) {
        self.closed = closed;
    }

    /// Process as many complete tokens as possible from the front of `data`.
    pub fn receive<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        data: &mut &[u8],
    ) -> ParseResult<()> {
        while !data.is_empty() {
            let save_line = locator.line_number();
            let save_column = locator.column_number();
            let token_start = *data;

            let b = data[0];
            *data = &data[1..];

            let is_whitespace = matches!(b, b' ' | b'\n' | b'\t' | b'\r');

            let processed = self.process_token(handler, key_symbol_table, limits, locator, b, data)?;

            if !processed {
                *data = token_start;
                locator.set_line(save_line);
                locator.set_column(save_column);
                return Ok(());
            }

            if !is_whitespace && b != b'"' {
                let bytes_consumed = token_start.len() - data.len();
                locator.add_column(bytes_consumed);
            }
        }
        Ok(())
    }

    fn check_token_limit(&mut self, limits: &ParserLimits) -> ParseResult<()> {
        let max = limits.max_token_count;
        if max > 0 {
            self.token_count += 1;
            if self.token_count > max {
                return Err(ParseError::new(format!(
                    "Maximum token count exceeded: {max}"
                )));
            }
        }
        Ok(())
    }

    fn process_token<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        b: u8,
        data: &mut &[u8],
    ) -> ParseResult<bool> {
        match b {
            b'"' => {
                if matches!(self.state, State::ExpectColon | State::AfterValue) {
                    return Err(ParseError::new("Unexpected string"));
                }
                let is_key = self.state == State::ExpectKey;
                if !self.process_string(handler, key_symbol_table, limits, locator, data, is_key)? {
                    return Ok(false);
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = if is_key {
                    State::ExpectColon
                } else {
                    State::AfterValue
                };
                Ok(true)
            }

            b',' => {
                if self.state != State::AfterValue {
                    return Err(ParseError::new("Unexpected ','"));
                }
                if self.context_stack.is_empty() {
                    return Err(ParseError::new("Unexpected comma at root level"));
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = true;
                self.state = if self.context_stack.last() == Some(&Context::Object) {
                    State::ExpectKey
                } else {
                    State::ExpectValue
                };
                Ok(true)
            }

            b':' => {
                if self.state != State::ExpectColon {
                    return Err(ParseError::new("Unexpected ':'"));
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::ExpectValue;
                Ok(true)
            }

            b'{' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected '{'"));
                }
                let max_depth = limits.max_nesting_depth;
                if max_depth > 0 && self.depth_stack.len() >= max_depth as usize {
                    return Err(ParseError::new(format!(
                        "Maximum nesting depth exceeded: {max_depth}"
                    )));
                }
                handler.start_object()?;
                self.context_stack.push(Context::Object);
                self.depth_stack.push(NestToken::StartObject);
                if self.reject_duplicate_keys {
                    self.duplicate_key_stack.push(HashSet::new());
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::ExpectKey;
                Ok(true)
            }

            b'}' => {
                if self.context_stack.last() != Some(&Context::Object) {
                    return Err(ParseError::new("Unexpected '}'"));
                }
                if self.state != State::ExpectKey && self.state != State::AfterValue {
                    return Err(ParseError::new("Unexpected '}'"));
                }
                if self.after_comma {
                    return Err(ParseError::new("Trailing comma before '}'"));
                }
                handler.end_object()?;
                self.context_stack.pop();
                self.depth_stack.pop();
                if self.reject_duplicate_keys {
                    self.duplicate_key_stack.pop();
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b'[' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected '['"));
                }
                let max_depth = limits.max_nesting_depth;
                if max_depth > 0 && self.depth_stack.len() >= max_depth as usize {
                    return Err(ParseError::new(format!(
                        "Maximum nesting depth exceeded: {max_depth}"
                    )));
                }
                handler.start_array()?;
                self.context_stack.push(Context::Array);
                self.depth_stack.push(NestToken::StartArray);
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::ExpectValue;
                Ok(true)
            }

            b']' => {
                if self.context_stack.last() != Some(&Context::Array) {
                    return Err(ParseError::new("Unexpected ']'"));
                }
                if self.state != State::ExpectValue && self.state != State::AfterValue {
                    return Err(ParseError::new("Unexpected ']'"));
                }
                if self.after_comma {
                    return Err(ParseError::new("Trailing comma before ']'"));
                }
                handler.end_array()?;
                self.context_stack.pop();
                self.depth_stack.pop();
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b' ' | b'\n' | b'\t' | b'\r' => self.process_whitespace(handler, locator, b, data),

            b't' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'true'"));
                }
                if !self.process_literal(handler, data, b"rue", Some(true))? {
                    return Ok(false);
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b'f' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'false'"));
                }
                if !self.process_literal(handler, data, b"alse", Some(false))? {
                    return Ok(false);
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b'n' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'null'"));
                }
                if !self.process_literal(handler, data, b"ull", None)? {
                    return Ok(false);
                }
                self.seen_any_token = true;
                self.check_token_limit(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            _ => {
                if b == b'-' || (b >= b'0' && b <= b'9') {
                    if self.state != State::ExpectValue {
                        return Err(ParseError::new("Unexpected number"));
                    }
                    if !self.process_number(handler, limits, b, data)? {
                        return Ok(false);
                    }
                    self.seen_any_token = true;
                    self.check_token_limit(limits)?;
                    self.after_comma = false;
                    self.state = State::AfterValue;
                    return Ok(true);
                }
                Err(ParseError::new(format!(
                    "Unexpected character: {}",
                    b as char
                )))
            }
        }
    }

    fn process_string<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        data: &mut &[u8],
        is_key: bool,
    ) -> ParseResult<bool> {
        let content_start = *data;
        let start_pos = 0usize;
        let mut using_escape = false;
        let mut span_start = start_pos;
        let mut src_char_count = 0usize;
        let mut saw_non_ascii = false;

        let try_intern = is_key && !self.key_intern_disabled;
        let mut key_hash = if try_intern {
            KeySymbolTable::initial_hash()
        } else {
            0
        };

        let length_limit = if is_key {
            limits.max_name_length
        } else {
            limits.max_string_length
        };

        loop {
            if data.is_empty() {
                if !self.closed {
                    return Ok(false);
                }
                return Err(ParseError::new("Unclosed string"));
            }

            let byte_pos = content_start.len() - data.len();
            let b = data[0];
            *data = &data[1..];

            if length_limit > 0 && byte_pos - start_pos > length_limit as usize {
                return Err(ParseError::new(format!(
                    "{}{length_limit}",
                    if is_key {
                        "Maximum key length exceeded: "
                    } else {
                        "Maximum string length exceeded: "
                    }
                )));
            }

            if b == b'"' {
                let end_pos = byte_pos;
                let value = if !using_escape {
                    let key_bytes = &content_start[..end_pos];
                    if try_intern {
                        self.key_intern_attempts += 1;
                        if let Some(found) = key_symbol_table.lookup(key_bytes, key_hash) {
                            self.key_intern_hits += 1;
                            found.to_owned()
                        } else {
                            let decoded = decode_span(key_bytes, saw_non_ascii)?;
                            key_symbol_table.put(key_bytes, key_hash, decoded.clone());
                            decoded
                        }
                    } else {
                        decode_span(key_bytes, saw_non_ascii)?
                    }
                } else {
                    if end_pos > span_start {
                        let tail = decode_span(&content_start[span_start..end_pos], saw_non_ascii)?;
                        src_char_count += append_string_as_utf16(&mut self.escape_units, &tail);
                    }
                    String::from_utf16_lossy(&self.escape_units)
                };

                if try_intern {
                    if self.key_intern_hits == 0
                        && self.key_intern_attempts >= KEY_INTERN_DISABLE_THRESHOLD
                    {
                        self.key_intern_disabled = true;
                    }
                }

                if !using_escape {
                    src_char_count = value.encode_utf16().count();
                }

                if is_key {
                    if self.reject_duplicate_keys {
                        let set = self
                            .duplicate_key_stack
                            .last_mut()
                            .expect("duplicate key stack out of sync");
                        if !set.insert(value.clone()) {
                            return Err(ParseError::new(format!("Duplicate key: {value}")));
                        }
                    }
                    handler.key(&value)?;
                } else {
                    handler.string_value(&value)?;
                }

                locator.add_column(2 + src_char_count);
                if using_escape {
                    self.escape_units.clear();
                }
                return Ok(true);
            }

            if b & 0x80 != 0 {
                saw_non_ascii = true;
            }
            if try_intern {
                key_hash = KeySymbolTable::hash_byte(key_hash, b);
            }

            if b == b'\\' {
                if !using_escape {
                    if self.escape_units.capacity() > 16384 {
                        self.escape_units = Vec::with_capacity(256);
                    } else {
                        self.escape_units.clear();
                    }
                    using_escape = true;
                    span_start = start_pos;
                }

                if byte_pos > span_start {
                    let span = decode_span(&content_start[span_start..byte_pos], saw_non_ascii)?;
                    src_char_count += append_string_as_utf16(&mut self.escape_units, &span);
                }

                let escape_start = byte_pos;
                match self.process_escape_sequence(data)? {
                    None => return Ok(false),
                    Some(code_unit) => {
                        self.escape_units.push(code_unit);
                        let escape_end = content_start.len() - data.len();
                        src_char_count += escape_end - escape_start;
                        span_start = escape_end;
                    }
                }
            } else if b < 0x20 {
                return Err(ParseError::new("Unescaped control character in string"));
            }
        }
    }

    fn process_escape_sequence(&self, data: &mut &[u8]) -> ParseResult<Option<u16>> {
        if data.is_empty() {
            if !self.closed {
                return Ok(None);
            }
            return Err(ParseError::new("Unexpected EOF in escape sequence"));
        }

        let b = data[0];
        *data = &data[1..];

        match b {
            b'"' => Ok(Some(b'"' as u16)),
            b'\\' => Ok(Some(b'\\' as u16)),
            b'/' => Ok(Some(b'/' as u16)),
            b'b' => Ok(Some(b'\x08' as u16)),
            b'f' => Ok(Some(b'\x0c' as u16)),
            b'n' => Ok(Some(b'\n' as u16)),
            b'r' => Ok(Some(b'\r' as u16)),
            b't' => Ok(Some(b'\t' as u16)),
            b'u' => self.process_unicode_escape(data),
            _ => Err(ParseError::new(format!(
                "Invalid escape sequence: \\{}",
                b as char
            ))),
        }
    }

    fn process_unicode_escape(&self, data: &mut &[u8]) -> ParseResult<Option<u16>> {
        let mut value: u32 = 0;

        for _ in 0..4 {
            if data.is_empty() {
                if !self.closed {
                    return Ok(None);
                }
                return Err(ParseError::new("Incomplete Unicode escape"));
            }
            let b = data[0];
            *data = &data[1..];
            let digit = unhex(b)?;
            value = (value << 4) | digit;
        }

        Ok(Some(value as u16))
    }

    fn process_whitespace<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        locator: &SharedLocator,
        first: u8,
        data: &mut &[u8],
    ) -> ParseResult<bool> {
        if first == b'\n' || first == b'\r' {
            locator
                .set_line(locator.line_number() + 1);
            locator.set_column(0);
        }

        if !self.needs_whitespace {
            consume_whitespace_run(data, &locator);
            return Ok(true);
        }

        let ws_start = *data;
        consume_whitespace_run(data, &locator);
        let ws_len = ws_start.len() - data.len();
        let mut ws_bytes = Vec::with_capacity(1 + ws_len);
        ws_bytes.push(first);
        ws_bytes.extend_from_slice(&ws_start[..ws_len]);
        let ws = decode_span(&ws_bytes, false)?;
        handler.whitespace(&ws)?;
        Ok(true)
    }

    fn process_literal<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        data: &mut &[u8],
        remaining: &[u8],
        bool_value: Option<bool>,
    ) -> ParseResult<bool> {
        if data.len() < remaining.len() {
            if !self.closed {
                return Ok(false);
            }
            return Err(ParseError::new("Incomplete literal"));
        }

        for (i, &expected) in remaining.iter().enumerate() {
            if data[i] != expected {
                return Err(ParseError::new("Invalid literal"));
            }
        }

        *data = &data[remaining.len()..];

        match bool_value {
            Some(v) => handler.boolean_value(v)?,
            None => handler.null_value()?,
        }
        Ok(true)
    }

    fn process_number<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        limits: &ParserLimits,
        first: u8,
        data: &mut &[u8],
    ) -> ParseResult<bool> {
        let number_start = *data;
        let start_len = number_start.len() + 1;

        let negative = first == b'-';
        let mut cursor = *data;

        let c = if negative {
            if cursor.is_empty() {
                if !self.closed {
                    return Ok(false);
                }
                return Err(ParseError::new("Invalid number: just '-'"));
            }
            let ch = cursor[0];
            cursor = &cursor[1..];
            ch
        } else {
            first
        };

        let mut ival: i64 = 0;
        let mut int_overflow = false;

        if c == b'0' {
            if !cursor.is_empty() {
                let next = cursor[0];
                if next >= b'0' && next <= b'9' {
                    return Err(ParseError::new("Numbers cannot have leading zeros"));
                }
            } else if !self.closed {
                return Ok(false);
            }
        } else if c >= b'1' && c <= b'9' {
            ival = (c - b'0') as i64;
            let mut saw_non_digit = false;
            while !cursor.is_empty() {
                let d = cursor[0];
                if d >= b'0' && d <= b'9' {
                    cursor = &cursor[1..];
                    if !int_overflow {
                        let digit = (d - b'0') as i64;
                        if ival > (i64::MAX - digit) / 10 {
                            int_overflow = true;
                        } else {
                            ival = ival * 10 + digit;
                        }
                    }
                } else {
                    saw_non_digit = true;
                    break;
                }
            }
            if !saw_non_digit && !self.closed {
                return Ok(false);
            }
        } else {
            return Err(ParseError::new("Invalid number format"));
        }

        let mut has_fraction = false;
        let mut dval: i64 = 0;
        let mut mul: i64 = 1;
        let mut frac_overflow = false;

        if !cursor.is_empty() {
            if cursor[0] == b'.' {
                has_fraction = true;
                cursor = &cursor[1..];

                if cursor.is_empty() {
                    if !self.closed {
                        return Ok(false);
                    }
                    return Err(ParseError::new("Decimal point must be followed by digit"));
                }

                let digit = cursor[0];
                if digit < b'0' || digit > b'9' {
                    return Err(ParseError::new("Decimal point must be followed by digit"));
                }
                dval = (digit - b'0') as i64;
                mul = 10;
                cursor = &cursor[1..];

                while !cursor.is_empty() {
                    let d = cursor[0];
                    if d >= b'0' && d <= b'9' {
                        cursor = &cursor[1..];
                        if !frac_overflow {
                            let dg = (d - b'0') as i64;
                            if mul > i64::MAX / 10 || dval > (i64::MAX - dg) / 10 {
                                frac_overflow = true;
                            } else {
                                dval = dval * 10 + dg;
                                mul *= 10;
                            }
                        }
                    } else {
                        break;
                    }
                }

                if cursor.is_empty() && !self.closed {
                    return Ok(false);
                }
            }
        } else if !self.closed {
            return Ok(false);
        }

        let mut has_exponent = false;
        let mut exp_negative = false;
        let mut exponent: i32 = 0;
        let mut exp_overflow = false;

        if !cursor.is_empty() {
            let c2 = cursor[0];
            if c2 == b'e' || c2 == b'E' {
                has_exponent = true;
                cursor = &cursor[1..];

                if cursor.is_empty() {
                    if !self.closed {
                        return Ok(false);
                    }
                    return Err(ParseError::new("Incomplete exponent"));
                }

                let mut sign = cursor[0];
                cursor = &cursor[1..];
                if sign == b'+' || sign == b'-' {
                    exp_negative = sign == b'-';
                    if cursor.is_empty() {
                        if !self.closed {
                            return Ok(false);
                        }
                        return Err(ParseError::new("Exponent must have digit"));
                    }
                    sign = cursor[0];
                    cursor = &cursor[1..];
                }

                if sign < b'0' || sign > b'9' {
                    return Err(ParseError::new("Exponent must have digit"));
                }
                exponent = (sign - b'0') as i32;

                while !cursor.is_empty() {
                    let d = cursor[0];
                    if d >= b'0' && d <= b'9' {
                        cursor = &cursor[1..];
                        if !exp_overflow {
                            exponent = exponent * 10 + (d - b'0') as i32;
                            if exponent > 100_000 {
                                exp_overflow = true;
                            }
                        }
                    } else {
                        break;
                    }
                }

                if cursor.is_empty() && !self.closed {
                    return Ok(false);
                }
            }
        } else if !self.closed {
            return Ok(false);
        }

        *data = cursor;

        let consumed = start_len - data.len();
        if limits.max_number_length > 0 && consumed > limits.max_number_length as usize {
            return Err(ParseError::new(format!(
                "Maximum number length exceeded: {}",
                limits.max_number_length
            )));
        }

        let num = compose_number(
            first,
            number_start,
            data,
            negative,
            ival,
            int_overflow,
            has_fraction,
            dval,
            mul,
            frac_overflow,
            has_exponent,
            exp_negative,
            exponent,
            exp_overflow,
        )?;
        handler.number_value(&num)?;
        Ok(true)
    }
}

fn consume_whitespace_run(data: &mut &[u8], locator: &SharedLocator) {
    while !data.is_empty() {
        let saved = *data;
        let b = data[0];
        *data = &data[1..];

        match b {
            b' ' | b'\t' => locator.add_column(1),
            b'\n' | b'\r' => {
                locator.set_line(locator.line_number() + 1);
                locator.set_column(0);
            }
            _ => {
                *data = saved;
                break;
            }
        }
    }
}

fn decode_span(bytes: &[u8], saw_non_ascii: bool) -> ParseResult<String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if !saw_non_ascii {
        return Ok(bytes.iter().map(|&b| b as char).collect());
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(s.to_owned()),
        Err(_) => Err(ParseError::new("Malformed UTF-8")),
    }
}

fn append_string_as_utf16(units: &mut Vec<u16>, s: &str) -> usize {
    let start = units.len();
    for ch in s.chars() {
        let mut buf = [0u16; 2];
        for u in ch.encode_utf16(&mut buf) {
            units.push(*u);
        }
    }
    units.len() - start
}

fn unhex(b: u8) -> ParseResult<u32> {
    if b >= b'0' && b <= b'9' {
        Ok((b - b'0') as u32)
    } else if b >= b'A' && b <= b'F' {
        Ok((b - b'A' + 10) as u32)
    } else if b >= b'a' && b <= b'f' {
        Ok((b - b'a' + 10) as u32)
    } else {
        Err(ParseError::new(format!("Invalid hex digit: {}", b as char)))
    }
}

fn number_token_bytes(first: u8, original: &[u8], remaining: &[u8]) -> Vec<u8> {
    let consumed = original.len() - remaining.len();
    let mut bytes = Vec::with_capacity(1 + consumed);
    bytes.push(first);
    bytes.extend_from_slice(&original[..consumed]);
    bytes
}

fn compose_number(
    first: u8,
    number_start: &[u8],
    data: &[u8],
    negative: bool,
    ival: i64,
    int_overflow: bool,
    has_fraction: bool,
    dval: i64,
    mul: i64,
    frac_overflow: bool,
    has_exponent: bool,
    exp_negative: bool,
    exponent: i32,
    exp_overflow: bool,
) -> ParseResult<Number> {
    let num_bytes = number_token_bytes(first, number_start, data);
    let num_str = decode_span(&num_bytes, false)?;

    if !has_fraction && !has_exponent {
        if int_overflow {
            return Ok(Number::BigInt(num_str));
        }
        let value = if negative { -ival } else { ival };
        if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
            return Ok(Number::I32(value as i32));
        }
        return Ok(Number::I64(value));
    }

    if int_overflow || frac_overflow || exp_overflow {
        let value: f64 = num_str.parse().map_err(|_| {
            ParseError::new(format!("Invalid number: {num_str}"))
        })?;
        return Ok(Number::F64(value));
    }

    let mantissa = if has_fraction {
        ival as f64 + (dval as f64 / mul as f64)
    } else {
        ival as f64
    };
    let mut value = mantissa;
    if has_exponent {
        let signed_exponent = if exp_negative {
            -(exponent as f64)
        } else {
            exponent as f64
        };
        if signed_exponent < -290.0 || signed_exponent > 290.0 {
            let half = signed_exponent / 2.0;
            value = mantissa * 10f64.powf(half) * 10f64.powf(signed_exponent - half);
        } else {
            value = mantissa * 10f64.powf(signed_exponent);
        }
    }
    if negative {
        value = -value;
    }
    Ok(Number::F64(value))
}
