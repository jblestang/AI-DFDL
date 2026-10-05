//! `dfdl-xml`: Panic-free, `no_std` + `alloc` XML parser foundation.
//!
//! Provides streaming XML tokenization, event parsing, and namespace resolution required by DFDL schema compilation.

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod event;
pub mod limits;
pub mod reader;

pub use event::{Attribute, XmlEvent};
pub use limits::XmlReaderLimits;
pub use reader::XmlReader;
