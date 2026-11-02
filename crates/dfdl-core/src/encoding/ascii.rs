//! ASCII and US-ASCII character encoding implementation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;

/// ASCII / US-ASCII character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AsciiEncoding;

impl Encoding for AsciiEncoding {
    #[inline]
    fn name(&self) -> &str {
        "US-ASCII"
    }

    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            let ch = if b <= 127 { b as char } else { '?' };
            out.push(ch);
        }
        Ok(out)
    }

    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        let text = self.decode_text_bytes(bytes).unwrap_or_default();
        let mut offsets = Vec::with_capacity(bytes.len().saturating_add(1));
        for i in 0..=bytes.len() {
            let _ = try_push(&mut offsets, i);
        }
        (text, offsets)
    }

    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let b = if ch.is_ascii() { ch as u8 } else { b'?' };
            out.push(b);
        }
        out
    }

    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|c| !c.is_ascii())
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
