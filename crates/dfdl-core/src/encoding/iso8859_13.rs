//! ISO-8859-13 (Latin-7 / Baltic Rim) character encoding implementation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;

/// Standard ISO-8859-13 / Latin-7 byte to Unicode character mapping for 0xA0..=0xFF.
#[rustfmt::skip]
static ISO_8859_13_HIGH: [char; 96] = [
    '\u{00A0}', '\u{201D}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{201E}', '\u{00A6}', '\u{00A7}',
    '\u{00D8}', '\u{00A9}', '\u{0156}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00C6}',
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{201C}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{00F8}', '\u{00B9}', '\u{0157}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{00E6}',
    '\u{0104}', '\u{012E}', '\u{0100}', '\u{0106}', '\u{00C4}', '\u{00C5}', '\u{0118}', '\u{0112}',
    '\u{010C}', '\u{00C9}', '\u{0179}', '\u{0116}', '\u{0122}', '\u{0136}', '\u{012A}', '\u{013B}',
    '\u{0160}', '\u{0143}', '\u{0145}', '\u{00D3}', '\u{014C}', '\u{00D5}', '\u{00D6}', '\u{00D7}',
    '\u{0172}', '\u{0141}', '\u{015A}', '\u{016A}', '\u{00DC}', '\u{017B}', '\u{017D}', '\u{00DF}',
    '\u{0105}', '\u{012F}', '\u{0101}', '\u{0107}', '\u{00E4}', '\u{00E5}', '\u{0119}', '\u{0113}',
    '\u{010D}', '\u{00E9}', '\u{017A}', '\u{0117}', '\u{0123}', '\u{0137}', '\u{012B}', '\u{013C}',
    '\u{0161}', '\u{0144}', '\u{0146}', '\u{00F3}', '\u{014D}', '\u{00F5}', '\u{00F6}', '\u{00F7}',
    '\u{0173}', '\u{0142}', '\u{015B}', '\u{016B}', '\u{00FC}', '\u{017C}', '\u{017E}', '\u{2019}',
];

/// ISO-8859-13 (Latin-7) character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Iso8859_13Encoding;

impl Encoding for Iso8859_13Encoding {
    #[inline]
    fn name(&self) -> &str {
        "ISO-8859-13"
    }

    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            if b < 0xA0 {
                out.push(b as char);
            } else {
                let ch = b
                    .checked_sub(0xA0)
                    .and_then(|idx| ISO_8859_13_HIGH.get(idx as usize).copied())
                    .unwrap_or('?');
                out.push(ch);
            }
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
        for c in text.chars() {
            let u = c as u32;
            if u < 0xA0 {
                out.push(u as u8);
            } else if let Some(pos) = ISO_8859_13_HIGH.iter().position(|&hc| hc == c) {
                let byte_val = 0xA0_usize
                    .checked_add(pos)
                    .and_then(|val| u8::try_from(val).ok())
                    .unwrap_or(b'?');
                out.push(byte_val);
            } else {
                out.push(b'?');
            }
        }
        out
    }

    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|c| {
            let u = *c as u32;
            u >= 0xA0 && !ISO_8859_13_HIGH.contains(c)
        })
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
