use std::fmt;

/// Error during JSON parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// Error during JSON writing.
#[derive(Debug)]
pub struct WriteError {
    source: std::io::Error,
}

impl WriteError {
    pub fn new(source: std::io::Error) -> Self {
        Self { source }
    }
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json write error: {}", self.source)
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl From<std::io::Error> for WriteError {
    fn from(source: std::io::Error) -> Self {
        Self::new(source)
    }
}

pub type ParseResult<T> = Result<T, ParseError>;
pub type WriteResult<T> = Result<T, WriteError>;
