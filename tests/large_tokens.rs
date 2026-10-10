//! A large token arriving in small chunks must cost O(n), not O(n^2):
//! the parser may neither re-copy its leftover nor re-scan the pending
//! token from its start on every receive().

mod common;

use common::parse_chunked;
use std::time::{Duration, Instant};

const SIZE: usize = 2_000_000;
const CHUNK: usize = 1024;
// Linear parsing takes milliseconds; the quadratic version takes many seconds.
const BUDGET: Duration = Duration::from_secs(3);

fn timed(json: &[u8]) -> common::RecordingHandler {
    let start = Instant::now();
    let h = parse_chunked(json, CHUNK).unwrap();
    assert!(start.elapsed() < BUDGET, "took {:?}", start.elapsed());
    h
}

#[test]
fn huge_plain_string_in_small_chunks() {
    let json = format!("\"{}\"", "a".repeat(SIZE));
    let h = timed(json.as_bytes());
    assert_eq!(h.strings[0].len(), SIZE);
}

#[test]
fn huge_string_with_early_escape_in_small_chunks() {
    // An escape at the front defeats any "skip the plain prefix" shortcut.
    let json = format!("\"\\n{}\\u00e9{}\"", "b".repeat(SIZE / 2), "c".repeat(SIZE / 2));
    let h = timed(json.as_bytes());
    assert_eq!(h.strings[0].chars().count(), SIZE + 2);
    assert!(h.strings[0].starts_with("\nb"));
    assert!(h.strings[0].contains("\u{e9}c"));
}

#[test]
fn huge_non_ascii_string_in_small_chunks() {
    let json = format!("\"{}\"", "\u{4e2d}".repeat(SIZE / 3));
    let h = timed(json.as_bytes());
    assert_eq!(h.strings[0].chars().count(), SIZE / 3);
}

#[test]
fn huge_key_in_small_chunks() {
    let json = format!("{{\"{}\":1}}", "k".repeat(SIZE));
    let h = timed(json.as_bytes());
    assert_eq!(h.keys[0].len(), SIZE);
}
