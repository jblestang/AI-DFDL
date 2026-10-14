//! Generic engineering unit conversion driven strictly by DFDL schema metadata.
//!
//! Applies scale factors, mathematical resolutions, and physical units discovered
//! in the DFDL schema to transform raw decoded values into human-readable engineering
//! representations for Wireshark inspection.
//!
//! This module has zero hardcoded field lists; all logic relies dynamically on
//! the metadata extracted from the loaded DFDL schema.

use crate::schema_meta::FieldMetadata;

/// Converted field holding both raw representation and formatted engineering value.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertedValue {
    /// Raw unparsed value string as extracted from the DFDL infoset.
    pub raw: String,
    /// Human-readable engineering value with units (e.g. `197.684 NM`, `07:35:54.601 UTC`).
    pub formatted: String,
    /// Physical measurement unit as declared in the schema.
    pub unit: String,
    /// Fractional scale or resolution description.
    pub resolution: String,
    /// Detailed documentation tooltip for Wireshark protocol tree.
    pub tooltip: String,
}

/// Converts a raw field string into a formatted engineering representation.
///
/// Multiplies numeric raw values by the metadata's `scale_factor` and appends
/// formatted units. If no metadata is found, the raw value is preserved intact.
#[must_use]
pub fn convert_value(name: &str, raw_str: &str, meta_opt: Option<&FieldMetadata>) -> ConvertedValue {
    let raw_trimmed = raw_str.trim();

    let meta = match meta_opt {
        Some(m) => m,
        None => {
            return ConvertedValue {
                raw: raw_trimmed.to_string(),
                formatted: raw_trimmed.to_string(),
                unit: String::new(),
                resolution: String::new(),
                tooltip: name.to_string(),
            };
        }
    };

    if let Ok(raw_num) = raw_trimmed.parse::<f64>() {
        convert_numeric_value(raw_num, raw_trimmed, meta)
    } else if let Some(hex_str) = raw_trimmed.strip_prefix("0x").or_else(|| raw_trimmed.strip_prefix("0X")) {
        if let Ok(hex_val) = i64::from_str_radix(hex_str, 16) {
            convert_numeric_value(hex_val as f64, raw_trimmed, meta)
        } else {
            convert_non_numeric_value(raw_str, meta)
        }
    } else {
        convert_non_numeric_value(raw_str, meta)
    }
}

/// Converts a numeric raw value using schema scale factor and declared unit.
fn convert_numeric_value(raw_num: f64, raw_str: &str, meta: &FieldMetadata) -> ConvertedValue {
    // Check if there is an enum mapping for this integer value.
    // Enum shall have the string value as main value, and numeric value as secondary information.
    let raw_int = raw_num.round() as i64;
    let is_int = (raw_num - raw_int as f64).abs() < f64::EPSILON;
    if is_int {
        if let Some(label) = meta.get_enum_label(raw_int) {
            let formatted = format!("{label} ({raw_int})");
            return ConvertedValue {
                raw: raw_str.to_string(),
                formatted,
                unit: meta.unit.clone(),
                resolution: meta.resolution_str.clone(),
                tooltip: build_tooltip(meta),
            };
        } else if !meta.enum_map.is_empty() {
            let formatted = format!("Unknown ({raw_int})");
            return ConvertedValue {
                raw: raw_str.to_string(),
                formatted,
                unit: meta.unit.clone(),
                resolution: meta.resolution_str.clone(),
                tooltip: build_tooltip(meta),
            };
        }
    }

    let scale = meta.scale_factor;
    let converted_num = raw_num * scale;
    let unit = meta.unit.trim();
    let name_lower = meta.name.to_lowercase();

    let formatted = match unit {
        "seconds" | "s" => {
            format_seconds_with_time(converted_num, raw_num)
        }
        "NM" | "nm" => {
            let km = converted_num * 1.852;
            format!("{converted_num:.3} NM ({km:.2} km)")
        }
        "degrees" | "deg" | "°" => {
            format!("{converted_num:.3}°")
        }
        "FL" | "fl" => {
            let feet = converted_num * 100.0;
            format!("FL {converted_num:.1} ({feet:.0} ft)")
        }
        "feet" | "ft" => {
            let fl = converted_num / 100.0;
            format!("{converted_num:.0} ft (FL {fl:.1})")
        }
        "NM/s" => {
            let knots = converted_num * 3600.0;
            let m_s = knots * 0.514444;
            format!("{knots:.1} kt ({converted_num:.4} NM/s, {m_s:.1} m/s)")
        }
        "meters" | "m" => {
            format!("{converted_num:.2} m")
        }
        "dBm" | "dbm" => {
            format!("{converted_num:.0} dBm")
        }
        "octal" => {
            let oct = (raw_num as u32) & 0x0FFF;
            format!("{oct:04o} (raw: {raw_num:.0})")
        }
        "hex" => {
            let h = raw_num as u64;
            format!("0x{h:X} ({h})")
        }
        _ if !unit.is_empty() => {
            let u_lower = unit.to_lowercase();
            if matches!(
                u_lower.as_str(),
                "dimensionless"
                    | "code"
                    | "bitfield"
                    | "track_id"
                    | "type_id"
                    | "compound"
                    | "bytes"
                    | "byte"
            ) {
                if (scale - 1.0).abs() >= f64::EPSILON {
                    format!("{converted_num:.3}")
                } else {
                    format!("{raw_num:.0}")
                }
            } else if (scale - 1.0).abs() >= f64::EPSILON {
                format!("{converted_num:.3} {unit}")
            } else {
                format!("{raw_num:.0} {unit}")
            }
        }
        _ => {
            // No explicit unit declared
            if name_lower.contains("time") && raw_num > 100.0 && scale != 1.0 {
                format_seconds_with_time(converted_num, raw_num)
            } else if scale != 1.0 {
                format!("{converted_num:.3}")
            } else {
                raw_str.to_string()
            }
        }
    };

    let tooltip = build_tooltip(meta);

    ConvertedValue {
        raw: raw_str.to_string(),
        formatted,
        unit: meta.unit.clone(),
        resolution: meta.resolution_str.clone(),
        tooltip,
    }
}

/// Formats non-numeric values (hex payloads, character strings, etc.).
fn convert_non_numeric_value(raw_str: &str, meta: &FieldMetadata) -> ConvertedValue {
    let formatted = if raw_str.starts_with('"') && raw_str.ends_with('"') {
        raw_str.to_string()
    } else if raw_str.chars().all(|c| c.is_ascii_graphic() || c.is_whitespace()) && !raw_str.is_empty() {
        format!("\"{}\"", raw_str.trim())
    } else {
        raw_str.to_string()
    };

    let tooltip = build_tooltip(meta);

    ConvertedValue {
        raw: raw_str.to_string(),
        formatted,
        unit: meta.unit.clone(),
        resolution: meta.resolution_str.clone(),
        tooltip,
    }
}

/// Formats a seconds value into `seconds (HH:MM:SS.mmm UTC)`.
fn format_seconds_with_time(seconds: f64, _raw: f64) -> String {
    let total_secs = seconds.max(0.0);
    let hours = (total_secs / 3600.0).floor() as u32 % 24;
    let mins = ((total_secs % 3600.0) / 60.0).floor() as u32;
    let secs = (total_secs % 60.0).floor() as u32;
    let millis = ((total_secs.fract()) * 1000.0).round() as u32;

    format!("{seconds:.3} s ({hours:02}:{mins:02}:{secs:02}.{millis:03} UTC)")
}

/// Generates a documentation tooltip string describing the field and its resolution.
fn build_tooltip(meta: &FieldMetadata) -> String {
    let mut parts = Vec::new();
    if !meta.description.is_empty() {
        parts.push(meta.description.clone());
    }
    if !meta.item_ref.is_empty() {
        parts.push(format!("Item: {}", meta.item_ref));
    }
    if !meta.unit.is_empty() {
        parts.push(format!("Unit: {}", meta.unit));
    }
    if !meta.resolution_str.is_empty() {
        parts.push(format!("Resolution: {}", meta.resolution_str));
    }
    if !meta.lsb_str.is_empty() {
        parts.push(format!("LSB: {}", meta.lsb_str));
    }
    if !meta.enum_map.is_empty() {
        let mut sorted_enums: Vec<_> = meta.enum_map.iter().collect();
        sorted_enums.sort_by_key(|(k, _)| **k);
        let enum_str = sorted_enums
            .into_iter()
            .take(8)
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("Values: [{enum_str}]"));
    }
    if !meta.documentation.is_empty() && meta.documentation != meta.description {
        let single_line = meta
            .documentation
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        parts.push(single_line);
    }
    if parts.is_empty() {
        meta.name.clone()
    } else {
        parts.join(" | ")
    }
}
