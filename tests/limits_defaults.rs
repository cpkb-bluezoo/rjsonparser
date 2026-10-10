//! Resource-limit tests ported from the Java LimitsTest.

use rjsonparser::{ContentHandler, DefaultHandler, Parser};

fn nested_array(depth: usize) -> String {
    "[".repeat(depth) + &"]".repeat(depth)
}

fn digits(count: usize) -> String {
    "9".repeat(count)
}

fn quoted(len: usize, fill: char) -> String {
    format!("\"{}\"", fill.to_string().repeat(len))
}

fn big_array(elements: usize) -> String {
    let items: Vec<String> = (0..elements).map(|i| i.to_string()).collect();
    format!("[{}]", items.join(","))
}

fn run<H: ContentHandler>(p: &mut Parser<H>, json: &str) -> bool {
    let mut data = json.as_bytes();
    p.receive(&mut data).is_ok() && p.close().is_ok()
}

fn ok(configure: impl FnOnce(&mut Parser<DefaultHandler>), json: &str) -> bool {
    let mut p = Parser::new(DefaultHandler);
    configure(&mut p);
    run(&mut p, json)
}

#[test]
fn nesting_depth_custom_and_default() {
    assert!(ok(|p| p.set_max_nesting_depth(3), &nested_array(3)));
    assert!(!ok(|p| p.set_max_nesting_depth(3), &nested_array(4)));
    assert!(ok(|_| {}, &nested_array(1000)));
    assert!(!ok(|_| {}, &nested_array(1001)));
    assert!(ok(|p| p.set_max_nesting_depth(0), &nested_array(5000)));
}

#[test]
fn number_length_custom_default_disabled() {
    assert!(ok(|p| p.set_max_number_length(5), &digits(5)));
    assert!(!ok(|p| p.set_max_number_length(5), &digits(6)));
    assert!(ok(|_| {}, &digits(1000)));
    assert!(!ok(|_| {}, &digits(1001)));
    assert!(ok(|p| p.disable_all_limits(), &digits(2000)));
}

#[test]
fn string_length_custom_default_disabled() {
    assert!(ok(|p| p.set_max_string_length(5), &quoted(5, 'a')));
    assert!(!ok(|p| p.set_max_string_length(5), &quoted(6, 'a')));
    assert!(ok(|p| p.disable_all_limits(), &quoted(100_000, 'a')));
    assert!(ok(|_| {}, &quoted(100_000, 'a')));
}

#[test]
fn name_length_applies_to_keys_only() {
    let key = |n| format!("{{{}:1}}", quoted(n, 'k'));
    assert!(ok(|p| p.set_max_name_length(5), &key(5)));
    assert!(!ok(|p| p.set_max_name_length(5), &key(6)));
    let value = format!("{{{}:{}}}", quoted(5, 'k'), quoted(50, 'v'));
    assert!(ok(
        |p| {
            p.set_max_name_length(5);
            p.set_max_string_length(0);
        },
        &value
    ));
}

#[test]
fn document_length_custom_and_default() {
    assert!(ok(|p| p.set_max_document_length(7), "[1,2,3]"));
    assert!(!ok(|p| p.set_max_document_length(6), "[1,2,3]"));
    assert!(ok(|_| {}, &quoted(100_000, 'a')));
}

#[test]
fn document_length_counts_across_chunks() {
    let mut p = Parser::new(DefaultHandler);
    p.set_max_document_length(6);
    let mut a = &b"[1,2"[..];
    p.receive(&mut a).unwrap();
    let mut b = &b",3]"[..];
    assert!(p.receive(&mut b).is_err());
}

#[test]
fn token_count_custom_and_default() {
    assert!(!ok(|p| p.set_max_token_count(100), &big_array(1000)));
    assert!(ok(|p| p.set_max_token_count(10_000), &big_array(1000)));
    assert!(ok(|_| {}, &big_array(10_000)));
}

#[test]
fn disable_all_limits_overrides_custom() {
    let tight = |p: &mut Parser<DefaultHandler>| {
        p.set_max_nesting_depth(2);
        p.set_max_string_length(2);
        p.set_max_number_length(2);
        p.set_max_name_length(2);
    };
    assert!(!ok(tight, &nested_array(5)));
    for json in [nested_array(5), quoted(50, 'a'), digits(50)] {
        assert!(ok(
            |p| {
                tight(p);
                p.disable_all_limits();
            },
            &json
        ));
    }
}

#[test]
fn limits_survive_reset() {
    let mut p = Parser::new(DefaultHandler);
    p.set_max_string_length(5);
    assert!(run(&mut p, &quoted(5, 'a')));
    p.reset();
    assert!(!run(&mut p, &quoted(6, 'a')));
}
