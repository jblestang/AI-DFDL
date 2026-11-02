//! 3-bit octal character encoding implementation (`X-DFDL-OCTAL`).
//!
//! Conforms to DFDL Specification §11 for packed 3-bit octal representations.
//! Characters `'0'..'7'` map directly to octal numeric values `0..7`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 3-bit octal character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OctalEncoding;

impl Encoding for OctalEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-OCTAL"
    }

    /// Decodes raw bytes into octal characters `'0'..'7'`.
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

    /// Encodes octal characters into individual code point bytes.
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

    /// Returns the first non-octal character that cannot be encoded.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (3 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(3)
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

    /// Decodes 3-bit integer code `0..7` into ASCII character `'0'..'7'`.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        (b'0'.saturating_add(code as u8 & 0x07)) as char
    }

    /// Strictly maps `'0'..'7'` to octal codes `0..7`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ch.to_digit(8).map(u64::from)
    }

    /// Default replacement code point when encoding invalid characters.
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
