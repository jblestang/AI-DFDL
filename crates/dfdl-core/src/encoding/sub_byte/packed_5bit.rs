//! 5-bit packed character encoding implementation (`X-DFDL-5-BIT-PACKED`).
//!
//! Conforms to DFDL Specification §11 for packed 5-bit character representations.
//! Character alphabet consists of 32 characters:
//! - Codes `0..7` represent digits `'0'..'7'`
//! - Codes `8..31` represent letters `'A'..'Z'` excluding `'I'` and `'O'`
//! - Letters `'O'` and `'I'` are alias-mapped to `'0'` and `'1'` respectively on encode

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 32-character lookup table for 5-bit packed encoding.
pub const PACKED_5BIT_CHARS: &[u8; 32] = b"01234567ABCDEFGHJKLMNPQRSTUVWXYZ";

/// 5-bit packed character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Packed5BitEncoding;

impl Encoding for Packed5BitEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-5-BIT-PACKED"
    }

    /// Decodes raw bytes using the 32-character packed 5-bit lookup table.
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

    /// Returns the first character that cannot be encoded in 5-bit packed representation.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (5 bits).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(5)
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

    /// Decodes 5-bit integer code `0..31` into character from `PACKED_5BIT_CHARS`.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        PACKED_5BIT_CHARS
            .get(code as usize)
            .map_or('?', |b| *b as char)
    }

    /// Strictly maps character to 5-bit code, treating `'O'`/`'I'` as aliases for `'0'`/`'1'`.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        match ch.to_ascii_uppercase() {
            'O' => Some(0),
            'I' => Some(1),
            up => PACKED_5BIT_CHARS
                .iter()
                .position(|b| *b as char == up)
                .map(|i| i as u64),
        }
    }

    /// Default replacement code point when encoding invalid characters (code 29).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        29
    }
}
