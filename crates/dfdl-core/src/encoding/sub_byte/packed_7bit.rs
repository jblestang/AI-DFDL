//! 7-bit packed US-ASCII character encoding implementation (`X-DFDL-US-ASCII-7-BIT-PACKED`).
//!
//! Conforms to DFDL Specification §11 for packed 7-bit ASCII representations (codes `0..=127`).
//! Characters strictly conform to standard 7-bit ASCII character codes.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 7-bit packed US-ASCII character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Packed7BitEncoding;

impl Encoding for Packed7BitEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-US-ASCII-7-BIT-PACKED"
    }

    /// Decodes raw bytes into 7-bit ASCII characters.
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

    /// Returns the first non-ASCII character that cannot be encoded.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (7 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(7)
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

    /// Decodes 7-bit integer code point `0..=127` into ASCII char.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        if code <= 127 {
            code as u8 as char
        } else {
            '?'
        }
    }

    /// Strictly maps standard ASCII characters `0..=127` to 7-bit code.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        ch.is_ascii().then_some(ch as u64)
    }

    /// Default replacement code point when encoding invalid characters (`0x3F` = '?').
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0x3F
    }
}
