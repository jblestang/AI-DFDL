//! UTF-8 character encoding implementation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use super::traits::Encoding;

/// UTF-8 character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Utf8Encoding;

/// Decodes UTF-8 replacing each malformed sequence with U+FFFD (`encodingErrorPolicy="replace"`).
///
/// Returns the text and, for every character, its starting byte offset in `bytes`, followed by a
/// final entry holding the total length.
#[must_use]
pub fn utf8_lossy_with_offsets(bytes: &[u8]) -> (String, Vec<usize>) {
    let mut text = String::new();
    let mut offsets = Vec::new();
    let mut pos = 0usize;
    while let Some(rest) = bytes.get(pos..).filter(|r| !r.is_empty()) {
        let (valid, bad) = match core::str::from_utf8(rest) {
            Ok(s) => (s, 0),
            Err(e) => {
                let valid_len = e.valid_up_to();
                let valid = core::str::from_utf8(rest.get(..valid_len).unwrap_or_default())
                    .unwrap_or_default();
                (valid, e.error_len().unwrap_or(rest.len().saturating_sub(valid_len)))
            }
        };
        for (i, c) in valid.char_indices() {
            offsets.push(pos.saturating_add(i));
            text.push(c);
        }
        pos = pos.saturating_add(valid.len());
        if bad > 0 {
            offsets.push(pos);
            text.push('\u{FFFD}');
            pos = pos.saturating_add(bad);
        }
    }
    offsets.push(bytes.len());
    (text, offsets)
}

impl Encoding for Utf8Encoding {
    #[inline]
    fn name(&self) -> &str {
        "UTF-8"
    }

    #[inline]
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        if let Ok(valid) = core::str::from_utf8(bytes) {
            Ok(String::from(valid))
        } else {
            Ok(utf8_lossy_with_offsets(bytes).0)
        }
    }

    #[inline]
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        utf8_lossy_with_offsets(bytes)
    }

    #[inline]
    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    #[inline]
    fn find_unmappable_char(&self, _text: &str) -> Option<char> {
        None
    }

    #[inline]
    fn has_malformed_input(&self, bytes: &[u8]) -> bool {
        core::str::from_utf8(bytes).is_err()
    }

    #[inline]
    fn is_variable_width(&self) -> bool {
        true
    }

    #[inline]
    fn unit_bits(&self) -> usize {
        8
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        8
    }
}
