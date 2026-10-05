//! Transactional Bit and Byte I/O substrate module.
//!
//! Aligned with DFDL 1.0 §9.2 Data Syntax Grammar.

pub mod bitstream;
pub mod sink;
pub mod source;
pub mod traits;

pub use bitstream::{BitReader, BitWriter};
pub use sink::{FixedSliceByteSink, VecByteSink};
pub use source::{BoundedSource, SliceByteSource};
pub use traits::{BitOrder, ByteOrder, ByteSink, ByteSource, Checkpoint};
