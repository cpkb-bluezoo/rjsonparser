//! Push-based JSON parser and writer for Rust.
//!
//! **rjsonparser** ports the [jsonparser](https://github.com/cpkb-bluezoo/jsonparser)
//! Java library to Rust, using the same streaming design:
//!
//! - Incremental [`Parser::receive`] parsing — constant memory, no internal buffering
//!   of incomplete tokens beyond the caller-visible leftover
//! - [`ContentHandler`] callbacks instead of DOM trees
//! - Symmetric [`Writer`] for encoding
//! - Zero dependencies beyond the Rust standard library
//!
//! # Example
//!
//! ```
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
//! let mut input = &json[..];
//! parser.receive(&mut input).unwrap();
//! parser.close().unwrap();
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
