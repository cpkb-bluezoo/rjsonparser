use std::sync::Arc;

use crate::error::ParseResult;
use crate::locator::Locator;
use crate::number::Number;

/// Application callback interface for JSON parse events (Java `JSONContentHandler`).
///
/// # Streaming string values
///
/// A string value is reported as an explicit
/// `start_string_value`/`string_value_content` (zero or more times)/
/// `end_string_value` sequence, rather than one callback carrying the
/// whole value. This is what lets the parser stream a value of any size
/// through a fixed-capacity buffer with no buffering of its own, and it
/// avoids the ambiguity a single `(chunk, end: bool)` callback invites
/// (an awkward zero-length final chunk, and nothing to stop `end` from
/// appearing settable at any point): three explicit events describe the
/// sequence unambiguously instead of asking the caller to infer it from
/// an argument.
///
/// Keys and whitespace runs don't get this treatment:
///
/// - A key is read whole from the caller's ring and decoded into the
///   parser's character buffer, so it must fit both (see
///   [`crate::Parser::set_max_name_length`] and
///   [`crate::Parser::set_string_chunk_size`]); a ring too small for it is
///   an error, not something this API streams around. [`Self::key`] always
///   reports a key in one call.
/// - Whitespace has no delimiter marking a run's end at all (unlike a
///   string's closing quote), so there is nothing to bracket with
///   start/end events in the first place: [`Self::whitespace`] may
///   simply be called more than once for what is logically one run.
pub trait ContentHandler {
    fn start_object(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn end_object(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn start_array(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn end_array(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn number_value(&mut self, _value: &Number) -> ParseResult<()> {
        Ok(())
    }

    /// Signals the start of a string value. Followed by zero or more
    /// [`Self::string_value_content`] calls, then exactly one
    /// [`Self::end_string_value`].
    fn start_string_value(&mut self) -> ParseResult<()> {
        Ok(())
    }

    /// Reports a chunk of the current string value's content. Never called
    /// with an empty chunk, and never more than
    /// [`crate::Parser::set_string_chunk_size`] bytes. The chunk borrows the
    /// parser's character buffer, so it is only valid during the call. It is
    /// provisional until [`Self::end_string_value`], because a malformed
    /// string is only detected when it is reached.
    fn string_value_content(&mut self, _chunk: &str) -> ParseResult<()> {
        Ok(())
    }

    /// Signals that the string value most recently started by
    /// [`Self::start_string_value`] is complete.
    fn end_string_value(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn boolean_value(&mut self, _value: bool) -> ParseResult<()> {
        Ok(())
    }

    fn null_value(&mut self) -> ParseResult<()> {
        Ok(())
    }

    /// Reports a chunk of whitespace. May be called more than once for
    /// what is logically a single run of whitespace -- there is no
    /// delimiter marking a run's end, so no start/end pair brackets this
    /// the way a string value gets one. Only called at all when
    /// [`Self::needs_whitespace`] returns true.
    fn whitespace(&mut self, _whitespace: &str) -> ParseResult<()> {
        Ok(())
    }

    /// Reports an object key. Always reported in one call -- a key must fit
    /// the caller's ring and the parser's character buffer (see the trait
    /// docs); if it doesn't, `receive` returns an error.
    fn key(&mut self, _key: &str) -> ParseResult<()> {
        Ok(())
    }

    /// Called once when the tokenizer is created for a document. The
    /// locator is live: keep it and read it from inside later callbacks.
    fn set_locator(&mut self, _locator: Arc<dyn Locator + Send + Sync>) {}

    /// When false (default), the parser skips whitespace string extraction.
    fn needs_whitespace(&self) -> bool {
        false
    }
}

/// No-op content handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultHandler;

impl ContentHandler for DefaultHandler {}

macro_rules! forward_content_handler {
    ($target:ty) => {
        impl<T: ContentHandler + ?Sized> ContentHandler for $target {
            fn start_object(&mut self) -> ParseResult<()> {
                (**self).start_object()
            }
            fn end_object(&mut self) -> ParseResult<()> {
                (**self).end_object()
            }
            fn start_array(&mut self) -> ParseResult<()> {
                (**self).start_array()
            }
            fn end_array(&mut self) -> ParseResult<()> {
                (**self).end_array()
            }
            fn number_value(&mut self, value: &Number) -> ParseResult<()> {
                (**self).number_value(value)
            }
            fn start_string_value(&mut self) -> ParseResult<()> {
                (**self).start_string_value()
            }
            fn string_value_content(&mut self, chunk: &str) -> ParseResult<()> {
                (**self).string_value_content(chunk)
            }
            fn end_string_value(&mut self) -> ParseResult<()> {
                (**self).end_string_value()
            }
            fn boolean_value(&mut self, value: bool) -> ParseResult<()> {
                (**self).boolean_value(value)
            }
            fn null_value(&mut self) -> ParseResult<()> {
                (**self).null_value()
            }
            fn whitespace(&mut self, whitespace: &str) -> ParseResult<()> {
                (**self).whitespace(whitespace)
            }
            fn key(&mut self, key: &str) -> ParseResult<()> {
                (**self).key(key)
            }
            fn set_locator(&mut self, locator: Arc<dyn Locator + Send + Sync>) {
                (**self).set_locator(locator)
            }
            fn needs_whitespace(&self) -> bool {
                (**self).needs_whitespace()
            }
        }
    };
}

// A borrowed handler drives the parser like an owned one.
forward_content_handler!(&mut T);
// A boxed handler drives the parser like an owned one.
forward_content_handler!(Box<T>);
