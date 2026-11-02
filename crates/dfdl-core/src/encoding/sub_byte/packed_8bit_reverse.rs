//! 8-bit packed reverse character encoding implementation (`X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE`).
//!
//! Conforms to DFDL Specification §11 for packed inverted 8-bit representations.
//! Byte value is mathematically inverted: `inv = 255 - code`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 8-bit packed inverted character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Packed8BitReverseEncoding;

impl Encoding for Packed8BitReverseEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE"
    }

    /// Decodes raw bytes by inverting each byte (`255 - b`).
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            out.push((255u8.saturating_sub(b)) as char);
        }
        Ok(out)
    }

    /// Decodes raw bytes into text with character offset mappings.
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        let text = self.decode_text_bytes(bytes).unwrap_or_default();
        let mut offsets = Vec::with_capacity(bytes.len().saturating_add(1));
        for i in 0..=bytes.len() {
            let _ = try_push(&mut offsets, i);
        }
        (text, offsets)
    }

    /// Encodes characters by inverting each byte (`255 - c`).
    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        text.chars()
            .map(|c| {
                if (c as u32) <= 255 {
                    255u8.saturating_sub(c as u8)
                } else {
                    b'?'
                }
            })
            .collect()
    }

    /// Returns the first character outside 8-bit range `0..=255`.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|c| (*c as u32) > 255)
    }

    /// Number of bits per character code unit (8 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(8)
    }

    /// Unit alignment in bits (1 bit for sub-byte streams).
    #[inline]
    fn unit_bits(&self) -> usize {
        1
    }

    /// Mandatory bit alignment (1 bit for packed representations).
    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        1
    }

    /// Decodes inverted 8-bit integer code `inv = 255 - code` into char.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        let inv = 255u8.saturating_sub(code as u8);
        core::char::from_u32(inv as u32).unwrap_or('?')
    }

    /// Strictly maps code point to inverted 8-bit value `255 - ch`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        if (ch as u32) <= 255 {
            Some((255u8.saturating_sub(ch as u8)) as u64)
        } else {
            None
        }
    }

    /// Default replacement code point (`0`).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
