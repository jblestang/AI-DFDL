//! UTF-32 character encoding implementation (Big-Endian and Little-Endian).

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;

/// UTF-32 character encoding handler with configurable endianness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Utf32Encoding {
    /// Whether multi-byte code units are encoded in big-endian byte order.
    pub is_big_endian: bool,
}

impl Utf32Encoding {
    /// UTF-32 Big-Endian instance.
    pub const BE: Self = Self { is_big_endian: true };
    /// UTF-32 Little-Endian instance.
    pub const LE: Self = Self { is_big_endian: false };
}

impl Default for Utf32Encoding {
    fn default() -> Self {
        Self::BE
    }
}

fn decode_utf32(bytes: &[u8], is_big_endian: bool) -> DFDLResult<String> {
    let mut out = String::new();
    let mut i = 0usize;
    while i.saturating_add(3) < bytes.len() {
        let b0 = *bytes.get(i).unwrap_or(&0) as u32;
        let b1 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u32;
        let b2 = *bytes.get(i.saturating_add(2)).unwrap_or(&0) as u32;
        let b3 = *bytes.get(i.saturating_add(3)).unwrap_or(&0) as u32;
        let code_point = if is_big_endian {
            (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
        } else {
            (b3 << 24) | (b2 << 16) | (b1 << 8) | b0
        };
        let ch = char::from_u32(code_point).unwrap_or('?');
        out.push(ch);
        i = i.saturating_add(4);
    }
    Ok(out)
}

fn decode_utf32_with_offsets(bytes: &[u8], is_big_endian: bool) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut offsets = Vec::new();
    let mut i = 0usize;
    while i.saturating_add(3) < bytes.len() {
        let b0 = *bytes.get(i).unwrap_or(&0) as u32;
        let b1 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u32;
        let b2 = *bytes.get(i.saturating_add(2)).unwrap_or(&0) as u32;
        let b3 = *bytes.get(i.saturating_add(3)).unwrap_or(&0) as u32;
        let code_point = if is_big_endian {
            (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
        } else {
            (b3 << 24) | (b2 << 16) | (b1 << 8) | b0
        };
        let ch = char::from_u32(code_point).unwrap_or('?');
        let _ = try_push(&mut offsets, i);
        out.push(ch);
        i = i.saturating_add(4);
    }
    let _ = try_push(&mut offsets, bytes.len());
    (out, offsets)
}

impl Encoding for Utf32Encoding {
    #[inline]
    fn name(&self) -> &str {
        if self.is_big_endian {
            "UTF-32BE"
        } else {
            "UTF-32LE"
        }
    }

    #[inline]
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        decode_utf32(bytes, self.is_big_endian)
    }

    #[inline]
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        decode_utf32_with_offsets(bytes, self.is_big_endian)
    }

    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for c in text.chars() {
            let u = c as u32;
            if self.is_big_endian {
                out.push((u >> 24) as u8);
                out.push((u >> 16) as u8);
                out.push((u >> 8) as u8);
                out.push((u & 0xFF) as u8);
            } else {
                out.push((u & 0xFF) as u8);
                out.push((u >> 8) as u8);
                out.push((u >> 16) as u8);
                out.push((u >> 24) as u8);
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
        32
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        8
    }
}
