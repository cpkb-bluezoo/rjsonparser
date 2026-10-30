//! JSON tokenizer scanning the caller's ring directly (Java `JSONTokenizer` port).
//!
//! JSON's structural grammar (braces, brackets, colon, comma, whitespace,
//! digits, `-+.eE`, the literals, and the string delimiters `"` and `\`) is
//! all ASCII, and UTF-8 is self-synchronizing: none of those bytes can occur
//! inside a multi-byte sequence. So everything is tokenized against raw bytes,
//! and UTF-8 is decoded only for the content of a string value or key.
//!
//! The tokenizer owns exactly one buffer: a fixed-size character buffer that
//! string content is decoded into and handed to the handler from as a `&str`.
//! It holds none of the document's bytes. Whatever it cannot use yet (a key or
//! literal that is not finished, the start of an escape sequence or multi-byte
//! character, an escaped high surrogate whose partner has not been seen) is
//! left unconsumed in the caller's ring, to be presented again with more data.
//! The only state kept between calls is a handful of scalars (the grammar
//! state, and a number's digits so far).

use std::collections::HashSet;
use std::io::Write;

use crate::error::{ParseError, ParseResult};
use crate::handler::ContentHandler;
use crate::keys::{KeySymbolTable, MAX_KEY_BYTES};
use crate::limits::ParserLimits;
use crate::locator::{Locator, SharedLocator};
use crate::number::Number;

const KEY_INTERN_DISABLE_THRESHOLD: i32 = 64;

/// Decoding a run in slices needs at least this much free room in the
/// character buffer; a chunk size below it could not hold the longest escape
/// or character.
pub(crate) const MIN_DECODE_ROOM: usize = 16;

/// Nesting token pushed onto [`Tokenizer::depth_stack`] (Java `Token`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NestToken {
    StartObject,
    StartArray,
}

/// The ring's readable bytes: up to two contiguous spans, the second empty
/// unless the content wraps the ring's physical end. Indexing through this
/// is what lets a token straddle the wrap without the bytes being moved.
#[derive(Clone, Copy)]
pub(crate) struct Input<'a> {
    a: &'a [u8],
    b: &'a [u8],
}

impl<'a> Input<'a> {
    pub(crate) fn new(a: &'a [u8], b: &'a [u8]) -> Self {
        Self { a, b }
    }

    pub(crate) fn len(&self) -> usize {
        self.a.len() + self.b.len()
    }

    pub(crate) fn get(&self, i: usize) -> u8 {
        if i < self.a.len() {
            self.a[i]
        } else {
            self.b[i - self.a.len()]
        }
    }

    /// The contiguous bytes from `i` to the end of whichever span holds it.
    fn span_from(&self, i: usize) -> &'a [u8] {
        if i < self.a.len() {
            &self.a[i..]
        } else {
            &self.b[i - self.a.len()..]
        }
    }

    /// `[from, to)` as one slice, if it does not straddle the wrap.
    fn contiguous(&self, from: usize, to: usize) -> Option<&'a [u8]> {
        let split = self.a.len();
        if to <= split {
            Some(&self.a[from..to])
        } else if from >= split {
            Some(&self.b[from - split..to - split])
        } else {
            None
        }
    }

    pub(crate) fn skip(&self, n: usize) -> Input<'a> {
        if n >= self.a.len() {
            Input {
                a: &self.b[n - self.a.len()..],
                b: &[],
            }
        } else {
            Input {
                a: &self.a[n..],
                b: self.b,
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumStage {
    None,
    AfterMinus,     // "-" seen: a digit must follow
    Zero,           // a leading "0": no more integer digits allowed
    Integer,        // integer digits 1-9...
    FractionDot,    // "." seen: a digit must follow
    Fraction,       // fraction digits
    ExponentMark,   // "e" or "E" seen: a sign or digit must follow
    ExponentSign,   // exponent sign seen: a digit must follow
    Exponent,       // exponent digits
}

// While the mantissa is at most this, any next digit fits an i64
const DIGIT_ALWAYS_FITS_BELOW: i64 = 922_337_203_685_477_579;

/// A number read in a single forward pass, one byte at a time, keeping only
/// the value so far. It needs no look-ahead and no buffering, and can span
/// any number of `receive()` calls.
struct NumberState {
    stage: NumStage,
    negative: bool,
    is_real: bool,
    /// Bytes of the number so far, for `max_number_length` and the locator.
    length: usize,
    /// All integer and fraction digits read as one integer, as many as fit.
    mantissa: i64,
    /// A further digit would overflow `mantissa`, so later digits are dropped.
    mantissa_full: bool,
    /// Power of ten to scale `mantissa` by.
    decimal_shift: i32,
    /// A dropped digit was not zero, so the true value is above `mantissa`.
    dropped_non_zero: bool,
    /// The first dropped digit, or -1.
    first_dropped: i32,
    exponent_negative: bool,
    /// Exponent digits, saturating (anything this large is infinity or zero).
    exponent: i32,
    /// An integer too large for an i64 is kept exactly, as its digits. This is
    /// the number's own value, not buffered input, and is empty otherwise.
    big_digits: String,
}

impl NumberState {
    fn new() -> Self {
        Self {
            stage: NumStage::None,
            negative: false,
            is_real: false,
            length: 0,
            mantissa: 0,
            mantissa_full: false,
            decimal_shift: 0,
            dropped_non_zero: false,
            first_dropped: -1,
            exponent_negative: false,
            exponent: 0,
            big_digits: String::new(),
        }
    }

    fn start(&mut self, first: u8) {
        self.negative = first == b'-';
        self.is_real = false;
        self.length = 1;
        self.mantissa = 0;
        self.mantissa_full = false;
        self.decimal_shift = 0;
        self.dropped_non_zero = false;
        self.first_dropped = -1;
        self.exponent_negative = false;
        self.exponent = 0;
        self.big_digits.clear();
        self.stage = if self.negative {
            NumStage::AfterMinus
        } else if first == b'0' {
            NumStage::Zero
        } else {
            self.mantissa = i64::from(first - b'0');
            NumStage::Integer
        };
    }

    /// Takes a whole run of integer or fraction digits at once, working on
    /// local copies of the accumulators. This is the common case in numeric
    /// data, so it avoids the per-byte state dispatch. Stops at the first byte
    /// that is not a digit, or when the mantissa nears overflow; the general
    /// code then deals with that byte.
    fn take_digit_run(&mut self, span: &[u8]) -> usize {
        let mut m = self.mantissa;
        let mut n = 0;
        while n < span.len() && m <= DIGIT_ALWAYS_FITS_BELOW {
            let digit = span[n].wrapping_sub(b'0');
            if digit > 9 {
                break;
            }
            m = m * 10 + i64::from(digit);
            n += 1;
        }
        if n > 0 {
            self.mantissa = m;
            if self.stage == NumStage::Fraction {
                self.decimal_shift -= n as i32;
            }
            self.length += n;
        }
        n
    }

    /// Adds one digit of the integer part or the fraction. Digits are read into
    /// `mantissa` until the next would overflow an i64; after that they are
    /// dropped (integer digits still scale the value, and still feed
    /// `big_digits` so a plain integer stays exact).
    fn take_digit(&mut self, digit: u8, fraction: bool) {
        let d = i64::from(digit);
        if !self.mantissa_full {
            if self.mantissa <= (i64::MAX - d) / 10 {
                self.mantissa = self.mantissa * 10 + d;
                if fraction {
                    self.decimal_shift -= 1;
                }
                return;
            }
            self.mantissa_full = true;
            self.dropped_non_zero = false;
            self.first_dropped = -1;
            if !fraction {
                self.big_digits = self.mantissa.to_string();
            }
        }
        if !fraction {
            self.decimal_shift += 1;
            self.big_digits.push(char::from(b'0' + digit));
        }
        if digit != 0 {
            self.dropped_non_zero = true;
        }
        if self.first_dropped < 0 {
            self.first_dropped = i32::from(digit);
        }
    }

    fn take_exponent_digit(&mut self, digit: u8) {
        if self.exponent < 100_000 {
            self.exponent = self.exponent * 10 + i32::from(digit);
        }
    }

    fn integer_value(&self) -> Number {
        if !self.big_digits.is_empty() {
            return Number::BigInt(if self.negative {
                format!("-{}", self.big_digits)
            } else {
                self.big_digits.clone()
            });
        }
        let value = if self.negative { -self.mantissa } else { self.mantissa };
        if value >= i64::from(i32::MIN) && value <= i64::from(i32::MAX) {
            Number::I32(value as i32)
        } else {
            Number::I64(value)
        }
    }

    /// The double nearest to the number. Exact when its significant digits fit
    /// `mantissa`. If more digits were dropped the true value lies strictly
    /// between `mantissa` and `mantissa + 1` (in units of the shift), so the
    /// double is the same for both ends unless a rounding boundary falls
    /// between them; in that rare case the nearer end, judged by the first
    /// dropped digit, is used and the result may be one ulp out.
    fn real_value(&self) -> Number {
        let exponent = if self.exponent_negative { -self.exponent } else { self.exponent };
        let e10 = exponent + self.decimal_shift;
        if !(self.mantissa_full && self.dropped_non_zero) {
            return Number::F64(decimal_to_f64(self.negative, self.mantissa, e10));
        }
        let low = decimal_to_f64(false, self.mantissa, e10);
        let high = if self.mantissa == i64::MAX {
            low
        } else {
            decimal_to_f64(false, self.mantissa + 1, e10)
        };
        let value = if low == high || self.first_dropped < 5 { low } else { high };
        Number::F64(if self.negative { -value } else { value })
    }
}

/// `mantissa * 10^e10`, correctly rounded.
fn decimal_to_f64(negative: bool, mantissa: i64, e10: i32) -> f64 {
    // Exact fast path (Clinger): an integer below 2^53 and a net power of ten
    // within 10^22 need one correctly rounded multiply or divide by an exactly
    // representable power. Anything else goes through `str::parse`, which is
    // correctly rounded for every input.
    const POW10: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
        1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    let m = mantissa as u64;
    let value = if m == 0 {
        0.0
    } else if m < (1u64 << 53) && (-22..=22).contains(&e10) {
        let f = m as f64;
        if e10 < 0 {
            f / POW10[(-e10) as usize]
        } else {
            f * POW10[e10 as usize]
        }
    } else {
        // at most 20 + 1 + 11 bytes: no allocation
        let mut buf = [0u8; 48];
        let mut cursor: &mut [u8] = &mut buf;
        write!(cursor, "{m}e{e10}").expect("a mantissa and exponent fit 48 bytes");
        let len = 48 - cursor.len();
        std::str::from_utf8(&buf[..len])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .expect("a decimal literal always parses")
    };
    if negative {
        -value
    } else {
        value
    }
}

/// An escape sequence resolved from the raw bytes, or not yet fully present.
enum Escape {
    NeedMore,
    Unit(u16, usize),
}

/// Streaming JSON tokenizer calling [`ContentHandler`] methods as tokens are recognized.
pub(crate) struct Tokenizer {
    reject_duplicate_keys: bool,
    needs_whitespace: bool,

    context_stack: Vec<Context>,
    state: State,

    pub seen_any_token: bool,
    pub depth_stack: Vec<NestToken>,
    after_comma: bool,
    /// The last byte was a CR, so a following LF is the same line break.
    after_cr: bool,

    key_intern_attempts: i32,
    key_intern_hits: i32,
    key_intern_disabled: bool,

    duplicate_key_stack: Vec<HashSet<String>>,

    token_count: i64,

    /// A string value is delivered to the handler as it is read, never
    /// buffered: `in_string` is true between `start_string_value` and
    /// `end_string_value`.
    in_string: bool,
    /// Raw content bytes of the current string value consumed so far.
    string_bytes: u64,

    /// The one buffer: string content (and keys) decoded here, capacity fixed
    /// at `chunk_size` bytes and never grown. Always empty between calls.
    chars: String,
    chars_cap: usize,
    chunk_size: usize,

    number: NumberState,
}

impl Tokenizer {
    pub fn new(reject_duplicate_keys: bool, needs_whitespace: bool, chunk_size: usize) -> Self {
        Self {
            reject_duplicate_keys,
            needs_whitespace,
            context_stack: Vec::new(),
            state: State::ExpectValue,
            seen_any_token: false,
            depth_stack: Vec::new(),
            after_comma: false,
            after_cr: false,
            key_intern_attempts: 0,
            key_intern_hits: 0,
            key_intern_disabled: false,
            duplicate_key_stack: Vec::new(),
            token_count: 0,
            in_string: false,
            string_bytes: 0,
            chars: String::new(),
            chars_cap: 0,
            chunk_size,
            number: NumberState::new(),
        }
    }

    /// Sets the size of the character buffer; takes effect from the next string.
    pub fn set_chunk_size(&mut self, chunk_size: usize) {
        self.chunk_size = chunk_size;
    }

    fn ensure_chars(&mut self) {
        if self.chars_cap != self.chunk_size {
            self.chars = String::with_capacity(self.chunk_size);
            self.chars_cap = self.chunk_size;
        }
    }

    /// Room left in the character buffer. Content is never pushed past
    /// `chunk_size`, so the `String` never reallocates.
    fn free(&self) -> usize {
        self.chunk_size - self.chars.len()
    }

    /// Called when no more input will arrive. Completes a number that ends the
    /// document (such as the whole document `42`), and rejects anything left
    /// unfinished.
    pub fn end_of_input<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        locator: &SharedLocator,
    ) -> ParseResult<()> {
        if self.in_string {
            return Err(ParseError::new("Unclosed string"));
        }
        match self.number.stage {
            NumStage::None => Ok(()),
            NumStage::AfterMinus => Err(ParseError::new("Invalid number: just '-'")),
            NumStage::FractionDot => Err(ParseError::new("Decimal point must be followed by digit")),
            NumStage::ExponentMark => Err(ParseError::new("Incomplete exponent")),
            NumStage::ExponentSign => Err(ParseError::new("Exponent must have digit")),
            _ => self.emit_number(handler, locator),
        }
    }

    /// Processes as many complete tokens as possible from the front of
    /// `input`, advancing `consumed` past each one as it completes (so the
    /// caller knows how much to release even if this returns an error).
    pub fn receive<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
    ) -> ParseResult<()> {
        let total = input.len();
        while *consumed < total {
            if self.in_string {
                if !self.continue_string(handler, limits, locator, input, consumed)? {
                    return Ok(());
                }
                continue;
            }
            if self.number.stage != NumStage::None {
                if !self.continue_number(handler, limits, locator, input, consumed)? {
                    return Ok(());
                }
                continue;
            }

            let b = input.get(*consumed);
            if matches!(b, b' ' | b'\n' | b'\t' | b'\r') {
                self.process_whitespace(handler, locator, input, consumed)?;
                continue;
            }
            self.after_cr = false;

            if !self.process_token(handler, key_symbol_table, limits, locator, input, consumed, b)? {
                return Ok(());
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

    /// Bookkeeping common to every token once it is known to be complete.
    fn token_done(&mut self, limits: &ParserLimits) -> ParseResult<()> {
        self.seen_any_token = true;
        self.check_token_limit(limits)
    }

    /// Process the token starting with byte `b` at `*consumed`. Returns false
    /// if the token is not all there yet, in which case nothing of it has
    /// been consumed.
    #[allow(clippy::too_many_arguments)]
    fn process_token<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
        b: u8,
    ) -> ParseResult<bool> {
        match b {
            b'"' => {
                if matches!(self.state, State::ExpectColon | State::AfterValue) {
                    return Err(ParseError::new("Unexpected string"));
                }
                if self.state == State::ExpectKey {
                    // A key is a single token: it must be complete in the ring
                    if !self.process_key(handler, key_symbol_table, limits, locator, input, consumed)? {
                        return Ok(false);
                    }
                    self.token_done(limits)?;
                    self.after_comma = false;
                    self.state = State::ExpectColon;
                    return Ok(true);
                }
                // A string value is streamed: report its start now and deliver
                // its content as it arrives (see continue_string)
                *consumed += 1;
                locator.add_column(1);
                self.ensure_chars();
                self.chars.clear();
                handler.start_string_value()?;
                self.in_string = true;
                self.string_bytes = 0;
                self.token_done(limits)?;
                self.after_comma = false;
                Ok(true)
            }

            b',' => {
                if self.state != State::AfterValue {
                    return Err(ParseError::new("Unexpected ','"));
                }
                if self.context_stack.is_empty() {
                    return Err(ParseError::new("Unexpected comma at root level"));
                }
                *consumed += 1;
                locator.add_column(1);
                self.token_done(limits)?;
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
                *consumed += 1;
                locator.add_column(1);
                self.token_done(limits)?;
                self.after_comma = false;
                self.state = State::ExpectValue;
                Ok(true)
            }

            b'{' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected '{'"));
                }
                self.check_depth(limits)?;
                *consumed += 1;
                locator.add_column(1);
                handler.start_object()?;
                self.context_stack.push(Context::Object);
                self.depth_stack.push(NestToken::StartObject);
                if self.reject_duplicate_keys {
                    self.duplicate_key_stack.push(HashSet::new());
                }
                self.token_done(limits)?;
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
                *consumed += 1;
                locator.add_column(1);
                handler.end_object()?;
                self.context_stack.pop();
                self.depth_stack.pop();
                if self.reject_duplicate_keys {
                    self.duplicate_key_stack.pop();
                }
                self.token_done(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b'[' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected '['"));
                }
                self.check_depth(limits)?;
                *consumed += 1;
                locator.add_column(1);
                handler.start_array()?;
                self.context_stack.push(Context::Array);
                self.depth_stack.push(NestToken::StartArray);
                self.token_done(limits)?;
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
                *consumed += 1;
                locator.add_column(1);
                handler.end_array()?;
                self.context_stack.pop();
                self.depth_stack.pop();
                self.token_done(limits)?;
                self.after_comma = false;
                self.state = State::AfterValue;
                Ok(true)
            }

            b't' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'true'"));
                }
                self.process_literal(handler, limits, locator, input, consumed, b"rue", Some(true))
            }

            b'f' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'false'"));
                }
                self.process_literal(handler, limits, locator, input, consumed, b"alse", Some(false))
            }

            b'n' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected 'null'"));
                }
                self.process_literal(handler, limits, locator, input, consumed, b"ull", None)
            }

            b'-' | b'0'..=b'9' => {
                if self.state != State::ExpectValue {
                    return Err(ParseError::new("Unexpected number"));
                }
                self.number.start(b);
                *consumed += 1;
                self.token_done(limits)?;
                self.after_comma = false;
                Ok(true)
            }

            _ => Err(ParseError::new(format!("Unexpected character: {}", b as char))),
        }
    }

    fn check_depth(&self, limits: &ParserLimits) -> ParseResult<()> {
        let max_depth = limits.max_nesting_depth;
        if max_depth > 0 && self.depth_stack.len() >= max_depth as usize {
            return Err(ParseError::new(format!(
                "Maximum nesting depth exceeded: {max_depth}"
            )));
        }
        Ok(())
    }

    /// A literal (`true`, `false`, `null`), first byte at `*consumed`, `rest`
    /// the remainder. The bytes that are there are checked as they arrive, so
    /// a bad literal is rejected at the same point however the input is
    /// chunked; an incomplete one is left unconsumed.
    #[allow(clippy::too_many_arguments)]
    fn process_literal<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
        rest: &[u8],
        value: Option<bool>,
    ) -> ParseResult<bool> {
        let first = *consumed + 1;
        let available = input.len() - first;
        for (i, &expected) in rest.iter().enumerate().take(available) {
            if input.get(first + i) != expected {
                return Err(ParseError::new("Invalid literal"));
            }
        }
        if available < rest.len() {
            return Ok(false);
        }
        *consumed = first + rest.len();
        locator.add_column(1 + rest.len());
        self.token_done(limits)?;
        self.after_comma = false;
        self.state = State::AfterValue;
        match value {
            Some(v) => handler.boolean_value(v)?,
            None => handler.null_value()?,
        }
        Ok(true)
    }

    /// Consumes a run of whitespace. A run has no delimiter marking its end the
    /// way a string has a closing quote, so whatever is there is consumed and
    /// reported at once; if more arrives in the next call, that is simply a
    /// second `whitespace` call for what is logically the same run.
    fn process_whitespace<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
    ) -> ParseResult<()> {
        let start = *consumed;
        let total = input.len();
        let mut line = locator.line_number();
        let mut column = locator.column_number();
        let mut after_cr = self.after_cr;
        let mut p = start;
        while p < total {
            match input.get(p) {
                // CR, LF and CRLF are each one line break; `after_cr` carries
                // the CR across calls so the pair is not counted twice if a
                // chunk boundary falls between them
                b'\n' => {
                    if !after_cr {
                        line += 1;
                    }
                    column = 1;
                    after_cr = false;
                }
                b'\r' => {
                    line += 1;
                    column = 1;
                    after_cr = true;
                }
                b' ' | b'\t' => {
                    column += 1;
                    after_cr = false;
                }
                _ => break,
            }
            p += 1;
        }
        locator.set_line(line);
        locator.set_column(column);
        self.after_cr = after_cr;
        *consumed = p;

        if self.needs_whitespace {
            let mut q = start;
            while q < p {
                let span = input.span_from(q);
                let n = span.len().min(p - q);
                let ws = std::str::from_utf8(&span[..n]).expect("whitespace is ASCII");
                handler.whitespace(ws)?;
                q += n;
            }
        }
        Ok(())
    }

    /// Continues the string value in progress, consuming as much of `input` as
    /// can be delivered. Nothing is buffered: plain runs are decoded into the
    /// character buffer and handed to the handler as soon as the run ends or
    /// the buffer fills, so a string of any length takes a fixed amount of
    /// memory.
    ///
    /// A fragment that cannot be completed yet (a multi-byte UTF-8 sequence or
    /// an escape sequence cut off by the end of the input, or an escaped high
    /// surrogate whose partner has not been seen) is left unconsumed.
    ///
    /// Returns true if the closing quote was consumed, false if more data is
    /// needed to make progress.
    fn continue_string<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
    ) -> ParseResult<bool> {
        self.ensure_chars();
        let limit = limits.max_string_length;
        let total = input.len();

        loop {
            let run_start = *consumed;
            let run_end = scan_plain(input, run_start, total);

            if run_end > run_start {
                // Checked before the run is delivered, so a string over the
                // limit is rejected without its excess ever being reported
                if limit > 0 && self.string_bytes + (run_end - run_start) as u64 > limit as u64 {
                    return Err(string_too_long(limit));
                }
                let (next, units) = self.decode_run(Some(&mut *handler), input, run_start, run_end)?;
                self.string_bytes += (next - run_start) as u64;
                locator.add_column(units);
                *consumed = next;
                if next < run_end {
                    // A character cut off by the end of the input: wait for more
                    self.flush_chars(handler)?;
                    return Ok(false);
                }
            }

            if run_end == total {
                self.flush_chars(handler)?;
                return Ok(false);
            }

            let b = input.get(run_end);
            if b == b'"' {
                self.flush_chars(handler)?;
                *consumed = run_end + 1;
                locator.add_column(1);
                self.in_string = false;
                self.state = State::AfterValue;
                handler.end_string_value()?;
                return Ok(true);
            }
            if b != b'\\' {
                return Err(ParseError::new("Unescaped control character in string"));
            }

            // Escape sequence: needs all of its bytes to be present
            let Some((ch, n)) = resolve_escape(input, run_end)? else {
                self.flush_chars(handler)?;
                return Ok(false);
            };
            if ch.len_utf8() > self.free() {
                self.flush_chars(handler)?;
            }
            self.chars.push(ch);
            *consumed = run_end + n;
            self.string_bytes += n as u64;
            locator.add_column(n); // raw escape bytes are always ASCII
            if limit > 0 && self.string_bytes > limit as u64 {
                return Err(string_too_long(limit));
            }
        }
    }

    /// Decodes the plain bytes `[from, to)` of `input` into the character
    /// buffer, in slices that fit what is free in it, handing the characters to
    /// the handler whenever it fills. So however much the ring holds, a content
    /// call carries at most `chunk_size` bytes. UTF-8 never decodes to more
    /// bytes than it started with, so a slice no longer than the free space
    /// always fits.
    ///
    /// With no handler (decoding a key) the buffer filling is an error.
    /// Returns the position reached and the UTF-16 units written; the position
    /// is short of `to` only if a character is cut off by the end of the input.
    fn decode_run<H: ContentHandler + ?Sized>(
        &mut self,
        mut handler: Option<&mut H>,
        input: Input<'_>,
        from: usize,
        to: usize,
    ) -> ParseResult<(usize, usize)> {
        // a content call of a few bytes is not worth making for a string,
        // but a key only needs room for what is left of it
        let low_water = if handler.is_some() { MIN_DECODE_ROOM } else { 1 };
        let mut p = from;
        let mut units = 0;
        while p < to {
            if self.free() < low_water {
                self.make_room(&mut handler)?;
            }
            let span = input.span_from(p);
            let take = span.len().min(to - p).min(self.free());
            let slice = &span[..take];
            match std::str::from_utf8(slice) {
                Ok(s) => {
                    units += utf16_len(s);
                    self.chars.push_str(s);
                    p += take;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    if valid > 0 {
                        let s = std::str::from_utf8(&slice[..valid]).expect("validated prefix");
                        units += utf16_len(s);
                        self.chars.push_str(s);
                        p += valid;
                    }
                    if e.error_len().is_some() {
                        return Err(malformed_utf8());
                    }
                    // A character cut short by the end of the slice, which may
                    // be the end of a ring span, the free space, or the data
                    match decode_char_at(input, p)? {
                        Some((ch, n)) => {
                            if ch.len_utf8() > self.free() {
                                self.make_room(&mut handler)?;
                            }
                            self.chars.push(ch);
                            units += ch.len_utf16();
                            p += n;
                        }
                        None => return Ok((p, units)),
                    }
                }
            }
        }
        Ok((p, units))
    }

    /// Frees the character buffer: a string's content goes to the handler; a
    /// key has nowhere to go, so it must fit.
    fn make_room<H: ContentHandler + ?Sized>(&mut self, handler: &mut Option<&mut H>) -> ParseResult<()> {
        match handler.as_deref_mut() {
            Some(h) => self.flush_chars(h),
            None => Err(ParseError::new(
                "Object key is longer than the string chunk size",
            )),
        }
    }

    /// Hands any decoded characters to the handler and empties the buffer.
    fn flush_chars<H: ContentHandler + ?Sized>(&mut self, handler: &mut H) -> ParseResult<()> {
        if !self.chars.is_empty() {
            handler.string_value_content(&self.chars)?;
            self.chars.clear();
        }
        Ok(())
    }

    /// An object key, opening quote at `*consumed`. A key is a single token:
    /// all of it must be in the ring, so that it is reported to the handler in
    /// one call. If the closing quote has not arrived, nothing is consumed.
    ///
    /// Scanning for the closing quote, backslashes and control characters is
    /// done against raw bytes; only the content between delimiters is decoded,
    /// and only once its extent is known.
    fn process_key<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        key_symbol_table: &mut KeySymbolTable,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
    ) -> ParseResult<bool> {
        let total = input.len();
        let start = *consumed + 1;
        let limit = limits.max_name_length;

        let mut has_escape = false;
        let mut i = start;
        let end = loop {
            if i >= total {
                return Ok(false); // need more data
            }
            // Raw byte count is a conservative (never under-protective) proxy
            // for decoded length, checked mid-scan so a pathologically long
            // key is rejected without waiting for its end
            if limit > 0 && i - start > limit as usize {
                return Err(ParseError::new(format!(
                    "Maximum key length exceeded: {limit}"
                )));
            }
            let b = input.get(i);
            if b == b'"' {
                break i;
            }
            if b == b'\\' {
                has_escape = true;
                i += 2; // the escaped byte cannot end the key
                continue;
            }
            if b < 0x20 {
                return Err(ParseError::new("Unescaped control character in string"));
            }
            i += 1;
        };

        self.ensure_chars();
        self.chars.clear();

        // Interning needs the key verbatim, contiguous and short
        let interned = if !has_escape && !self.key_intern_disabled && end - start <= MAX_KEY_BYTES {
            input.contiguous(start, end)
        } else {
            None
        };

        let src_units;
        if let Some(bytes) = interned {
            let hash = bytes
                .iter()
                .fold(KeySymbolTable::initial_hash(), |h, &b| KeySymbolTable::hash_byte(h, b));
            self.key_intern_attempts += 1;
            if let Some(found) = key_symbol_table.lookup(bytes, hash) {
                self.key_intern_hits += 1;
                *consumed = end + 1;
                let dups = self.duplicate_set();
                return report_key(handler, locator, dups, found, utf16_len(found));
            }
            src_units = self.decode_key(input, start, end)?;
            key_symbol_table.put(bytes, hash, self.chars.clone());
            if self.key_intern_hits == 0 && self.key_intern_attempts >= KEY_INTERN_DISABLE_THRESHOLD {
                self.key_intern_disabled = true;
            }
        } else {
            src_units = self.decode_key(input, start, end)?;
        }

        *consumed = end + 1;
        let dups = if self.reject_duplicate_keys {
            self.duplicate_key_stack.last_mut()
        } else {
            None
        };
        report_key(handler, locator, dups, &self.chars, src_units)
    }

    fn duplicate_set(&mut self) -> Option<&mut HashSet<String>> {
        if self.reject_duplicate_keys {
            self.duplicate_key_stack.last_mut()
        } else {
            None
        }
    }

    /// Decodes the key content `[start, end)` (all of it present, escapes
    /// included) into the character buffer. Returns the key's length in the
    /// source, in UTF-16 units, for the locator.
    fn decode_key(&mut self, input: Input<'_>, start: usize, end: usize) -> ParseResult<usize> {
        let mut src_units = 0;
        let mut p = start;
        while p < end {
            let run_end = scan_plain(input, p, end);
            if run_end > p {
                let (next, units) = self.decode_run::<dyn ContentHandler>(None, input, p, run_end)?;
                if next < run_end {
                    return Err(malformed_utf8());
                }
                src_units += units;
                p = next;
            }
            if p >= end {
                break;
            }
            // at a backslash
            let Some((ch, n)) = resolve_escape(input, p)? else {
                return Err(ParseError::new("Incomplete escape sequence in key"));
            };
            if ch.len_utf8() > self.free() {
                return Err(ParseError::new(
                    "Object key is longer than the string chunk size",
                ));
            }
            self.chars.push(ch);
            src_units += n; // raw escape bytes are always ASCII
            p += n;
        }
        Ok(src_units)
    }

    /// Continues the number in progress with the bytes of `input`.
    ///
    /// Returns true if the number ended (at a byte left unconsumed), false if
    /// `input` ran out first.
    fn continue_number<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        limits: &ParserLimits,
        locator: &SharedLocator,
        input: Input<'_>,
        consumed: &mut usize,
    ) -> ParseResult<bool> {
        let limit = limits.max_number_length;
        let total = input.len();
        let too_long = |limit: i32| ParseError::new(format!("Maximum number length exceeded: {limit}"));

        while *consumed < total {
            let n = &mut self.number;
            if !n.mantissa_full && matches!(n.stage, NumStage::Integer | NumStage::Fraction) {
                let taken = n.take_digit_run(input.span_from(*consumed));
                if taken > 0 {
                    *consumed += taken;
                    if limit > 0 && n.length > limit as usize {
                        return Err(too_long(limit));
                    }
                    if *consumed >= total {
                        break;
                    }
                }
            }
            let b = input.get(*consumed);
            let digit = b.wrapping_sub(b'0');
            let is_digit = digit <= 9;
            match n.stage {
                NumStage::AfterMinus => {
                    if b == b'0' {
                        n.stage = NumStage::Zero;
                    } else if is_digit {
                        n.stage = NumStage::Integer;
                        n.take_digit(digit, false);
                    } else {
                        return Err(ParseError::new("Invalid number format"));
                    }
                }
                NumStage::Zero => {
                    if is_digit {
                        return Err(ParseError::new("Numbers cannot have leading zeros"));
                    } else if b == b'.' {
                        n.is_real = true;
                        n.stage = NumStage::FractionDot;
                    } else if b == b'e' || b == b'E' {
                        n.is_real = true;
                        n.stage = NumStage::ExponentMark;
                    } else {
                        self.emit_number(handler, locator)?;
                        return Ok(true);
                    }
                }
                NumStage::Integer => {
                    if is_digit {
                        n.take_digit(digit, false);
                    } else if b == b'.' {
                        n.is_real = true;
                        n.stage = NumStage::FractionDot;
                    } else if b == b'e' || b == b'E' {
                        n.is_real = true;
                        n.stage = NumStage::ExponentMark;
                    } else {
                        self.emit_number(handler, locator)?;
                        return Ok(true);
                    }
                }
                NumStage::FractionDot => {
                    if !is_digit {
                        return Err(ParseError::new("Decimal point must be followed by digit"));
                    }
                    n.stage = NumStage::Fraction;
                    n.take_digit(digit, true);
                }
                NumStage::Fraction => {
                    if is_digit {
                        n.take_digit(digit, true);
                    } else if b == b'e' || b == b'E' {
                        n.is_real = true;
                        n.stage = NumStage::ExponentMark;
                    } else {
                        self.emit_number(handler, locator)?;
                        return Ok(true);
                    }
                }
                NumStage::ExponentMark => {
                    if b == b'+' || b == b'-' {
                        n.exponent_negative = b == b'-';
                        n.stage = NumStage::ExponentSign;
                    } else if is_digit {
                        n.stage = NumStage::Exponent;
                        n.take_exponent_digit(digit);
                    } else {
                        return Err(ParseError::new("Exponent must have digit"));
                    }
                }
                NumStage::ExponentSign => {
                    if !is_digit {
                        return Err(ParseError::new("Exponent must have digit"));
                    }
                    n.stage = NumStage::Exponent;
                    n.take_exponent_digit(digit);
                }
                NumStage::Exponent => {
                    if is_digit {
                        n.take_exponent_digit(digit);
                    } else {
                        self.emit_number(handler, locator)?;
                        return Ok(true);
                    }
                }
                NumStage::None => unreachable!("not in a number"),
            }
            *consumed += 1;
            n.length += 1;
            if limit > 0 && n.length > limit as usize {
                return Err(too_long(limit));
            }
        }
        Ok(false)
    }

    /// Reports the finished number to the handler.
    fn emit_number<H: ContentHandler + ?Sized>(
        &mut self,
        handler: &mut H,
        locator: &SharedLocator,
    ) -> ParseResult<()> {
        let number = if self.number.is_real {
            self.number.real_value()
        } else {
            self.number.integer_value()
        };
        self.number.stage = NumStage::None;
        self.state = State::AfterValue;
        locator.add_column(self.number.length);
        handler.number_value(&number)
    }
}

fn string_too_long(limit: i32) -> ParseError {
    ParseError::new(format!("Maximum string length exceeded: {limit}"))
}

fn malformed_utf8() -> ParseError {
    ParseError::new("Malformed UTF-8")
}

/// Rejects a repeated key if asked to, advances the locator past the key and
/// its two quotes, and reports it.
fn report_key<H: ContentHandler + ?Sized>(
    handler: &mut H,
    locator: &SharedLocator,
    duplicates: Option<&mut HashSet<String>>,
    key: &str,
    src_units: usize,
) -> ParseResult<bool> {
    if let Some(seen) = duplicates {
        if !seen.insert(key.to_owned()) {
            return Err(ParseError::new(format!("Duplicate key: {key}")));
        }
    }
    locator.add_column(2 + src_units); // +2 for the opening and closing quotes
    handler.key(key)?;
    Ok(true)
}

/// Number of UTF-16 units in `s`: what a locator column counts.
///
/// Every character is one unit, or two if it is outside the BMP, and in UTF-8
/// that is every non-continuation byte, plus one more for each 4-byte lead.
/// Counting bytes needs no decoding and compiles to a vector loop. Plain ASCII,
/// the common case, is one unit per byte and is checked a word at a time.
fn utf16_len(s: &str) -> usize {
    if s.is_ascii() {
        return s.len();
    }
    s.as_bytes()
        .iter()
        .map(|&b| usize::from(b & 0xC0 != 0x80) + usize::from(b >= 0xF0))
        .sum()
}

/// Index of the first byte in `[from, to)` that ends a run of plain string
/// content (a quote, a backslash or a control character), or `to` if there is
/// none. UTF-8 is self-synchronizing, so none of those bytes can occur inside a
/// multi-byte sequence.
fn scan_plain(input: Input<'_>, from: usize, to: usize) -> usize {
    let mut i = from;
    while i < to {
        let span = input.span_from(i);
        let n = span.len().min(to - i);
        if let Some(k) = span[..n]
            .iter()
            .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
        {
            return i + k;
        }
        i += n;
    }
    to
}

/// Decodes the one character starting at `p`, which may straddle the ring's
/// wrap. `None` if too few bytes are present to tell yet.
fn decode_char_at(input: Input<'_>, p: usize) -> ParseResult<Option<(char, usize)>> {
    let need = match input.get(p) {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return Err(malformed_utf8()),
    };
    let mut bytes = [0u8; 4];
    bytes[0] = input.get(p);
    for (k, slot) in bytes.iter_mut().enumerate().take(need).skip(1) {
        if p + k >= input.len() {
            return Ok(None);
        }
        let b = input.get(p + k);
        if b & 0xC0 != 0x80 {
            return Err(malformed_utf8());
        }
        *slot = b;
    }
    match std::str::from_utf8(&bytes[..need]) {
        Ok(s) => Ok(Some((s.chars().next().expect("one character"), need))),
        Err(_) => Err(malformed_utf8()), // overlong, surrogate or out of range
    }
}

fn unhex(b: u8) -> ParseResult<u32> {
    match b {
        b'0'..=b'9' => Ok(u32::from(b - b'0')),
        b'A'..=b'F' => Ok(u32::from(b - b'A' + 10)),
        b'a'..=b'f' => Ok(u32::from(b - b'a' + 10)),
        _ => Err(ParseError::new(format!("Invalid hex digit: {}", b as char))),
    }
}

/// Up to 6 bytes starting at `i` (the longest escape), copied out so they can
/// be examined without a ring-span check per byte, wherever they sit.
fn peek_escape(input: Input<'_>, i: usize) -> ([u8; 6], usize) {
    let mut bytes = [0u8; 6];
    let span = input.span_from(i);
    if span.len() >= 6 {
        // the common case: a fixed-size copy, which is one or two loads
        let head: [u8; 6] = span[..6].try_into().expect("six bytes");
        return (head, 6);
    }
    let mut n = span.len();
    bytes[..n].copy_from_slice(span);
    // the rest, if the escape straddles the wrap
    while n < 6 && i + n < input.len() {
        bytes[n] = input.get(i + n);
        n += 1;
    }
    (bytes, n)
}

/// The escape sequence in `bytes`, which starts at its backslash. Bytes
/// present are checked in order, so an invalid escape is reported as soon as
/// it is seen.
fn parse_escape(bytes: &[u8]) -> ParseResult<Escape> {
    if bytes.len() < 2 {
        return Ok(Escape::NeedMore);
    }
    let c = bytes[1];
    let simple: u16 = match c {
        b'"' => u16::from(b'"'),
        b'\\' => u16::from(b'\\'),
        b'/' => u16::from(b'/'),
        b'b' => 0x08,
        b'f' => 0x0c,
        b'n' => u16::from(b'\n'),
        b'r' => u16::from(b'\r'),
        b't' => u16::from(b'\t'),
        b'u' => {
            let mut value = 0u32;
            for k in 2..6 {
                match bytes.get(k) {
                    Some(&h) => value = (value << 4) | unhex(h)?,
                    None => return Ok(Escape::NeedMore),
                }
            }
            return Ok(Escape::Unit(value as u16, 6));
        }
        _ => {
            return Err(ParseError::new(format!(
                "Invalid escape sequence: \\{}",
                c as char
            )))
        }
    };
    Ok(Escape::Unit(simple, 2))
}

/// What follows an escaped high surrogate.
enum Follow {
    /// Too few bytes are present to tell.
    NeedMore,
    /// Not an escaped low surrogate: the high surrogate is on its own.
    Not,
    /// An escaped low surrogate (`\uDC00` to `\uDFFF`), with its value.
    Low(u32),
}

fn low_surrogate_follows(bytes: &[u8]) -> Follow {
    match bytes {
        [] => Follow::NeedMore,
        [first, ..] if *first != b'\\' => Follow::Not,
        [_] => Follow::NeedMore,
        [_, second, ..] if *second != b'u' => Follow::Not,
        _ if bytes.len() < 6 => Follow::NeedMore,
        _ => {
            let mut value = 0u32;
            for &h in &bytes[2..6] {
                match unhex(h) {
                    Ok(d) => value = (value << 4) | d,
                    Err(_) => return Follow::Not, // reported when it is reached
                }
            }
            if (0xDC00..=0xDFFF).contains(&value) {
                Follow::Low(value)
            } else {
                Follow::Not
            }
        }
    }
}

/// The character for the escape at `i` and the source bytes it took, or `None`
/// if more data is needed to tell. An escaped surrogate pair is resolved
/// together so no content call ends between its halves. A surrogate with no
/// partner cannot be a Rust `char`, so it becomes U+FFFD.
fn resolve_escape(input: Input<'_>, i: usize) -> ParseResult<Option<(char, usize)>> {
    let (bytes, n) = peek_escape(input, i);
    let (unit, len) = match parse_escape(&bytes[..n])? {
        Escape::NeedMore => return Ok(None),
        Escape::Unit(unit, len) => (u32::from(unit), len),
    };
    if (0xD800..=0xDBFF).contains(&unit) {
        let (next, m) = peek_escape(input, i + len);
        return match low_surrogate_follows(&next[..m]) {
            Follow::NeedMore => Ok(None),
            Follow::Not => Ok(Some((char::REPLACEMENT_CHARACTER, len))),
            Follow::Low(low) => {
                let cp = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                Ok(Some((
                    char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER),
                    len + 6,
                )))
            }
        };
    }
    // a low surrogate on its own falls out as U+FFFD too
    Ok(Some((char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER), len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_len_counts_units_without_decoding() {
        for s in [
            "",
            "plain ascii",
            "caf\u{e9}",
            "\u{20ac}\u{4e2d}",
            "\u{1f600}",
            "a\u{e9}\u{20ac}\u{1f600}z\u{1f4a9}\u{10ffff}",
        ] {
            let expected: usize = s.chars().map(char::len_utf16).sum();
            assert_eq!(utf16_len(s), expected, "{s:?}");
        }
    }
}
