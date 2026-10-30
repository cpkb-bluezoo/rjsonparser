/// Configurable limits guarding against resource exhaustion (Jackson-aligned defaults).
///
/// A limit value `<= 0` disables that check.
#[derive(Debug, Clone)]
pub struct ParserLimits {
    /// Maximum object/array nesting depth. Default: 1000.
    pub max_nesting_depth: i32,
    /// Maximum characters in a single number token. Default: 1000.
    pub max_number_length: i32,
    /// Maximum size in raw bytes (before unescaping, excluding the quotes) of a
    /// single string value. Default: 0 (unlimited). A string is streamed to the
    /// handler a piece at a time, so its length costs the parser nothing; this is
    /// an opt-in cap for a handler that accumulates strings.
    pub max_string_length: i32,
    /// Maximum size in raw bytes (before unescaping, excluding the quotes) of a
    /// single object key. Default: 4096. A key is read whole from the caller's
    /// buffer, so it must fit that buffer (and the string chunk size).
    pub max_name_length: i32,
    /// Maximum total bytes in one document. Default: 0 (unlimited).
    pub max_document_length: i64,
    /// Maximum total tokens in one document. Default: 0 (unlimited).
    pub max_token_count: i64,
}

impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            max_nesting_depth: 1000,
            max_number_length: 1000,
            max_string_length: 0,
            max_name_length: 4096,
            max_document_length: 0,
            max_token_count: 0,
        }
    }
}

impl ParserLimits {
    /// Disables every limit (sets all six to 0).
    pub fn disable_all(&mut self) {
        self.max_nesting_depth = 0;
        self.max_number_length = 0;
        self.max_string_length = 0;
        self.max_name_length = 0;
        self.max_document_length = 0;
        self.max_token_count = 0;
    }
}
