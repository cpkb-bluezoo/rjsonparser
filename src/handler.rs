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
