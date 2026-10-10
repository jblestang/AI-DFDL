//! DFDL Property Resolution, Scoping, and Inheritance Engine.
//!
//! Follows DFDL 1.0 Specification §6.3, §7, and §8.1.
//! Implements local annotation overriding, schema format defaults, and inheritance rules.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::traits::{BitOrder, ByteOrder};
use crate::schema::ir::{
    CompiledAssert, DfdlProp, LengthKind, NilKind, OccursCountKind, ParseUnparsePolicy,
    Representation, ResolvedProperties, SeparatorPosition, SeparatorSuppressionPolicy, SequenceKind,
};
use crate::util::try_push;

fn is_valid_dfdl_regex(pat: &str) -> bool {
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

/// Parses a `dfdl:byteOrder` value, whether literal or the result of a runtime expression.
///
/// Unknown values raise a Schema Definition Error (a runtime SDE when the value came from an
/// expression), so literal and computed values are validated identically.
pub fn parse_byte_order(value: &str) -> DFDLResult<ByteOrder> {
    match value.trim() {
        "bigEndian" | "BE" => Ok(ByteOrder::BigEndian),
        "littleEndian" | "LE" => Ok(ByteOrder::LittleEndian),
        other => Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            &alloc::format!("Schema Definition Error: Unknown value for byteOrder property: {}", other),
        )),
    }
}

fn is_string_literal_property(key: &str) -> bool {
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

fn property_forbids_literal_whitespace(key: &str) -> bool {
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

fn is_standard_dfdl_entity_name(name: &str) -> bool {
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

fn is_single_character_property(key: &str) -> bool {
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

fn is_character_class_entity(entity: &str) -> bool {
    matches!(entity, "NL" | "WSP" | "WSP*" | "WSP+" | "ES")
}

fn property_disallows_character_classes(key: &str) -> bool {
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

fn validate_dfdl_property_entities(key: &str, value: &str) -> DFDLResult<()> {
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
                        "fillByte" | "nilValue" | "initiator" | "terminator" | "separator"
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

/// Individual key-value property binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyBinding {
    /// Property name (e.g. `dfdl:byteOrder` or `byteOrder`).
    pub key: String,
    /// Raw string value assigned to property.
    pub value: String,
}

/// Scoped property store handling property binding and inheritance resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PropertyStore {
    bindings: Vec<PropertyBinding>,
    asserts: Vec<CompiledAssert>,
    set_variables: Vec<(crate::types::QName, String)>,
    new_variable_instances: Vec<(crate::types::QName, Option<String>)>,
    /// Number of discriminator statements attached locally.
    pub discriminator_count: usize,
    assert_errors: Vec<String>,
    in_scope_namespaces: Vec<(String, String)>,
}

impl PropertyStore {
    /// Constructs an empty [`PropertyStore`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            asserts: Vec::new(),
            set_variables: Vec::new(),
            new_variable_instances: Vec::new(),
            discriminator_count: 0,
            assert_errors: Vec::new(),
            in_scope_namespaces: Vec::new(),
        }
    }

    /// Returns the in-scope namespace bindings associated with this property store.
    #[inline]
    #[must_use]
    pub fn in_scope_namespaces(&self) -> &[(String, String)] {
        &self.in_scope_namespaces
    }

    /// Sets the in-scope namespace bindings associated with this property store.
    pub fn set_in_scope_namespaces(&mut self, ns: Vec<(String, String)>) {
        self.in_scope_namespaces = ns;
    }

    /// Adds namespace bindings to the in-scope namespaces of this property store.
    pub fn add_namespaces(&mut self, ns: &[(String, String)]) {
        for (prefix, uri) in ns {
            if !self.in_scope_namespaces.iter().any(|(p, _)| p == prefix) {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
    }

    /// Adds or updates namespace bindings in the in-scope namespaces of this property store.
    ///
    /// If a prefix is already bound, its URI is updated to reflect inner XML shadowing.
    pub fn update_namespaces(&mut self, ns: &[(String, String)]) {
        for (prefix, uri) in ns {
            if let Some(existing) = self.in_scope_namespaces.iter_mut().find(|(p, _)| p == prefix) {
                existing.1 = uri.clone();
            } else {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
    }

    /// Returns `true` if the property store contains no bindings, asserts, set_variables, or assert_errors.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
            && self.asserts.is_empty()
            && self.set_variables.is_empty()
            && self.new_variable_instances.is_empty()
            && self.assert_errors.is_empty()
    }

    /// Adds a DFDL setVariable declaration to this property store (§7.7).
    pub fn add_set_variable(&mut self, var_name: &str, value_expr: &str) {
        let clean = var_name
            .trim_start_matches("tns:")
            .trim_start_matches("ex:");
        let _ = try_push(
            &mut self.set_variables,
            (crate::types::QName::local(clean), value_expr.to_string()),
        );
    }

    /// Adds a DFDL newVariableInstance declaration to this property store (§7.7).
    pub fn add_new_variable_instance(&mut self, var_name: &str, default_val_expr: Option<&str>) {
        let clean = var_name
            .trim_start_matches("tns:")
            .trim_start_matches("ex:");
        let _ = try_push(
            &mut self.new_variable_instances,
            (
                crate::types::QName::local(clean),
                default_val_expr.map(alloc::string::ToString::to_string),
            ),
        );
    }

    /// Adds a recorded assertion definition error to be reported during term validation.
    pub fn add_assert_error(&mut self, err: &str) {
        let _ = try_push(&mut self.assert_errors, err.to_string());
    }

    /// Returns recorded assertion definition errors for this property store.
    #[inline]
    #[must_use]
    pub fn assert_errors(&self) -> &[String] {
        &self.assert_errors
    }

    /// Returns recorded newVariableInstances declarations on this property store.
    #[inline]
    #[must_use]
    pub fn new_variable_instances(&self) -> &[(crate::types::QName, Option<String>)] {
        &self.new_variable_instances
    }

    /// Returns recorded setVariable statements on this property store.
    #[inline]
    #[must_use]
    pub fn set_variables(&self) -> &[(crate::types::QName, String)] {
        &self.set_variables
    }

    /// Copies all local properties, assertions, and setVariables from `other` into `self`.
    pub fn extend(&mut self, other: &PropertyStore) {
        let self_has_enums = self.bindings.iter().any(|b| b.key == "enumeration");
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if binding.key == "enumeration" {
                if !self_has_enums
                    && !self.bindings.iter().any(|b| b.key == "enumeration" && b.value == binding.value)
                {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if self.get_property(&binding.key).is_none() {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        self.add_namespaces(&other.in_scope_namespaces);
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Copies local properties, assertions, and setVariables from `other` into `self`,
    /// excluding any property keys already defined in `exclude`.
    pub fn extend_excluding(&mut self, other: &PropertyStore, exclude: &PropertyStore) {
        let self_has_enums = self.bindings.iter().any(|b| b.key == "enumeration");
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if binding.key == "enumeration" {
                if !self_has_enums
                    && !self.bindings.iter().any(|b| b.key == "enumeration" && b.value == binding.value)
                {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if exclude.get_property(&binding.key).is_none()
                && self.get_property(&binding.key).is_none()
            {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        for (prefix, uri) in &other.in_scope_namespaces {
            if prefix.is_empty() {
                if !self.in_scope_namespaces.iter().any(|(p, _)| p.is_empty()) {
                    let _ = try_push(&mut self.in_scope_namespaces, (String::new(), uri.clone()));
                }
            } else if let Some(existing) = self.in_scope_namespaces.iter_mut().find(|(p, _)| p == prefix) {
                existing.1 = uri.clone();
            } else {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Overrides local properties, assertions, and setVariables with those from `other`.
    pub fn override_with(&mut self, other: &PropertyStore) {
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") || binding.key == "enumeration" {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Returns reference to attached assert items.
    #[inline]
    #[must_use]
    pub fn asserts(&self) -> &[CompiledAssert] {
        &self.asserts
    }

    /// Returns true if any asserts are attached.
    #[inline]
    #[must_use]
    pub fn has_asserts(&self) -> bool {
        !self.asserts.is_empty()
    }

    /// Merges missing parent properties into this property store.
    pub fn merge(&mut self, parent: &PropertyStore) {
        self.merge_parent(parent);
    }

    /// Merges missing inheritable parent properties into this property store.
    pub fn merge_parent(&mut self, parent: &PropertyStore) {
        for binding in &parent.bindings {
            if binding.key == "inputValueCalc"
                || binding.key == "outputValueCalc"
                || binding.key == "initiator"
                || binding.key == "terminator"
                || binding.key == "separator"
                || binding.key == "separatorPosition"
                || binding.key == "separatorSuppressionPolicy"
                || binding.key == "sequenceKind"
                || binding.key == "initiatedContent"
                || binding.key == "hiddenGroupRef"
                || binding.key == "choiceDispatchKey"
                || binding.key == "choiceBranchKey"
                || binding.key == "choiceLengthKind"
                || binding.key == "choiceLength"
                || binding.key == "leadingSkip"
                || binding.key == "trailingSkip"
                || binding.key == "occursCount"
                || binding.key == "assert"
                || binding.key == "discriminator"
                || binding.key == "setVariable"
                || binding.key == "newVariableInstance"
                || binding.key == "testKind"
                || binding.key == "length"
                || binding.key == "lengthKind"
                || binding.key == "lengthUnits"
                || binding.key == "alignmentUnits"
                || binding.key == "occursCountKind"
                || binding.key == "fillByte"
                || binding.key == "prefixLengthType"
                || binding.key == "prefixIncludesPrefixLength"
                || binding.key == "representation"
                || binding.key == "layer"
                //|| binding.key == "dfdlx:layer"
                || binding.key == "layerTransform"
                //|| binding.key == "dfdlx:layerTransform"
                //|| binding.key == "daf:layerTransform"
                || binding.key == "layerLength"
                || binding.key == "layerLengthUnits"
                || binding.key == "layerBoundaryMark"
                || binding.key == "repType"
                //|| binding.key == "dfdlx:repType"
                || binding.key == "repValues"
                //|| binding.key == "dfdlx:repValues"
                || binding.key == "repValueRanges"
                //|| binding.key == "dfdlx:repValueRanges"
            {
                continue;
            }
            if self.get_property(&binding.key).is_none() {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
    }

    /// Adds a DFDL assertion definition to this property store.
    pub fn add_assert(&mut self, test_expr: &str, message: Option<&str>) {
        self.add_assert_with_kind(crate::schema::ir::TestKind::Expression, test_expr, message);
    }

    /// Adds a DFDL assertion definition with explicit TestKind.
    pub fn add_assert_with_kind(
        &mut self,
        test_kind: crate::schema::ir::TestKind,
        test_expr: &str,
        message: Option<&str>,
    ) {
        self.add_assert_with_failure_type(
            test_kind,
            test_expr,
            message,
            crate::schema::ir::FailureType::ProcessingError,
        );
    }

    /// Adds a DFDL assertion definition with explicit TestKind and FailureType.
    pub fn add_assert_with_failure_type(
        &mut self,
        test_kind: crate::schema::ir::TestKind,
        test_expr: &str,
        message: Option<&str>,
        failure_type: crate::schema::ir::FailureType,
    ) {
        let _ = try_push(
            &mut self.asserts,
            CompiledAssert {
                test_kind,
                test_expr: test_expr.to_string(),
                message: message.map(String::from),
                failure_type,
            },
        );
    }

    /// Validates DFDL entity syntax and validity across all stored property bindings.
    pub fn validate_property_entities(&self) -> DFDLResult<()> {
        for binding in &self.bindings {
            validate_dfdl_property_entities(&binding.key, &binding.value)?;
        }
        Ok(())
    }

    /// Sets or overrides a property key-value binding.
    pub fn set_property(&mut self, key: &str, value: &str) -> DFDLResult<()> {
        let norm_key = key.trim_start_matches("dfdl:");
        for binding in &mut self.bindings {
            if binding.key == norm_key {
                binding.value = value.to_string();
                return Ok(());
            }
        }
        try_push(
            &mut self.bindings,
            PropertyBinding {
                key: norm_key.to_string(),
                value: value.to_string(),
            },
        )
    }

    /// Removes a property binding by key (with or without `dfdl:` prefix).
    pub fn remove_property(&mut self, key: &str) {
        let norm_key = key.trim_start_matches("dfdl:");
        self.bindings.retain(|b| b.key != norm_key);
    }

    /// Adds an enumeration value facet to this property store.
    pub fn add_enumeration(&mut self, value: &str) -> DFDLResult<()> {
        for b in &self.bindings {
            if b.key == "enumeration" && b.value == value {
                let msg = alloc::format!(
                    "Schema Definition Error: Enumerations must be unique: duplicate value '{}'",
                    value
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        try_push(
            &mut self.bindings,
            PropertyBinding {
                key: "enumeration".to_string(),
                value: value.to_string(),
            },
        )
    }

    /// Adds an enumeration representation value mapping for dfdlx:repType.
    pub fn add_enumeration_rep(&mut self, enum_val: &str, rep_val: &str) {
        let _ = try_push(
            &mut self.bindings,
            PropertyBinding {
                key: alloc::format!("repValue:{}", enum_val),
                value: rep_val.to_string(),
            },
        );
    }

    /// Retrieves an explicitly set local property value by key name.
    #[must_use]
    pub fn get_property(&self, key: &str) -> Option<&str> {
        let norm_key = key.trim_start_matches("dfdl:");
        for binding in &self.bindings {
            if binding.key == norm_key {
                return Some(&binding.value);
            }
        }
        None
    }

    /// Returns a reference slice of all property bindings stored locally.
    #[must_use]
    pub fn bindings(&self) -> &[PropertyBinding] {
        &self.bindings
    }

    /// Resolves property value along inheritance chain (local -> parent -> default).
    #[must_use]
    pub fn resolve_property<'a>(
        &'a self,
        parent: Option<&'a PropertyStore>,
        key: &str,
        default_val: &'a str,
    ) -> &'a str {
        if let Some(val) = self.get_property(key) {
            return val;
        }
        if let Some(p) = parent {
            if let Some(val) = p.get_property(key) {
                return val;
            }
        }
        default_val
    }

    /// Compiles property store into [`ResolvedProperties`] for a schema term.
    pub fn to_resolved_properties(
        &self,
        parent: Option<&PropertyStore>,
    ) -> DFDLResult<ResolvedProperties> {
        let rep_str = self.resolve_property(parent, "representation", "text");
        let rep = match rep_str {
            "binary" => Representation::Binary,
            "text" => Representation::Text,
            _ => {
                let msg = alloc::format!("Invalid representation property: {}", rep_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let bo_str = self.resolve_property(parent, "byteOrder", "bigEndian");
        let (byte_order, byte_order_expr) = if bo_str.starts_with('{') {
            (ByteOrder::BigEndian, Some(String::from(bo_str)))
        } else {
            (parse_byte_order(bo_str)?, None)
        };

        let bit_str = self.resolve_property(parent, "bitOrder", "mostSignificantBitFirst");
        let bit_order = match bit_str {
            "mostSignificantBitFirst" | "MSBF" => BitOrder::MostSignificantBitFirst,
            "leastSignificantBitFirst" | "LSBF" => BitOrder::LeastSignificantBitFirst,
            _ => {
                let msg = alloc::format!("Invalid bitOrder property: {}", bit_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let text_bidi = self.resolve_property(parent, "textBidi", "no");
        if text_bidi == "yes" {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: textBidi='yes' is not supported",
            ));
        }

        let floating = self.resolve_property(parent, "floating", "no");
        if floating == "yes" {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: floating='yes' is not supported",
            ));
        }

        let default_lk = if self.get_property("length").is_some() {
            "explicit"
        } else {
            "implicit"
        };
        let lk_str = self.resolve_property(parent, "lengthKind", default_lk);
        let length_kind = match lk_str {
            "explicit" => LengthKind::Explicit,
            "implicit" => LengthKind::Implicit,
            "prefixed" => LengthKind::Prefixed,
            "expression" => LengthKind::Expression,
            "delimited" => LengthKind::Delimited,
            "pattern" => LengthKind::Pattern,
            "endOfParent" => LengthKind::EndOfParent,
            _ => {
                let msg = alloc::format!("Invalid lengthKind property: {}", lk_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        if length_kind == LengthKind::Pattern {
            let lp_str = self
                .get_property("lengthPattern")
                .or_else(|| parent.and_then(|p| p.get_property("lengthPattern")))
                .unwrap_or("");
            if lp_str.is_empty() {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "dfdl:lengthPattern property missing when lengthKind='pattern'",
                ));
            }
            if !is_valid_dfdl_regex(lp_str) {
                let msg = alloc::format!(
                    "Schema Definition Error: InvalidRegex: '{}' is not a valid regular expression",
                    lp_str
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let len_str = self
            .get_property("length")
            .or_else(|| parent.and_then(|p| p.get_property("length")));
        let (length, length_expr) = if let Some(s) = len_str {
            if s.starts_with('{') && s.ends_with('}') {
                (None, Some(s.to_string()))
            } else if let Ok(val) = s.parse::<usize>() {
                (Some(val), None)
            } else {
                (None, Some(s.to_string()))
            }
        } else {
            (None, None)
        };

        let lu_str = self.resolve_property(parent, "lengthUnits", "bytes");
        let length_units = match lu_str {
            "bits" => crate::schema::ir::LengthUnits::Bits,
            "characters" => crate::schema::ir::LengthUnits::Characters,
            _ => crate::schema::ir::LengthUnits::Bytes,
        };

        let au_str = self.resolve_property(parent, "alignmentUnits", "bytes");
        let alignment_units = match au_str {
            "bytes" => crate::schema::ir::AlignmentUnits::Bytes,
            "bits" => crate::schema::ir::AlignmentUnits::Bits,
            _ => {
                let msg = alloc::format!("Invalid alignmentUnits property: {}", au_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let align_str = self.resolve_property(parent, "alignment", "1");
        let alignment = if align_str == "implicit" {
            match alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => 1,
                crate::schema::ir::AlignmentUnits::Bits => 8,
            }
        } else {
            align_str.parse::<usize>().unwrap_or(1)
        };

        let ak_str = self.resolve_property(parent, "alignmentKind", "automatic");
        let alignment_kind = match ak_str {
            "manual" => crate::schema::ir::AlignmentKind::Manual,
            _ => crate::schema::ir::AlignmentKind::Automatic,
        };

        let encoding = String::from(self.resolve_property(parent, "encoding", "UTF-8"));

        let separator = self.get_property("separator").map(String::from);

        let sep_pos_str = self.resolve_property(parent, "separatorPosition", "infix");
        let separator_position = match sep_pos_str {
            "infix" => SeparatorPosition::Infix,
            "prefix" => SeparatorPosition::Prefix,
            "postfix" => SeparatorPosition::Postfix,
            _ => {
                let msg = alloc::format!("Invalid separatorPosition property: {}", sep_pos_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let sep_supp_str = self.resolve_property(parent, "separatorSuppressionPolicy", "never");
        let separator_suppression_policy = match sep_supp_str {
            "never" => SeparatorSuppressionPolicy::Never,
            "trailingEmpty" => SeparatorSuppressionPolicy::TrailingEmpty,
            "trailingEmptyStrict" => SeparatorSuppressionPolicy::TrailingEmptyStrict,
            "anyEmpty" => SeparatorSuppressionPolicy::AnyEmpty,
            _ => {
                let msg = alloc::format!(
                    "Invalid separatorSuppressionPolicy property: {}",
                    sep_supp_str
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let seq_kind_str = self.resolve_property(parent, "sequenceKind", "ordered");
        let sequence_kind = match seq_kind_str {
            "ordered" => SequenceKind::Ordered,
            "unordered" => SequenceKind::Unordered,
            _ => {
                let msg = alloc::format!("Invalid sequenceKind property: {}", seq_kind_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let initiator = self.get_property("initiator").map(String::from);
        let terminator = self.get_property("terminator").map(String::from);

        let ock_str = self.resolve_property(parent, "occursCountKind", "implicit");
        let occurs_count_kind = match ock_str {
            "fixed" => OccursCountKind::Fixed,
            "implicit" => OccursCountKind::Implicit,
            "parsed" => OccursCountKind::Parsed,
            "expression" => OccursCountKind::Expression,
            "stopValue" => OccursCountKind::StopValue,
            _ => OccursCountKind::Implicit,
        };

        let occurs_count_expr = self.get_property("occursCount").map(String::from);

        let discriminator = self.get_property("discriminator").map(String::from);
        let discriminator_test_kind = match self.get_property("discriminatorTestKind") {
            Some("pattern") => crate::schema::ir::TestKind::Pattern,
            _ => crate::schema::ir::TestKind::Expression,
        };
        let discriminator_message = self.get_property("discriminatorMessage").map(String::from);

        let nk_str = self.resolve_property(parent, "nilKind", "literalValue");
        let nil_kind = match nk_str {
            "literalValue" => NilKind::LiteralValue,
            "literalCharacter" => NilKind::LiteralCharacter,
            "logicalValue" => NilKind::LogicalValue,
            _ => NilKind::LiteralValue,
        };

        let nil_value = self
            .get_property("nilValue")
            .or_else(|| parent.and_then(|p| p.get_property("nilValue")))
            .map(String::from);

        let is_nillable = self.get_property("nillable") == Some("true");

        if is_nillable
            && (nil_kind == NilKind::LiteralValue || nil_kind == NilKind::LiteralCharacter)
            && nil_value.as_deref() == Some("")
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: nilValue cannot be empty string (DFDL-06-036R). When empty string is desired, %ES; must be used.",
            ));
        }

        let nvdp_str = self.resolve_property(parent, "nilValueDelimiterPolicy", "both");
        let nil_value_delimiter_policy = match nvdp_str {
            "both" => crate::schema::ir::NilValueDelimiterPolicy::Both,
            "initiator" => crate::schema::ir::NilValueDelimiterPolicy::Initiator,
            "terminator" => crate::schema::ir::NilValueDelimiterPolicy::Terminator,
            "none" => crate::schema::ir::NilValueDelimiterPolicy::None,
            _ => crate::schema::ir::NilValueDelimiterPolicy::Both,
        };

        let leading_skip_str = self.get_property("leadingSkip").unwrap_or("0");
        let leading_skip = if leading_skip_str.starts_with('-') {
            let msg = alloc::format!("Invalid leadingSkip property: {}", leading_skip_str);
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        } else {
            leading_skip_str.parse::<usize>().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Invalid leadingSkip property: {}", leading_skip_str),
                )
            })?
        };

        let trailing_skip_str = self.get_property("trailingSkip").unwrap_or("0");
        let trailing_skip = if trailing_skip_str.starts_with('-') {
            let msg = alloc::format!("Invalid trailingSkip property: {}", trailing_skip_str);
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        } else {
            trailing_skip_str.parse::<usize>().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Invalid trailingSkip property: {}", trailing_skip_str),
                )
            })?
        };

        let base_str = self.resolve_property(parent, "textStandardBase", "10");
        let text_standard_base = match base_str {
            "2" => 2,
            "8" => 8,
            "10" => 10,
            "16" => 16,
            _ => {
                let msg = alloc::format!("Invalid textStandardBase property: {}", base_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let ttk_str = self.resolve_property(parent, "textTrimKind", "none");
        let text_trim_kind = match ttk_str {
            "none" => crate::schema::ir::TextTrimKind::None,
            "head" => crate::schema::ir::TextTrimKind::Head,
            "tail" => crate::schema::ir::TextTrimKind::Tail,
            "both" | "padChar" => crate::schema::ir::TextTrimKind::Both,
            _ => {
                let msg = alloc::format!("Invalid textTrimKind property: {}", ttk_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let parse_justification = |prop_name: &str, val_str: &str| match val_str {
            "left" => Ok(crate::schema::ir::TextJustification::Left),
            "right" => Ok(crate::schema::ir::TextJustification::Right),
            "center" => Ok(crate::schema::ir::TextJustification::Center),
            "none" => Ok(crate::schema::ir::TextJustification::None),
            _ => {
                let msg = alloc::format!("Invalid {} property: {}", prop_name, val_str);
                Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
            }
        };

        let tsj_str = self.resolve_property(parent, "textStringJustification", "left");
        let text_string_justification = parse_justification("textStringJustification", tsj_str)?;

        let tnj_str = self.resolve_property(parent, "textNumberJustification", "right");
        let text_number_justification = parse_justification("textNumberJustification", tnj_str)?;

        let tbj_str = self.resolve_property(parent, "textBooleanJustification", "left");
        let text_boolean_justification = parse_justification("textBooleanJustification", tbj_str)?;

        let tcj_str = self.resolve_property(parent, "textCalendarJustification", "left");
        let text_calendar_justification = parse_justification("textCalendarJustification", tcj_str)?;

        let tnrm_str = self.resolve_property(parent, "textNumberRoundingMode", "roundHalfEven");
        let text_number_rounding_mode = match tnrm_str {
            "roundCeiling" => crate::schema::ir::TextNumberRoundingMode::RoundCeiling,
            "roundFloor" => crate::schema::ir::TextNumberRoundingMode::RoundFloor,
            "roundDown" => crate::schema::ir::TextNumberRoundingMode::RoundDown,
            "roundUp" => crate::schema::ir::TextNumberRoundingMode::RoundUp,
            "roundHalfEven" => crate::schema::ir::TextNumberRoundingMode::RoundHalfEven,
            "roundHalfDown" => crate::schema::ir::TextNumberRoundingMode::RoundHalfDown,
            "roundHalfUp" => crate::schema::ir::TextNumberRoundingMode::RoundHalfUp,
            "roundUnnecessary" => crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary,
            _ => {
                let msg = alloc::format!("Invalid textNumberRoundingMode property: {}", tnrm_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };
        let text_number_rounding_explicit =
            self.resolve_property(parent, "textNumberRounding", "pattern") == "explicit";
        let text_number_rounding_increment = if text_number_rounding_explicit {
            let inc = self.resolve_property(parent, "textNumberRoundingIncrement", "").trim();
            if inc.is_empty() {
                None
            } else {
                Some(inc.to_string())
            }
        } else {
            None
        };

        let text_pad_char_raw = self
            .get_property("textStringPadCharacter")
            .or_else(|| self.get_property("textPadChar"))
            .or_else(|| parent.and_then(|p| p.get_property("textStringPadCharacter")))
            .or_else(|| parent.and_then(|p| p.get_property("textPadChar")))
            .unwrap_or(" ");
        let text_pad_char = decode_dfdl_character_entities(text_pad_char_raw);

        let fill_byte_raw = self.get_property("fillByte");
        let fill_byte_defined = fill_byte_raw.is_some();
        let fill_byte = if let Some(fb) = fill_byte_raw {
            if fb.is_empty() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: fillByte property cannot be empty",
                ));
            }
            if fb.starts_with('%') && fb.ends_with(';') {
                let inner = &fb[1..fb.len().saturating_sub(1)];
                if let Some(hex_val) = inner
                    .strip_prefix("#r")
                    .or_else(|| inner.strip_prefix("#x"))
                {
                    u8::from_str_radix(hex_val, 16).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Invalid byte entity in fillByte: must be 1 character or byte",
                        )
                    })?
                } else if let Some(dec_val) = inner.strip_prefix("#d") {
                    dec_val.parse::<u8>().map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Invalid decimal entity in fillByte: must be 1 character or byte",
                        )
                    })?
                } else {
                    match fb {
                        "%NUL;" => 0,
                        "%SP;" => b' ',
                        "%HT;" => b'\t',
                        "%LF;" => b'\n',
                        "%CR;" => b'\r',
                        _ => {
                            return Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                &alloc::format!(
                                    "Schema Definition Error: Unsupported entity '{}' for fillByte: must be 1 character or byte",
                                    fb
                                ),
                            ));
                        }
                    }
                }
            } else if fb.chars().count() == 1 {
                let Some(ch) = fb.chars().next() else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte '{}' must be 1 character or byte",
                            fb
                        ),
                    ));
                };
                let enc = self.get_property("encoding").unwrap_or("UTF-8");
                let enc_upper = enc.to_ascii_uppercase();
                if enc_upper.contains("6-BIT")
                    || enc_upper.contains("5-BIT")
                    || enc_upper.contains("7-BIT")
                    || enc_upper.contains("BIT-PACKED")
                {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte property requires a single-byte character for encoding '{}', but '{}' is not a byte-sized encoding",
                            enc, enc
                        ),
                    ));
                }
                if enc.eq_ignore_ascii_case("UTF-8") && ch.len_utf8() > 1 {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte must be a single-byte character for encoding '{}', but '{}' takes {} bytes",
                            enc, ch, ch.len_utf8()
                        ),
                    ));
                }
                ch as u8
            } else {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: fillByte '{}' must be 1 character or byte",
                        fb
                    ),
                ));
            }
        } else {
            0
        };

        let input_value_calc = self.get_property("inputValueCalc").map(String::from);
        let output_value_calc = self.get_property("outputValueCalc").map(String::from);

        let hidden_group_ref = self
            .get_property("hiddenGroupRef")
            .or_else(|| parent.and_then(|p| p.get_property("hiddenGroupRef")))
            .map(String::from);

        let parent_hidden = parent.is_some_and(|p| {
            p.get_property("hiddenGroupRef").is_some()
                || p.get_property("hidden") == Some("true")
                || p.get_property("is_hidden_group") == Some("true")
                || p.get_property("is_hidden") == Some("true")
        });
        let is_hidden = hidden_group_ref.is_some()
            || parent_hidden
            || self.get_property("hidden") == Some("true")
            || self.get_property("is_hidden_group") == Some("true")
            || self.get_property("is_hidden") == Some("true");

        let mut asserts = self.asserts.clone();
        if let Some(test_expr) = self.get_property("assert") {
            let test_kind = if self.get_property("testKind") == Some("pattern") {
                crate::schema::ir::TestKind::Pattern
            } else {
                crate::schema::ir::TestKind::Expression
            };
            let _ = try_push(
                &mut asserts,
                CompiledAssert {
                    test_kind,
                    test_expr: test_expr.to_string(),
                    message: None,
                    failure_type: crate::schema::ir::FailureType::ProcessingError,
                },
            );
        }

        if self.get_property("discriminator").is_some() && !asserts.is_empty() {
            let msg = "Schema Definition Error: A component cannot have both a discriminator and an assert statement.";
            return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
        }

        for assert_item in &asserts {
            if assert_item.test_kind == crate::schema::ir::TestKind::Pattern
                && !is_valid_dfdl_regex(&assert_item.test_expr)
            {
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid regex pattern in dfdl:assert testPattern: '{}'",
                    assert_item.test_expr
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let prefix_length_type = self
            .get_property("prefixLengthType")
            .or_else(|| parent.and_then(|p| p.get_property("prefixLengthType")))
            .map(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc);

        let prefix_includes_prefix_length = self
            .get_property("prefixIncludesPrefixLength")
            .or_else(|| parent.and_then(|p| p.get_property("prefixIncludesPrefixLength")))
            .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"));

        let tncp_str = self.resolve_property(parent, "textNumberCheckPolicy", "lax");
        let text_number_check_policy = match tncp_str {
            "strict" => crate::schema::ir::TextNumberCheckPolicy::Strict,
            _ => crate::schema::ir::TextNumberCheckPolicy::Lax,
        };

        let text_number_pattern = self
            .get_property("textNumberPattern")
            .or_else(|| parent.and_then(|p| p.get_property("textNumberPattern")))
            .map(String::from);

        let text_decimal_sep_opt = self
            .get_property("textStandardDecimalSeparator")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardDecimalSeparator")));
        let text_standard_decimal_separator = match text_decimal_sep_opt {
            Some(sep) if !sep.is_empty() => {
                if sep.starts_with('{') && sep.ends_with('}') {
                    String::from(sep)
                } else {
                    let decoded = decode_dfdl_character_entities(sep);
                    if decoded.chars().count() != 1 {
                        let msg = alloc::format!(
                            "Schema Definition Error: Length of string must be exactly 1 character (cannot have more than one character) for textStandardDecimalSeparator, got '{}'",
                            sep
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    decoded
                }
            }
            Some(_) => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardDecimalSeparator",
                ));
            }
            None => {
                // If not explicitly set, default for resolution while permitting empty checks
                String::from(".")
            }
        };

        if let Some(ref pat) = text_number_pattern {
            if pat.starts_with(';') {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Invalid textNumberPattern: A positive part is required in the pattern",
                ));
            }
            if (pat.contains('E') || pat.contains('e')) && pat.contains(',') {
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid textNumberPattern: Cannot have grouping separator in scientific notation '{}'",
                    pat
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if pat.ends_with('*') {
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid textNumberPattern: Malformed pattern: pad character missing after '*' in \"{}\"",
                    pat
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if pat.contains('V') {
                for subpat in pat.split(';') {
                    if let Some(idx) = subpat.find('V') {
                        let after_v = &subpat[idx.saturating_add(1)..];
                        if after_v.contains('#') {
                            let msg = alloc::format!(
                                "Schema Definition Error: Invalid textNumberPattern '{}': In textNumberPattern with 'V', pattern must consist of '#', then digits 0-9 then 'V' then digits 0-9 (the portion after 'V' must contain only digits 0-9)",
                                pat
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
            if pat.contains(';') && pat.contains("text") {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Property textNumberPattern must contain only digits 0-9",
                ));
            }
        }

        let text_number_rep = self.resolve_property(parent, "textNumberRep", "standard");
        if text_number_rep == "zoned" {
            if let Some(ref pat) = text_number_pattern {
                if pat.contains('@')
                    || pat.contains('E')
                    || pat.contains('e')
                    || pat.contains(';')
                    || pat.chars().filter(|&c| c == '+').count() > 1
                {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: textNumberPattern for zoned number rep cannot contain '@', 'E', 'e', ';', or multiple '+'",
                    ));
                }
            }
        }

        let text_grp_sep_opt = self
            .get_property("textStandardGroupingSeparator")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardGroupingSeparator")));
        if let Some(ref pat) = text_number_pattern {
            if pat.contains(',') && text_grp_sep_opt.is_none() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Property textStandardGroupingSeparator is not defined",
                ));
            }
        }
        let text_standard_grouping_separator = match text_grp_sep_opt {
            Some(sep) if !sep.is_empty() => {
                if sep.starts_with('{') && sep.ends_with('}') {
                    String::from(sep)
                } else {
                    let decoded = decode_dfdl_character_entities(sep);
                    if decoded.chars().count() != 1 {
                        let msg = alloc::format!(
                            "Schema Definition Error: Length of string must be exactly 1 character (cannot have more than one character) for textStandardGroupingSeparator, got '{}'",
                            sep
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    decoded
                }
            }
            Some(_) => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardGroupingSeparator",
                ));
            }
            None => String::from(","),
        };

        let text_exp_sep_opt = self
            .get_property("textStandardExponentRep")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")));
        let text_standard_exponent_rep = match text_exp_sep_opt {
            Some(exp) => decode_dfdl_character_entities(exp),
            None => String::from("E"),
        };

        if text_number_rep == "standard" {
            let has_custom_num_props = text_decimal_sep_opt.is_some()
                || text_grp_sep_opt.is_some()
                || text_exp_sep_opt.is_some();
            if has_custom_num_props {
                let nan_rep = self
                    .get_property("textStandardNaNRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardNaNRep")))
                    .unwrap_or("");
                let inf_rep = self
                    .get_property("textStandardInfinityRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardInfinityRep")))
                    .unwrap_or("");
                let zero_rep = self
                    .get_property("textStandardZeroRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardZeroRep")))
                    .unwrap_or("");

                let mut vals = alloc::vec::Vec::new();
                if !text_standard_decimal_separator.is_empty() {
                    vals.push((
                        "textStandardDecimalSeparator",
                        text_standard_decimal_separator.as_str(),
                    ));
                }
                if !text_standard_grouping_separator.is_empty() {
                    vals.push((
                        "textStandardGroupingSeparator",
                        text_standard_grouping_separator.as_str(),
                    ));
                }
                if !text_standard_exponent_rep.is_empty() {
                    vals.push((
                        "textStandardExponentRep",
                        text_standard_exponent_rep.as_str(),
                    ));
                }
                if !nan_rep.is_empty() {
                    vals.push(("textStandardNaNRep", nan_rep));
                }
                if !inf_rep.is_empty() {
                    vals.push(("textStandardInfinityRep", inf_rep));
                }
                if !zero_rep.is_empty() {
                    vals.push(("textStandardZeroRep", zero_rep));
                }

                for (idx1, &(name1, val1)) in vals.iter().enumerate() {
                    for &(name2, val2) in vals.iter().skip(idx1.saturating_add(1)) {
                        if val1 == val2 {
                            let msg = alloc::format!(
                                "Schema Definition Error: Non-distinct property values for textStandardDecimalSeparator, textStandardGroupingSeparator, textStandardExponentRep, textStandardInfinityRep, textStandardNaNRep, textStandardZeroRep ('{}' and '{}' both have value '{}')",
                                name1, name2, val1
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
        }

        let facets = crate::schema::ir::SimpleTypeFacets {
            min_inclusive: self.get_property("minInclusive").map(String::from),
            max_inclusive: self.get_property("maxInclusive").map(String::from),
            min_exclusive: self.get_property("minExclusive").map(String::from),
            max_exclusive: self.get_property("maxExclusive").map(String::from),
            pattern: self.get_property("pattern").map(String::from),
            enumeration: self
                .bindings
                .iter()
                .filter(|b| b.key == "enumeration")
                .map(|b| b.value.clone())
                .collect(),
            min_length: self.get_property("minLength").and_then(|s| s.parse().ok()),
            max_length: self.get_property("maxLength").and_then(|s| s.parse().ok()),
            length: self
                .get_property("xsdLength")
                .or_else(|| self.get_property("facetLength"))
                .and_then(|s| s.parse().ok()),
            total_digits: self
                .get_property("totalDigits")
                .and_then(|s| s.parse().ok()),
            fraction_digits: self
                .get_property("fractionDigits")
                .and_then(|s| s.parse().ok()),
            rep_values: self
                .bindings
                .iter()
                .filter_map(|b| {
                    b.key.strip_prefix("repValue:").map(|k| (String::from(k), b.value.clone()))
                })
                .collect(),
        };

        let length_pattern = self
            .get_property("lengthPattern")
            .or_else(|| parent.and_then(|p| p.get_property("lengthPattern")))
            .map(String::from);

        let binary_number_rep = match self.resolve_property(parent, "binaryNumberRep", "binary") {
            "packed" => crate::schema::ir::BinaryNumberRep::Packed,
            "bcd" => crate::schema::ir::BinaryNumberRep::Bcd,
            "ibm4690Packed" => crate::schema::ir::BinaryNumberRep::Ibm4690Packed,
            _ => crate::schema::ir::BinaryNumberRep::Binary,
        };

        let binary_calendar_rep = match self.resolve_property(parent, "binaryCalendarRep", "bcd") {
            "binarySeconds" => crate::schema::ir::BinaryCalendarRep::BinarySeconds,
            "binaryMilliseconds" => crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds,
            "packed" => crate::schema::ir::BinaryCalendarRep::Packed,
            "ibm4690Packed" => crate::schema::ir::BinaryCalendarRep::Ibm4690Packed,
            _ => crate::schema::ir::BinaryCalendarRep::Bcd,
        };



        if let Some(l_str) = self.get_property("length") {
            if let Ok(val) = l_str.parse::<i64>() {
                if val < 0 {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: dfdl:length cannot be negative: {}",
                        val
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let initiated_content_str = self.resolve_property(parent, "initiatedContent", "no");
        let initiated_content = matches!(initiated_content_str, "yes");

        let truncate_str = self.resolve_property(parent, "truncateSpecifiedLengthString", "no");
        let truncate_specified_length_string = matches!(truncate_str, "yes");

        let encoding_error_policy_error =
            self.resolve_property(parent, "encodingErrorPolicy", "replace") == "error";
        let encoding_error_policy_defined = self.get_property("encodingErrorPolicy").is_some()
            || parent.is_some_and(|p| p.get_property("encodingErrorPolicy").is_some());

        let binary_decimal_virtual_point = self
            .resolve_property(parent, "binaryDecimalVirtualPoint", "0")
            .parse::<i32>()
            .unwrap_or(0);

        let cal_pat_kind = self.resolve_property(parent, "calendarPatternKind", "implicit");
        let calendar_pattern = if cal_pat_kind == "explicit" {
            self.get_property("calendarPattern")
                .or_else(|| parent.and_then(|p| p.get_property("calendarPattern")))
                .map(String::from)
        } else {
            None
        };
        let calendar_language = self
            .get_property("calendarLanguage")
            .or_else(|| parent.and_then(|p| p.get_property("calendarLanguage")))
            .map(String::from);
        if let Some(ref lang) = calendar_language {
            let trimmed = lang.trim();
            if !trimmed.starts_with('{') {
                crate::kernel::parser::calendar::validate_calendar_language_syntax(trimmed)?;
            }
        }
        let calendar_time_zone = self
            .get_property("calendarTimeZone")
            .or_else(|| parent.and_then(|p| p.get_property("calendarTimeZone")))
            .map(String::from);
        let empty_element_parse_policy =
            match self.resolve_property(parent, "emptyElementParsePolicy", "treatAsEmpty") {
                "treatAsAbsent" => crate::schema::ir::EmptyElementParsePolicy::TreatAsAbsent,
                _ => crate::schema::ir::EmptyElementParsePolicy::TreatAsEmpty,
            };

        let choice_length_kind = match self.resolve_property(parent, "choiceLengthKind", "implicit") {
            "explicit" => LengthKind::Explicit,
            _ => LengthKind::Implicit,
        };
        let choice_length = self
            .get_property("choiceLength")
            .or_else(|| parent.and_then(|p| p.get_property("choiceLength")))
            .and_then(|s| s.parse::<usize>().ok());

        let choice_dispatch_key = self.get_property("choiceDispatchKey").map(String::from);
        let choice_branch_key = self.get_property("choiceBranchKey").map(String::from);
        let choice_branch_key_ranges = self.get_property("choiceBranchKeyRanges").map(String::from);

        let binary_calendar_epoch = self
            .get_property("binaryCalendarEpoch")
            .or_else(|| parent.and_then(|p| p.get_property("binaryCalendarEpoch")))
            .map(String::from);

        if let Some(ref epoch_str) = binary_calendar_epoch {
            if !is_valid_xs_date_time(epoch_str) {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Invalid binaryCalendarEpoch '{}'; must be a valid xs:dateTime",
                        epoch_str
                    ),
                ));
            }
        }



        let pup_str = self
            .get_property("parseUnparsePolicy")
            .or_else(|| self.get_property("dfdlx:parseUnparsePolicy"))
            .or_else(|| self.get_property("daf:parseUnparsePolicy"))
            .or_else(|| {
                parent.and_then(|p| {
                    p.get_property("parseUnparsePolicy")
                        .or_else(|| p.get_property("dfdlx:parseUnparsePolicy"))
                        .or_else(|| p.get_property("daf:parseUnparsePolicy"))
                })
            })
            .unwrap_or("both");
        let parse_unparse_policy = match pup_str {
            "both" => ParseUnparsePolicy::Both,
            "parseOnly" => ParseUnparsePolicy::ParseOnly,
            "unparseOnly" => ParseUnparsePolicy::UnparseOnly,
            _ => {
                let msg = alloc::format!("Schema Definition Error: Invalid parseUnparsePolicy '{}'", pup_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let output_new_line = self
            .get_property("outputNewLine")
            .or_else(|| parent.and_then(|p| p.get_property("outputNewLine")))
            .map(String::from);

        if let Some(ref onl) = output_new_line {
            if onl.is_empty() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: dfdl:outputNewLine cannot be empty string",
                ));
            }
        }

        if let Some(p) = parent {
            let parent_pup = p
                .get_property("parseUnparsePolicy")
                .or_else(|| p.get_property("dfdlx:parseUnparsePolicy"))
                .or_else(|| p.get_property("daf:parseUnparsePolicy"));
            if let Some(p_policy) = parent_pup {
                if (p_policy == "both" && (pup_str == "parseOnly" || pup_str == "unparseOnly"))
                    || (p_policy == "parseOnly" && pup_str == "unparseOnly")
                    || (p_policy == "unparseOnly" && pup_str == "parseOnly")
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: Incompatible parseUnparsePolicy: parent is '{}' and child is '{}'",
                        p_policy, pup_str
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let effective_ns: &[(String, String)] = if self.in_scope_namespaces.is_empty() {
            parent.map(|p| p.in_scope_namespaces.as_slice()).unwrap_or(&[])
        } else {
            &self.in_scope_namespaces
        };
        if !effective_ns.is_empty() {
            if let Some(ref expr) = input_value_calc {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = output_value_calc {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = discriminator {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = length_expr {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = occurs_count_expr {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = byte_order_expr {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = choice_dispatch_key {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            for a in &asserts {
                super::validate_expression_namespaces(&a.test_expr, effective_ns)?;
            }
            for (_, expr) in &self.set_variables {
                super::validate_expression_namespaces(expr, effective_ns)?;
            }
            for (_, opt_expr) in &self.new_variable_instances {
                if let Some(expr) = opt_expr {
                    super::validate_expression_namespaces(expr, effective_ns)?;
                }
            }
        }

        let byte_order_prop = if let Some(ref bo_expr) = byte_order_expr {
            DfdlProp::parse_with(bo_expr, parse_byte_order)?
        } else {
            DfdlProp::constant(byte_order)
        };
        let encoding_prop = DfdlProp::parse_str(&encoding)?;
        let separator_prop = separator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let initiator_prop = initiator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let terminator_prop = terminator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let length_prop = if let Some(s) = len_str {
            if let Ok(p) = DfdlProp::parse_with(s, |raw| {
                raw.trim().parse::<usize>().map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Invalid length constant '{}': {}", raw, e),
                    )
                })
            }) {
                Some(p)
            } else if length_kind == LengthKind::Explicit {
                Some(DfdlProp::parse_with(s, |raw| {
                    raw.trim().parse::<usize>().map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Invalid length constant '{}': {}", raw, e),
                        )
                    })
                })?)
            } else {
                None
            }
        } else {
            None
        };
        let occurs_count_prop = if let Some(ref oce) = occurs_count_expr {
            DfdlProp::parse_with(oce, |raw| {
                raw.trim().parse::<usize>().map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Invalid occursCount constant '{}': {}", raw, e),
                    )
                })
            }).ok()
        } else {
            None
        };
        let nil_value_prop = nil_value.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let text_standard_decimal_separator_prop = DfdlProp::parse_str(&text_standard_decimal_separator)?;
        let text_standard_grouping_separator_prop = DfdlProp::parse_str(&text_standard_grouping_separator)?;
        let calendar_pattern_prop = calendar_pattern.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let calendar_language_prop = calendar_language.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let calendar_time_zone_prop = calendar_time_zone.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let output_new_line_prop = output_new_line.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let text_standard_exponent_rep_prop = self
            .get_property("textStandardExponentRep")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_with(s, |raw| Ok(decode_dfdl_character_entities(raw))).ok()
                }
            });
        let text_boolean_true_rep_prop = self
            .get_property("textBooleanTrueRep")
            .or_else(|| parent.and_then(|p| p.get_property("textBooleanTrueRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_str(s).ok()
                }
            });
        let text_boolean_false_rep_prop = self
            .get_property("textBooleanFalseRep")
            .or_else(|| parent.and_then(|p| p.get_property("textBooleanFalseRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_str(s).ok()
                }
            });

        Ok(ResolvedProperties {
            representation: rep,
            byte_order,
            byte_order_expr,
            byte_order_prop,
            bit_order,
            length_kind,
            length,
            length_expr,
            length_prop,
            prefix_length_type,
            prefix_includes_prefix_length,
            length_pattern,
            length_units,
            alignment,
            alignment_units,
            alignment_kind,
            leading_skip,
            trailing_skip,
            encoding,
            encoding_prop,
            separator,
            separator_prop,
            separator_position,
            separator_suppression_policy,
            sequence_kind,
            initiator,
            initiator_prop,
            initiated_content,
            choice_length_kind,
            choice_length,
            terminator,
            terminator_prop,
            occurs_count_kind,
            occurs_count_expr,
            occurs_count_prop,
            discriminator,
            discriminator_test_kind,
            discriminator_message,
            nil_kind,
            nil_value,
            nil_value_prop,
            nil_value_delimiter_policy,
            text_trim_kind,
            text_pad_char,
            fill_byte,
            fill_byte_defined,
            fill_byte_raw: fill_byte_raw.map(String::from),
            input_value_calc,
            output_value_calc,
            asserts,
            hidden_group_ref,
            is_hidden,
            set_variables: self.set_variables.clone(),
            new_variable_instances: self.new_variable_instances.clone(),
            text_number_check_policy,
            text_number_pattern,
            text_standard_decimal_separator,
            text_standard_decimal_separator_prop,
            text_standard_grouping_separator,
            text_standard_grouping_separator_prop,
            truncate_specified_length_string,
            text_string_justification,
            text_number_justification,
            text_boolean_justification,
            text_calendar_justification,
            text_number_rounding_mode,
            text_number_rounding_explicit,
            text_number_rounding_increment,
            encoding_error_policy_error,
            encoding_error_policy_defined,
            binary_decimal_virtual_point,
            calendar_pattern,
            calendar_pattern_prop,
            calendar_language,
            calendar_language_prop,
            calendar_time_zone,
            calendar_time_zone_prop,
            binary_number_rep,
            binary_packed_sign_codes: self
                .get_property("binaryPackedSignCodes")
                .or_else(|| parent.and_then(|p| p.get_property("binaryPackedSignCodes")))
                .map(String::from),
            binary_calendar_rep,
            text_standard_base,
            facets,
            choice_dispatch_key,
            choice_branch_key,
            choice_branch_key_ranges,
            empty_element_parse_policy,
            binary_calendar_epoch,
            parse_unparse_policy,
            output_new_line,
            output_new_line_prop,
            text_standard_nan_rep: self
                .get_property("textStandardNaNRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardNaNRep")))
                .map(decode_dfdl_character_entities),
            text_standard_infinity_rep: self
                .get_property("textStandardInfinityRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardInfinityRep")))
                .map(decode_dfdl_character_entities),
            text_standard_zero_rep: self
                .get_property("textStandardZeroRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardZeroRep")))
                .map(String::from),
            text_number_pad_character: self
                .get_property("textNumberPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textNumberPadCharacter")))
                .map(decode_dfdl_character_entities),
            text_calendar_pad_character: self
                .get_property("textCalendarPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textCalendarPadCharacter")))
                .map(decode_dfdl_character_entities),
            text_standard_exponent_rep: self
                .get_property("textStandardExponentRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")))
                .map(decode_dfdl_character_entities),
            text_standard_exponent_rep_prop,
            ignore_case: self
                .get_property("ignoreCase")
                .or_else(|| parent.and_then(|p| p.get_property("ignoreCase")))
                .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")),
            text_boolean_true_rep: self
                .get_property("textBooleanTrueRep")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanTrueRep")))
                .map(String::from),
            text_boolean_true_rep_prop,
            text_boolean_false_rep: self
                .get_property("textBooleanFalseRep")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanFalseRep")))
                .map(String::from),
            text_boolean_false_rep_prop,
            text_boolean_pad_character: self
                .get_property("textBooleanPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanPadCharacter")))
                .map(decode_dfdl_character_entities),
            binary_boolean_true_rep: self
                .get_property("binaryBooleanTrueRep")
                .or_else(|| parent.and_then(|p| p.get_property("binaryBooleanTrueRep")))
                .map_or(crate::schema::ir::BinaryBooleanRep::NotSpecified, |s| {
                    if s.is_empty() {
                        crate::schema::ir::BinaryBooleanRep::Empty
                    } else if let Ok(v) = s.parse::<i64>() {
                        crate::schema::ir::BinaryBooleanRep::Value(v)
                    } else {
                        crate::schema::ir::BinaryBooleanRep::NotSpecified
                    }
                }),
            binary_boolean_false_rep: self
                .get_property("binaryBooleanFalseRep")
                .or_else(|| parent.and_then(|p| p.get_property("binaryBooleanFalseRep")))
                .map_or(crate::schema::ir::BinaryBooleanRep::NotSpecified, |s| {
                    if s.is_empty() {
                        crate::schema::ir::BinaryBooleanRep::Empty
                    } else if let Ok(v) = s.parse::<i64>() {
                        crate::schema::ir::BinaryBooleanRep::Value(v)
                    } else {
                        crate::schema::ir::BinaryBooleanRep::NotSpecified
                    }
                }),
            document_final_terminator_can_be_missing: !self
                .get_property("documentFinalTerminatorCanBeMissing")
                .or_else(|| parent.and_then(|p| p.get_property("documentFinalTerminatorCanBeMissing")))
                .is_some_and(|v| v.eq_ignore_ascii_case("no") || v.eq_ignore_ascii_case("false")),
            decimal_signed: !self
                .get_property("decimalSigned")
                .or_else(|| parent.and_then(|p| p.get_property("decimalSigned")))
                .is_some_and(|v| v.eq_ignore_ascii_case("no") || v.eq_ignore_ascii_case("false")),
            text_number_rep: match self.resolve_property(parent, "textNumberRep", "standard") {
                "zoned" => crate::schema::ir::TextNumberRep::Zoned,
                _ => crate::schema::ir::TextNumberRep::Standard,
            },
            text_zoned_sign_style: match self.resolve_property(parent, "textZonedSignStyle", "asciiStandard") {
                "asciiTranslatedEBCDIC" => crate::schema::ir::TextZonedSignStyle::AsciiTranslatedEbcdic,
                "asciiCARealiaModified" => crate::schema::ir::TextZonedSignStyle::AsciiCaRealiaModified,
                "asciiTandemModified" => crate::schema::ir::TextZonedSignStyle::AsciiTandemModified,
                _ => crate::schema::ir::TextZonedSignStyle::AsciiStandard,
            },
            calendar_check_policy: match self.resolve_property(parent, "calendarCheckPolicy", "lax") {
                "strict" => crate::schema::ir::CalendarCheckPolicy::Strict,
                _ => crate::schema::ir::CalendarCheckPolicy::Lax,
            },
            calendar_first_day_of_week: match self.resolve_property(parent, "calendarFirstDayOfWeek", "Sunday") {
                "Monday" => crate::schema::ir::CalendarFirstDayOfWeek::Monday,
                "Tuesday" => crate::schema::ir::CalendarFirstDayOfWeek::Tuesday,
                "Wednesday" => crate::schema::ir::CalendarFirstDayOfWeek::Wednesday,
                "Thursday" => crate::schema::ir::CalendarFirstDayOfWeek::Thursday,
                "Friday" => crate::schema::ir::CalendarFirstDayOfWeek::Friday,
                "Saturday" => crate::schema::ir::CalendarFirstDayOfWeek::Saturday,
                _ => crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
            },
            text_pad_kind: {
                let s = self
                    .get_property("textPadKind")
                    .or_else(|| parent.and_then(|p| p.get_property("textPadKind")))
                    .unwrap_or("none");
                match s {
                    "none" => crate::schema::ir::TextPadKind::None,
                    "padChar" => crate::schema::ir::TextPadKind::PadChar,
                    _ => {
                        let msg = alloc::format!("Invalid textPadKind property: {}", s);
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            },
            text_output_min_length: {
                let toml = self
                    .get_property("textOutputMinLength")
                    .or_else(|| parent.and_then(|p| p.get_property("textOutputMinLength")))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let facet_min = self
                    .get_property("minLength")
                    .or_else(|| self.get_property("length"))
                    .or_else(|| parent.and_then(|p| p.get_property("minLength").or_else(|| p.get_property("length"))))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                toml.max(facet_min)
            },
            escape_scheme: {
                let es_ref = self
                    .get_property("escapeSchemeRef")
                    .or_else(|| parent.and_then(|p| p.get_property("escapeSchemeRef")));
                if es_ref == Some("") {
                    None
                } else {
                    let escape_kind_str = self
                        .get_property("escapeKind")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeKind")));
                    let escape_char = self
                        .get_property("escapeCharacter")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeCharacter")));
                    let escape_block_start = self
                        .get_property("escapeBlockStart")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeBlockStart")));

                    if escape_kind_str.is_some() || escape_char.is_some() || escape_block_start.is_some() {
                        let escape_kind = match escape_kind_str.unwrap_or("escapeCharacter") {
                            "escapeBlock" => crate::schema::ir::EscapeKind::EscapeBlock,
                            _ => crate::schema::ir::EscapeKind::EscapeCharacter,
                        };
                        let escape_character = escape_char.map(decode_dfdl_character_entities);
                        let escape_escape_char_raw = self
                            .get_property("escapeEscapeCharacter")
                            .or_else(|| parent.and_then(|p| p.get_property("escapeEscapeCharacter")));
                        let escape_escape_character = escape_escape_char_raw
                            .map(decode_dfdl_character_entities)
                            .filter(|s| !s.is_empty());
                        let escape_block_start = escape_block_start.map(decode_dfdl_character_entities);
                        let escape_block_end = self
                            .get_property("escapeBlockEnd")
                            .or_else(|| parent.and_then(|p| p.get_property("escapeBlockEnd")))
                            .map(decode_dfdl_character_entities);
                        let extra_escaped_characters = self
                            .get_property("extraEscapedCharacters")
                            .or_else(|| parent.and_then(|p| p.get_property("extraEscapedCharacters")))
                            .map(|s| {
                                s.split_whitespace()
                                    .flat_map(|item| decode_dfdl_character_entities(item).chars().collect::<Vec<_>>())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let gen_block_str = self
                            .get_property("generateEscapeBlock")
                            .or_else(|| parent.and_then(|p| p.get_property("generateEscapeBlock")))
                            .unwrap_or("whenNeeded");
                        let generate_escape_block = match gen_block_str {
                            "always" => crate::schema::ir::GenerateEscapeBlock::Always,
                            _ => crate::schema::ir::GenerateEscapeBlock::WhenNeeded,
                        };
                        Some(crate::schema::ir::CompiledEscapeScheme {
                            escape_kind,
                            escape_character,
                            escape_escape_character,
                            escape_block_start,
                            escape_block_end,
                            extra_escaped_characters,
                            generate_escape_block,
                        })
                    } else {
                        None
                    }
                }
            },
            rep_type: self
                .get_property("repType")
                .or_else(|| self.get_property("dfdlx:repType"))
                .or_else(|| parent.and_then(|p| p.get_property("repType").or_else(|| p.get_property("dfdlx:repType"))))
                .map(String::from),
            rep_simple_type: None,
            layer: self
                .get_property("layer")
                .or_else(|| self.get_property("dfdlx:layer"))
                .or_else(|| self.get_property("layerTransform"))
                .or_else(|| self.get_property("dfdlx:layerTransform"))
                .or_else(|| self.get_property("daf:layerTransform"))
                .map(String::from),
            in_scope_namespaces: effective_ns.to_vec(),
            string_as_xml: self
                .get_property("runtimeProperties")
                .or_else(|| self.get_property("dfdlx:runtimeProperties"))
                .or_else(|| {
                    parent.and_then(|p| {
                        p.get_property("runtimeProperties")
                            .or_else(|| p.get_property("dfdlx:runtimeProperties"))
                    })
                })
                .map(|p| p.contains("stringAsXml=true"))
                .unwrap_or(false),
            empty_value_delimiter_policy: self
                .get_property("emptyValueDelimiterPolicy")
                .or_else(|| parent.and_then(|p| p.get_property("emptyValueDelimiterPolicy")))
                .map(|s| match s.trim().to_ascii_lowercase().as_str() {
                    "none" => crate::schema::ir::EmptyValueDelimiterPolicy::None,
                    "initiator" => crate::schema::ir::EmptyValueDelimiterPolicy::Initiator,
                    "terminator" => crate::schema::ir::EmptyValueDelimiterPolicy::Terminator,
                    _ => crate::schema::ir::EmptyValueDelimiterPolicy::Both,
                })
                .unwrap_or(crate::schema::ir::EmptyValueDelimiterPolicy::Both),
        })
    }
}

fn is_valid_xs_date_time(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (date_part, time_part) = match s.split_once('T') {
        Some(parts) => parts,
        None => return false,
    };
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return false;
    }
    let (Some(year_str), Some(month_str), Some(day_str)) =
        (date_parts.first(), date_parts.get(1), date_parts.get(2))
    else {
        return false;
    };
    if year_str.len() < 4 || !year_str.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if month_str.len() != 2
        || !month_str.chars().all(|c| c.is_ascii_digit())
        || month_str.parse::<u32>().map_or(true, |m| m == 0 || m > 12)
    {
        return false;
    }
    if day_str.len() != 2
        || !day_str.chars().all(|c| c.is_ascii_digit())
        || day_str.parse::<u32>().map_or(true, |d| d == 0 || d > 31)
    {
        return false;
    }

    let (time_no_tz, _tz) = if let Some(stripped) = time_part.strip_suffix('Z') {
        (stripped, Some("Z"))
    } else if let Some(idx) = time_part.rfind('+').or_else(|| time_part.rfind('-')) {
        let (t, tz) = time_part.split_at(idx);
        let tz_digits = tz.get(1..).unwrap_or("");
        let b2 = tz_digits.as_bytes().get(2).copied();
        let sub_0_2 = tz_digits.get(..2).unwrap_or("");
        let sub_3 = tz_digits.get(3..).unwrap_or("");
        if tz_digits.len() != 5
            || b2 != Some(b':')
            || !sub_0_2.chars().all(|c| c.is_ascii_digit())
            || !sub_3.chars().all(|c| c.is_ascii_digit())
        {
            return false;
        }
        (t, Some(tz))
    } else {
        (time_part, None)
    };

    let (base_time, frac) = time_no_tz
        .split_once('.')
        .map_or((time_no_tz, None), |(b, f)| (b, Some(f)));
    if let Some(f) = frac {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }

    let time_parts: Vec<&str> = base_time.split(':').collect();
    if time_parts.len() != 3 {
        return false;
    }
    for p in &time_parts {
        if p.len() != 2 || !p.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let (Some(h_str), Some(m_str), Some(s_str)) =
        (time_parts.first(), time_parts.get(1), time_parts.get(2))
    else {
        return false;
    };
    let h = match h_str.parse::<u32>() {
        Ok(v) if v <= 24 => v,
        _ => return false,
    };
    let m = match m_str.parse::<u32>() {
        Ok(v) if v <= 59 => v,
        _ => return false,
    };
    let s = match s_str.parse::<u32>() {
        Ok(v) if v <= 60 => v,
        _ => return false,
    };
    if h == 24 && (m != 0 || s != 0) {
        return false;
    }

    true
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    /// Literal and computed byteOrder values share one validator (DFDL §13.1).
    #[test]
    fn test_parse_byte_order_accepts_known_and_rejects_unknown() {
        assert_eq!(parse_byte_order("bigEndian").unwrap(), ByteOrder::BigEndian);
        assert_eq!(parse_byte_order(" LE ").unwrap(), ByteOrder::LittleEndian);
        let err = parse_byte_order("middleEndian").unwrap_err();
        assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
        assert!(err.to_string().contains("Unknown value for byteOrder property: middleEndian"));
    }

    #[test]
    fn test_static_byte_order_literal_is_validated() {
        let mut store = PropertyStore::new();
        store.set_property("byteOrder", "fatEndian").unwrap();
        let err = store.to_resolved_properties(None).unwrap_err();
        assert!(err.to_string().contains("fatEndian"));
    }

    #[test]
    fn test_property_forbids_literal_whitespace() {
        let mut store = PropertyStore::new();
        // escapeCharacter with raw space must fail
        store.set_property("escapeCharacter", " ").unwrap();
        let err = store.validate_property_entities().unwrap_err();
        assert!(err.to_string().contains("must not contain any whitespace"));
        assert!(err.to_string().contains("Use DFDL Entities"));

        // escapeBlockStart with raw space must fail
        let mut store2 = PropertyStore::new();
        store2
            .set_property("escapeBlockStart", "[ start ]")
            .unwrap();
        let err2 = store2.validate_property_entities().unwrap_err();
        assert!(err2.to_string().contains("must not contain any whitespace"));

        // textStringPadCharacter with raw space must fail
        let mut store3 = PropertyStore::new();
        store3.set_property("textStringPadCharacter", " ").unwrap();
        let err3 = store3.validate_property_entities().unwrap_err();
        assert!(err3.to_string().contains("must not contain any whitespace"));

        // Entity %SP; must succeed
        let mut store4 = PropertyStore::new();
        assert!(store4.set_property("escapeCharacter", "%SP;").is_ok());
        assert!(store4.validate_property_entities().is_ok());
        let mut store5 = PropertyStore::new();
        assert!(store5
            .set_property("escapeBlockStart", "[%SP;start%SP;]")
            .is_ok());
        assert!(store5.validate_property_entities().is_ok());
    }

    #[test]
    fn test_byte_entity_prohibition_and_separator_validation() {
        let mut store = PropertyStore::new();
        // Byte entity in textStandardDecimalSeparator must fail
        store
            .set_property("textStandardDecimalSeparator", "%#r2E;")
            .unwrap();
        let err = store.validate_property_entities().unwrap_err();
        assert!(err.to_string().contains("DFDL Byte Entity"));

        // Byte entity in fillByte must succeed
        let mut store2 = PropertyStore::new();
        assert!(store2.set_property("fillByte", "%#r20;").is_ok());
        assert!(store2.validate_property_entities().is_ok());

        // More than one char in textStandardDecimalSeparator must fail in resolution
        let mut store2 = PropertyStore::new();
        assert!(store2
            .set_property("textStandardDecimalSeparator", ". , *")
            .is_ok());
        let res = store2.to_resolved_properties(None);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("more than one character"));

        // Duplicate / non-distinct values must fail in resolution
        let mut store3 = PropertyStore::new();
        assert!(store3
            .set_property("textStandardDecimalSeparator", ".")
            .is_ok());
        assert!(store3
            .set_property("textStandardGroupingSeparator", ".")
            .is_ok());
        let res3 = store3.to_resolved_properties(None);
        assert!(res3.is_err());
        assert!(res3
            .unwrap_err()
            .to_string()
            .contains("Non-distinct property"));
    }

    #[test]
    fn test_single_character_property_validation() {
        // Multi-character literal pad character
        let mut store1 = PropertyStore::new();
        store1.set_property("textStringPadCharacter", "o0").unwrap();
        let err1 = store1.validate_property_entities().unwrap_err();
        assert!(err1
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Multi-entity pad character
        let mut store2 = PropertyStore::new();
        store2
            .set_property("textStringPadCharacter", "%HT;%CR;")
            .unwrap();
        let err2 = store2.validate_property_entities().unwrap_err();
        assert!(err2
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Character class %NL; in single character property
        let mut store3 = PropertyStore::new();
        store3
            .set_property("textStandardGroupingSeparator", "%NL;")
            .unwrap();
        let err3 = store3.validate_property_entities().unwrap_err();
        assert!(err3.to_string().contains("disallowed character class"));

        // Empty string in textStandardGroupingSeparator
        let mut store4 = PropertyStore::new();
        store4
            .set_property("textStandardGroupingSeparator", "")
            .unwrap();
        let err4 = store4.to_resolved_properties(None).unwrap_err();
        assert!(err4
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Valid single character entity %SP;
        let mut store5 = PropertyStore::new();
        store5
            .set_property("textStringPadCharacter", "%SP;")
            .unwrap();
        assert!(store5.validate_property_entities().is_ok());
    }

    #[test]
    fn test_parse_unparse_policy_validation() {
        // Valid policies
        let mut store1 = PropertyStore::new();
        store1.set_property("parseUnparsePolicy", "parseOnly").unwrap();
        let resolved = store1.to_resolved_properties(None).unwrap();
        assert_eq!(resolved.parse_unparse_policy, ParseUnparsePolicy::ParseOnly);

        let mut store2 = PropertyStore::new();
        store2.set_property("parseUnparsePolicy", "unparseOnly").unwrap();
        let resolved2 = store2.to_resolved_properties(None).unwrap();
        assert_eq!(resolved2.parse_unparse_policy, ParseUnparsePolicy::UnparseOnly);

        // Invalid policy
        let mut store3 = PropertyStore::new();
        store3.set_property("parseUnparsePolicy", "invalid").unwrap();
        let err3 = store3.to_resolved_properties(None).unwrap_err();
        assert!(err3.to_string().contains("Invalid parseUnparsePolicy"));

        // Incompatible parent parseOnly and child unparseOnly
        let mut parent_store = PropertyStore::new();
        parent_store.set_property("parseUnparsePolicy", "parseOnly").unwrap();
        let mut child_store = PropertyStore::new();
        child_store.set_property("parseUnparsePolicy", "unparseOnly").unwrap();
        let err4 = child_store.to_resolved_properties(Some(&parent_store)).unwrap_err();
        assert!(err4.to_string().contains("Incompatible parseUnparsePolicy"));
    }
    /// fillByte presence is tracked so the unparser can reject padding without it.
    #[test]
    fn test_fill_byte_defined_flag() {
        let mut store = PropertyStore::new();
        assert!(!store.to_resolved_properties(None).unwrap().fill_byte_defined);
        store.set_property("fillByte", "%NUL;").unwrap();
        assert!(store.to_resolved_properties(None).unwrap().fill_byte_defined);
    }

    /// Verifies that extend_excluding copies properties except those in the exclude store.
    #[test]
    fn test_extend_excluding() {
        let mut target = PropertyStore::new();
        target.set_property("representation", "text").unwrap();

        let mut st_props = PropertyStore::new();
        st_props.set_property("length", "10").unwrap();
        st_props.set_property("encoding", "UTF-8").unwrap();

        let mut direct_props = PropertyStore::new();
        direct_props.set_property("length", "{ 2 }").unwrap();

        target.extend_excluding(&st_props, &direct_props);

        // "length" was excluded because it exists in direct_props
        assert_eq!(target.get_property("length"), None);
        // "encoding" was inherited from st_props
        assert_eq!(target.get_property("encoding"), Some("UTF-8"));
        // "representation" was preserved
        assert_eq!(target.get_property("representation"), Some("text"));
    }

    #[test]
    fn test_dfdl_entities_encode_decode_roundtrip() {
        assert_eq!(decode_dfdl_character_entities("%NEL;"), "\u{0085}");
        assert_eq!(decode_dfdl_character_entities("%LF;"), "\n");
        assert_eq!(decode_dfdl_character_entities("%CR;"), "\r");
        assert_eq!(decode_dfdl_character_entities("%CR;%LF;"), "\r\n");
        assert_eq!(decode_dfdl_character_entities("%%"), "%");

        assert_eq!(encode_dfdl_character_entities("\u{0085}"), "%NEL;");
        assert_eq!(encode_dfdl_character_entities("\n"), "%LF;");
        assert_eq!(encode_dfdl_character_entities("\r"), "%CR;");
        assert_eq!(encode_dfdl_character_entities("\r\n"), "%CR;%LF;");
        assert_eq!(encode_dfdl_character_entities("%"), "%%");
        assert_eq!(encode_dfdl_character_entities("hello\nworld%"), "hello%LF;world%%");
    }

    #[test]
    fn test_property_store_remove_property() {
        let mut store = PropertyStore::new();
        store.set_property("ref", "myFormat").unwrap();
        store.set_property("dfdl:lengthKind", "explicit").unwrap();
        assert_eq!(store.get_property("ref"), Some("myFormat"));
        assert_eq!(store.get_property("lengthKind"), Some("explicit"));

        store.remove_property("ref");
        assert_eq!(store.get_property("ref"), None);

        store.remove_property("dfdl:lengthKind");
        assert_eq!(store.get_property("lengthKind"), None);
    }

    #[test]
    fn test_property_store_update_namespaces_and_inheritance() {
        let mut store = PropertyStore::new();
        store.add_namespaces(&[
            (String::new(), alloc::string::String::from("urn:default")),
            (alloc::string::String::from("ex1"), alloc::string::String::from("urn:old")),
        ]);

        // Update namespaces simulates an inner element shadowing a prefix
        store.update_namespaces(&[
            (alloc::string::String::from("ex1"), alloc::string::String::from("http://example.com/new")),
            (alloc::string::String::from("ex2"), alloc::string::String::from("http://example.com/2")),
        ]);

        let ns = store.in_scope_namespaces();
        assert_eq!(
            ns.iter().find(|(p, _)| p == "ex1").map(|(_, u)| u.as_str()),
            Some("http://example.com/new")
        );
        assert_eq!(
            ns.iter().find(|(p, _)| p == "ex2").map(|(_, u)| u.as_str()),
            Some("http://example.com/2")
        );
        assert_eq!(
            ns.iter().find(|(p, _)| p.is_empty()).map(|(_, u)| u.as_str()),
            Some("urn:default")
        );

        // Test extending another store preserves target's existing prefix
        let mut child_store = PropertyStore::new();
        child_store.add_namespaces(&[
            (alloc::string::String::from("ex1"), alloc::string::String::from("http://example.com/child")),
        ]);
        child_store.extend(&store);
        let child_ns = child_store.in_scope_namespaces();
        assert_eq!(
            child_ns.iter().find(|(p, _)| p == "ex1").map(|(_, u)| u.as_str()),
            Some("http://example.com/child")
        );
        assert_eq!(
            child_ns.iter().find(|(p, _)| p == "ex2").map(|(_, u)| u.as_str()),
            Some("http://example.com/2")
        );
    }

    /// Verifies xs:dateTime validation coverage across all branches and binaryCalendarEpoch resolution.
    #[test]
    fn test_is_valid_xs_date_time_and_binary_calendar_epoch() {
        // Valid date times
        assert!(is_valid_xs_date_time("2026-10-07T23:59:59Z"));
        assert!(is_valid_xs_date_time("1970-01-01T00:00:00+02:00"));
        assert!(is_valid_xs_date_time("-0044-03-15T12:30:00-05:00"));
        assert!(is_valid_xs_date_time("2020-02-29T23:59:59.123456Z"));
        assert!(is_valid_xs_date_time("2020-01-01T24:00:00Z"));
        assert!(is_valid_xs_date_time("2020-01-01T12:00:00"));

        // Invalid date times
        assert!(!is_valid_xs_date_time("notADateTime"));
        assert!(!is_valid_xs_date_time("2020-01T12:00:00"));
        assert!(!is_valid_xs_date_time("20-01-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-13-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-00-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-32T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-00T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:00+0200"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:00."));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T25:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:60:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:61"));
        assert!(!is_valid_xs_date_time("2020-01-01T24:01:00"));

        // binaryCalendarEpoch property validation
        let mut store_valid = PropertyStore::new();
        store_valid.set_property("binaryCalendarEpoch", "1970-01-01T00:00:00Z").unwrap();
        assert!(store_valid.to_resolved_properties(None).is_ok());

        let mut store_invalid = PropertyStore::new();
        store_invalid.set_property("binaryCalendarEpoch", "invalid-epoch").unwrap();
        assert!(store_invalid.to_resolved_properties(None).is_err());
    }

    #[test]
    fn test_property_store_methods_and_character_entities() {
        // 1. All character entity codes in encode and decode
        let all_ctrls = "\u{2028}\0\t\x7F\x1B\x07\x08\x0C\x0B\x18\x06\x15\x05\x04\x03\x02\x01\x0E\x0F\x16\x17\x19\x1A\x1C\x1D\x1E\x1F\x10\x11\x12\x13\x14";
        let encoded = encode_dfdl_character_entities(all_ctrls);
        assert!(encoded.contains("%LS;"));
        assert!(encoded.contains("%NUL;"));
        assert!(encoded.contains("%HT;"));
        assert!(encoded.contains("%DEL;"));
        assert!(encoded.contains("%ESC;"));
        assert!(encoded.contains("%BEL;"));
        assert!(encoded.contains("%BS;"));
        assert!(encoded.contains("%FF;"));
        assert!(encoded.contains("%VT;"));
        assert!(encoded.contains("%CAN;"));
        assert!(encoded.contains("%ACK;"));
        assert!(encoded.contains("%NAK;"));
        assert!(encoded.contains("%ENQ;"));
        assert!(encoded.contains("%EOT;"));
        assert!(encoded.contains("%ETX;"));
        assert!(encoded.contains("%STX;"));
        assert!(encoded.contains("%SOH;"));
        assert!(encoded.contains("%SO;"));
        assert!(encoded.contains("%SI;"));
        assert!(encoded.contains("%SYN;"));
        assert!(encoded.contains("%ETB;"));
        assert!(encoded.contains("%EM;"));
        assert!(encoded.contains("%SUB;"));
        assert!(encoded.contains("%FS;"));
        assert!(encoded.contains("%GS;"));
        assert!(encoded.contains("%RS;"));
        assert!(encoded.contains("%US;"));
        assert!(encoded.contains("%DLE;"));
        assert!(encoded.contains("%DC1;"));
        assert!(encoded.contains("%DC2;"));
        assert!(encoded.contains("%DC3;"));
        assert!(encoded.contains("%DC4;"));

        let decoded = decode_dfdl_character_entities(&encoded);
        assert_eq!(decoded, all_ctrls);

        // 2. is_empty, add_set_variable, add_new_variable_instance, add_assert_error
        let mut s = PropertyStore::new();
        assert!(s.is_empty());
        s.add_set_variable("tns:myVar", "10");
        assert!(!s.is_empty());
        assert_eq!(s.set_variables().len(), 1);
        assert_eq!(s.set_variables()[0].0.local_name, "myVar");

        s.add_new_variable_instance("ex:newVar", Some("42"));
        assert_eq!(s.new_variable_instances().len(), 1);

        s.add_assert_error("some error");
        assert_eq!(s.assert_errors(), &["some error"]);

        // 3. override_with
        let mut other = PropertyStore::new();
        other.set_property("byteOrder", "littleEndian").unwrap();
        other.add_set_variable("otherVar", "99");
        other.add_new_variable_instance("otherInst", None);
        other.add_assert_error("other error");
        s.override_with(&other);
        assert_eq!(s.get_property("byteOrder"), Some("littleEndian"));
        assert_eq!(s.set_variables().len(), 2);
        assert_eq!(s.new_variable_instances().len(), 2);
        assert_eq!(s.assert_errors().len(), 2);

        // 4. merge_parent non-inheritable property exclusions
        let mut parent = PropertyStore::new();
        parent.set_property("inputValueCalc", "{42}").unwrap();
        parent.set_property("outputValueCalc", "{42}").unwrap();
        parent.set_property("initiator", "%SP;").unwrap();
        parent.set_property("terminator", "%NL;").unwrap();
        parent.set_property("separator", ",").unwrap();
        parent.set_property("sequenceKind", "ordered").unwrap();
        parent.set_property("initiatedContent", "no").unwrap();
        parent.set_property("choiceDispatchKey", "{.}").unwrap();
        parent.set_property("choiceBranchKey", "1").unwrap();
        parent.set_property("hiddenGroupRef", "tns:g").unwrap();
        parent.set_property("length", "10").unwrap();
        parent.set_property("lengthKind", "explicit").unwrap();
        parent.set_property("occursCount", "5").unwrap();
        parent.set_property("occursCountKind", "fixed").unwrap();
        parent.set_property("fillByte", "0").unwrap();
        parent.set_property("prefixLengthType", "tns:len").unwrap();
        parent.set_property("representation", "binary").unwrap();
        parent.set_property("repType", "tns:rep").unwrap();
        parent.set_property("repValues", "1 2").unwrap();
        parent.set_property("encoding", "UTF-8").unwrap(); // inheritable!

        let mut child = PropertyStore::new();
        child.merge_parent(&parent);
        assert_eq!(child.get_property("encoding"), Some("UTF-8"));
        assert_eq!(child.get_property("inputValueCalc"), None);
        assert_eq!(child.get_property("outputValueCalc"), None);
        assert_eq!(child.get_property("initiator"), None);
        assert_eq!(child.get_property("terminator"), None);
        assert_eq!(child.get_property("separator"), None);
        assert_eq!(child.get_property("choiceDispatchKey"), None);
        assert_eq!(child.get_property("length"), None);

        // 5. Property syntax validation errors
        assert!(validate_dfdl_property_entities("initiator", "bad\x01val").is_err());
        assert!(validate_dfdl_property_entities("escapeCharacter", "%WSP;").is_err());
        assert!(validate_dfdl_property_entities("textStandardZeroRep", "%NL;").is_err());

        // 6. %ES; empty entity decoding and CRLF/control encoding
        assert_eq!(decode_dfdl_character_entities("a%ES;b"), "ab");
        let enc_ctrl = encode_dfdl_character_entities("a\r\nb\x01\u{0080}c");
        assert!(enc_ctrl.contains("%CR;%LF;"));
        assert!(enc_ctrl.contains("%SOH;"));
        assert!(enc_ctrl.contains("%#x80;"));

        // 7. extend_excluding with variables, repValue, and enumeration
        let mut s_src = PropertyStore::new();
        s_src.add_set_variable("var1", "{1}");
        s_src.add_new_variable_instance("var2", Some("{2}"));
        s_src.set_property("repValue:1", "one").unwrap();
        s_src.set_property("enumeration", "val").unwrap();

        let mut s_dest = PropertyStore::new();
        let s_excl = PropertyStore::new();
        s_dest.extend_excluding(&s_src, &s_excl);
        assert_eq!(s_dest.set_variables().len(), 1);
        assert_eq!(s_dest.new_variable_instances().len(), 1);
        assert_eq!(s_dest.get_property("repValue:1"), Some("one"));
        assert_eq!(s_dest.get_property("enumeration"), Some("val"));

        // 8. leadingSkip and trailingSkip invalid property validation
        let mut s_skips = PropertyStore::new();
        s_skips.set_property("leadingSkip", "-5").unwrap();
        assert!(s_skips.to_resolved_properties(None).is_err());
        s_skips.set_property("leadingSkip", "not_a_number").unwrap();
        assert!(s_skips.to_resolved_properties(None).is_err());

        let mut s_tskip = PropertyStore::new();
        s_tskip.set_property("trailingSkip", "-1").unwrap();
        assert!(s_tskip.to_resolved_properties(None).is_err());
        s_tskip.set_property("trailingSkip", "invalid").unwrap();
        assert!(s_tskip.to_resolved_properties(None).is_err());

        // 9. fillByte validation error branches
        let mut s_fb1 = PropertyStore::new();
        s_fb1.set_property("fillByte", "").unwrap();
        assert!(s_fb1.to_resolved_properties(None).is_err());

        let mut s_fb2 = PropertyStore::new();
        s_fb2.set_property("fillByte", "%#d999;").unwrap();
        assert!(s_fb2.to_resolved_properties(None).is_err());

        let mut s_fb3 = PropertyStore::new();
        s_fb3.set_property("encoding", "X-DFDL-6-BIT-DFI-264.2").unwrap();
        s_fb3.set_property("fillByte", "A").unwrap();
        assert!(s_fb3.to_resolved_properties(None).is_err());

        let mut s_fb4 = PropertyStore::new();
        s_fb4.set_property("encoding", "UTF-8").unwrap();
        s_fb4.set_property("fillByte", "\u{00E9}").unwrap();
        assert!(s_fb4.to_resolved_properties(None).is_err());

        let mut s_fb5 = PropertyStore::new();
        s_fb5.set_property("fillByte", "ABC").unwrap();
        assert!(s_fb5.to_resolved_properties(None).is_err());
    }

    /// Verifies PropertyStore namespace retention, assertion builder helpers, and enum validation error branches.
    #[test]
    fn test_property_store_additional_coverage() {
        // 1. In-scope namespaces setter and getter roundtrip
        let mut store = PropertyStore::new();
        let ns = alloc::vec![("ns1".into(), "http://example.com/1".into())];
        store.set_in_scope_namespaces(ns);
        assert_eq!(store.in_scope_namespaces().len(), 1);

        // 2. Assertion builder methods
        store.add_assert("true()", Some("Assertion message"));
        store.add_assert_with_kind(crate::schema::ir::TestKind::Pattern, "[a-z]+", None);
        assert_eq!(store.asserts.len(), 2);

        // 3. Invalid bitOrder property rejection
        let mut s_bo = PropertyStore::new();
        s_bo.set_property("bitOrder", "unknownBitOrder").unwrap();
        assert!(s_bo.to_resolved_properties(None).is_err());

        // 4. lengthKind endOfParent and invalid lengthKind
        let mut s_eop = PropertyStore::new();
        s_eop.set_property("lengthKind", "endOfParent").unwrap();
        let props_eop = s_eop.to_resolved_properties(None).unwrap();
        assert_eq!(props_eop.length_kind, LengthKind::EndOfParent);

        let mut s_lk_bad = PropertyStore::new();
        s_lk_bad.set_property("lengthKind", "badLengthKind").unwrap();
        assert!(s_lk_bad.to_resolved_properties(None).is_err());

        // 5. Invalid separatorPosition and separatorSuppressionPolicy rejection
        let mut s_sp_bad = PropertyStore::new();
        s_sp_bad.set_property("separatorPosition", "badPos").unwrap();
        assert!(s_sp_bad.to_resolved_properties(None).is_err());

        let mut s_ssp_bad = PropertyStore::new();
        s_ssp_bad.set_property("separatorSuppressionPolicy", "badSSP").unwrap();
        assert!(s_ssp_bad.to_resolved_properties(None).is_err());
    }
}

