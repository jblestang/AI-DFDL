//! EBCDIC (IBM037 / CP037) character encoding implementation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;
use super::traits::Encoding;

/// Standard IBM037 / EBCDIC-US byte to Unicode character mapping table (0x00..=0xFF).
#[rustfmt::skip]
static EBCDIC_IBM037_TABLE: [char; 256] = [
    '\u{0000}', '\u{0001}', '\u{0002}', '\u{0003}', '\u{009C}', '\u{0009}', '\u{0086}', '\u{007F}',
    '\u{0097}', '\u{008D}', '\u{008E}', '\u{000B}', '\u{000C}', '\u{000D}', '\u{000E}', '\u{000F}',
    '\u{0010}', '\u{0011}', '\u{0012}', '\u{0013}', '\u{009D}', '\u{0085}', '\u{0008}', '\u{0087}',
    '\u{0018}', '\u{0019}', '\u{0092}', '\u{008F}', '\u{001C}', '\u{001D}', '\u{001E}', '\u{001F}',
    '\u{0080}', '\u{0081}', '\u{0082}', '\u{0083}', '\u{0084}', '\u{000A}', '\u{0017}', '\u{001B}',
    '\u{0088}', '\u{0089}', '\u{008A}', '\u{008B}', '\u{008C}', '\u{0005}', '\u{0006}', '\u{0007}',
    '\u{0090}', '\u{0091}', '\u{0016}', '\u{0093}', '\u{0094}', '\u{0095}', '\u{0096}', '\u{0004}',
    '\u{0098}', '\u{0099}', '\u{009A}', '\u{009B}', '\u{0014}', '\u{0015}', '\u{009E}', '\u{001A}',
    ' ',        '\u{00A0}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E1}', '\u{00E3}', '\u{00E5}',
    '\u{00E7}', '\u{00F1}', '[',        '.',        '<',        '(',        '+',        '!',
    '&',        '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00ED}', '\u{00EE}', '\u{00EF}',
    '\u{00EC}', '\u{00DF}', ']',        '$',        '*',        ')',        ';',        '^',
    '-',        '/',        '\u{00C2}', '\u{00C4}', '\u{00C0}', '\u{00C1}', '\u{00C3}', '\u{00C5}',
    '\u{00C7}', '\u{00D1}', '¦',        ',',        '%',        '_',        '>',        '?',
    '\u{00F8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00C8}', '\u{00CD}', '\u{00CE}', '\u{00CF}',
    '\u{00CC}', '`',        ':',        '#',        '@',        '\'',       '=',        '"',
    '\u{00D8}', 'a',        'b',        'c',        'd',        'e',        'f',        'g',
    'h',        'i',        '\u{00AB}', '\u{00BB}', '\u{00F0}', '\u{00FD}', '\u{00FE}', '\u{00B1}',
    '\u{00B0}', 'j',        'k',        'l',        'm',        'n',        'o',        'p',
    'q',        'r',        '\u{00AA}', '\u{00BA}', '\u{00E6}', '\u{00B8}', '\u{00C6}', '\u{00A4}',
    '\u{00B5}', '~',        's',        't',        'u',        'v',        'w',        'x',
    'y',        'z',        '\u{00A1}', '\u{00BF}', '\u{00D0}', '\u{00DD}', '\u{00DE}', '\u{00AE}',
    '^',        '£',        '¥',        '·',        '©',        '§',        '¶',        '¼',
    '½',        '¾',        '¬',        '|',        '¯',        '¨',        '´',        '×',
    '{',        'A',        'B',        'C',        'D',        'E',        'F',        'G',
    'H',        'I',        '\u{00AD}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00F3}', '\u{00F5}',
    '}',        'J',        'K',        'L',        'M',        'N',        'O',        'P',
    'Q',        'R',        '\u{00B9}', '\u{00FB}', '\u{00FC}', '\u{00F9}', '\u{00FA}', '\u{00FF}',
    '\\',       '\u{00F7}', 'S',        'T',        'U',        'V',        'W',        'X',
    'Y',        'Z',        '\u{00B2}', '\u{00D4}', '\u{00D6}', '\u{00D2}', '\u{00D3}', '\u{00D5}',
    '0',        '1',        '2',        '3',        '4',        '5',        '6',        '7',
    '8',        '9',        '\u{00B3}', '\u{00DB}', '\u{00DC}', '\u{00D9}', '\u{00DA}', '\u{009F}',
];

/// EBCDIC (IBM037 / CP037) character encoding handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EbcdicEncoding;

impl Encoding for EbcdicEncoding {
    #[inline]
    fn name(&self) -> &str {
        "IBM037"
    }

    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            let ch = EBCDIC_IBM037_TABLE.get(b as usize).copied().unwrap_or('?');
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
        for c in text.chars() {
            if let Some(pos) = EBCDIC_IBM037_TABLE.iter().position(|&tc| tc == c) {
                out.push(pos as u8);
            } else {
                out.push(0x6F); // '?' in EBCDIC IBM037 is 0x6F
            }
        }
        out
    }

    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        text.chars().find(|c| !EBCDIC_IBM037_TABLE.contains(c))
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
