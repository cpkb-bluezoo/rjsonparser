# rjsonparser

Push-based JSON parser and writer for Rust.

**rjsonparser** ports [jsonparser](https://github.com/cpkb-bluezoo/jsonparser) to Rust, using the same design as [rprotobuf](https://github.com/cpkb-bluezoo/rprotobuf) and [rmimeparser](https://github.com/cpkb-bluezoo/rmimeparser):

- Incremental `receive()` parsing — SAX-style events, no DOM
- [`ContentHandler`](src/handler.rs) callbacks (Java `JSONContentHandler`)
- Symmetric `Writer` for encoding (`JSONWriter`)
- Zero dependencies beyond the Rust standard library

## Parser

```rust
use rjsonparser::{ContentHandler, Number, Parser, ParseResult};

struct Events {
    count: usize,
}

impl ContentHandler for Events {
    fn number_value(&mut self, _value: &Number) -> ParseResult<()> {
        self.count += 1;
        Ok(())
    }
}

let mut handler = Events { count: 0 };
let mut parser = Parser::new(&mut handler);
let mut input = &br#"[1,2,3]"#[..];
parser.receive(&mut input).unwrap();
parser.close().unwrap();
assert_eq!(handler.count, 3);
```

### Streaming (NIO-style buffer contract)

```rust
// Compact/flip pattern: unconsumed bytes are also retained in the parser
// leftover so close() can finalize a trailing number like `42`.
loop {
    // read more bytes into `buf`...
    let mut slice = &buf[..filled];
    parser.receive(&mut slice)?;
}
parser.close()?;
```

## Writer

```rust
use rjsonparser::{Number, Writer};

let mut w = Writer::buffer(128);
w.write_start_object()?;
w.write_key("name")?;
w.write_string("Alice")?;
w.write_key("age")?;
w.write_number(&Number::I32(30))?;
w.write_end_object()?;
let bytes = w.finish()?;
```

## Testing

```bash
# Same corpus CI uses: https://github.com/nst/JSONTestSuite
./scripts/fetch-json-test-suite.sh
cargo test
```

`fetch-json-test-suite.sh` clones the suite into `JSONTestSuite/` (gitignored). CI does the equivalent checkout before `cargo test`.

## Relationship to other bluezoo libraries

| Library | Format | Pattern |
|---------|--------|---------|
| [jsonparser](https://github.com/cpkb-bluezoo/jsonparser) | JSON (Java) | `JSONContentHandler` + `receive` |
| **rjsonparser** | JSON (Rust) | `ContentHandler` + `receive` |
| [rprotobuf](https://github.com/cpkb-bluezoo/rprotobuf) | Protobuf (Rust) | `Handler` + `receive` |
| [rmimeparser](https://github.com/cpkb-bluezoo/rmimeparser) | MIME (Rust) | `MimeHandler` + `receive` |

## License

LGPL-2.1-or-later. See [LICENSE](LICENSE).
