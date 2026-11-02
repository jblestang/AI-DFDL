//! 4-bit hexadecimal character encoding implementation (`X-DFDL-HEX`).
//!
//! Conforms to DFDL Specification §11 for packed 4-bit hexadecimal representations.
//! Characters `'0'..'9'` and `'A'..'F'` map to hexadecimal values `0..15`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 4-bit hexadecimal character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HexEncoding;

impl Encoding for HexEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-HEX"
    }

    /// Decodes raw bytes into uppercase hex characters `'0'..'F'`.
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

    /// Encodes hexadecimal characters into individual code point bytes.
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

    /// Returns the first non-hexadecimal character that cannot be encoded.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (4 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(4)
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

    /// Decodes 4-bit integer code `0..15` into uppercase character `'0'..'F'`.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        core::char::from_digit(code as u32, 16)
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('?')
    }

    /// Strictly maps hexadecimal characters `'0'..'9'`, `'a'..'f'`, `'A'..'F'` to `0..15`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ch.to_digit(16).map(u64::from)
    }

    /// Default replacement code point when encoding invalid characters.
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
