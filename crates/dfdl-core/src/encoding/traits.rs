//! Core character encoding trait for DFDL.
//!
//! Conforms to DFDL 1.0 Specification §11 (`dfdl:encoding`).

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;

/// Core character encoding trait representing DFDL text encoding rules.
///
/// Implemented by specific encoding handlers (UTF-8, ASCII, ISO-8859-*, UTF-16, UTF-32,
/// EBCDIC, sub-byte, and custom encodings).
pub trait Encoding: Send + Sync + 'static {
    /// Canonical or standard name of the character encoding.
    fn name(&self) -> &str;

    /// Decodes raw bytes into a Unicode [`String`].
    ///
    /// # Errors
    /// Returns [`crate::error::DFDLError`] if decoding fails strictly under error policy.
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String>;

    /// Decodes raw bytes into a Unicode [`String`] along with starting byte offsets for each character.
    ///
    /// The returned vector has `offsets[i]` equal to the byte index in `bytes` where character `i`
    /// starts, plus a final entry equal to `bytes.len()`.
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>);

    /// Encodes a Unicode string into raw bytes.
    fn encode_text_string(&self, text: &str) -> Vec<u8>;

    /// Returns the first unmappable character in `text` under this encoding, if any.
    fn find_unmappable_char(&self, text: &str) -> Option<char>;

    /// Returns true if the byte stream contains malformed sequences under this encoding.
    fn has_malformed_input(&self, bytes: &[u8]) -> bool {
        let _ = bytes;
        false
    }

    /// Number of bits per code unit for sub-byte encodings (e.g. 1-7 bits), or None if byte-aligned.
    fn char_bits(&self) -> Option<usize> {
        None
    }

    /// Mandatory character alignment / unit width in bits.
    fn unit_bits(&self) -> usize {
        8
    }

    /// Mandatory text alignment in bits.
    fn mandatory_alignment_bits(&self) -> usize {
        8
    }

    /// Whether this is a variable-width character encoding (such as UTF-8).
    fn is_variable_width(&self) -> bool {
        false
    }

    /// Decodes an integer code point into a character for sub-byte encodings.
    fn decode_sub_byte_char(&self, code: u64) -> char {
        core::char::from_u32(code as u32).unwrap_or('?')
    }

    /// Strictly maps a character to its code in a sub-byte encoding (`None` if unmappable).
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        self.char_bits()?;
        Some(ch as u64)
    }

    /// Code used in place of an unmappable character (policy `replace`).
    fn sub_byte_replacement_code(&self) -> u64 {
        0
    }

    /// Encodes a character into its integer code point and bit length for sub-byte encodings.
    fn encode_sub_byte_char(&self, ch: char) -> Option<(u64, usize)> {
        let cb = self.char_bits()?;
        let code = self
            .strict_sub_byte_code(ch)
            .unwrap_or_else(|| self.sub_byte_replacement_code());
        Some((code, cb))
    }
}
