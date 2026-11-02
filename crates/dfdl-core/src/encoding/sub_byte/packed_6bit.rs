//! 6-bit packed US-ASCII character encoding implementation (`X-DFDL-US-ASCII-6-BIT-PACKED`).
//!
//! Conforms to DFDL Specification §11 for packed 6-bit ASCII representations.
//! Mapped range is ASCII `0x20..=0x5F` (space, punctuation, digits, uppercase letters).
//! Characters with code `< 32` are normalized with `+ 0x40`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 6-bit packed US-ASCII character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Packed6BitEncoding;

impl Encoding for Packed6BitEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-US-ASCII-6-BIT-PACKED"
    }

    /// Decodes raw bytes into 6-bit ASCII characters.
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

    /// Returns the first character outside ASCII range `0x20..=0x5F`.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (6 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(6)
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

    /// Decodes 6-bit integer code `0..63` into ASCII character in `0x20..=0x5F`.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        let c = code as u8 & 0x3F;
        (if c < 32 { c.saturating_add(0x40) } else { c }) as char
    }

    /// Strictly maps ASCII character in `0x20..=0x5F` to 6-bit code `(ch & 0x3F)`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ('\u{20}'..='\u{5F}').contains(&ch).then_some((ch as u64) & 0x3F)
    }

    /// Default replacement code point when encoding invalid characters (`0x1F`).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0x1F
    }
}
