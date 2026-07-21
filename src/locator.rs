/// Locator information for the JSON parser.
///
/// Line and column numbers are 1-based and are only valid during the
/// corresponding handler callback or error.
pub trait Locator {
    /// Current line number (first line is 1).
    fn line_number(&self) -> usize;

    /// Current column number (first column is 1).
    fn column_number(&self) -> usize;
}
