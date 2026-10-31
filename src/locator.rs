use std::cell::Cell;

/// Locator information for the JSON parser (Java `JSONLocator`).
///
/// Follows the rule of SAX's `org.xml.sax.Locator`: line and column numbers
/// start at 1, and the position is where the current event *ends*, the first
/// character after the text associated with it. A column counts UTF-16 units
/// since the last line end, and CR, LF and CRLF are each one line break.
///
/// A handler receives one of these from [`crate::ContentHandler::set_locator`]
/// and may keep it, reading the live position from inside any later callback.
pub trait Locator {
    /// Current line number (first line is 1).
    fn line_number(&self) -> usize;

    /// Current column number (first column is 1).
    fn column_number(&self) -> usize;
}

/// Live line/column storage the tokenizer updates as it consumes bytes.
///
/// `Cell`s: `receive` is always called on one thread, so the parser, the
/// handler and any locator the handler keeps all live on it, and nothing here
/// needs to be `Send` or `Sync`.
#[derive(Debug)]
pub(crate) struct SharedLocator {
    line: Cell<usize>,
    column: Cell<usize>,
}

impl SharedLocator {
    pub(crate) fn new() -> Self {
        Self {
            line: Cell::new(1),
            column: Cell::new(1),
        }
    }

    pub(crate) fn set_line(&self, line: usize) {
        self.line.set(line);
    }

    pub(crate) fn set_column(&self, column: usize) {
        self.column.set(column);
    }

    pub(crate) fn add_column(&self, delta: usize) {
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
