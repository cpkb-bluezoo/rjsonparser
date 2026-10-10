use crate::error::ParseResult;
use crate::locator::Locator;
use crate::number::Number;

/// Application callback interface for JSON parse events (Java `JSONContentHandler`).
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

    fn string_value(&mut self, _value: &str) -> ParseResult<()> {
        Ok(())
    }

    fn boolean_value(&mut self, _value: bool) -> ParseResult<()> {
        Ok(())
    }

    fn null_value(&mut self) -> ParseResult<()> {
        Ok(())
    }

    fn whitespace(&mut self, _whitespace: &str) -> ParseResult<()> {
        Ok(())
    }

    fn key(&mut self, _key: &str) -> ParseResult<()> {
        Ok(())
    }

    /// Called once when the tokenizer is created for a document.
    fn set_locator(&mut self, _locator: &dyn Locator) {}

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
            fn string_value(&mut self, value: &str) -> ParseResult<()> {
                (**self).string_value(value)
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
            fn set_locator(&mut self, locator: &dyn Locator) {
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
