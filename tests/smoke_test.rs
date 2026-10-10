use rjsonparser::{ContentHandler, Number, Parser, Writer};

#[derive(Default)]
struct EventRecorder {
    events: Vec<String>,
}

impl ContentHandler for EventRecorder {
    fn start_object(&mut self) -> rjsonparser::ParseResult<()> {
        self.events.push("start_object".into());
        Ok(())
    }

    fn end_object(&mut self) -> rjsonparser::ParseResult<()> {
        self.events.push("end_object".into());
        Ok(())
    }

    fn start_array(&mut self) -> rjsonparser::ParseResult<()> {
        self.events.push("start_array".into());
        Ok(())
    }

    fn end_array(&mut self) -> rjsonparser::ParseResult<()> {
        self.events.push("end_array".into());
        Ok(())
    }

    fn key(&mut self, key: &str) -> rjsonparser::ParseResult<()> {
        self.events.push(format!("key:{key}"));
        Ok(())
    }

    fn string_value(&mut self, value: &str) -> rjsonparser::ParseResult<()> {
        self.events.push(format!("string:{value}"));
        Ok(())
    }

    fn number_value(&mut self, value: &Number) -> rjsonparser::ParseResult<()> {
        self.events.push(format!("number:{value}"));
        Ok(())
    }

    fn boolean_value(&mut self, value: bool) -> rjsonparser::ParseResult<()> {
        self.events.push(format!("bool:{value}"));
        Ok(())
    }

    fn null_value(&mut self) -> rjsonparser::ParseResult<()> {
        self.events.push("null".into());
        Ok(())
    }
}

#[test]
fn parse_object_records_events() {
    let json = br#"{"a":1}"#;
    let mut handler = EventRecorder::default();
    let mut parser = Parser::new(&mut handler);
    let mut input = &json[..];
    parser.receive(&mut input).unwrap();
    parser.close().unwrap();

    assert_eq!(
        handler.events,
        vec![
            "start_object".to_string(),
            "key:a".to_string(),
            "number:1".to_string(),
            "end_object".to_string(),
        ]
    );
}

#[test]
fn chunked_receive() {
    let json = br#"{"a":1}"#;
    let mut handler = EventRecorder::default();
    let mut parser = Parser::new(&mut handler);

    let mut chunk1 = &json[..3];
    parser.receive(&mut chunk1).unwrap();
    assert!(parser.is_underflow());

    let mut chunk2 = &json[3..];
    parser.receive(&mut chunk2).unwrap();
    parser.close().unwrap();

    assert_eq!(
        handler.events,
        vec![
            "start_object".to_string(),
            "key:a".to_string(),
            "number:1".to_string(),
            "end_object".to_string(),
        ]
    );
}

#[test]
fn writer_roundtrip_basic() {
    let mut w = Writer::buffer(64);
    w.write_start_object().unwrap();
    w.write_key("name").unwrap();
    w.write_string("Alice").unwrap();
    w.write_key("age").unwrap();
    w.write_i32(30).unwrap();
    w.write_end_object().unwrap();
    let bytes = w.finish().unwrap();

    let mut handler = EventRecorder::default();
    let mut parser = Parser::new(&mut handler);
    let mut input = bytes.as_slice();
    parser.receive(&mut input).unwrap();
    parser.close().unwrap();

    assert_eq!(
        handler.events,
        vec![
            "start_object".to_string(),
            "key:name".to_string(),
            "string:Alice".to_string(),
            "key:age".to_string(),
            "number:30".to_string(),
            "end_object".to_string(),
        ]
    );
}

#[test]
fn bare_number_with_close() {
    let json = b"42";
    let mut handler = EventRecorder::default();
    let mut parser = Parser::new(&mut handler);
    let mut input = &json[..];
    parser.receive(&mut input).unwrap();
    parser.close().unwrap();

    assert_eq!(handler.events, vec!["number:42".to_string()]);
}

/// The parser owns its handler: a boxed handler picked at run time can be
/// fed chunk by chunk from inside another object and taken back at the end.
#[test]
fn parser_owns_a_boxed_handler_across_chunks() {
    use rjsonparser::{ContentHandler, ParseResult, Parser};

    #[derive(Default)]
    struct Keys(Vec<String>);
    impl ContentHandler for Keys {
        fn key(&mut self, key: &str) -> ParseResult<()> {
            self.0.push(key.to_string());
            Ok(())
        }
    }
    trait KeyList: ContentHandler {
        fn keys(&self) -> &[String];
    }
    impl KeyList for Keys {
        fn keys(&self) -> &[String] {
            &self.0
        }
    }

    struct Response {
        parser: Parser<Box<dyn KeyList>>,
    }
    let mut r = Response { parser: Parser::new(Box::new(Keys::default()) as Box<dyn KeyList>) };
    for chunk in [&b"{\"a\": 1, \"lo"[..], &b"ng\": [true], \"c\": nu"[..], &b"ll}"[..]] {
        let mut data = chunk;
        r.parser.receive(&mut data).unwrap();
    }
    r.parser.close().unwrap();
    assert_eq!(r.parser.handler().keys(), ["a", "long", "c"]);
    let handler = r.parser.into_handler();
    assert_eq!(handler.keys().len(), 3);

    // Borrowed handlers still work as before.
    let mut keys = Keys::default();
    let mut p = Parser::new(&mut keys);
    let mut data = &b"{\"x\": 2}"[..];
    p.receive(&mut data).unwrap();
    p.close().unwrap();
    drop(p);
    assert_eq!(keys.0, ["x"]);
}
