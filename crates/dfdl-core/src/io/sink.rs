//! Byte sink implementations: `FixedSliceByteSink` and `VecByteSink`.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::traits::ByteSink;
use crate::types::BitOffset;
use crate::util::{checked_add_usize, try_extend_from_slice, try_push};

/// Implements [`ByteSink`] writing to a fixed-capacity mutable byte slice without allocating.
#[derive(Debug)]
pub struct FixedSliceByteSink<'a> {
    buf: &'a mut [u8],
    position: usize,
}

impl<'a> FixedSliceByteSink<'a> {
    /// Constructs a new [`FixedSliceByteSink`] wrapping a mutable byte slice.
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, position: 0 }
    }
}

impl<'a> ByteSink for FixedSliceByteSink<'a> {
    fn write_byte(&mut self, byte: u8) -> DFDLResult<()> {
        if let Some(slot) = self.buf.get_mut(self.position) {
            *slot = byte;
            self.position = checked_add_usize(self.position, 1)?;
            Ok(())
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Output buffer capacity exhausted in fixed slice sink",
            ))
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> DFDLResult<()> {
        let end = checked_add_usize(self.position, bytes.len())?;
        if let Some(target) = self.buf.get_mut(self.position..end) {
            target.copy_from_slice(bytes);
            self.position = end;
            Ok(())
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Output buffer capacity exhausted writing byte block",
            ))
        }
    }

    fn position(&self) -> BitOffset {
        BitOffset(self.position.saturating_mul(8))
    }

    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()> {
        let byte_pos = pos
            .to_byte_offset()
            .ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::Unparse, "Sink position must be byte-aligned")
            })?
            .0;

        if byte_pos <= self.buf.len() {
            self.position = byte_pos;
            Ok(())
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::Unparse,
                "Attempted to set sink position past buffer bounds",
            ))
        }
    }
}

/// Implements [`ByteSink`] writing to a dynamic `Vec<u8>` using fallible growth (`try_reserve`).
#[derive(Debug, Clone, Default)]
pub struct VecByteSink {
    buf: Vec<u8>,
}

impl VecByteSink {
    /// Constructs a new empty [`VecByteSink`].
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Consumes the sink and returns the underlying byte vector.
    #[inline]
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    /// Returns a slice reference to written content.
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }
}

impl ByteSink for VecByteSink {
    fn write_byte(&mut self, byte: u8) -> DFDLResult<()> {
        try_push(&mut self.buf, byte)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> DFDLResult<()> {
        try_extend_from_slice(&mut self.buf, bytes)
    }

    fn position(&self) -> BitOffset {
        BitOffset(self.buf.len().saturating_mul(8))
    }

    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()> {
        let byte_pos = pos
            .to_byte_offset()
            .ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::Unparse, "Sink position must be byte-aligned")
            })?
            .0;

        if byte_pos <= self.buf.len() {
            self.buf.truncate(byte_pos);
            Ok(())
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::Unparse,
                "Cannot extend vector sink via set_position",
            ))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_slice_sink() {
        let mut buffer = [0u8; 4];
        {
            let mut sink = FixedSliceByteSink::new(&mut buffer);
            sink.write_byte(0xAA).unwrap();
            sink.write_bytes(&[0xBB, 0xCC]).unwrap();
            assert_eq!(sink.position(), BitOffset(24));
            sink.write_byte(0xDD).unwrap();
            assert!(sink.write_byte(0xEE).is_err());
        }
        assert_eq!(buffer, [0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn test_vec_byte_sink_rollback() {
        let mut sink = VecByteSink::new();
        sink.write_bytes(&[1, 2, 3, 4, 5]).unwrap();
        assert_eq!(sink.as_slice(), &[1, 2, 3, 4, 5]);

        sink.set_position(BitOffset(24)).unwrap(); // rollback to 3 bytes
        assert_eq!(sink.as_slice(), &[1, 2, 3]);

        // Non-aligned position returns error
        assert!(sink.set_position(BitOffset(10)).is_err());

        // Position past length returns error (64 bits = 8 bytes > 3 bytes)
        assert!(sink.set_position(BitOffset(64)).is_err());

        // write_byte and into_vec
        sink.write_byte(99).unwrap();
        assert_eq!(sink.position(), BitOffset(32));
        let vec = sink.into_vec();
        assert_eq!(vec, alloc::vec![1, 2, 3, 99]);
    }

    #[test]
    fn test_fixed_slice_sink_limits_and_positioning() {
        let mut buffer = [0u8; 4];
        let mut sink = FixedSliceByteSink::new(&mut buffer);

        // write_bytes overflow
        assert!(sink.write_bytes(&[1, 2, 3, 4, 5]).is_err());

        // Valid write_bytes
        assert!(sink.write_bytes(&[1, 2]).is_ok());
        assert_eq!(sink.position(), BitOffset(16));

        // set_position rollback to 1 byte
        assert!(sink.set_position(BitOffset(8)).is_ok());
        assert_eq!(sink.position(), BitOffset(8));

        // Non-aligned position
        assert!(sink.set_position(BitOffset(5)).is_err());

        // Out of bounds position
        assert!(sink.set_position(BitOffset(80)).is_err());
    }
}
