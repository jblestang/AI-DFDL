//! 8-bit packed character encoding implementation (`X-DFDL-8-BIT-PACKED`).
//!
//! Conforms to DFDL Specification §11 for packed 8-bit representations (codes `0..=255`).
//! Supports raw byte value mapping to ISO-8859-1 Unicode scalar code points.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 8-bit packed character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Packed8BitEncoding;

impl Encoding for Packed8BitEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-8-BIT-PACKED"
    }

    /// Decodes raw bytes into characters.
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            out.push(self.decode_sub_byte_char(b as u64));
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

    /// Encodes characters into individual code point bytes.
    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let (code, _) = self
                .encode_sub_byte_char(ch)
                .unwrap_or((self.sub_byte_replacement_code(), 8));
            out.push(code as u8);
        }
        out
    }

    /// Returns the first character outside 8-bit range `0..=255`.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
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

    /// Decodes 8-bit integer code point `0..=255` into char.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        core::char::from_u32(code as u32).unwrap_or('?')
    }

    /// Strictly maps code point `0..=255` to 8-bit code.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ((ch as u32) <= 255).then_some(ch as u64)
    }

    /// Default replacement code point when encoding invalid characters (`0`).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
