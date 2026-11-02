//! UTF-16 character encoding implementation (Big-Endian and Little-Endian).

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;

/// UTF-16 character encoding handler with configurable endianness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Utf16Encoding {
    /// Whether multi-byte units are encoded in big-endian byte order.
    pub is_big_endian: bool,
}

impl Utf16Encoding {
    /// UTF-16 Big-Endian instance.
    pub const BE: Self = Self { is_big_endian: true };
    /// UTF-16 Little-Endian instance.
    pub const LE: Self = Self { is_big_endian: false };
}

impl Default for Utf16Encoding {
    fn default() -> Self {
        Self::BE
    }
}

fn decode_utf16(bytes: &[u8], is_big_endian: bool) -> DFDLResult<String> {
    let mut u16_units = Vec::new();
    let mut i = 0usize;
    while i.saturating_add(1) < bytes.len() {
        let b1 = *bytes.get(i).unwrap_or(&0) as u16;
        let b2 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u16;
        let unit = if is_big_endian {
            (b1 << 8) | b2
        } else {
            (b2 << 8) | b1
        };
        try_push(&mut u16_units, unit)?;
        i = i.saturating_add(2);
    }
    let mut out = String::new();
    for res in char::decode_utf16(u16_units) {
        let ch = res.unwrap_or('?');
        out.push(ch);
    }
    Ok(out)
}

fn decode_utf16_with_offsets(bytes: &[u8], is_big_endian: bool) -> (String, Vec<usize>) {
    let mut u16_units = Vec::new();
    let mut unit_offsets = Vec::new();
    let mut i = 0usize;
    while i.saturating_add(1) < bytes.len() {
        let b1 = *bytes.get(i).unwrap_or(&0) as u16;
        let b2 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u16;
        let unit = if is_big_endian {
            (b1 << 8) | b2
        } else {
            (b2 << 8) | b1
        };
        if try_push(&mut u16_units, unit).is_err() || try_push(&mut unit_offsets, i).is_err() {
            break;
        }
        i = i.saturating_add(2);
    }
    let mut out = String::new();
    let mut offsets = Vec::new();
    let mut u16_idx = 0usize;
    for res in char::decode_utf16(u16_units.iter().copied()) {
        if let Some(&off) = unit_offsets.get(u16_idx) {
            let _ = try_push(&mut offsets, off);
        }
        let ch = res.unwrap_or('?');
        out.push(ch);
        u16_idx = u16_idx.saturating_add(if ch.len_utf16() == 2 { 2 } else { 1 });
    }
    let _ = try_push(&mut offsets, bytes.len());
    (out, offsets)
}

impl Encoding for Utf16Encoding {
    #[inline]
    fn name(&self) -> &str {
        if self.is_big_endian {
            "UTF-16BE"
        } else {
            "UTF-16LE"
        }
    }

    #[inline]
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        decode_utf16(bytes, self.is_big_endian)
    }

    #[inline]
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        decode_utf16_with_offsets(bytes, self.is_big_endian)
    }

    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for u in text.encode_utf16() {
            if self.is_big_endian {
                out.push((u >> 8) as u8);
                out.push((u & 0xFF) as u8);
            } else {
                out.push((u & 0xFF) as u8);
                out.push((u >> 8) as u8);
            }
        }
        out
    }

    #[inline]
    fn find_unmappable_char(&self, _text: &str) -> Option<char> {
        None
    }

    #[inline]
    fn unit_bits(&self) -> usize {
        16
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        8
    }
}
