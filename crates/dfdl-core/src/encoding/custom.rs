//! Custom and fallback character encoding implementation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;
use super::utf8::utf8_lossy_with_offsets;

/// Custom or uncataloged character encoding handler.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CustomEncoding {
    /// Encoding identifier string.
    pub name: String,
}

impl CustomEncoding {
    /// Creates a new custom encoding handler for the specified name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Encoding for CustomEncoding {
    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        if let Ok(valid) = core::str::from_utf8(bytes) {
            Ok(String::from(valid))
        } else {
            let mut out = String::with_capacity(bytes.len());
            for &b in bytes {
                out.push(b as char);
            }
            Ok(out)
        }
    }

    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        if let Ok(text) = self.decode_text_bytes(bytes) {
            let mut offsets = Vec::with_capacity(bytes.len().saturating_add(1));
            for i in 0..=bytes.len() {
                let _ = try_push(&mut offsets, i);
            }
            (text, offsets)
        } else {
            utf8_lossy_with_offsets(bytes)
        }
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
    fn unit_bits(&self) -> usize {
        8
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        8
    }
}
