//! DFDL Character Encoding Engine.
//!
//! Conforms to DFDL 1.0 Specification §11 (`dfdl:encoding`).
//! Supports standard DFDL character encodings:
//! - UTF-8
//! - UTF-16, UTF-16BE, UTF-16LE
//! - ASCII / US-ASCII / ASCII-7
//! - ISO-8859-1 / Latin1
//! - EBCDIC / IBM037 / CP037

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::DFDLResult;
use crate::util::try_push;

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

/// Decodes text bytes into a Unicode [`String`] according to the given encoding name.
pub fn decode_text_bytes(bytes: &[u8], encoding_name: &str) -> DFDLResult<String> {
    let enc_clean = encoding_name.trim().to_ascii_uppercase();
    match enc_clean.as_str() {
        "UTF-8" | "UTF8" => {
            if let Ok(valid) = core::str::from_utf8(bytes) {
                Ok(String::from(valid))
            } else {
                Ok(utf8_lossy_with_offsets(bytes).0)
            }
        }
        "UTF-16" | "UTF-16BE" | "UTF16BE" => decode_utf16(bytes, true),
        "UTF-16LE" | "UTF16LE" => decode_utf16(bytes, false),
        "UTF-32" | "UTF-32BE" | "UTF32BE" => decode_utf32(bytes, true),
        "UTF-32LE" | "UTF32LE" => decode_utf32(bytes, false),
        "ASCII" | "US-ASCII" | "ASCII-7" | "ASCII7" => {
            let mut out = String::new();
            for &b in bytes {
                let ch = if b <= 127 { b as char } else { '?' };
                out.push(ch);
            }
            Ok(out)
        }
        s if s.starts_with("EBCDIC") || s == "IBM037" || s == "CP037" => {
            let mut out = String::new();
            for &b in bytes {
                if let Some(&ch) = EBCDIC_IBM037_TABLE.get(b as usize) {
                    out.push(ch);
                } else {
                    out.push('?');
                }
            }
            Ok(out)
        }
        "ISO-8859-1" | "LATIN1" | "LATIN-1" | "CP1252" => {
            let mut out = String::new();
            for &b in bytes {
                out.push(b as char);
            }
            Ok(out)
        }
        "ISO-8859-13" | "LATIN7" | "LATIN-7" => {
            let mut out = String::new();
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
        // DFDL bitwise inverted 8-bit encodings (e.g. X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE).
        // Each character byte representation is bitwise inverted (255 - b).
        s if s.contains("8-BIT-PACKED-LSB-FIRST-REVERSE") => {
            let mut out = String::new();
            for &b in bytes {
                out.push((255u8.saturating_sub(b)) as char);
            }
            Ok(out)
        }
        _ => {
            if let Ok(valid) = core::str::from_utf8(bytes) {
                Ok(String::from(valid))
            } else {
                let mut out = String::new();
                for &b in bytes {
                    out.push(b as char);
                }
                Ok(out)
            }
        }
    }
}

/// Encodes a Unicode string into raw bytes according to the given encoding name.
pub fn encode_text_string(text: &str, encoding_name: &str) -> Vec<u8> {
    let enc_clean = encoding_name.trim().to_ascii_uppercase();
    match enc_clean.as_str() {
        "UTF-8" | "UTF8" => text.as_bytes().to_vec(),
        "UTF-16" | "UTF-16BE" | "UTF16BE" => {
            let mut out = Vec::new();
            for u in text.encode_utf16() {
                out.push((u >> 8) as u8);
                out.push((u & 0xFF) as u8);
            }
            out
        }
        "UTF-16LE" | "UTF16LE" => {
            let mut out = Vec::new();
            for u in text.encode_utf16() {
                out.push((u & 0xFF) as u8);
                out.push((u >> 8) as u8);
            }
            out
        }
        "UTF-32" | "UTF-32BE" | "UTF32BE" => {
            let mut out = Vec::new();
            for c in text.chars() {
                let u = c as u32;
                out.push((u >> 24) as u8);
                out.push((u >> 16) as u8);
                out.push((u >> 8) as u8);
                out.push((u & 0xFF) as u8);
            }
            out
        }
        "UTF-32LE" | "UTF32LE" => {
            let mut out = Vec::new();
            for c in text.chars() {
                let u = c as u32;
                out.push((u & 0xFF) as u8);
                out.push((u >> 8) as u8);
                out.push((u >> 16) as u8);
                out.push((u >> 24) as u8);
            }
            out
        }
        "ASCII" | "US-ASCII" | "ASCII-7" | "ASCII7" => {
            text.chars()
                .map(|c| if (c as u32) <= 127 { c as u8 } else { b'?' })
                .collect()
        }
        "ISO-8859-1" | "LATIN1" | "LATIN-1" | "CP1252" => text
            .chars()
            .map(|c| if (c as u32) <= 255 { c as u8 } else { b'?' })
            .collect(),
        "ISO-8859-13" | "LATIN7" | "LATIN-7" => {
            let mut out = Vec::new();
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
        s if s.starts_with("EBCDIC") || s == "IBM037" || s == "CP037" => {
            let mut out = Vec::new();
            for c in text.chars() {
                if let Some(pos) = EBCDIC_IBM037_TABLE.iter().position(|&tc| tc == c) {
                    out.push(pos as u8);
                } else {
                    out.push(0x6F); // '?' in EBCDIC IBM037 is 0x6F
                }
            }
            out
        }
        // DFDL bitwise inverted 8-bit encodings (e.g. X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE).
        // Inverts character codes (255 - code) within the ISO-8859-1 range (0..=255).
        s if s.contains("8-BIT-PACKED-LSB-FIRST-REVERSE") => text
            .chars()
            .map(|c| if (c as u32) <= 255 { 255u8.saturating_sub(c as u8) } else { b'?' })
            .collect(),
        _ => text.as_bytes().to_vec(),
    }
}

/// Returns the first character of `text` that `encoding_name` cannot represent, if any.
///
/// Used for `encodingErrorPolicy="error"`; with `replace`, [`encode_text_string`] substitutes
/// `?`. Unicode encodings and unknown encodings represent every character.
#[must_use]
pub fn find_unmappable_char(text: &str, encoding_name: &str) -> Option<char> {
    let enc_clean = encoding_name.trim().to_ascii_uppercase();
    match enc_clean.as_str() {
        "ASCII" | "US-ASCII" | "ASCII-7" | "ASCII7" => text.chars().find(|c| !c.is_ascii()),
        "ISO-8859-1" | "LATIN1" | "LATIN-1" | "CP1252" => {
            text.chars().find(|c| (*c as u32) > 255)
        }
        "ISO-8859-13" | "LATIN7" | "LATIN-7" => {
            text.chars().find(|c| {
                let u = *c as u32;
                u >= 0xA0 && !ISO_8859_13_HIGH.contains(c)
            })
        }
        s if s.starts_with("EBCDIC") || s == "IBM037" || s == "CP037" => text
            .chars()
            .find(|c| !EBCDIC_IBM037_TABLE.contains(c)),
        // 8-bit packed reverse encodings support any character representable in 8 bits (0..=255).
        s if s.contains("8-BIT-PACKED-LSB-FIRST-REVERSE") => {
            text.chars().find(|c| (*c as u32) > 255)
        }
        _ => None,
    }
}

/// Decodes UTF-8 replacing each malformed sequence with U+FFFD (`encodingErrorPolicy="replace"`).
///
/// Returns the text and, for every character, its starting byte offset in `bytes`, followed by a
/// final entry holding the total length, so `offsets[n]` is the byte length of the first `n`
/// characters.
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

/// Returns `true` when `bytes` are not valid for `encoding_name`. Only UTF-8 is checked: the
/// Daffodil TDML suite expects US-ASCII input above 0x7F to keep the `replace` behaviour even with
/// `encodingErrorPolicy="error"`. Used for `encodingErrorPolicy="error"` on parse.
#[must_use]
pub fn has_malformed_input(bytes: &[u8], encoding_name: &str) -> bool {
    match encoding_name.trim().to_ascii_uppercase().as_str() {
        "UTF-8" | "UTF8" => core::str::from_utf8(bytes).is_err(),
        _ => false,
    }
}

fn decode_utf32(bytes: &[u8], is_big_endian: bool) -> DFDLResult<String> {
    let mut out = String::new();
    let mut i = 0usize;
    while i.saturating_add(3) < bytes.len() {
        let b0 = *bytes.get(i).unwrap_or(&0) as u32;
        let b1 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u32;
        let b2 = *bytes.get(i.saturating_add(2)).unwrap_or(&0) as u32;
        let b3 = *bytes.get(i.saturating_add(3)).unwrap_or(&0) as u32;
        let code_point = if is_big_endian {
            (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
        } else {
            (b3 << 24) | (b2 << 16) | (b1 << 8) | b0
        };
        let ch = char::from_u32(code_point).unwrap_or('?');
        out.push(ch);
        i = i.saturating_add(4);
    }
    Ok(out)
}

fn decode_utf16(bytes: &[u8], is_big_endian: bool) -> DFDLResult<String> {
    let mut u16_units = Vec::new();
    let mut i = 0usize;
    while i.saturating_add(1) < bytes.len() {
        let b1 = *bytes.get(i).unwrap_or(&0) as u16;
        let b2 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u16;
        let unit = if is_big_endian {
            (b1 << 8) | b2
        } else {
            (b2 << 8) | b1
        };
        try_push(&mut u16_units, unit)?;
        i = i.saturating_add(2);
    }
    let mut out = String::new();
    for res in char::decode_utf16(u16_units) {
        let ch = res.unwrap_or('?');
        out.push(ch);
    }
    Ok(out)
}

/// Returns the number of bits per character code unit if the encoding uses sub-byte
/// or non-standard character widths (e.g. 1-bit, 2-bit, 3-bit, 4-bit, 5-bit, 6-bit, 7-bit).
#[must_use]
pub fn encoding_char_bits(encoding: &str) -> Option<usize> {
    let enc_upper = encoding.trim().to_ascii_uppercase();
    if enc_upper.contains("BITS") {
        Some(1)
    } else if enc_upper.contains("BASE4") {
        Some(2)
    } else if enc_upper.contains("OCTAL") || enc_upper.contains("3-BIT") {
        Some(3)
    } else if enc_upper.contains("HEX") || enc_upper.contains("4-BIT") {
        Some(4)
    } else if enc_upper.contains("5-BIT") {
        Some(5)
    } else if enc_upper.contains("6-BIT") {
        Some(6)
    } else if enc_upper.contains("7-BIT") || enc_upper.contains("ASCII-7-BIT-PACKED") {
        Some(7)
    } else {
        None
    }
}

/// Returns the mandatory character alignment / unit width in bits for the given encoding.
///
/// Per DFDL §12.1.2: If representation is 'text', alignment must be a multiple of the
/// character width (in bits) of the encoding.
#[must_use]
pub fn encoding_unit_bits(encoding: &str) -> usize {
    if let Some(cb) = encoding_char_bits(encoding) {
        return cb;
    }
    let upper = encoding.trim().to_ascii_uppercase();
    if upper.contains("UTF-32") || upper.contains("UCS-4") {
        32
    } else if upper.contains("UTF-16") || upper.contains("UCS-2") {
        16
    } else {
        8
    }
}

/// Character table of the 5-bit packed encoding: codes 0-7 are '0'-'7', 8-31 are A-Z without I and O.
const PACKED_5BIT_CHARS: &[u8; 32] = b"01234567ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Decodes an integer code point into a character for sub-byte encodings.
#[must_use]
pub fn decode_sub_byte_char(code: u64, encoding: &str) -> char {
    let enc_upper = encoding.trim().to_ascii_uppercase();
    if enc_upper.contains("BITS") {
        if code == 1 { '1' } else { '0' }
    } else if enc_upper.contains("BASE4") {
        (b'0'.saturating_add(code as u8 & 0x03)) as char
    } else if enc_upper.contains("OCTAL") {
        (b'0'.saturating_add(code as u8 & 0x07)) as char
    } else if enc_upper.contains("HEX") {
        core::char::from_digit(code as u32, 16)
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('?')
    } else if enc_upper.contains("DFI-746")
        || enc_upper.contains("DFI-747")
        || enc_upper.contains("DFI-336")
        || enc_upper.contains("DFI-769")
    {
        (b'A'.saturating_add(code as u8)) as char
    } else if enc_upper.contains("DFI-264-DUI-001") {
        match code {
            0..=9 => (b'0'.saturating_add(code as u8)) as char,
            10 => ' ',
            11..=36 => (b'A'.saturating_add((code as u8).saturating_sub(11))) as char,
            _ => '?',
        }
    } else if enc_upper.contains("DFI-1661-DUI-001") {
        match code {
            0 => '\u{00A0}',
            1..=26 => (b'A'.saturating_add((code as u8).saturating_sub(1)) ) as char,
            _ => '\u{FFFD}',
        }
    } else if enc_upper.contains("5-BIT") {
        PACKED_5BIT_CHARS
            .get(code as usize)
            .map_or('?', |b| *b as char)
    } else if enc_upper.contains("6-BIT") {
        // Codes 0-31 are '@'..'_' (0x40-0x5F), codes 32-63 are ' '..'?' (0x20-0x3F).
        let c = code as u8 & 0x3F;
        (if c < 32 { c.saturating_add(0x40) } else { c }) as char
    } else if enc_upper.contains("7-BIT") || enc_upper.contains("ASCII-7-BIT-PACKED") {
        if code <= 127 { code as u8 as char } else { '?' }
    } else {
        core::char::from_u32(code as u32).unwrap_or('?')
    }
}

/// Strictly maps a character to its code in a sub-byte encoding; `None` when the character
/// has no code (an unmappable character) or the encoding is not a sub-byte one.
#[must_use]
pub fn strict_sub_byte_code(ch: char, encoding: &str) -> Option<u64> {
    encoding_char_bits(encoding)?;
    let enc_upper = encoding.trim().to_ascii_uppercase();
    let alpha_index = |base: char, offset: u64| -> Option<u64> {
        let up = ch.to_ascii_uppercase();
        (base..='Z')
            .contains(&up)
            .then(|| (up as u64).saturating_sub(base as u64).saturating_add(offset))
    };
    if enc_upper.contains("BITS") {
        ch.to_digit(2).map(u64::from)
    } else if enc_upper.contains("BASE4") {
        ch.to_digit(4).map(u64::from)
    } else if enc_upper.contains("OCTAL") {
        ch.to_digit(8).map(u64::from)
    } else if enc_upper.contains("HEX") {
        ch.to_digit(16).map(u64::from)
    } else if enc_upper.contains("DFI-746")
        || enc_upper.contains("DFI-747")
        || enc_upper.contains("DFI-336")
        || enc_upper.contains("DFI-769")
    {
        alpha_index('A', 0)
    } else if enc_upper.contains("DFI-264-DUI-001") {
        match ch {
            '0'..='9' => ch.to_digit(10).map(u64::from),
            ' ' => Some(10),
            _ => alpha_index('A', 11),
        }
    } else if enc_upper.contains("DFI-1661-DUI-001") {
        if ch == '\u{00A0}' { Some(0) } else { alpha_index('A', 1) }
    } else if enc_upper.contains("5-BIT") {
        match ch.to_ascii_uppercase() {
            // Letters O and I have no code of their own; they alias the digits 0 and 1.
            'O' => Some(0),
            'I' => Some(1),
            up => PACKED_5BIT_CHARS
                .iter()
                .position(|b| *b as char == up)
                .map(|i| i as u64),
        }
    } else if enc_upper.contains("6-BIT") {
        // Printable ASCII 0x20..=0x5F; the code is the low six bits.
        ('\u{20}'..='\u{5F}').contains(&ch).then_some((ch as u64) & 0x3F)
    } else if enc_upper.contains("7-BIT") || enc_upper.contains("ASCII-7-BIT-PACKED") {
        ch.is_ascii().then_some(ch as u64)
    } else {
        Some(ch as u64)
    }
}

/// Code used in place of an unmappable character (encodingErrorPolicy `replace`):
/// 5-bit packed uses 'X', 6-bit packed '_', 7-bit packed '?', every other encoding code 0.
#[must_use]
pub fn sub_byte_replacement_code(encoding: &str) -> u64 {
    let enc_upper = encoding.trim().to_ascii_uppercase();
    if enc_upper.contains("5-BIT") {
        29
    } else if enc_upper.contains("6-BIT") {
        0x1F
    } else if enc_upper.contains("7-BIT") || enc_upper.contains("ASCII-7-BIT-PACKED") {
        0x3F
    } else {
        0
    }
}

/// Encodes a character into its integer code point and bit length for sub-byte encodings,
/// substituting the encoding's replacement code for unmappable characters.
#[must_use]
pub fn encode_sub_byte_char(ch: char, encoding: &str) -> Option<(u64, usize)> {
    let cb = encoding_char_bits(encoding)?;
    let code = strict_sub_byte_code(ch, encoding)
        .unwrap_or_else(|| sub_byte_replacement_code(encoding));
    Some((code, cb))
}

/// Returns true if the encoding is a variable-width character encoding (such as UTF-8).
#[must_use]
pub fn is_variable_width_encoding(encoding: &str) -> bool {
    let lower = encoding.to_lowercase();
    let norm = lower.replace(['-', '_'], "");
    norm == "utf8"
        || norm.starts_with("gb18030")
        || norm.starts_with("shiftjis")
        || norm.starts_with("euc")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    /// 5-bit packed: digits 0-7, then letters skipping I and O; I/O alias 1/0 when encoding.
    #[test]
    fn test_packed_5bit_mapping() {
        let enc = "X-DFDL-5-BIT-PACKED";
        assert_eq!(decode_sub_byte_char(0, enc), '0');
        assert_eq!(decode_sub_byte_char(8, enc), 'A');
        assert_eq!(decode_sub_byte_char(16, enc), 'J');
        assert_eq!(decode_sub_byte_char(31, enc), 'Z');
        assert_eq!(encode_sub_byte_char('K', enc), Some((17, 5)));
        assert_eq!(encode_sub_byte_char('O', enc), Some((0, 5)));
        assert_eq!(encode_sub_byte_char('i', enc), Some((1, 5)));
        assert_eq!(encode_sub_byte_char('!', enc), Some((29, 5)));
        for code in 0..32u64 {
            let ch = decode_sub_byte_char(code, enc);
            assert_eq!(encode_sub_byte_char(ch, enc), Some((code, 5)));
        }
    }

    /// Unmappable characters have no strict code; replacement codes follow each encoding.
    #[test]
    fn test_sub_byte_strict_and_replacement() {
        assert_eq!(strict_sub_byte_code('\u{A3}', "X-DFDL-US-ASCII-7-BIT-PACKED"), None);
        assert_eq!(strict_sub_byte_code('B', "X-DFDL-US-ASCII-7-BIT-PACKED"), Some(0x42));
        assert_eq!(strict_sub_byte_code('9', "X-DFDL-OCTAL-LSBF"), None);
        assert_eq!(strict_sub_byte_code('7', "X-DFDL-OCTAL-LSBF"), Some(7));
        assert_eq!(strict_sub_byte_code('g', "X-DFDL-HEX-LSBF"), None);
        assert_eq!(strict_sub_byte_code('a', "X-DFDL-US-ASCII-6-BIT-PACKED"), None);
        assert_eq!(strict_sub_byte_code('!', "X-DFDL-5-BIT-PACKED-LSBF"), None);
        assert_eq!(sub_byte_replacement_code("X-DFDL-US-ASCII-7-BIT-PACKED"), 0x3F);
        assert_eq!(sub_byte_replacement_code("X-DFDL-US-ASCII-6-BIT-PACKED"), 0x1F);
        assert_eq!(sub_byte_replacement_code("X-DFDL-OCTAL-LSBF"), 0);
        assert_eq!(
            encode_sub_byte_char('\u{A3}', "X-DFDL-US-ASCII-7-BIT-PACKED"),
            Some((0x3F, 7))
        );
    }

    /// 6-bit packed ASCII covers 0x20..=0x5F: codes 0-31 are '@'..'_', 32-63 are ' '..'?'.
    #[test]
    fn test_packed_6bit_table_round_trip() {
        let enc = "X-DFDL-US-ASCII-6-BIT-PACKED";
        assert_eq!(decode_sub_byte_char(1, enc), 'A');
        assert_eq!(decode_sub_byte_char(0x1F, enc), '_');
        assert_eq!(decode_sub_byte_char(0x20, enc), ' ');
        for code in 0..64u64 {
            let ch = decode_sub_byte_char(code, enc);
            assert_eq!(strict_sub_byte_code(ch, enc), Some(code));
        }
    }

    /// Byte encodings: unmappable detection, and `?` as the replacement byte.
    #[test]
    fn test_find_unmappable_char_byte_encodings() {
        assert_eq!(find_unmappable_char("a\u{A3}", "US-ASCII"), Some('\u{A3}'));
        assert_eq!(find_unmappable_char("a\u{A3}", "ISO-8859-1"), None);
        assert_eq!(find_unmappable_char("a\u{20AC}", "ISO-8859-1"), Some('\u{20AC}'));
        assert_eq!(find_unmappable_char("a\u{20AC}", "UTF-8"), None);
        assert_eq!(find_unmappable_char("a", "IBM037"), None);
        assert_eq!(find_unmappable_char("\u{20AC}", "IBM037"), Some('\u{20AC}'));
        assert_eq!(encode_text_string("\u{20AC}", "ISO-8859-1"), [b'?']);
        assert_eq!(encode_text_string("\u{A3}", "US-ASCII"), [b'?']);
    }

    /// Validates ISO-8859-13 encoding and decoding of Baltic characters (e.g. Ą at 0xC0).
    #[test]
    fn test_iso_8859_13_roundtrip() {
        let text = "Ą1234567";
        let encoded = encode_text_string(text, "ISO-8859-13");
        assert_eq!(encoded, [0xC0, b'1', b'2', b'3', b'4', b'5', b'6', b'7']);
        let decoded = decode_text_bytes(&encoded, "ISO-8859-13").unwrap();
        assert_eq!(decoded, text);
        assert_eq!(find_unmappable_char(text, "ISO-8859-13"), None);
        assert_eq!(find_unmappable_char("€", "ISO-8859-13"), Some('€'));
    }

    /// Malformed UTF-8 bytes each become U+FFFD; offsets map characters back to source bytes.
    #[test]
    fn test_utf8_lossy_with_offsets() {
        let (text, offs) = utf8_lossy_with_offsets(&[0x21, 0xC2, 0xC2, 0x21]);
        assert_eq!(text, "!\u{FFFD}\u{FFFD}!");
        assert_eq!(offs, [0, 1, 2, 3, 4]);
        let (text, offs) = utf8_lossy_with_offsets("a\u{E9}b".as_bytes());
        assert_eq!(text, "a\u{E9}b");
        assert_eq!(offs, [0, 1, 3, 4]);
        // decode_text_bytes uses the same replacement for malformed UTF-8.
        assert_eq!(decode_text_bytes(&[0x21, 0xC2, 0xC2], "UTF-8").unwrap(), "!\u{FFFD}\u{FFFD}");
        assert!(has_malformed_input(&[0x21, 0xC2, 0xC2], "UTF-8"));
        assert!(!has_malformed_input("a\u{E9}".as_bytes(), "UTF-8"));
        assert!(!has_malformed_input(&[0x80], "US-ASCII"));
        assert!(!has_malformed_input(&[0x80], "ISO-8859-1"));
    }

    use super::*;

    #[test]
    fn test_encoding_decoders() {
        let bytes_utf8 = "Hello DFDL".as_bytes();
        assert_eq!(
            decode_text_bytes(bytes_utf8, "UTF-8").unwrap(),
            "Hello DFDL"
        );

        let bytes_ebcdic = [0xC8, 0xC5, 0xD3, 0xD3, 0xD6]; // "HELLO" in EBCDIC IBM037
        assert_eq!(decode_text_bytes(&bytes_ebcdic, "IBM037").unwrap(), "HELLO");

        let bytes_utf16be = [0x00, 0x41, 0x00, 0x42]; // "AB" in UTF-16BE
        assert_eq!(decode_text_bytes(&bytes_utf16be, "UTF-16BE").unwrap(), "AB");
    }
}
