//! Byte and Bit I/O traits, ByteOrder, BitOrder, and Checkpoint primitives.
//!
//! Aligned with DFDL 1.0 §9.2 Data Syntax Grammar.

use crate::error::DFDLResult;
use crate::types::BitOffset;

/// Byte ordering specification for multi-byte values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ByteOrder {
    /// Big-Endian (Most Significant Byte First).
    #[default]
    BigEndian,
    /// Little-Endian (Least Significant Byte First).
    LittleEndian,
}

/// Bit ordering specification for bit-addressed operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BitOrder {
    /// Most Significant Bit First (Bit 7 is bit offset 0 in a byte).
    #[default]
    MostSignificantBitFirst,
    /// Least Significant Bit First (Bit 0 is bit offset 0 in a byte).
    LeastSignificantBitFirst,
}

/// Checkpoint marker for input and output state transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Checkpoint {
    /// Bit position at checkpoint.
    pub bit_position: BitOffset,
    /// Transaction depth sequence number.
    pub sequence: u64,
}

/// Abstract trait for byte-oriented data sources.
pub trait ByteSource {
    /// Reads a single byte from the source.
    fn read_byte(&mut self) -> DFDLResult<u8>;

    /// Reads up to `buf.len()` bytes into `buf`.
    fn read_bytes(&mut self, buf: &mut [u8]) -> DFDLResult<()>;

    /// Returns the current bit position of the source.
    fn position(&self) -> BitOffset;

    /// Sets the current bit position of the source.
    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()>;

    /// Returns the remaining available bytes in the source if known.
    fn remaining_bytes(&self) -> usize;
}

/// Abstract trait for byte-oriented data sinks.
pub trait ByteSink {
    /// Writes a single byte to the sink.
    fn write_byte(&mut self, byte: u8) -> DFDLResult<()>;

    /// Writes a slice of bytes to the sink.
    fn write_bytes(&mut self, buf: &[u8]) -> DFDLResult<()>;

    /// Returns the current bit position written to the sink.
    fn position(&self) -> BitOffset;

    /// Truncates or rolls back the sink to a previous position.
    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_enum_defaults() {
        assert_eq!(ByteOrder::default(), ByteOrder::BigEndian);
        assert_eq!(BitOrder::default(), BitOrder::MostSignificantBitFirst);
    }
}
