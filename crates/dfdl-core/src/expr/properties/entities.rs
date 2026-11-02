//! DFDL Character and Byte Entities (DFDL 1.0 §23.4 & §13.2.1).
//!
//! Handles encoding, decoding, syntax validation, and character class restrictions
//! for DFDL String Literal properties.

extern crate alloc;
use alloc::string::String;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};

/// Validates whether a DFDL regular expression pattern is syntactically sound.
pub(crate) fn is_valid_dfdl_regex(pat: &str) -> bool {
    let mut in_brace = false;
    for c in pat.chars() {
        if c == '{' {
            in_brace = true;
        } else if c == '}' {
            in_brace = false;
        } else if in_brace && c.is_whitespace() {
            return false;
        }
    }
    crate::pattern::DfdlRegex::new(pat).is_ok()
}

pub(crate) fn is_string_literal_property(key: &str) -> bool {
    let norm_key = key.trim_start_matches("dfdl:");
    matches!(
        norm_key,
        "initiator"
            | "terminator"
            | "separator"
            | "nilValue"
            | "textBooleanTrueRep"
            | "textBooleanFalseRep"
            | "textStandardDecimalSeparator"
            | "textStandardGroupingSeparator"
            | "textStandardExponentRep"
            | "textStandardInfinityRep"
            | "textInfinityRep"
            | "textStandardNaNRep"
            | "textStandardNanRep"
            | "textNanRep"
            | "textStandardZeroRep"
            | "escapeCharacter"
            | "escapeEscapeCharacter"
            | "extraEscapedCharacters"
            | "escapeBlockStart"
            | "escapeBlockEnd"
            | "textPadChar"
            | "textStringPadCharacter"
            | "textNumberPadCharacter"
            | "textBooleanPadCharacter"
            | "textCalendarPadCharacter"
            | "fillByte"
    )
}

pub(crate) fn property_forbids_literal_whitespace(key: &str) -> bool {
    let norm_key = key.trim_start_matches("dfdl:");
    matches!(
        norm_key,
        "escapeCharacter"
            | "escapeEscapeCharacter"
            | "escapeBlockStart"
            | "escapeBlockEnd"
            | "textPadChar"
            | "textStringPadCharacter"
            | "textNumberPadCharacter"
            | "textBooleanPadCharacter"
            | "textCalendarPadCharacter"
    )
}

pub(crate) fn is_standard_dfdl_entity_name(name: &str) -> bool {
    matches!(
        name,
        "ACK"
            | "BEL"
            | "BS"
            | "CAN"
            | "CR"
            | "DC1"
            | "DC2"
            | "DC3"
            | "DC4"
            | "DEL"
            | "DLE"
            | "EM"
            | "ENQ"
            | "EOT"
            | "ESC"
            | "ETB"
            | "ETX"
            | "FF"
            | "FS"
            | "GS"
            | "HT"
            | "LF"
            | "LS"
            | "NAK"
            | "NEL"
            | "NL"
            | "NUL"
            | "RS"
            | "SI"
            | "SO"
            | "SOH"
            | "SP"
            | "STX"
            | "SUB"
            | "SYN"
            | "US"
            | "VT"
            | "ES"
            | "WSP"
            | "WSP*"
            | "WSP+"
            | "NBSP"
    )
}

pub(crate) fn is_single_character_property(key: &str) -> bool {
    let norm_key = key.trim_start_matches("dfdl:");
    matches!(
        norm_key,
        "textStringPadCharacter"
            | "textNumberPadCharacter"
            | "textBooleanPadCharacter"
            | "textCalendarPadCharacter"
            | "textPadChar"
            | "textStandardGroupingSeparator"
            | "textStandardDecimalSeparator"
            | "escapeCharacter"
            | "escapeEscapeCharacter"
    )
}

pub(crate) fn is_character_class_entity(entity: &str) -> bool {
    matches!(entity, "NL" | "WSP" | "WSP*" | "WSP+" | "ES")
}

pub(crate) fn property_disallows_character_classes(key: &str) -> bool {
    let norm = key.trim_start_matches("dfdl:");
    is_single_character_property(norm)
        || norm == "extraEscapedCharacters"
        || matches!(
            norm,
            "textStandardExponentRep"
                | "textStandardNaNRep"
                | "textStandardNanRep"
                | "textNanRep"
                | "textStandardInfinityRep"
                | "textInfinityRep"
                | "textBooleanTrueRep"
                | "textBooleanFalseRep"
        )
}

pub(crate) fn validate_dfdl_property_entities(key: &str, value: &str) -> DFDLResult<()> {
    if !is_string_literal_property(key) {
        return Ok(());
    }
    let trimmed = value.trim();
    if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
        if !trimmed.ends_with('}') {
            let msg = alloc::format!(
                "Schema Definition Error: '{}' is an unterminated expression",
                value
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
        return Ok(());
    }
    if property_forbids_literal_whitespace(key) && value.chars().any(|c| c.is_ascii_whitespace()) {
        let msg = alloc::format!(
            "Schema Definition Error: The string ({}) must not contain any whitespace in property '{}' or escapeScheme. Use DFDL Entities for whitespace characters.",
            value, key
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(&b) = bytes.get(i) {
            if b < 0x20 && b != b'\t' && b != b'\r' && b != b'\n' {
                let msg = alloc::format!(
                    "Schema Definition Error: Control character (0x{:02X}) not allowed directly in property '{}'. Use DFDL entities.",
                    b, key
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        if bytes.get(i) == Some(&b'%') {
            let rest = value.get(i..).unwrap_or("");
            if rest.starts_with("%%") || rest.starts_with("%,") {
                i = i.saturating_add(2);
                continue;
            }
            if let Some(semi_pos) = rest.find(';') {
                let entity = rest.get(1..semi_pos).unwrap_or("");
                if property_disallows_character_classes(key) && is_character_class_entity(entity) {
                    let msg = alloc::format!(
                        "Schema Definition Error: {} contains disallowed character class(es): %{};",
                        key,
                        entity
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                let norm_prop_key = key.trim_start_matches("dfdl:");
                if norm_prop_key == "textStandardZeroRep"
                    && matches!(entity, "NL" | "CR" | "LF")
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: {} contains disallowed character class(es): %{};",
                        key,
                        entity
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if key.trim_start_matches("dfdl:") == "separator" && entity == "ES" {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Separator contains disallowed %ES;",
                    ));
                }
                if is_standard_dfdl_entity_name(entity) {
                    i = i.saturating_add(semi_pos.saturating_add(1));
                    continue;
                }
                if let Some(hex_part) = entity.strip_prefix("#x") {
                    if !hex_part.is_empty() && hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
                        i = i.saturating_add(semi_pos.saturating_add(1));
                        continue;
                    }
                } else if let Some(byte_part) = entity.strip_prefix("#r") {
                    let norm_key = key.trim_start_matches("dfdl:");
                    let allows_byte_entity = matches!(
                        norm_key,
                        "fillByte"
                            | "nilValue"
                            | "initiator"
                            | "terminator"
                            | "separator"
                            | "textStringPadCharacter"
                            | "textNumberPadCharacter"
                            | "textBooleanPadCharacter"
                            | "textCalendarPadCharacter"
                            | "textPadChar"
                    );
                    if !allows_byte_entity {
                        let msg = alloc::format!(
                            "Schema Definition Error: DFDL Byte Entity (%{};) is not allowed in property '{}'",
                            entity, key
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    if !byte_part.is_empty() && byte_part.chars().all(|c| c.is_ascii_hexdigit()) {
                        i = i.saturating_add(semi_pos.saturating_add(1));
                        continue;
                    }
                } else if let Some(dec_part) = entity
                    .strip_prefix("#d")
                    .or_else(|| entity.strip_prefix('#'))
                {
                    if !dec_part.is_empty() && dec_part.chars().all(|c| c.is_ascii_digit()) {
                        i = i.saturating_add(semi_pos.saturating_add(1));
                        continue;
                    }
                }
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid DFDL Entity or Byte Entity '%{};'",
                    entity
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let msg = alloc::format!(
                "Schema Definition Error: Invalid DFDL Entity or Byte Entity in property '{}'",
                key
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
        i = i.saturating_add(1);
    }
    if key.trim_start_matches("dfdl:") == "extraEscapedCharacters" {
        // Each whitespace-separated item must denote exactly one character.
        for item in value.split_whitespace() {
            if decode_dfdl_character_entities(item).chars().count() != 1 {
                let msg = alloc::format!(
                    "Schema Definition Error: Length of string must be exactly 1 character for each item of property '{}', got '{}'",
                    key, item
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
    }
    if is_single_character_property(key) {
        let norm_key = key.trim_start_matches("dfdl:");
        let decoded = decode_dfdl_character_entities(value);
        if norm_key == "escapeEscapeCharacter" && decoded.is_empty() {
            // Allowed per DFDL §13.2.1 Table 41: empty string disables escape-escaping
        } else if decoded.chars().count() != 1 {
            let msg = alloc::format!(
                "Schema Definition Error: Length of string must be exactly 1 character for property '{}', got '{}'",
                key, value
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
    }
    Ok(())
}

/// Decodes DFDL character entities into standard Unicode characters.
pub fn decode_dfdl_character_entities(val: &str) -> String {
    if !val.contains('%') {
        return String::from(val);
    }
    let mut out = String::new();
    let mut i = 0;
    let bytes = val.as_bytes();
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'%') {
            let rest = val.get(i..).unwrap_or("");
            if rest.starts_with("%%") {
                out.push('%');
                i = i.saturating_add(2);
                continue;
            }
            if let Some(semi) = rest.find(';') {
                let ent = rest.get(1..semi).unwrap_or("");
                if ent == "ES" {
                    i = i.saturating_add(semi.saturating_add(1));
                    continue;
                }
                let decoded_char = match ent {
                    "SP" => Some(' '),
                    "HT" => Some('\t'),
                    "LF" => Some('\n'),
                    "CR" => Some('\r'),
                    "NL" => Some('\n'),
                    "NUL" => Some('\0'),
                    "DEL" => Some('\x7F'),
                    "LS" => Some('\u{2028}'),
                    "NEL" => Some('\u{0085}'),
                    "NBSP" => Some('\u{00A0}'),
                    "ESC" => Some('\x1B'),
                    "BEL" => Some('\x07'),
                    "BS" => Some('\x08'),
                    "FF" => Some('\x0C'),
                    "VT" => Some('\x0B'),
                    "CAN" => Some('\x18'),
                    "ACK" => Some('\x06'),
                    "NAK" => Some('\x15'),
                    "ENQ" => Some('\x05'),
                    "EOT" => Some('\x04'),
                    "ETX" => Some('\x03'),
                    "STX" => Some('\x02'),
                    "SOH" => Some('\x01'),
                    "SO" => Some('\x0E'),
                    "SI" => Some('\x0F'),
                    "SYN" => Some('\x16'),
                    "ETB" => Some('\x17'),
                    "EM" => Some('\x19'),
                    "SUB" => Some('\x1A'),
                    "FS" => Some('\x1C'),
                    "GS" => Some('\x1D'),
                    "RS" => Some('\x1E'),
                    "US" => Some('\x1F'),
                    "DLE" => Some('\x10'),
                    "DC1" => Some('\x11'),
                    "DC2" => Some('\x12'),
                    "DC3" => Some('\x13'),
                    "DC4" => Some('\x14'),
                    _ => {
                        if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#r")) {
                            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                        } else if let Some(dec) =
                            ent.strip_prefix("#d").or_else(|| ent.strip_prefix('#'))
                        {
                            dec.parse::<u32>().ok().and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                if let Some(ch) = decoded_char {
                    out.push(ch);
                    i = i.saturating_add(semi.saturating_add(1));
                    continue;
                }
            }
        }
        if let Some(ch) = val.get(i..).and_then(|s| s.chars().next()) {
            out.push(ch);
            i = i.saturating_add(ch.len_utf8());
        } else {
            break;
        }
    }
    out
}

/// Encodes special characters and control characters into DFDL character entity references (DFDL §23.4).
pub fn encode_dfdl_character_entities(val: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < val.len() {
        let rest = val.get(i..).unwrap_or("");
        if rest.starts_with("\r\n") {
            out.push_str("%CR;%LF;");
            i = i.saturating_add(2);
            continue;
        }
        let ch = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        match ch {
            '%' => out.push_str("%%"),
            '\r' => out.push_str("%CR;"),
            '\n' => out.push_str("%LF;"),
            '\u{0085}' => out.push_str("%NEL;"),
            '\u{2028}' => out.push_str("%LS;"),
            '\0' => out.push_str("%NUL;"),
            '\t' => out.push_str("%HT;"),
            '\x7F' => out.push_str("%DEL;"),
            '\x1B' => out.push_str("%ESC;"),
            '\x07' => out.push_str("%BEL;"),
            '\x08' => out.push_str("%BS;"),
            '\x0C' => out.push_str("%FF;"),
            '\x0B' => out.push_str("%VT;"),
            '\x18' => out.push_str("%CAN;"),
            '\x06' => out.push_str("%ACK;"),
            '\x15' => out.push_str("%NAK;"),
            '\x05' => out.push_str("%ENQ;"),
            '\x04' => out.push_str("%EOT;"),
            '\x03' => out.push_str("%ETX;"),
            '\x02' => out.push_str("%STX;"),
            '\x01' => out.push_str("%SOH;"),
            '\x0E' => out.push_str("%SO;"),
            '\x0F' => out.push_str("%SI;"),
            '\x16' => out.push_str("%SYN;"),
            '\x17' => out.push_str("%ETB;"),
            '\x19' => out.push_str("%EM;"),
            '\x1A' => out.push_str("%SUB;"),
            '\x1C' => out.push_str("%FS;"),
            '\x1D' => out.push_str("%GS;"),
            '\x1E' => out.push_str("%RS;"),
            '\x1F' => out.push_str("%US;"),
            '\x10' => out.push_str("%DLE;"),
            '\x11' => out.push_str("%DC1;"),
            '\x12' => out.push_str("%DC2;"),
            '\x13' => out.push_str("%DC3;"),
            '\x14' => out.push_str("%DC4;"),
            c if c.is_control() => {
                use core::fmt::Write;
                let _ = write!(out, "%#x{:02X};", c as u32);
            }
            c => out.push(c),
        }
        i = i.saturating_add(ch.len_utf8());
    }
    out
}
