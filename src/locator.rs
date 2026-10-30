use std::sync::atomic::{AtomicUsize, Ordering};

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
/// Atomics rather than `Cell`s so a handler holding the locator does not make
/// the parser `!Send`. There is a single writer, so loads and stores are
/// relaxed and cost the same as plain ones.
#[derive(Debug)]
pub(crate) struct SharedLocator {
    line: AtomicUsize,
    column: AtomicUsize,
}

impl SharedLocator {
    pub(crate) fn new() -> Self {
        Self {
            line: AtomicUsize::new(1),
            column: AtomicUsize::new(1),
        }
    }

    pub(crate) fn set_line(&self, line: usize) {
        self.line.store(line, Ordering::Relaxed);
    }

    pub(crate) fn set_column(&self, column: usize) {
        self.column.store(column, Ordering::Relaxed);
    }

    pub(crate) fn add_column(&self, delta: usize) {
        self.column
            .store(self.column.load(Ordering::Relaxed) + delta, Ordering::Relaxed);
    }
}

impl Locator for SharedLocator {
    fn line_number(&self) -> usize {
        self.line.load(Ordering::Relaxed)
    }

    fn column_number(&self) -> usize {
        self.column.load(Ordering::Relaxed)
    }
}
