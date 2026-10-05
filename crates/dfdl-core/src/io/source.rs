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
}
