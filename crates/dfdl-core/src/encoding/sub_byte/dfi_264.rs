//! Aviation DFI-264-DUI-001 character encoding implementation (`X-DFDL-6-BIT-DFI-264-DUI-001`).
//!
//! Conforms to tactical datalink and aviation standard DFI-264-DUI-001:
//! - Codes `0..=9` map to ASCII digits `'0'..'9'`
//! - Code `10` maps to space `' '`
//! - Codes `11..=36` map to uppercase letters `'A'..'Z'`

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;
use crate::util::try_push;

/// Aviation DFI-264-DUI-001 character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dfi264Dui001Encoding {
    /// Number of bits per character code unit (typically 6).
    pub bit_width: usize,
}

impl Default for Dfi264Dui001Encoding {
    fn default() -> Self {
        Self { bit_width: 6 }
    }
}

impl Encoding for Dfi264Dui001Encoding {
    /// Canonical DFDL encoding identifier.
    #[inline]
    fn name(&self) -> &str {
        "X-DFDL-6-BIT-DFI-264-DUI-001"
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

    /// Returns the first character that cannot be encoded in DFI-264-DUI-001.
    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|&c| self.strict_sub_byte_code(c).is_none())
    }

    /// Number of bits per character code unit (`bit_width`).
    #[inline]
    fn char_bits(&self) -> Option<usize> {
        Some(self.bit_width)
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

    /// Decodes integer code into character according to DFI-264-DUI-001.
    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        match code {
            0..=9 => (b'0'.saturating_add(code as u8)) as char,
            10 => ' ',
            11..=36 => (b'A'.saturating_add((code as u8).saturating_sub(11))) as char,
            _ => '?',
        }
    }

    /// Strictly maps digit, space, or uppercase letter to DFI-264-DUI-001 code.
    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        match ch {
            '0'..='9' => ch.to_digit(10).map(u64::from),
            ' ' => Some(10),
            _ => {
                let up = ch.to_ascii_uppercase();
                ('A'..='Z')
                    .contains(&up)
                    .then(|| (up as u64).saturating_sub('A' as u64).saturating_add(11))
            }
        }
    }

    /// Default replacement code point (`0`).
    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }
}
