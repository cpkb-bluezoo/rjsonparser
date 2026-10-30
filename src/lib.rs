//! Push-based JSON parser and writer for Rust.
//!
//! **rjsonparser** ports the [jsonparser](https://github.com/cpkb-bluezoo/jsonparser)
//! Java library to Rust, using the same streaming design:
//!
//! - Incremental [`Parser::receive`] parsing straight out of a caller-owned
//!   [`loopbuf::LoopBuf`] ring -- the same ring every call, no copy at the
//!   `receive` boundary at all. The parser consumes only whole tokens and
//!   leaves anything incomplete in the ring to be presented again with more
//!   data; it holds none of the document's bytes itself
//! - One fixed-size character buffer for string content, handed to the
//!   handler as a `&str`, so memory does not grow with the size of a string
//! - [`ContentHandler`] callbacks instead of DOM trees
//! - Symmetric [`Writer`] for encoding
//! - Only [`loopbuf`] as a runtime dependency, used for the feed buffer
//!
//! # Example
//!
//! ```
//! use loopbuf::LoopBuf;
//! use rjsonparser::{ContentHandler, Number, Parser, Writer};
//!
//! struct Events {
//!     log: Vec<String>,
//! }
//!
//! impl ContentHandler for Events {
//!     fn start_object(&mut self) -> rjsonparser::ParseResult<()> {
//!         self.log.push("start_object".into());
//!         Ok(())
//!     }
//!     fn key(&mut self, key: &str) -> rjsonparser::ParseResult<()> {
//!         self.log.push(format!("key:{key}"));
//!         Ok(())
//!     }
//!     fn number_value(&mut self, value: &Number) -> rjsonparser::ParseResult<()> {
//!         self.log.push(format!("number:{value}"));
//!         Ok(())
//!     }
//!     fn end_object(&mut self) -> rjsonparser::ParseResult<()> {
//!         self.log.push("end_object".into());
//!         Ok(())
//!     }
//! }
//!
//! let json = br#"{"a":1}"#;
//! let mut handler = Events { log: Vec::new() };
//! let mut parser = Parser::new(&mut handler);
//! let mut buf = LoopBuf::with_capacity(64);
//! buf.extend_from_slice(json);
//! parser.receive(&mut buf).unwrap();
//! parser.close(&mut buf).unwrap();
//! ```

mod keys;
mod limits;
mod locator;
mod number;
mod parser;
mod tokenizer;
mod writer;

pub mod error;
pub mod handler;

pub use error::{ParseError, ParseResult, WriteError, WriteResult};
pub use handler::{ContentHandler, DefaultHandler};
pub use limits::ParserLimits;
pub use locator::Locator;
pub use number::Number;
pub use parser::Parser;
pub use writer::{IndentConfig, Writer};
