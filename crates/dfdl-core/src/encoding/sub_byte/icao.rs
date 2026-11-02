//! 6-bit ICAO character encoding implementation (`X-DFDL-ICAO-6-BIT`).
//!
//! Conforms to DFDL Specification §11 and ICAO Annex 10 standard:
//! - Codes `1..=26` map to uppercase letters `'A'..'Z'`
//! - Codes `48..=57` map to ASCII digits `'0'..'9'`
//! - Code `0` and code `32` map to space `' '`
//! - Replacement code point is `32` (space)

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// 6-bit ICAO character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IcaoEncoding;

impl Encoding for IcaoEncoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-ICAO-6-BIT"
    }

    /// Decodes raw bytes into ICAO characters.
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

    /// Returns the first non-ICAO character that cannot be encoded.
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

    /// Decodes 6-bit ICAO code point into char.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        let c = (code as u8) & 0x3F;
        match c {
            1..=26 => (b'A'.saturating_add(c).saturating_sub(1)) as char,
            48..=57 => c as char,
            0 | 32 => ' ',
            _ => ' ',
        }
    }

    /// Strictly maps uppercase letter, digit, or space to 6-bit ICAO code.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        match ch.to_ascii_uppercase() {
            'A'..='Z' => Some((ch as u64).saturating_sub('A' as u64).saturating_add(1)),
            '0'..='9' => Some(ch as u64),
            ' ' => Some(32),
            _ => None,
        }
    }

    /// Default replacement code point when encoding invalid characters (`32` = space).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        32
    }
}
