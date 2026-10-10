//! Byte source implementations: `SliceByteSource` and `BoundedSource`.

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::traits::ByteSource;
use crate::types::BitOffset;
use crate::util::{checked_add_usize, get_checked, get_slice_checked};

/// Implements [`ByteSource`] wrapping a contiguous in-memory byte slice.
#[derive(Debug, Clone, Copy)]
pub struct SliceByteSource<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> SliceByteSource<'a> {
    /// Constructs a new [`SliceByteSource`] from a byte slice reference.
    #[inline]
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
}

impl<'a> ByteSource for SliceByteSource<'a> {
    fn read_byte(&mut self) -> DFDLResult<u8> {
        let b = *get_checked(self.data, self.position).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                "Unexpected end of data in slice byte source",
            )
        })?;

        self.position = checked_add_usize(self.position, 1)?;
        Ok(b)
    }

    fn read_bytes(&mut self, buf: &mut [u8]) -> DFDLResult<()> {
        let sub = get_slice_checked(self.data, self.position, buf.len()).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                "Unexpected end of data reading byte block",
            )
        })?;

        buf.copy_from_slice(sub);
        self.position = checked_add_usize(self.position, buf.len())?;
        Ok(())
    }

    fn position(&self) -> BitOffset {
        BitOffset(self.position.saturating_mul(8))
    }

    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()> {
        let byte_pos = pos.to_byte_offset().ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                "Byte source position must be byte-aligned",
            )
        })?;

        if byte_pos.0 <= self.data.len() {
            self.position = byte_pos.0;
            Ok(())
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Attempted to seek past end of slice byte source",
            ))
        }
    }

    fn remaining_bytes(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }
}

/// Wraps an underlying [`ByteSource`] with a strict maximum byte boundary.
#[derive(Debug)]
pub struct BoundedSource<S: ByteSource> {
    inner: S,
    start_pos: usize,
    max_bytes: usize,
}

impl<S: ByteSource> BoundedSource<S> {
    /// Wraps `inner` byte source restricting reads to `max_bytes` from current position.
    pub fn new(inner: S, max_bytes: usize) -> DFDLResult<Self> {
        let start_pos = inner
            .position()
            .to_byte_offset()
            .ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    "BoundedSource requires byte-aligned start position",
                )
            })?
            .0;

        Ok(Self {
            inner,
            start_pos,
            max_bytes,
        })
    }
}

impl<S: ByteSource> ByteSource for BoundedSource<S> {
    fn read_byte(&mut self) -> DFDLResult<u8> {
        let current_pos = self
            .inner
            .position()
            .to_byte_offset()
            .ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Position not byte aligned in BoundedSource",
                )
            })?
            .0;

        let elapsed = current_pos.checked_sub(self.start_pos).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::InternalInvariant,
                "BoundedSource position underflow",
            )
        })?;

        if elapsed >= self.max_bytes {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Bounded region limit exceeded in byte source",
            ));
        }

        self.inner.read_byte()
    }

    fn read_bytes(&mut self, buf: &mut [u8]) -> DFDLResult<()> {
        let current_pos = self
            .inner
            .position()
            .to_byte_offset()
            .ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Position not byte aligned in BoundedSource",
                )
            })?
            .0;

        let elapsed = current_pos.checked_sub(self.start_pos).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::InternalInvariant,
                "BoundedSource position underflow",
            )
        })?;

        let new_elapsed = elapsed.checked_add(buf.len()).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Integer overflow checking region bounds",
            )
        })?;

        if new_elapsed > self.max_bytes {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Bounded region limit exceeded reading bytes",
            ));
        }

        self.inner.read_bytes(buf)
    }

    fn position(&self) -> BitOffset {
        self.inner.position()
    }

    fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()> {
        let byte_pos = pos
            .to_byte_offset()
            .ok_or_else(|| DFDLError::new(DFDLErrorKind::Parse, "Position must be byte-aligned"))?
            .0;

        let elapsed = byte_pos.checked_sub(self.start_pos).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                "Cannot seek before start of bounded region",
            )
        })?;

        if elapsed <= self.max_bytes {
            self.inner.set_position(pos)
        } else {
            Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Cannot seek past end of bounded region",
            ))
        }
    }

    fn remaining_bytes(&self) -> usize {
        let current_pos = self
            .inner
            .position()
            .to_byte_offset()
            .map(|b| b.0)
            .unwrap_or(self.start_pos);
        let elapsed = current_pos.saturating_sub(self.start_pos);
        self.max_bytes.saturating_sub(elapsed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_byte_source_reads() {
        let data = [0x10, 0x20, 0x30, 0x40];
        let mut src = SliceByteSource::new(&data);

        assert_eq!(src.read_byte().unwrap(), 0x10);
        let mut buf = [0u8; 2];
        src.read_bytes(&mut buf).unwrap();
        assert_eq!(buf, [0x20, 0x30]);
        assert_eq!(src.remaining_bytes(), 1);
        assert_eq!(src.read_byte().unwrap(), 0x40);
        assert!(src.read_byte().is_err());
    }

    #[test]
    fn test_bounded_source_overrun_protection() {
        let data = [1, 2, 3, 4, 5];
        let src = SliceByteSource::new(&data);
        let mut bounded = BoundedSource::new(src, 3).unwrap();

        let mut buf = [0u8; 3];
        bounded.read_bytes(&mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3]);

        assert!(bounded.read_byte().is_err());
    }

    #[test]
    fn test_slice_byte_source_seeking_and_limits() {
        let data = [10, 20, 30, 40, 50];
        let mut src = SliceByteSource::new(&data);

        // read_bytes beyond length
        let mut big_buf = [0u8; 10];
        assert!(src.read_bytes(&mut big_buf).is_err());

        // Valid set_position
        assert!(src.set_position(BitOffset(16)).is_ok());
        assert_eq!(src.position(), BitOffset(16));
        assert_eq!(src.read_byte().unwrap(), 30);

        // Non-aligned position
        assert!(src.set_position(BitOffset(5)).is_err());

        // Past end of slice (64 bits = 8 bytes > 5 bytes, byte-aligned)
        assert!(src.set_position(BitOffset(64)).is_err());
    }

    #[test]
    fn test_bounded_source_seeking_and_limits() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut src = SliceByteSource::new(&data);
        src.set_position(BitOffset(16)).unwrap(); // start at byte 2
        let mut bounded = BoundedSource::new(src, 4).unwrap();

        assert_eq!(bounded.remaining_bytes(), 4);
        assert_eq!(bounded.position(), BitOffset(16));

        // read_bytes beyond region limit
        let mut buf_over = [0u8; 5];
        assert!(bounded.read_bytes(&mut buf_over).is_err());

        // Valid seek within bounded region (byte 4 = bit 32, offset 2 within bound)
        assert!(bounded.set_position(BitOffset(32)).is_ok());
        assert_eq!(bounded.remaining_bytes(), 2);
        assert_eq!(bounded.read_byte().unwrap(), 5);

        // Seek before start of bounded region (byte 1 < start byte 2)
        assert!(bounded.set_position(BitOffset(8)).is_err());

        // Non-aligned seek
        assert!(bounded.set_position(BitOffset(17)).is_err());

        // Seek past end of bounded region (byte 7 > start 2 + 4)
        assert!(bounded.set_position(BitOffset(56)).is_err());
    }

    struct MockUnalignedByteSource {
        pos: BitOffset,
    }

    impl ByteSource for MockUnalignedByteSource {
        fn read_byte(&mut self) -> DFDLResult<u8> {
            Ok(0)
        }
        fn read_bytes(&mut self, _buf: &mut [u8]) -> DFDLResult<()> {
            Ok(())
        }
        fn position(&self) -> BitOffset {
            self.pos
        }
        fn set_position(&mut self, pos: BitOffset) -> DFDLResult<()> {
            self.pos = pos;
            Ok(())
        }
        fn remaining_bytes(&self) -> usize {
            10
        }
    }

    #[test]
    fn test_bounded_source_unaligned_and_underflow_error_branches() {
        // 1. Initial unaligned position
        let mock_unaligned = MockUnalignedByteSource { pos: BitOffset(3) };
        assert!(BoundedSource::new(mock_unaligned, 10).is_err());

        // 2. Unaligned position during read_byte and read_bytes
        let mock_aligned = MockUnalignedByteSource { pos: BitOffset(0) };
        let mut bounded = BoundedSource::new(mock_aligned, 10).unwrap();
        bounded.inner.pos = BitOffset(5);
        assert!(bounded.read_byte().is_err());
        assert!(bounded.read_bytes(&mut [0u8; 1]).is_err());

        // 3. Underflow (current_pos < start_pos)
        bounded.inner.pos = BitOffset(0);
        bounded.start_pos = 10;
        assert!(bounded.read_byte().is_err());
        assert!(bounded.read_bytes(&mut [0u8; 1]).is_err());

        // 4. Mock source trait method execution
        let mut m = MockUnalignedByteSource { pos: BitOffset(0) };
        assert_eq!(m.read_byte().unwrap(), 0);
        assert!(m.read_bytes(&mut [0u8; 2]).is_ok());
        assert_eq!(m.position(), BitOffset(0));
        assert!(m.set_position(BitOffset(8)).is_ok());
        assert_eq!(m.remaining_bytes(), 10);

        // 5. BoundedSource set_position, remaining_bytes, and overflow
        let mut b_ok = BoundedSource::new(m, 5).unwrap();
        assert_eq!(b_ok.remaining_bytes(), 5);
        assert!(b_ok.set_position(BitOffset(8)).is_ok());

        // Exceeding max_bytes limit
        b_ok.start_pos = 0;
        b_ok.inner.pos = BitOffset(0);
        assert!(b_ok.read_bytes(&mut [0u8; 10]).is_err());
    }
}
