//! 2-bit base-4 character encoding implementation (`X-DFDL-BASE4`).
//!
//! Conforms to DFDL Specification §11 for packed 2-bit quaternary representations.
//! Characters `'0'..'3'` map directly to numeric values `0..3`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 2-bit base-4 character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Base4Encoding;

impl Encoding for Base4Encoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-BASE4"
    }

    /// Decodes raw bytes into base-4 characters `'0'..'3'`.
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

    /// Encodes base-4 characters into individual code point bytes.
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

    /// Returns the first non-base-4 character that cannot be encoded.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (2 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(2)
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

    /// Decodes 2-bit integer code `0..3` into ASCII character `'0'..'3'`.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        (b'0'.saturating_add(code as u8 & 0x03)) as char
    }

    /// Strictly maps `'0'..'3'` to numeric codes `0..3`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ch.to_digit(4).map(u64::from)
    }

    /// Default replacement code point when encoding invalid characters.
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
