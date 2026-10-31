# rjsonparser

Push-based JSON parser and writer for Rust.

**rjsonparser** ports [jsonparser](https://github.com/cpkb-bluezoo/jsonparser) to Rust, using the same design as [rprotobuf](https://github.com/cpkb-bluezoo/rprotobuf) and [rmimeparser](https://github.com/cpkb-bluezoo/rmimeparser):

- Incremental `receive()` parsing — SAX-style events, no DOM
- [`ContentHandler`](src/handler.rs) callbacks (Java `JSONContentHandler`)
- Symmetric `Writer` for encoding (`JSONWriter`)
- Only [`loopbuf`](https://github.com/cpkb-bluezoo/loopbuf) as a dependency: the caller owns a fixed-capacity ring, and the parser leaves whatever it cannot use yet in it

## Parser

```rust
use loopbuf::LoopBuf;
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
// The parser owns its handler: pass `&mut handler` to keep using it
// afterwards, or a `Box<dyn ContentHandler>` / the handler itself and get
// it back with `handler()` / `into_handler()`.
let mut parser = Parser::new(&mut handler);
let mut buf = LoopBuf::with_capacity(4096);
buf.extend_from_slice(b"[1,2,3]");
parser.receive(&mut buf).unwrap();
parser.close(&mut buf).unwrap();
assert_eq!(handler.count, 3);
```

### Streaming (caller-owned buffer contract)

The caller owns the [`LoopBuf`](https://github.com/cpkb-bluezoo/loopbuf) ring.
`receive` consumes only whole tokens and leaves anything incomplete (a key or
literal that is not finished, the start of an escape sequence or multi-byte
character, a partial byte order mark) in the ring. Read more in after it and
call `receive` again; reclaiming consumed space moves no data.

```rust
loop {
    let (spare, _) = buf.spare_capacity_mut();
    let n = reader.read(spare)?;
    if n == 0 {
        break;
    }
    buf.commit_write(n);
    parser.receive(&mut buf)?;
}
parser.close(&mut buf)?; // completes a trailing number like `42`
```

### Memory and limits

Memory does not grow with document size or string length. Besides the ring,
the parser has one character buffer of a fixed size
(`set_string_chunk_size`, 32768 bytes by default). String content is decoded
into it and handed to the handler as a `&str` as it arrives
(`start_string_value`, `string_value_content`, `end_string_value`), so a string
of any length costs the same.

Keys are delivered whole, so a key must fit both the ring and that buffer.
The ring must be larger than the longest key (`ParserLimits::max_name_length`,
4096 raw bytes by default) plus its quotes. If it is full of one unfinished
token, `receive` returns a "receive buffer too small" error instead of
buffering for itself. Strings are unlimited by default
(`ParserLimits::max_string_length` is an opt-in cap for handlers that
accumulate them).

Limits are a `ParserLimits` struct, each field `None` for no limit
(`max_nesting_depth`, `max_number_length`, `max_string_length`,
`max_name_length`, `max_document_length`, `max_token_count`): pass it to
`set_limits`, change one with `limits_mut()`, or start from
`ParserLimits::unlimited()`.

An escaped surrogate pair is delivered as one character. A surrogate with no
partner cannot be represented in a Rust `&str`, so it is reported as U+FFFD.

Threads: `receive` is always called on one thread, so nothing requires a handler
or the `Locator` it keeps (an `Rc<dyn Locator>`) to be `Send` or `Sync`, and the
parser itself is not `Send`.

Positions: a handler may keep the `Locator` from `set_locator` and read it
inside any callback. Lines and columns start at 1, a column counts UTF-16
units, and the position is where the event ends.

## Writer

The writer is the mirror of `receive(&mut LoopBuf)`: it encodes straight into a
caller-owned `LoopBuf` and keeps no buffer of its own, only where it is in the
document.

```rust
use loopbuf::LoopBuf;
use rjsonparser::{Number, Writer};

let mut ring = LoopBuf::with_capacity(4096);
let mut w = Writer::new();
w.write_start_object(&mut ring)?;
w.write_key(&mut ring, "name")?;
w.write_string(&mut ring, "Alice")?;
w.write_key(&mut ring, "age")?;
w.write_number(&mut ring, &Number::I32(30))?;
w.write_end_object(&mut ring)?;
// `ring` now holds the JSON: drain it to a socket or file, or parse it
```

Every call is **atomic**: it appends the whole token (with its separator and
indentation) or nothing, and leaves the writer's state unchanged. So
`err.is_would_block()` means the ring has no room right now: drain it and repeat
the same call.

```rust
loop {
    match w.write_key(&mut ring, "name") {
        Err(e) if e.is_would_block() => { io::copy(&mut ring, &mut socket)?; }
        other => break other?,
    }
}
```

A token larger than the ring's whole capacity can never fit and is an
`InvalidInput` error. Write a long string in pieces with `write_start_string`,
`write_string_content` (which takes as much as fits and returns how much) and
`write_end_string`; they are also what a parser's streamed string events feed,
so a string of any length passes through a small ring. NaN and the infinities
have no JSON representation and are rejected before anything is written.

## Testing

```bash
# Same corpus CI uses: https://github.com/nst/JSONTestSuite
./scripts/fetch-json-test-suite.sh
cargo test
```

`fetch-json-test-suite.sh` clones the suite into `JSONTestSuite/` (gitignored). CI does the equivalent checkout before `cargo test`. The suite runs as the `json_test_suite` integration test (one reported case per `y_*` / `n_*` / `i_*` file); it is not wired through `build.rs`, so dependents do not pay for suite discovery when compiling the library.

## Relationship to other bluezoo libraries

| Library | Format | Pattern |
|---------|--------|---------|
| [jsonparser](https://github.com/cpkb-bluezoo/jsonparser) | JSON (Java) | `JSONContentHandler` + `receive` |
| **rjsonparser** | JSON (Rust) | `ContentHandler` + `receive` |
| [rprotobuf](https://github.com/cpkb-bluezoo/rprotobuf) | Protobuf (Rust) | `Handler` + `receive` |
| [rmimeparser](https://github.com/cpkb-bluezoo/rmimeparser) | MIME (Rust) | `MimeHandler` + `receive` |

## License

LGPL-3.0-or-later. See [LICENSE](LICENSE).
