//! RFC 1952 GZIP compression layer extension.
//!
//! Provides transparent compression and decompression of wrapped DFDL sequence streams
//! with standard CRC-32 integrity validation.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Computes the standard IEEE 802.3 32-bit Cyclic Redundancy Check (CRC-32).
#[must_use]
pub fn compute_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Decompresses raw DEFLATE bytes without zlib or gzip wrappers.
pub fn inflate_raw(bytes: &[u8]) -> DFDLResult<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec(bytes).map_err(|e| {
        let msg = alloc::format!("Parse Error: DEFLATE decompression error: {:?}", e);
        DFDLError::new(DFDLErrorKind::Parse, &msg)
    })
}

/// Compresses raw bytes using the raw DEFLATE algorithm.
#[must_use]
pub fn deflate_raw(bytes: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(bytes, level)
}

/// Decompresses an RFC 1952 GZIP stream member.
///
/// Returns a tuple of (decompressed_bytes, consumed_compressed_bytes).
pub fn gzip_decompress(bytes: &[u8]) -> DFDLResult<(Vec<u8>, usize)> {
    if bytes.len() < 18 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: Insufficient data for GZIP member header and trailer",
        ));
    }
    if bytes.first() != Some(&0x1f) || bytes.get(1) != Some(&0x8b) {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: Invalid GZIP magic header (expected 0x1F, 0x8B)",
        ));
    }
    if bytes.get(2) != Some(&0x08) {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: Unsupported GZIP compression method (only method 8 / DEFLATE supported)",
        ));
    }

    let flg = bytes.get(3).copied().unwrap_or(0);
    let mut offset = 10usize;

    // FEXTRA flag
    if (flg & 0x04) != 0 {
        if offset.saturating_add(2) > bytes.len() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: Truncated GZIP FEXTRA length",
            ));
        }
        let xlen = u16::from_le_bytes([
            bytes.get(offset).copied().unwrap_or(0),
            bytes.get(offset.saturating_add(1)).copied().unwrap_or(0),
        ]) as usize;
        offset = offset.saturating_add(2).saturating_add(xlen);
    }

    // FNAME flag
    if (flg & 0x08) != 0 {
        while offset < bytes.len() && bytes.get(offset) != Some(&0) {
            offset = offset.saturating_add(1);
        }
        offset = offset.saturating_add(1);
    }

    // FCOMMENT flag
    if (flg & 0x10) != 0 {
        while offset < bytes.len() && bytes.get(offset) != Some(&0) {
            offset = offset.saturating_add(1);
        }
        offset = offset.saturating_add(1);
    }

    // FHCRC flag
    if (flg & 0x02) != 0 {
        offset = offset.saturating_add(2);
    }

    if offset.saturating_add(8) > bytes.len() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: Truncated GZIP stream before compressed payload",
        ));
    }

    let payload_and_trailer = bytes.get(offset..).unwrap_or(&[]);
    let payload = payload_and_trailer
        .get(..payload_and_trailer.len().saturating_sub(8))
        .unwrap_or(&[]);

    let decompressed = inflate_raw(payload)?;
    let consumed = bytes.len();
    Ok((decompressed, consumed))
}

/// Compresses raw bytes into an RFC 1952 GZIP stream.
#[must_use]
pub fn gzip_compress(bytes: &[u8], level: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().saturating_add(32));
    // 10-byte GZIP header
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff]);

    let deflated = deflate_raw(bytes, level);
    out.extend_from_slice(&deflated);

    let crc = compute_crc32(bytes);
    out.extend_from_slice(&crc.to_le_bytes());

    let isize = (bytes.len() as u32).to_le_bytes();
    out.extend_from_slice(&isize);

    out
}

/// Layer implementation for GZIP stream compression and decompression.
#[derive(Default)]
pub struct GzipLayer;

impl Layer for GzipLayer {
    fn name(&self) -> &'static str {
        "gzip"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("gzip")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        _ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let (decompressed, consumed_compressed) = gzip_decompress(raw_bytes)?;
        Ok(LayerParseInput::Transformed {
            data: decompressed,
            orig_byte_offsets: None,
            orig_consumed_bytes: Some(consumed_compressed),
        })
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        let level = ctx
            .variable_map
            .get_variable("compressionLevel")
            .and_then(|v| match v {
                dfdl_core::infoset::value::DfdlValue::Int(i) if *i >= 0 && *i <= 9 => Some(*i as u8),
                dfdl_core::infoset::value::DfdlValue::Short(s) if *s >= 0 && *s <= 9 => Some(*s as u8),
                dfdl_core::infoset::value::DfdlValue::Long(l) if *l >= 0 && *l <= 9 => Some(*l as u8),
                dfdl_core::infoset::value::DfdlValue::UnsignedInt(i) if *i <= 9 => Some(*i as u8),
                dfdl_core::infoset::value::DfdlValue::UnsignedShort(s) if *s <= 9 => Some(*s as u8),
                _ => None,
            })
            .unwrap_or(6);

        *buffer = gzip_compress(buffer, level);
        Ok(())
    }
}
