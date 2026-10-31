/// Limits guarding a parser against resource exhaustion.
///
/// A limit of `None` is no limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserLimits {
    /// The deepest nesting of objects and arrays. Default: 1000.
    pub max_nesting_depth: Option<usize>,
    /// The most characters in one number token. Default: 1000.
    pub max_number_length: Option<usize>,
    /// The most raw bytes (before unescaping, excluding the quotes) in one
    /// string value. Default: none. A string is streamed to the handler a
    /// piece at a time, so its length costs the parser nothing; this is a cap
    /// for a handler that accumulates strings.
    pub max_string_length: Option<usize>,
    /// The most raw bytes (before unescaping, excluding the quotes) in one
    /// object key. Default: 4096. A key is read whole from the caller's
    /// buffer, so it must fit that buffer (and the string chunk size).
    pub max_name_length: Option<usize>,
    /// The most bytes in one document. Default: none.
    pub max_document_length: Option<u64>,
    /// The most tokens in one document. Default: none.
    pub max_token_count: Option<u64>,
}

impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            max_nesting_depth: Some(1000),
            max_number_length: Some(1000),
            max_string_length: None,
            max_name_length: Some(4096),
            max_document_length: None,
            max_token_count: None,
        }
    }
}

impl ParserLimits {
    /// No limit at all.
    pub fn unlimited() -> Self {
        Self {
            max_nesting_depth: None,
            max_number_length: None,
            max_string_length: None,
            max_name_length: None,
            max_document_length: None,
            max_token_count: None,
        }
    }
}
