use rjsonparser::{IndentConfig, Number, Writer};

#[test]
fn simple_object() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("name").unwrap();
    w.write_string("Alice").unwrap();
    w.write_key("age").unwrap();
    w.write_number(&Number::I32(30)).unwrap();
    w.write_end_object().unwrap();
    assert_eq!(
        String::from_utf8(w.finish().unwrap()).unwrap(),
        r#"{"name":"Alice","age":30}"#
    );
}

#[test]
fn simple_array() {
    let mut w = Writer::buffer(64);
    w.write_start_array().unwrap();
    w.write_number(&Number::I32(1)).unwrap();
    w.write_number(&Number::I32(2)).unwrap();
    w.write_number(&Number::I32(3)).unwrap();
    w.write_end_array().unwrap();
    assert_eq!(String::from_utf8(w.finish().unwrap()).unwrap(), "[1,2,3]");
}

#[test]
fn nested_structures() {
    let mut w = Writer::buffer(128);
    w.write_start_object().unwrap();
    w.write_key("users").unwrap();
    w.write_start_array().unwrap();
    w.write_start_object().unwrap();
    w.write_key("id").unwrap();
    w.write_number(&Number::I32(1)).unwrap();
    w.write_key("name").unwrap();
    w.write_string("Alice").unwrap();
    w.write_end_object().unwrap();
    w.write_start_object().unwrap();
    w.write_key("id").unwrap();
    w.write_number(&Number::I32(2)).unwrap();
    w.write_key("name").unwrap();
    w.write_string("Bob").unwrap();
    w.write_end_object().unwrap();
    w.write_end_array().unwrap();
    w.write_end_object().unwrap();
    assert_eq!(
        String::from_utf8(w.finish().unwrap()).unwrap(),
        r#"{"users":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#
    );
}

#[test]
fn string_escaping() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("text").unwrap();
    w.write_string("Hello\n\"World\"\t\r\u{0008}\u{000c}\\")
        .unwrap();
    w.write_end_object().unwrap();
    assert_eq!(
        String::from_utf8(w.finish().unwrap()).unwrap(),
        r#"{"text":"Hello\n\"World\"\t\r\b\f\\"}"#
    );
}

#[test]
fn control_character_escaping() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("ctrl").unwrap();
    w.write_string("test\u{0001}\u{001f}end").unwrap();
    w.write_end_object().unwrap();
    assert_eq!(
        String::from_utf8(w.finish().unwrap()).unwrap(),
        r#"{"ctrl":"test\u0001\u001fend"}"#
    );
}

#[test]
fn utf8_characters() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("emoji").unwrap();
    w.write_string("Hello 👋 World 🌍").unwrap();
    w.write_key("chinese").unwrap();
    w.write_string("你好世界").unwrap();
    w.write_end_object().unwrap();
    let json = String::from_utf8(w.finish().unwrap()).unwrap();
    assert!(json.contains("Hello 👋 World 🌍"));
    assert!(json.contains("你好世界"));
}

#[test]
fn primitive_values() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("bool_true").unwrap();
    w.write_boolean(true).unwrap();
    w.write_key("bool_false").unwrap();
    w.write_boolean(false).unwrap();
    w.write_key("null").unwrap();
    w.write_null().unwrap();
    w.write_key("int").unwrap();
    w.write_number(&Number::I32(42)).unwrap();
    w.write_key("float").unwrap();
    w.write_number(&Number::F64(3.14)).unwrap();
    w.write_end_object().unwrap();
    assert_eq!(
        String::from_utf8(w.finish().unwrap()).unwrap(),
        r#"{"bool_true":true,"bool_false":false,"null":null,"int":42,"float":3.14}"#
    );
}

#[test]
fn indent_spaces2_output() {
    let mut w = Writer::with_indent(Vec::new(), IndentConfig::spaces2());
    w.write_start_object().unwrap();
    w.write_key("a").unwrap();
    w.write_number(&Number::I32(1)).unwrap();
    w.write_end_object().unwrap();
    let json = String::from_utf8(w.finish().unwrap()).unwrap();
    assert_eq!(json, "{\n  \"a\": 1\n}");
}
