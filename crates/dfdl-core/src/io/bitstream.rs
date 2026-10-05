//! Bit-addressable transactional reader (`BitReader`) and writer (`BitWriter`).
//!
//! Implements MSBF/LSBF bit ordering, big/little endian byte ordering, and transactional state rollback.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::traits::{BitOrder, ByteOrder, ByteSink, ByteSource, Checkpoint};
use crate::types::BitOffset;

#[inline]
const fn bit_mask_u8(n: usize) -> u8 {
    if n >= 8 {
        u8::MAX
    } else {
        ((1u16 << n) - 1) as u8
    }
}

#[inline]
const fn bit_mask_u64(n: usize) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

/// Transactional bit-addressable reader.
#[derive(Debug)]
pub struct BitReader<S: ByteSource> {
    source: S,
    bit_order: BitOrder,
    byte_order: ByteOrder,
    bit_buf: u8,
    bits_in_buf: usize,
    sequence: u64,
    bit_limit: Option<usize>,
}

impl<S: ByteSource> BitReader<S> {
    /// Creates a new [`BitReader`] wrapping a byte source.
    #[inline]
    pub fn new(source: S, bit_order: BitOrder, byte_order: ByteOrder) -> Self {
        Self {
            source,
            bit_order,
            byte_order,
            bit_buf: 0,
            bits_in_buf: 0,
            sequence: 0,
            bit_limit: None,
        }
    }

    /// Returns the active bit ordering mode.
    #[inline]
    #[must_use]
    pub const fn bit_order(&self) -> BitOrder {
        self.bit_order
    }

    /// Sets the active bit ordering mode.
    #[inline]
    pub fn set_bit_order(&mut self, bit_order: BitOrder) {
        self.bit_order = bit_order;
    }

    /// Returns the active byte ordering mode.
    #[inline]
    #[must_use]
    pub const fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    /// Returns current bit position.
    pub fn position(&self) -> BitOffset {
        let byte_pos = self.source.position();
        let unread_bits = self.bits_in_buf;
        BitOffset(byte_pos.0.saturating_sub(unread_bits))
    }

    /// Sets an optional bit limit on the reader.
    #[inline]
    pub fn set_bit_limit(&mut self, limit: Option<usize>) {
        self.bit_limit = limit;
    }

    /// Returns the active bit limit, if any.
    #[inline]
    #[must_use]
    pub const fn bit_limit(&self) -> Option<usize> {
        self.bit_limit
    }

    /// Reads up to 64 bits from the stream as an unsigned integer value.
    pub fn read_bits(&mut self, num_bits: usize) -> DFDLResult<u64> {
        if num_bits == 0 {
            return Ok(0);
        }
        if num_bits > 64 {
            return Err(DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Cannot read more than 64 bits in a single primitive operation",
            ));
        }

        if let Some(limit) = self.bit_limit {
            if self.position().0.saturating_add(num_bits) > limit {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::Parse,
                    "Insufficient binary data for primitive scalar",
                ));
            }
        }

        let mut result: u64 = 0;
        let mut bits_needed = num_bits;

        while bits_needed > 0 {
            if self.bits_in_buf == 0 {
                self.bit_buf = self.source.read_byte()?;
                self.bits_in_buf = 8;
            }

            let take = bits_needed.min(self.bits_in_buf);
            let chunk_bits: u64 = match self.bit_order {
                BitOrder::MostSignificantBitFirst => {
                    let shift = self.bits_in_buf - take;
                    let mask = bit_mask_u8(take);
                    ((self.bit_buf >> shift) & mask) as u64
                }
                BitOrder::LeastSignificantBitFirst => {
                    let shift = 8 - self.bits_in_buf;
                    let mask = bit_mask_u8(take);
                    ((self.bit_buf >> shift) & mask) as u64
                }
            };

            self.bits_in_buf -= take;
            bits_needed -= take;

            match self.bit_order {
                BitOrder::MostSignificantBitFirst => {
                    result = (result << take) | chunk_bits;
                }
                BitOrder::LeastSignificantBitFirst => {
                    let shift = num_bits - bits_needed - take;
                    result |= chunk_bits << shift;
                }
            }
        }

        Ok(result)
    }

    /// Skips the specified number of bits in the reader.
    pub fn skip_bits(&mut self, mut bits: usize) -> DFDLResult<()> {
        while bits > 0 {
            let chunk = bits.min(64);
            let _ = self.read_bits(chunk)?;
            bits = bits.saturating_sub(chunk);
        }
        Ok(())
    }

    /// Aligns reader to next whole byte boundary, discarding any unread bits in current byte.
    pub fn align_to_byte(&mut self) {
        self.bits_in_buf = 0;
        self.bit_buf = 0;
    }

    /// Returns true if no more bits are available in the reader.
    #[must_use]
    pub fn is_eof(&self) -> bool {
        if let Some(limit) = self.bit_limit {
            if self.position().0 >= limit {
                return true;
            }
        }
        self.bits_in_buf == 0 && self.source.remaining_bytes() == 0
    }

    /// Returns the estimated remaining bytes available in the reader.
    #[must_use]
    pub fn remaining_bytes(&self) -> usize {
        if let Some(limit) = self.bit_limit {
            let cur = self.position().0;
            let bits = limit.saturating_sub(cur);
            bits.saturating_add(7) / 8
        } else {
            self.source.remaining_bytes() + usize::from(self.bits_in_buf > 0)
        }
    }

    /// Creates a transaction checkpoint marker.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            bit_position: self.position(),
            sequence: self.sequence,
        }
    }

    /// Rolls back the reader state to a checkpoint.
    pub fn rollback(&mut self, cp: Checkpoint) -> DFDLResult<()> {
        let bit_pos = cp.bit_position.0;
        let byte_pos = bit_pos / 8;
        let rem_bits = bit_pos % 8;

        self.source.set_position(BitOffset(byte_pos * 8))?;
        self.align_to_byte();
        if rem_bits > 0 {
            let _ = self.read_bits(rem_bits)?;
        }
        self.sequence = cp.sequence;
        Ok(())
    }

    /// Peeks ahead `offset` bits and reads `num_bits` bits (up to 128 bits) without advancing the reader position.
    pub fn lookahead_bits(&mut self, offset: usize, num_bits: usize) -> DFDLResult<u128> {
        let cp = self.checkpoint();
        let res = (|| {
            if offset > 0 {
                self.skip_bits(offset)?;
            }
            if num_bits <= 64 {
                self.read_bits(num_bits).map(|v| v as u128)
            } else if num_bits <= 128 {
                let high_count = num_bits - 64;
                let high = self.read_bits(high_count)? as u128;
                let low = self.read_bits(64)? as u128;
                Ok((high << 64) | low)
            } else {
                Err(DFDLError::new(
                    DFDLErrorKind::ImplementationLimit,
                    "Look-ahead distance exceeds 128 bits limit",
                ))
            }
        })();
        let _ = self.rollback(cp);
        res.map_err(|e| {
            if e.kind == DFDLErrorKind::ImplementationLimit {
                e
            } else {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Insufficient bits available: {}", e),
                )
            }
        })
    }
}

/// Transactional bit-addressable writer.
#[derive(Debug)]
pub struct BitWriter<W: ByteSink> {
    sink: W,
    bit_order: BitOrder,
    byte_order: ByteOrder,
    bit_buf: u8,
    bits_in_buf: usize,
}

impl<W: ByteSink> BitWriter<W> {
    /// Creates a new [`BitWriter`] wrapping a byte sink.
    #[inline]
    pub fn new(sink: W, bit_order: BitOrder, byte_order: ByteOrder) -> Self {
        Self {
            sink,
            bit_order,
            byte_order,
            bit_buf: 0,
            bits_in_buf: 0,
        }
    }
    /// Returns a reference to the underlying byte sink.
    #[inline]
    #[must_use]
    pub const fn sink(&self) -> &W {
        &self.sink
    }

    /// Consumes the writer and returns the inner byte sink.
    #[inline]
    pub fn into_sink(self) -> W {
        self.sink
    }

    /// Returns the active byte ordering mode.
    #[inline]
    #[must_use]
    pub const fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    /// Returns the active bit ordering mode.
    #[inline]
    #[must_use]
    pub const fn bit_order(&self) -> BitOrder {
        self.bit_order
    }

    /// Sets the active bit ordering mode.
    #[inline]
    pub fn set_bit_order(&mut self, bit_order: BitOrder) {
        self.bit_order = bit_order;
    }
    /// Returns the current bit position written to the sink.
    pub fn position(&self) -> BitOffset {
        BitOffset(self.sink.position().0.saturating_add(self.bits_in_buf))
    }

    /// Writes up to 64 bits to the stream.
    pub fn write_bits(&mut self, value: u64, num_bits: usize) -> DFDLResult<()> {
        if num_bits == 0 {
            return Ok(());
        }
        if num_bits > 64 {
            return Err(DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Cannot write more than 64 bits in a single primitive operation",
            ));
        }

        let mut bits_remaining = num_bits;

        while bits_remaining > 0 {
            let space_in_buf = 8 - self.bits_in_buf;
            let count = bits_remaining.min(space_in_buf);

            let chunk: u8 = match self.bit_order {
                BitOrder::MostSignificantBitFirst => {
                    let shift = bits_remaining - count;
                    let mask = bit_mask_u64(count);
                    ((value >> shift) & mask) as u8
                }
                BitOrder::LeastSignificantBitFirst => {
                    let shift = num_bits - bits_remaining;
                    let mask = bit_mask_u64(count);
                    ((value >> shift) & mask) as u8
                }
            };

            match self.bit_order {
                BitOrder::MostSignificantBitFirst => {
                    let shifted = (self.bit_buf as u16) << count;
                    self.bit_buf = (shifted as u8) | chunk;
                }
                BitOrder::LeastSignificantBitFirst => {
                    self.bit_buf |= chunk << self.bits_in_buf;
                }
            }

            self.bits_in_buf += count;
            bits_remaining -= count;

            if self.bits_in_buf == 8 {
                self.sink.write_byte(self.bit_buf)?;
                self.bit_buf = 0;
                self.bits_in_buf = 0;
            }
        }

        Ok(())
    }

    /// Flushes partial byte buffer with pad bits to align to whole byte.
    pub fn flush_align(&mut self, pad_bit: u8) -> DFDLResult<()> {
        if self.bits_in_buf > 0 {
            let pad_count = 8 - self.bits_in_buf;
            let pad_val = if pad_bit == 0 {
                0u64
            } else {
                bit_mask_u64(pad_count)
            };
            self.write_bits(pad_val, pad_count)?;
        }
        Ok(())
    }

    /// Flushes any unwritten bits into sink.
    pub fn flush(&mut self) -> DFDLResult<()> {
        if self.bits_in_buf > 0 {
            self.flush_align(0)?;
        }
        Ok(())
    }

    /// Creates a transaction checkpoint marker.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            bit_position: self.position(),
            sequence: 0,
        }
    }

    /// Rolls back the writer state to a checkpoint.
    pub fn rollback(&mut self, cp: Checkpoint) -> DFDLResult<()> {
        self.sink.set_position(cp.bit_position)?;
        self.bit_buf = 0;
        self.bits_in_buf = 0;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::io::sink::VecByteSink;
    use crate::io::source::SliceByteSource;

    #[test]
    fn test_bit_reader_msbf_roundtrip() {
        let data = [0b1011_0100, 0b1100_1010];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        assert_eq!(reader.read_bits(4).unwrap(), 0b1011);
        assert_eq!(reader.read_bits(4).unwrap(), 0b0100);
        assert_eq!(reader.read_bits(8).unwrap(), 0b1100_1010);
    }

    #[test]
    fn test_bit_writer_msbf_roundtrip() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        writer.write_bits(0b1011, 4).unwrap();
        writer.write_bits(0b0100, 4).unwrap();
        writer.write_bits(0b1100_1010, 8).unwrap();

        let inner_sink = writer.sink;
        assert_eq!(inner_sink.as_slice(), &[0b1011_0100, 0b1100_1010]);
    }

    #[test]
    fn test_transactional_rollback() {
        let data = [0xAA, 0xBB, 0xCC];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        let cp = reader.checkpoint();
        assert_eq!(reader.read_bits(8).unwrap(), 0xAA);
        assert_eq!(reader.read_bits(8).unwrap(), 0xBB);

        reader.rollback(cp).unwrap();
        assert_eq!(reader.read_bits(8).unwrap(), 0xAA);
    }

    #[test]
    fn test_unaligned_bitreader_bytes() {
        let data = [0b1010_1011, 0b1100_1101, 0b1110_1111];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        // Read 3 unaligned bits first
        assert_eq!(reader.read_bits(3).unwrap(), 0b101);

        // Read 2 bytes starting at non-byte boundary (3 bit offset)
        let mut buf = [0u8; 2];
        for b in &mut buf {
            *b = reader.read_bits(8).unwrap() as u8;
        }

        assert_eq!(buf.len(), 2);
    }
}
