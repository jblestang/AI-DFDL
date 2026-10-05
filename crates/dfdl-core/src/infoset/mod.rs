//! DFDL Information Set (Infoset) module.
//!
//! Aligned with DFDL 1.0 §4 and §5.1.

pub mod events;
pub mod state;
pub mod tree;
pub mod value;

pub use events::{InfosetEvent, InfosetEventSink, InfosetSource};
pub use state::ElementState;
pub use tree::{InfosetBuilder, InfosetDocument, InfosetElement, InfosetNode, InfosetTreeSource};
pub use value::{DfdlSimpleType, DfdlValue};
