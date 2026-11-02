//! Serialization and validation of DFDL simple and complex elements.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::state::ElementState;
use crate::infoset::tree::{InfosetDocument, InfosetElement};
use crate::infoset::value::DfdlValue;
use crate::io::bitstream::BitWriter;
use crate::io::sink::VecByteSink;
use crate::io::traits::{BitOrder, ByteOrder, ByteSink};
use crate::limits::WorkBudget;
use crate::schema::ir::{
    CompiledSchema, Representation, ResolvedProperties, TermKind,
};
use super::framing::fill_byte_value;
use super::UnparserEngine;

pub(crate) fn remap_pua_to_raw_chars(text: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(text.len());
    for c in text.chars() {
        let u = c as u32;
        if (0xE000..=0xE01F).contains(&u) || (0xE080..=0xE09F).contains(&u) {
            if let Some(mapped) = core::char::from_u32(u.saturating_sub(0xE000)) {
                out.push(mapped);
            } else {
                out.push(c);
            }
        } else if u == 0xE07F {
            out.push('\x7F');
        } else {
            out.push(c);
        }
    }
    out
}


pub(crate) fn validate_text_number_rounding(val: &DfdlValue, props: &ResolvedProperties) -> DFDLResult<()> {
    if props.text_number_rounding_mode != crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary {
        return Ok(());
    }
    let pat = match props.text_number_pattern.as_deref() {
        Some(p) => p,
        None => return Ok(()),
    };
    let pos_pat = pat.split(';').next().unwrap_or(pat);
    let max_frac_digits = if let Some(dot_pos) = pos_pat.find('.') {
        let start = dot_pos.saturating_add(1);
        pos_pat.get(start..).unwrap_or("")
            .chars()
            .take_while(|c| *c == '#' || *c == '0')
            .count()
    } else {
        0
    };
    let val_str = match val {
        DfdlValue::Decimal(s) => s.clone(),
        DfdlValue::Float(f) => alloc::format!("{}", f),
        DfdlValue::Double(d) => alloc::format!("{}", d),
        DfdlValue::String(s) => s.clone(),
        _ => return Ok(()),
    };
    if let Some(dot_idx) = val_str.find('.') {
        let start = dot_idx.saturating_add(1);
        let frac_str = val_str.get(start..).unwrap_or("");
        let trimmed_frac = frac_str.trim_end_matches('0');
        if trimmed_frac.len() > max_frac_digits {
            let msg = alloc::format!(
                "Unparse Error: rounding is required for value '{}' with pattern '{}' but textNumberRoundingMode is 'roundUnnecessary'",
                val_str, pat
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }
    }
    Ok(())
}

pub(crate) fn validate_unparse_value(
    val: &DfdlValue,
    st: &crate::infoset::DfdlSimpleType,
    props: &ResolvedProperties,
    max_hex_binary_length_in_bytes: Option<usize>,
) -> DFDLResult<()> {
    if props.binary_number_rep == crate::schema::ir::BinaryNumberRep::Bcd && val.is_negative() {
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: bcd only positive, cannot be negative: {}",
                val
            ),
        ));
    }

    if !props.decimal_signed && val.is_negative() {
        let rep_msg = match props.binary_number_rep {
            crate::schema::ir::BinaryNumberRep::Packed
            | crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                "Packed binary negative decimalSigned no"
            }
            _ => "Binary negative decimalSigned no",
        };
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: {}: cannot unparse negative value when decimalSigned is 'no'",
                rep_msg
            ),
        ));
    }

    if props.representation == crate::schema::ir::Representation::Text
        && props.text_standard_base != 10
        && val.is_negative()
    {
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                val,
                props.text_standard_base
            ),
        ));
    }

    if props.representation == crate::schema::ir::Representation::Text
        && props.text_standard_base != 10
    {
        return Ok(());
    }

    // Fast-path: When the infoset value matches the expected simple type, it is already valid by definition.
    match (st, val) {
        (crate::infoset::DfdlSimpleType::Int, DfdlValue::Int(_))
        | (crate::infoset::DfdlSimpleType::Long, DfdlValue::Long(_))
        | (crate::infoset::DfdlSimpleType::Short, DfdlValue::Short(_))
        | (crate::infoset::DfdlSimpleType::Byte, DfdlValue::Byte(_))
        | (crate::infoset::DfdlSimpleType::UnsignedLong, DfdlValue::UnsignedLong(_))
        | (crate::infoset::DfdlSimpleType::UnsignedInt, DfdlValue::UnsignedInt(_))
        | (crate::infoset::DfdlSimpleType::UnsignedShort, DfdlValue::UnsignedShort(_))
        | (crate::infoset::DfdlSimpleType::UnsignedByte, DfdlValue::UnsignedByte(_))
        | (crate::infoset::DfdlSimpleType::Boolean, DfdlValue::Boolean(_))
        | (crate::infoset::DfdlSimpleType::Float, DfdlValue::Float(_))
        | (crate::infoset::DfdlSimpleType::Double, DfdlValue::Double(_))
        | (crate::infoset::DfdlSimpleType::String, DfdlValue::String(_)) => return Ok(()),
        _ => {}
    }

    match st {
        crate::infoset::DfdlSimpleType::Int => {
            if let Some(val_num) = val.as_i128() {
                i32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:int", val),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<i32>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:int", trimmed),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::Long => {
            if let Some(val_num) = val.as_i128() {
                i64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:long", val),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<i64>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:long", trimmed),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::Short => {
            if let Some(val_num) = val.as_i128() {
                i16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:short", val),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<i16>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:short", trimmed),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::Byte => {
            if let Some(val_num) = val.as_i128() {
                i8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:byte", val),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<i8>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:byte", trimmed),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedLong => {
            if val.is_negative() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedLong",
                        val
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedLong",
                            val
                        ),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<u64>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedLong",
                            trimmed
                        ),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedInt => {
            if val.is_negative() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedInt",
                        val
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedInt",
                            val
                        ),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<u32>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedInt",
                            trimmed
                        ),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedShort => {
            if val.is_negative() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedShort",
                        val
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedShort",
                            val
                        ),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<u16>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedShort",
                            trimmed
                        ),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedByte => {
            if val.is_negative() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedByte",
                        val
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedByte",
                            val
                        ),
                    )
                })?;
            } else {
                let str_rep;
                let trimmed = match val {
                    DfdlValue::String(s) => s.trim(),
                    _ => {
                        str_rep = alloc::format!("{}", val);
                        str_rep.trim()
                    }
                };
                trimmed.parse::<u8>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedByte",
                            trimmed
                        ),
                    )
                })?;
            }
        }
        crate::infoset::DfdlSimpleType::Double => {
            if let DfdlValue::String(ref s) = val {
                let trimmed = s.trim();
                if trimmed.parse::<f64>().is_err() {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:double", trimmed),
                    ));
                }
            }
        }
        crate::infoset::DfdlSimpleType::Float => {
            if let DfdlValue::String(ref s) = val {
                let trimmed = s.trim();
                if trimmed.parse::<f32>().is_err() {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:float", trimmed),
                    ));
                }
            }
        }
        crate::infoset::DfdlSimpleType::HexBinary => {
            if let DfdlValue::String(ref s) = val {
                let clean = s.trim();
                if clean.len() % 2 != 0 {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:hexBinary: even number of characters required",
                            clean
                        ),
                    ));
                }
                if !clean.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:hexBinary: invalid hex digits",
                            clean
                        ),
                    ));
                }
                if let Some(max_len) = max_hex_binary_length_in_bytes {
                    let hex_len = clean.len() / 2;
                    if hex_len > max_len {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: xs:hexBinary length ({}) exceeds maximum allowed length of {} bytes",
                                hex_len, max_len
                            ),
                        ));
                    }
                }
                if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    if let Some(exp_len) = props.length {
                        let exp_bits = match props.length_units {
                            crate::schema::ir::LengthUnits::Bits => exp_len,
                            crate::schema::ir::LengthUnits::Bytes
                            | crate::schema::ir::LengthUnits::Characters => {
                                exp_len.saturating_mul(8)
                            }
                        };
                        let data_bits = (clean.len() / 2).saturating_mul(8);
                        if data_bits > exp_bits {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: xs:hexBinary calculated length ({} bits) is greater than explicit length ({} bits)",
                                    data_bits, exp_bits
                                ),
                            ));
                        }
                    }
                }
            } else if let DfdlValue::HexBinary(ref b) = val {
                if let Some(max_len) = max_hex_binary_length_in_bytes {
                    if b.len() > max_len {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: xs:hexBinary length ({}) exceeds maximum allowed length of {} bytes",
                                b.len(), max_len
                            ),
                        ));
                    }
                }
                if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    if let Some(exp_len) = props.length {
                        let exp_bits = match props.length_units {
                            crate::schema::ir::LengthUnits::Bits => exp_len,
                            crate::schema::ir::LengthUnits::Bytes
                            | crate::schema::ir::LengthUnits::Characters => {
                                exp_len.saturating_mul(8)
                            }
                        };
                        let data_bits = b.len().saturating_mul(8);
                        if data_bits > exp_bits {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: xs:hexBinary calculated length ({} bits) is greater than explicit length ({} bits)",
                                    data_bits, exp_bits
                                ),
                            ));
                        }
                    }
                }
            }
        }
        crate::infoset::DfdlSimpleType::Boolean => match val {
            DfdlValue::Boolean(_) => {}
            DfdlValue::String(ref s) => {
                let trimmed = s.trim();
                if !matches!(trimmed, "true" | "false" | "1" | "0") {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value '{}' is not a valid xs:boolean", trimmed),
                    ));
                }
            }
            _ => {
                let str_rep = alloc::format!("{}", val);
                let trimmed = str_rep.trim();
                if !matches!(trimmed, "true" | "false" | "1" | "0") {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value '{}' is not a valid xs:boolean", trimmed),
                    ));
                }
            }
        },
        crate::infoset::DfdlSimpleType::Date
        | crate::infoset::DfdlSimpleType::Time
        | crate::infoset::DfdlSimpleType::DateTime => {
            let str_rep;
            let clean = match val {
                DfdlValue::String(s)
                | DfdlValue::Time(s)
                | DfdlValue::Date(s)
                | DfdlValue::DateTime(s) => s.as_str().trim(),
                _ => {
                    str_rep = alloc::format!("{}", val);
                    str_rep.trim()
                }
            };
            match st {
                crate::infoset::DfdlSimpleType::Time => {
                    if !is_valid_xs_time(clean) {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Unparse Error: Value '{}' is not a valid xs:time", clean),
                        ));
                    }
                }
                crate::infoset::DfdlSimpleType::Date => {
                    if !is_valid_xs_date(clean) {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Unparse Error: Value '{}' is not a valid xs:date", clean),
                        ));
                    }
                }
                crate::infoset::DfdlSimpleType::DateTime if !is_valid_xs_date_time(clean) => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value '{}' is not a valid xs:dateTime", clean),
                    ));
                }
                _ => {}
            }
        }
        crate::infoset::DfdlSimpleType::Decimal => {
            let str_rep;
            let trimmed = match val {
                DfdlValue::Decimal(s) | DfdlValue::String(s) => s.trim(),
                _ => {
                    str_rep = alloc::format!("{}", val);
                    str_rep.trim()
                }
            };
            if props.facets.min_inclusive.as_deref() == Some("0") {
                let is_non_neg_int = !trimmed.starts_with('-')
                    && !trimmed.contains('.')
                    && trimmed
                        .strip_prefix('+')
                        .unwrap_or(trimmed)
                        .chars()
                        .all(|c| c.is_ascii_digit());
                if !is_non_neg_int {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is out of range for type xs:nonNegativeInteger",
                            trimmed
                        ),
                    ));
                }
            } else if props.facets.fraction_digits == Some(0) {
                let is_int = !trimmed.contains('.')
                    && trimmed
                        .strip_prefix('+')
                        .or_else(|| trimmed.strip_prefix('-'))
                        .unwrap_or(trimmed)
                        .chars()
                        .all(|c| c.is_ascii_digit());
                if !is_int {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:integer",
                            trimmed
                        ),
                    ));
                }
            }
        }
        _ => {}
    }

    if let Some(explicit_len) = props.length {
        let bits = explicit_len.saturating_mul(8);
        if bits > 0 && bits < 64 {
            if let Some(v_i128) = val.as_i128() {
                if val.is_negative() {
                    let shift = (bits.saturating_sub(1)) as u32;
                    let min_val = 0_i128.saturating_sub(1_i128.checked_shl(shift).unwrap_or(0));
                    if v_i128 < min_val {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Value {} cannot fit in signed binary number {} bit(s)",
                                val,
                                bits
                            ),
                        ));
                    }
                } else {
                    let max_val = (1_i128 << bits).saturating_sub(1);
                    if v_i128 > max_val {
                        let is_unsigned = matches!(
                            st,
                            crate::infoset::DfdlSimpleType::UnsignedLong
                                | crate::infoset::DfdlSimpleType::UnsignedInt
                                | crate::infoset::DfdlSimpleType::UnsignedShort
                                | crate::infoset::DfdlSimpleType::UnsignedByte
                        );
                        let kind_str = if is_unsigned { "unsigned" } else { "signed" };
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Value {} cannot fit in {} binary number {} bit(s)",
                                val,
                                kind_str,
                                bits
                            ),
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

fn is_valid_xs_time(clean: &str) -> bool {
    let time_no_tz = if let Some(stripped) = clean.strip_suffix('Z') {
        stripped
    } else if let Some(idx) = clean.rfind('+').or_else(|| clean.rfind('-')) {
        let (t, tz) = clean.split_at(idx);
        let tz_digits = &tz[1..];
        let mut tz_iter = tz_digits.split(':');
        let Some(tz_h_str) = tz_iter.next() else { return false; };
        let Some(tz_m_str) = tz_iter.next() else { return false; };
        if tz_iter.next().is_some() || tz_h_str.len() != 2 || tz_m_str.len() != 2 {
            return false;
        }
        let (Some(tz_h), Some(tz_m)) = (tz_h_str.parse::<u32>().ok(), tz_m_str.parse::<u32>().ok()) else {
            return false;
        };
        if tz_h > 14 || tz_m > 59 || (tz_h == 14 && tz_m != 0) {
            return false;
        }
        t
    } else {
        clean
    };

    let (time_main, frac) = if let Some((tm, f)) = time_no_tz.split_once('.') {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        (tm, Some(f))
    } else {
        (time_no_tz, None)
    };

    let mut parts = time_main.split(':');
    let Some(h_str) = parts.next() else { return false; };
    let Some(m_str) = parts.next() else { return false; };
    let Some(s_str) = parts.next() else { return false; };
    if parts.next().is_some() {
        return false;
    }
    if h_str.len() != 2 || m_str.len() != 2 || s_str.len() != 2 {
        return false;
    }
    let (Some(h), Some(m), Some(s)) = (
        h_str.parse::<u32>().ok(),
        m_str.parse::<u32>().ok(),
        s_str.parse::<u32>().ok(),
    ) else {
        return false;
    };

    if h == 24 {
        m == 0 && s == 0 && frac.is_none_or(|f| f.chars().all(|c| c == '0'))
    } else {
        h < 24 && m < 60 && s < 60
    }
}

fn is_valid_xs_date(clean: &str) -> bool {
    let date_no_tz = if let Some(stripped) = clean.strip_suffix('Z') {
        stripped
    } else if let Some(idx) = clean.rfind('+').or_else(|| {
        let mut dash_count: usize = 0;
        let mut tz_dash_idx = None;
        for (i, c) in clean.char_indices() {
            if c == '-' {
                dash_count = dash_count.saturating_add(1);
                let min_dashes = if clean.starts_with('-') { 3 } else { 2 };
                if dash_count > min_dashes {
                    tz_dash_idx = Some(i);
                    break;
                }
            }
        }
        tz_dash_idx
    }) {
        let (d, tz) = clean.split_at(idx);
        let tz_digits = &tz[1..];
        let mut tz_iter = tz_digits.split(':');
        let Some(tz_h_str) = tz_iter.next() else { return false; };
        let Some(tz_m_str) = tz_iter.next() else { return false; };
        if tz_iter.next().is_some() || tz_h_str.len() != 2 || tz_m_str.len() != 2 {
            return false;
        }
        let (Some(tz_h), Some(tz_m)) = (tz_h_str.parse::<u32>().ok(), tz_m_str.parse::<u32>().ok()) else {
            return false;
        };
        if tz_h > 14 || tz_m > 59 || (tz_h == 14 && tz_m != 0) {
            return false;
        }
        d
    } else {
        clean
    };

    let is_neg = date_no_tz.starts_with('-');
    let abs_date = if is_neg { &date_no_tz[1..] } else { date_no_tz };
    let mut parts = abs_date.split('-');
    let Some(y_str) = parts.next() else { return false; };
    let Some(m_str) = parts.next() else { return false; };
    let Some(d_str) = parts.next() else { return false; };
    if parts.next().is_some() {
        return false;
    }
    if y_str.len() < 4 || m_str.len() != 2 || d_str.len() != 2 {
        return false;
    }
    let (Some(y), Some(m), Some(d)) = (
        y_str.parse::<i64>().ok(),
        m_str.parse::<u32>().ok(),
        d_str.parse::<u32>().ok(),
    ) else {
        return false;
    };
    if y == 0 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    let eff_y = if is_neg { y.saturating_neg() } else { y };
    let is_leap = (eff_y % 4 == 0 && eff_y % 100 != 0) || (eff_y % 400 == 0);
    let max_days = match m {
        2 => if is_leap { 29 } else { 28 },
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    d <= max_days
}

fn is_valid_xs_date_time(clean: &str) -> bool {
    if let Some((d, t)) = clean.split_once('T') {
        is_valid_xs_date(d) && is_valid_xs_time(t)
    } else {
        false
    }
}

/// Measures the exact value length in bits of an infoset element according to schema (§17).
///
/// For complex elements, unparses children with their alignment and formatting, but excludes
/// outer initiator, terminator, and outer framing.
/// For simple elements, unparses the value without framing.
pub fn measure_element_content_bits(
    schema: &CompiledSchema,
    doc: Option<&InfosetDocument>,
    elem: &InfosetElement,
    path: &crate::types::InfosetPath,
    vmap: Option<&crate::expr::variables::VariableMap>,
) -> DFDLResult<usize> {
    let term = schema.find_term_by_path(path).or_else(|| {
        schema.find_term_by_name(&elem.name.local_name).and_then(|id| schema.get_term(id))
    }).ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Term not found for measurement")
    })?;

    let sink = VecByteSink::new();
    let mut writer = BitWriter::new(sink, term.properties.bit_order, term.properties.byte_order);
    let mut budget = WorkBudget::new(usize::MAX);
    let mut unparser = UnparserEngine::new(schema, &mut writer, &mut budget);
    if let Some(vm) = vmap {
        unparser.active_variable_map = Some(vm.clone());
    }
    unparser.doc = doc;
    unparser.base_path = path.clone();
    unparser.current_path = path.clone();

    if let TermKind::Element(ref el) = term.kind {
        match el.type_ir {
            crate::schema::ir::CompiledType::Complex(child_id) => {
                unparser.unparse_term(child_id, elem)?;
            }
            crate::schema::ir::CompiledType::Simple(_) => {
                if let ElementState::Value(ref val) = elem.state {
                    unparser.unparse_simple_value(val, &term.properties)?;
                }
            }
        }
    }
    Ok(writer.position().0)
}

impl<'a, S: ByteSink> UnparserEngine<'a, S> {
    pub(crate) fn unparse_simple_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let remapped_val;
        let val = if let DfdlValue::String(ref s) = val {
            if s.chars().any(|c| {
                (0xE000..=0xE01F).contains(&(c as u32))
                    || c == '\u{E07F}'
                    || (0xE080..=0xE09F).contains(&(c as u32))
            }) {
                remapped_val = DfdlValue::String(remap_pua_to_raw_chars(s));
                &remapped_val
            } else {
                val
            }
        } else {
            val
        };

        if props.length_kind == crate::schema::ir::LengthKind::Prefixed {
            let val_len = match val {
                DfdlValue::String(s) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => s.len().saturating_mul(8),
                    _ => s.len(),
                },
                DfdlValue::HexBinary(b) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => b.len().saturating_mul(8),
                    _ => b.len(),
                },
                DfdlValue::DateTime(s)
                | DfdlValue::Date(s)
                | DfdlValue::Time(s)
                | DfdlValue::Decimal(s) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => s.len().saturating_mul(8),
                    _ => s.len(),
                },
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) | DfdlValue::Float(_) | DfdlValue::Boolean(_) => {
                    match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => 32,
                        _ => 4,
                    }
                }
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) | DfdlValue::Double(_) => {
                    match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => 64,
                        _ => 8,
                    }
                }
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => 16,
                    _ => 2,
                },
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => 8,
                    _ => 1,
                },
            };
            self.write_prefix_length(val_len, props)?;
        }

        match props.representation {
            Representation::Binary => self.unparse_binary_value(val, props),
            Representation::Text => {
                // DFDL 1.0 §13.7: Raw binary data (xs:hexBinary or blob bytes) must always
                // be unparsed using binary byte output even if representation defaulted to text.
                if let DfdlValue::HexBinary(_) = val {
                    self.unparse_binary_value(val, props)
                } else {
                    self.unparse_text_value(val, props)
                }
            }
        }
    }

    pub(crate) fn resolve_explicit_length(&mut self, props: &ResolvedProperties) -> DFDLResult<Option<usize>> {
        if let Some(ref prop) = props.length_prop {
            match prop {
                crate::schema::ir::DfdlProp::Constant(l) => return Ok(Some(*l)),
                crate::schema::ir::DfdlProp::Expression { ast, .. } => {
                    let mut ctx = self.make_expr_context();
                    let val = crate::expr::eval_expr(ast, &mut ctx)?;
                    let len_opt = match val {
                        DfdlValue::Int(v) if v >= 0 => Some(v as usize),
                        DfdlValue::Long(v) if v >= 0 => Some(v as usize),
                        DfdlValue::UnsignedInt(v) => Some(v as usize),
                        DfdlValue::UnsignedLong(v) => Some(v as usize),
                        DfdlValue::UnsignedShort(v) => Some(v as usize),
                        DfdlValue::UnsignedByte(v) => Some(v as usize),
                        DfdlValue::Short(v) if v >= 0 => Some(v as usize),
                        DfdlValue::Byte(v) if v >= 0 => Some(v as usize),
                        DfdlValue::String(s) => s.trim().parse::<usize>().ok(),
                        _ => None,
                    };
                    if len_opt.is_some() {
                        return Ok(len_opt);
                    }
                }
            }
        } else if let Some(l) = props.length {
            return Ok(Some(l));
        } else if let Some(ref expr_str) = props.length_expr {
            if let Ok(ast) = crate::expr::parse_expr(expr_str) {
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                let len_opt = match val {
                    DfdlValue::Int(v) if v >= 0 => Some(v as usize),
                    DfdlValue::Long(v) if v >= 0 => Some(v as usize),
                    DfdlValue::UnsignedInt(v) => Some(v as usize),
                    DfdlValue::UnsignedLong(v) => Some(v as usize),
                    DfdlValue::UnsignedShort(v) => Some(v as usize),
                    DfdlValue::UnsignedByte(v) => Some(v as usize),
                    DfdlValue::Short(v) if v >= 0 => Some(v as usize),
                    DfdlValue::Byte(v) if v >= 0 => Some(v as usize),
                    DfdlValue::String(s) => s.trim().parse::<usize>().ok(),
                    _ => None,
                };
                if len_opt.is_some() {
                    return Ok(len_opt);
                }
            }
            let inner = expr_str.trim();
            let stripped = if inner.starts_with('{') && inner.ends_with('}') {
                &inner[1..inner.len().saturating_sub(1)]
            } else {
                inner
            };
            if let Ok(val) = stripped.trim().parse::<usize>() {
                return Ok(Some(val));
            }
        }
        Ok(None)
    }

    pub(crate) fn unparse_binary_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let explicit_len_opt = self.resolve_explicit_length(props)?;
        let requested_bits_opt = explicit_len_opt.map(|len| match props.length_units {
            crate::schema::ir::LengthUnits::Bits => len,
            crate::schema::ir::LengthUnits::Bytes | crate::schema::ir::LengthUnits::Characters => {
                len.saturating_mul(8)
            }
        });

        if props.binary_number_rep == crate::schema::ir::BinaryNumberRep::Binary {
            if let Some(bits) = requested_bits_opt {
                let is_unsigned_int = matches!(
                    val,
                    DfdlValue::UnsignedByte(_)
                        | DfdlValue::UnsignedShort(_)
                        | DfdlValue::UnsignedInt(_)
                        | DfdlValue::UnsignedLong(_)
                ) || (matches!(val, DfdlValue::Decimal(_)) && !props.decimal_signed);
                let is_signed_int = matches!(
                    val,
                    DfdlValue::Byte(_)
                        | DfdlValue::Short(_)
                        | DfdlValue::Int(_)
                        | DfdlValue::Long(_)
                ) || (matches!(val, DfdlValue::Decimal(_)) && props.decimal_signed);

                if is_unsigned_int && bits == 0 {
                    let msg = alloc::format!(
                        "Unparse Error: unsigned binary number: minimum length is 1 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }

                if is_signed_int
                    && bits < 2
                    && (bits == 0 || self.schema.disallow_signed_integer_length_1bit)
                {
                    let msg = alloc::format!(
                        "Unparse Error: signed binary number: minimum length is 2 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }

                let (min_bits, max_bits, type_name) = match val {
                    DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => (1, 8, "Byte"),
                    DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => (1, 16, "Short"),
                    DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => (1, 32, "Int"),
                    DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => (1, 64, "Long"),
                    _ => (0, 0, ""),
                };
                if max_bits > 0 && (bits < min_bits || bits > max_bits) {
                    let msg = alloc::format!(
                        "Schema Definition Error: Length in bits {} out of range. Expected between {} and {} for {}",
                        bits,
                        min_bits,
                        max_bits,
                        type_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        match props.binary_number_rep {
            crate::schema::ir::BinaryNumberRep::Packed => {
                let val_i64 = match val {
                    DfdlValue::Int(v) => *v as i64,
                    DfdlValue::Long(v) => *v,
                    DfdlValue::Short(v) => *v as i64,
                    DfdlValue::Byte(v) => *v as i64,
                    DfdlValue::UnsignedInt(v) => *v as i64,
                    DfdlValue::UnsignedLong(v) => *v as i64,
                    DfdlValue::UnsignedShort(v) => *v as i64,
                    DfdlValue::UnsignedByte(v) => *v as i64,
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..scale.unsigned_abs() {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                        rounded as i64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as packed decimal", val),
                        ));
                    }
                };
                let bytes = crate::util::encode_packed_decimal(val_i64);
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let final_bytes = if let Some(target) = min_bytes {
                    if bytes.len() < target {
                        let pad = target.saturating_sub(bytes.len());
                        let mut padded = alloc::vec![0u8; pad];
                        padded.extend_from_slice(&bytes);
                        padded
                    } else {
                        bytes
                    }
                } else {
                    bytes
                };
                self.writer.write_bytes(&final_bytes)?;
                return Ok(());
            }
            crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                let val_i64 = match val {
                    DfdlValue::Int(v) => *v as i64,
                    DfdlValue::Long(v) => *v,
                    DfdlValue::Short(v) => *v as i64,
                    DfdlValue::Byte(v) => *v as i64,
                    DfdlValue::UnsignedInt(v) => *v as i64,
                    DfdlValue::UnsignedLong(v) => *v as i64,
                    DfdlValue::UnsignedShort(v) => *v as i64,
                    DfdlValue::UnsignedByte(v) => *v as i64,
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..scale.unsigned_abs() {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                        rounded as i64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as ibm4690 packed decimal", val),
                        ));
                    }
                };
                let bytes = crate::util::encode_ibm4690_packed(val_i64);
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let final_bytes = if let Some(target) = min_bytes {
                    if bytes.len() < target {
                        let pad = target.saturating_sub(bytes.len());
                        let mut padded = alloc::vec![0xFFu8; pad];
                        padded.extend_from_slice(&bytes);
                        padded
                    } else {
                        bytes
                    }
                } else {
                    bytes
                };
                self.writer.write_bytes(&final_bytes)?;
                return Ok(());
            }
            crate::schema::ir::BinaryNumberRep::Bcd => {
                let val_u64 = match val {
                    DfdlValue::UnsignedInt(v) => *v as u64,
                    DfdlValue::UnsignedLong(v) => *v,
                    DfdlValue::UnsignedShort(v) => *v as u64,
                    DfdlValue::UnsignedByte(v) => *v as u64,
                    DfdlValue::Int(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Long(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Short(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Byte(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        if parsed < 0.0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    parsed
                                ),
                            ));
                        }
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..scale.unsigned_abs() {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = prod + 0.5;
                        rounded as u64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as bcd", val),
                        ));
                    }
                };
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let bytes = crate::util::encode_bcd(val_u64, min_bytes)?;
                self.writer.write_bytes(&bytes)?;
                return Ok(());
            }
            _ => {}
        }

        let needs_byte_swap = props.byte_order == ByteOrder::LittleEndian
            && props.bit_order == BitOrder::MostSignificantBitFirst;

        match val {
            DfdlValue::Int(v) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let raw_u64 = (*v as u32) as u64;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Long(v) => {
                let bits = requested_bits_opt.unwrap_or(
                    if *v >= (i32::MIN as i64) && *v <= (u32::MAX as i64) {
                        32
                    } else {
                        64
                    },
                );
                let raw_u64 = *v as u64;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else if bits == 32 {
                    raw_u64 & 0xFFFF_FFFF
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Short(v) => {
                let bits = requested_bits_opt.unwrap_or(16);
                let raw_u64 = (*v as u16) as u64;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Byte(v) => {
                let bits = requested_bits_opt.unwrap_or(8);
                self.writer.write_bits((*v as u8) as u64, bits)
            }
            DfdlValue::UnsignedInt(v) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let raw_u64 = *v as u64;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedLong(v) => {
                let bits = requested_bits_opt.unwrap_or(64);
                let raw_u64 = *v;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedShort(v) => {
                let bits = requested_bits_opt.unwrap_or(16);
                let raw_u64 = *v as u64;
                let val_u64 = if needs_byte_swap {
                    crate::util::order_integer_bytes(raw_u64, bits, ByteOrder::LittleEndian)
                } else {
                    raw_u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedByte(v) => {
                let bits = requested_bits_opt.unwrap_or(8);
                self.writer.write_bits(*v as u64, bits)
            }
            DfdlValue::Boolean(b) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let mask = if bits >= 64 {
                    u64::MAX
                } else {
                    (1u64 << bits).wrapping_sub(1)
                };
                let raw_val: u64 = if *b {
                    match props.binary_boolean_true_rep {
                        crate::schema::ir::BinaryBooleanRep::Value(t) => (t as u64) & mask,
                        crate::schema::ir::BinaryBooleanRep::Empty => {
                            match props.binary_boolean_false_rep {
                                crate::schema::ir::BinaryBooleanRep::Value(f) => (!f as u64) & mask,
                                _ => 1 & mask,
                            }
                        }
                        crate::schema::ir::BinaryBooleanRep::NotSpecified => 1 & mask,
                    }
                } else {
                    match props.binary_boolean_false_rep {
                        crate::schema::ir::BinaryBooleanRep::Value(f) => (f as u64) & mask,
                        crate::schema::ir::BinaryBooleanRep::Empty => {
                            match props.binary_boolean_true_rep {
                                crate::schema::ir::BinaryBooleanRep::Value(t) => (!t as u64) & mask,
                                _ => 0,
                            }
                        }
                        crate::schema::ir::BinaryBooleanRep::NotSpecified => 0,
                    }
                };
                let ordered = if needs_byte_swap {
                    match bits {
                        16 => (raw_val as u16).swap_bytes() as u64,
                        24 => {
                            let b0 = raw_val & 0xFF;
                            let b1 = (raw_val >> 8) & 0xFF;
                            let b2 = (raw_val >> 16) & 0xFF;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (raw_val as u32).swap_bytes() as u64,
                        64 => raw_val.swap_bytes(),
                        _ => raw_val,
                    }
                } else {
                    raw_val
                };
                self.writer.write_bits(ordered, bits)
            }
            DfdlValue::Float(f) => {
                let bits = if needs_byte_swap {
                    f.to_bits().swap_bytes()
                } else {
                    f.to_bits()
                };
                self.writer.write_bits(bits as u64, 32)
            }
            DfdlValue::Double(d) => {
                let bits = if needs_byte_swap {
                    d.to_bits().swap_bytes()
                } else {
                    d.to_bits()
                };
                self.writer.write_bits(bits, 64)
            }
            DfdlValue::HexBinary(b) => {
                let fill = fill_byte_value(props)?;
                let target_bytes = if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    self.resolve_explicit_length(props)?.map(|target_len| match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                        _ => target_len,
                    })
                } else if props.length_kind == crate::schema::ir::LengthKind::Implicit {
                    props
                        .facets
                        .length
                        .or(props.facets.max_length)
                        .or(props.facets.min_length)
                } else if props.length_kind == crate::schema::ir::LengthKind::Delimited {
                    props.facets.min_length
                } else {
                    None
                };
                if let Some(target) = target_bytes {
                    if b.len() > target {
                        let actual_bits = b.len().saturating_mul(8);
                        let target_bits = match (props.length_kind, props.length_units) {
                            (crate::schema::ir::LengthKind::Explicit, crate::schema::ir::LengthUnits::Bits) => {
                                self.resolve_explicit_length(props)?.unwrap_or(target.saturating_mul(8))
                            }
                            _ => target.saturating_mul(8),
                        };
                        let msg = alloc::format!(
                            "Unparse Error: Blob length {} bits exceeds {} bits",
                            actual_bits, target_bits
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                    }
                    for &byte in b {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                    if b.len() < target {
                        for _ in 0..(target.saturating_sub(b.len())) {
                            self.writer.write_bits(fill as u64, 8)?;
                        }
                    }
                } else {
                    for &byte in b {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                }
                Ok(())
            }
            DfdlValue::String(s) => {
                let clean = s.trim();
                if clean.len() % 2 == 0
                    && !clean.is_empty()
                    && clean.chars().all(|c| c.is_ascii_hexdigit())
                {
                    let b_arr = clean.as_bytes();
                    for i in (0..b_arr.len()).step_by(2) {
                        if let (Some(&b1), Some(&b2)) =
                            (b_arr.get(i), b_arr.get(i.saturating_add(1)))
                        {
                            if let (Some(h1), Some(h2)) =
                                ((b1 as char).to_digit(16), (b2 as char).to_digit(16))
                            {
                                self.writer.write_bits(((h1 << 4) | h2) as u64, 8)?;
                            }
                        }
                    }
                    Ok(())
                } else if let Ok(val_i64) = clean.parse::<i64>() {
                    let bits = requested_bits_opt.unwrap_or(32);
                    let val_u64 = match props.byte_order {
                        ByteOrder::BigEndian => {
                            (val_i64 as u64)
                                & (if bits == 64 {
                                    u64::MAX
                                } else {
                                    (1u64 << bits).wrapping_sub(1)
                                })
                        }
                        ByteOrder::LittleEndian => match bits {
                            32 => (val_i64 as u32).swap_bytes() as u64,
                            64 => (val_i64 as u64).swap_bytes(),
                            _ => val_i64 as u64,
                        },
                    };
                    self.writer.write_bits(val_u64, bits)
                } else {
                    for &byte in s.as_bytes() {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                    Ok(())
                }
            }
            DfdlValue::Decimal(s) => {
                let parsed = s.parse::<f64>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Invalid decimal: {}", s),
                    )
                })?;
                let scale = props.binary_decimal_virtual_point;
                let mut multiplier = 1.0f64;
                if scale >= 0 {
                    for _ in 0..scale {
                        multiplier *= 10.0;
                    }
                } else {
                    for _ in 0..scale.unsigned_abs() {
                        multiplier /= 10.0;
                    }
                }
                let prod = parsed * multiplier;
                let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                let val_i64 = rounded as i64;
                let bits = requested_bits_opt.unwrap_or(32);
                let raw_u64 = if bits >= 64 {
                    val_i64 as u64
                } else {
                    (val_i64 as u64) & ((1u64 << bits).wrapping_sub(1))
                };
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => raw_u64,
                    ByteOrder::LittleEndian => match bits {
                        16 => (raw_u64 as u16).swap_bytes() as u64,
                        24 => {
                            let b0 = raw_u64 & 0xFF;
                            let b1 = (raw_u64 >> 8) & 0xFF;
                            let b2 = (raw_u64 >> 16) & 0xFF;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (raw_u64 as u32).swap_bytes() as u64,
                        64 => raw_u64.swap_bytes(),
                        _ => raw_u64,
                    },
                };
                self.writer.write_bits(ordered, bits)
            }
            DfdlValue::DateTime(s)
            | DfdlValue::Date(s)
            | DfdlValue::Time(s) => {
                self.unparse_binary_calendar(s, props)
            }
        }
    }

    pub(crate) fn unparse_binary_calendar(
        &mut self,
        s_val: &str,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let requested_bytes = self.resolve_explicit_length(props)?.map(|len| match props.length_units {
            crate::schema::ir::LengthUnits::Bits => len.saturating_add(7) / 8,
            _ => len,
        });

        match props.binary_calendar_rep {
            crate::schema::ir::BinaryCalendarRep::Bcd
            | crate::schema::ir::BinaryCalendarRep::Ibm4690Packed => {
                let mut stack_digits = [0u8; 64];
                let heap_digits: Vec<u8>;
                let digits: &[u8] = if props.calendar_pattern.as_ref().map_or(true, |p| p.is_empty()) {
                    let mut n = 0;
                    for b in s_val.bytes() {
                        if b.is_ascii_digit() && n < 64 {
                            stack_digits[n] = b.saturating_sub(b'0');
                            n = n.saturating_add(1);
                        }
                    }
                    &stack_digits[..n]
                } else {
                    let pat = props.calendar_pattern.as_deref().unwrap_or("");
                    let dyn_lang;
                    let lang_ref: Option<&str> = match props.calendar_language_prop.as_ref() {
                        Some(crate::schema::ir::DfdlProp::Constant(c)) => Some(c.as_str()),
                        Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                            let mut ctx = self.make_expr_context();
                            dyn_lang = crate::expr::eval_expr(ast, &mut ctx).ok().map(|v| alloc::format!("{}", v));
                            dyn_lang.as_deref()
                        }
                        None => props.calendar_language.as_deref(),
                    };
                    let dyn_tz;
                    let tz_ref: Option<&str> = match props.calendar_time_zone_prop.as_ref() {
                        Some(crate::schema::ir::DfdlProp::Constant(c)) => Some(c.as_str()),
                        Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                            let mut ctx = self.make_expr_context();
                            dyn_tz = crate::expr::eval_expr(ast, &mut ctx).ok().map(|v| alloc::format!("{}", v));
                            dyn_tz.as_deref()
                        }
                        None => props.calendar_time_zone.as_deref(),
                    };
                    // Fast-path: When calendarPattern is standard time format "hhmmss" or "HHmmss"
                    // and s_val is a standard XML time "hh:mm:ss" without tz/language override,
                    // the BCD digits are simply the decimal digits of s_val in order.
                    // This bypasses format_calendar_with_pattern and avoids String allocation.
                    if matches!(pat, "hhmmss" | "HHmmss")
                        && lang_ref.is_none()
                        && tz_ref.is_none()
                        && s_val.len() >= 8
                        && s_val.as_bytes().get(2) == Some(&b':')
                        && s_val.as_bytes().get(5) == Some(&b':')
                    {
                        let mut n = 0;
                        for &b in &s_val.as_bytes()[..8] {
                            if b.is_ascii_digit() && n < 64 {
                                stack_digits[n] = b.saturating_sub(b'0');
                                n = n.saturating_add(1);
                            }
                        }
                        &stack_digits[..n]
                    } else {
                        let formatted = crate::kernel::parser::calendar::format_calendar_with_pattern(
                            s_val,
                            pat,
                            lang_ref,
                            tz_ref,
                        )?;
                        let mut n = 0;
                        for b in formatted.bytes() {
                            if b.is_ascii_digit() && n < 64 {
                                stack_digits[n] = b.saturating_sub(b'0');
                                n = n.saturating_add(1);
                            }
                        }
                        if n <= 64 && formatted.bytes().filter(|b| b.is_ascii_digit()).count() <= 64 {
                            &stack_digits[..n]
                        } else {
                            heap_digits = formatted
                                .bytes()
                                .filter(|b| b.is_ascii_digit())
                                .map(|b| b.saturating_sub(b'0'))
                                .collect();
                            &heap_digits[..]
                        }
                    }
                };

                let pad_count = if let Some(target_b) = requested_bytes {
                    let target_digits = target_b.saturating_mul(2);
                    target_digits.saturating_sub(digits.len())
                } else if !digits.len().is_multiple_of(2) {
                    1
                } else {
                    0
                };

                let total_digits = pad_count.saturating_add(digits.len());
                let mut packed_bytes = [0u8; 32];
                let mut byte_count = 0;
                for chunk_idx in (0..total_digits).step_by(2) {
                    let get_digit = |i: usize| -> u8 {
                        if i < pad_count {
                            0
                        } else {
                            digits.get(i.saturating_sub(pad_count)).copied().unwrap_or(0)
                        }
                    };
                    let high = get_digit(chunk_idx);
                    let low = get_digit(chunk_idx.saturating_add(1));
                    if byte_count < packed_bytes.len() {
                        packed_bytes[byte_count] = (high << 4) | (low & 0x0F);
                        byte_count = byte_count.saturating_add(1);
                    }
                }
                self.writer.write_bytes(&packed_bytes[..byte_count])?;
                Ok(())
            }
            crate::schema::ir::BinaryCalendarRep::Packed => {
                let mut stack_digits = [0u8; 64];
                let heap_digits: Vec<u8>;
                let digits: &[u8] = if props.calendar_pattern.as_ref().map_or(true, |p| p.is_empty()) {
                    let mut n = 0;
                    for b in s_val.bytes() {
                        if b.is_ascii_digit() && n < 63 {
                            stack_digits[n] = b.saturating_sub(b'0');
                            n = n.saturating_add(1);
                        }
                    }
                    stack_digits[n] = 0x0C;
                    n = n.saturating_add(1);
                    &stack_digits[..n]
                } else {
                    let pat = props.calendar_pattern.as_deref().unwrap_or("");
                    let lang = props.calendar_language.as_deref();
                    let tz = props.calendar_time_zone.as_deref();
                    let formatted = crate::kernel::parser::calendar::format_calendar_with_pattern(
                        s_val,
                        pat,
                        lang,
                        tz,
                    )?;
                    let mut n = 0;
                    for b in formatted.bytes() {
                        if b.is_ascii_digit() && n < 63 {
                            stack_digits[n] = b.saturating_sub(b'0');
                            n = n.saturating_add(1);
                        }
                    }
                    if n < 63 && formatted.bytes().filter(|b| b.is_ascii_digit()).count() <= 63 {
                        stack_digits[n] = 0x0C;
                        n = n.saturating_add(1);
                        &stack_digits[..n]
                    } else {
                        let mut d: Vec<u8> = formatted
                            .bytes()
                            .filter(|b| b.is_ascii_digit())
                            .map(|b| b.saturating_sub(b'0'))
                            .collect();
                        d.push(0x0C);
                        heap_digits = d;
                        &heap_digits[..]
                    }
                };
                let pad_count = if let Some(target_b) = requested_bytes {
                    let target_digits = target_b.saturating_mul(2);
                    target_digits.saturating_sub(digits.len())
                } else if !digits.len().is_multiple_of(2) {
                    1
                } else {
                    0
                };

                let total_digits = pad_count.saturating_add(digits.len());
                for chunk_idx in (0..total_digits).step_by(2) {
                    let get_digit = |i: usize| -> u8 {
                        if i < pad_count {
                            0
                        } else {
                            digits.get(i.saturating_sub(pad_count)).copied().unwrap_or(0)
                        }
                    };
                    let high = get_digit(chunk_idx);
                    let low = get_digit(chunk_idx.saturating_add(1));
                    let b = (high << 4) | (low & 0x0F);
                    self.writer.write_bits(b as u64, 8)?;
                }
                Ok(())
            }
            crate::schema::ir::BinaryCalendarRep::BinarySeconds
            | crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds => {
                if self.writer.is_byte_aligned() {
                    self.writer.write_bytes(s_val.as_bytes())?;
                } else {
                    for &byte in s_val.as_bytes() {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                }
                Ok(())
            }
        }
    }

    pub(crate) fn unparse_text_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let mut s: alloc::borrow::Cow<'_, str> = match val {
            DfdlValue::String(ref s_val) => alloc::borrow::Cow::Borrowed(s_val.as_str()),
            DfdlValue::Boolean(b) => {
                let eval_rep_opt = if *b {
                    match props.text_boolean_true_rep_prop.as_ref() {
                        Some(crate::schema::ir::DfdlProp::Constant(c)) => Some(c.clone()),
                        Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                            let mut ctx = self.make_expr_context();
                            crate::expr::eval_expr(ast, &mut ctx).ok().map(|v| alloc::format!("{}", v))
                        }
                        None => props.text_boolean_true_rep.clone(),
                    }
                } else {
                    match props.text_boolean_false_rep_prop.as_ref() {
                        Some(crate::schema::ir::DfdlProp::Constant(c)) => Some(c.clone()),
                        Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                            let mut ctx = self.make_expr_context();
                            crate::expr::eval_expr(ast, &mut ctx).ok().map(|v| alloc::format!("{}", v))
                        }
                        None => props.text_boolean_false_rep.clone(),
                    }
                };
                if let Some(eval_rep) = eval_rep_opt {
                    let first_alt = eval_rep.split_whitespace().next().unwrap_or("").to_string();
                    alloc::borrow::Cow::Owned(crate::expr::properties::decode_dfdl_character_entities(&first_alt))
                } else {
                    alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val))
                }
            }
            _ if props.text_standard_base != 10 => {
                if let Some(n) = val.as_i128() {
                    if n < 0 {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                                n,
                                props.text_standard_base
                            ),
                        ));
                    }
                    match props.text_standard_base {
                        2 => alloc::borrow::Cow::Owned(alloc::format!("{:b}", n)),
                        8 => alloc::borrow::Cow::Owned(alloc::format!("{:o}", n)),
                        16 => alloc::borrow::Cow::Owned(alloc::format!("{:x}", n)),
                        _ => alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val)),
                    }
                } else if let DfdlValue::String(ref s_val) = val {
                    let trimmed = s_val.trim();
                    if let Ok(n) = trimmed.parse::<i128>() {
                        if n < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                                    n,
                                    props.text_standard_base
                                ),
                            ));
                        }
                        match props.text_standard_base {
                            2 => alloc::borrow::Cow::Owned(alloc::format!("{:b}", n)),
                            8 => alloc::borrow::Cow::Owned(alloc::format!("{:o}", n)),
                            16 => alloc::borrow::Cow::Owned(alloc::format!("{:x}", n)),
                            _ => alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val)),
                        }
                    } else {
                        alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val))
                    }
                } else {
                    alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val))
                }
            }
            DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_)
            | DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_) => {
                let zero_rep_opt = if crate::kernel::parser::numbers::is_numeric_zero(val) {
                    props.text_standard_zero_rep.as_deref().and_then(|raw_zero| {
                        let eval_rep = self.eval_runtime_prop_str(raw_zero);
                        eval_rep.split_whitespace().next().and_then(|first| {
                            if !first.is_empty() {
                                Some(crate::expr::properties::decode_dfdl_character_entities(first))
                            } else {
                                None
                            }
                        })
                    })
                } else {
                    None
                };
                if let Some(z) = zero_rep_opt {
                    alloc::borrow::Cow::Owned(z)
                } else {
                    let dec = self.eval_runtime_prop_str(&props.text_standard_decimal_separator);
                    let grp = self.eval_runtime_prop_str(&props.text_standard_grouping_separator);
                    let exp = props
                        .text_standard_exponent_rep
                        .as_deref()
                        .map(|s| self.eval_runtime_prop_str(s));
                    alloc::borrow::Cow::Owned(crate::kernel::parser::numbers::format_text_number(
                        val,
                        props.text_number_pattern.as_deref(),
                        &dec,
                        &grp,
                        exp.as_deref(),
                        crate::kernel::parser::rounding::NumberRounding::from_props(props),
                    ))
                }
            }
            DfdlValue::DateTime(s_val) | DfdlValue::Date(s_val) | DfdlValue::Time(s_val) => {
                if let Some(ref pat) = props.calendar_pattern {
                    if !pat.is_empty() {
                        let lang = match props.calendar_language_prop.as_ref() {
                            Some(crate::schema::ir::DfdlProp::Constant(c)) => {
                                crate::kernel::parser::calendar::validate_calendar_language_syntax(c)?;
                                Some(c.clone())
                            }
                            Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                                let mut ctx = self.make_expr_context();
                                let v = crate::expr::eval_expr(ast, &mut ctx)?;
                                let s = alloc::format!("{}", v);
                                crate::kernel::parser::calendar::validate_calendar_language_syntax(&s)?;
                                Some(s)
                            }
                            None => if let Some(ref raw_lang) = props.calendar_language {
                                crate::kernel::parser::calendar::validate_calendar_language_syntax(raw_lang)?;
                                Some(raw_lang.clone())
                            } else {
                                None
                            },
                        };
                        let dyn_tz;
                        let tz_ref: Option<&str> = match props.calendar_time_zone_prop.as_ref() {
                            Some(crate::schema::ir::DfdlProp::Constant(c)) => Some(c.as_str()),
                            Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                                let mut ctx = self.make_expr_context();
                                dyn_tz = crate::expr::eval_expr(ast, &mut ctx).ok().map(|v| alloc::format!("{}", v));
                                dyn_tz.as_deref()
                            }
                            None => props.calendar_time_zone.as_deref(),
                        };
                        alloc::borrow::Cow::Owned(crate::kernel::parser::calendar::format_calendar_with_pattern(
                            s_val,
                            pat,
                            lang.as_deref(),
                            tz_ref,
                        )?)
                    } else {
                        alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val))
                    }
                } else {
                    alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val))
                }
            }
            _ => alloc::borrow::Cow::Owned(alloc::string::ToString::to_string(val)),
        };
        if s.chars().any(|c| {
            (0xE000..=0xE01F).contains(&(c as u32))
                || c == '\u{E07F}'
                || (0xE080..=0xE09F).contains(&(c as u32))
        }) {
            s = alloc::borrow::Cow::Owned(remap_pua_to_raw_chars(&s));
        }
        if let Some(cb) = crate::encoding::encoding_char_bits(&props.encoding) {
            return self.unparse_sub_byte_text(&s, val, props, cb);
        }
        if props.encoding_error_policy_error {
            if let Some(ch) = crate::encoding::find_unmappable_char(&s, &props.encoding) {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: UnmappableCharacterException: character '{}' (U+{:04X}) cannot be encoded in {}",
                        ch,
                        ch as u32,
                        props.encoding
                    ),
                ));
            }
        }
        validate_text_number_rounding(val, props)?;

        if let Some(ref scheme) = props.escape_scheme {
            s = alloc::borrow::Cow::Owned(self.apply_escape_scheme(&s, scheme));
        }

        let pad_str = match val {
            DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_)
            | DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_) => props
                .text_number_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            DfdlValue::Boolean(_) => props
                .text_boolean_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            DfdlValue::DateTime(_) | DfdlValue::Date(_) | DfdlValue::Time(_) => props
                .text_calendar_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            _ => props.text_pad_char.as_str(),
        };
        let mut pad_ch = pad_str.chars().next().unwrap_or(' ');
        let mut pad_byte = pad_str.as_bytes().first().copied().unwrap_or(b' ');
        if props.text_pad_kind == crate::schema::ir::TextPadKind::None && props.fill_byte_defined {
            pad_byte = props.fill_byte;
            pad_ch = props.fill_byte as char;
        }

        let is_chars = props.length_units == crate::schema::ir::LengthUnits::Characters;
        let justification = props.text_justification_for_value(val);

        if props.length_kind == crate::schema::ir::LengthKind::Delimited
            && props.text_pad_kind == crate::schema::ir::TextPadKind::PadChar
            && props.text_output_min_length > 0
        {
            let target_len = props.text_output_min_length;
            if is_chars {
                let char_count = s.chars().count();
                if char_count < target_len {
                    let pad_count = target_len.saturating_sub(char_count);
                    let mut padded = String::with_capacity(s.len().saturating_add(pad_count.saturating_mul(4)));
                    match justification {
                        crate::schema::ir::TextJustification::Right => {
                            for _ in 0..pad_count {
                                padded.push(pad_ch);
                            }
                            padded.push_str(&s);
                        }
                        crate::schema::ir::TextJustification::Center => {
                            let pad_left = pad_count.div_ceil(2);
                            let pad_right = pad_count.checked_div(2).unwrap_or(0);
                            for _ in 0..pad_left {
                                padded.push(pad_ch);
                            }
                            padded.push_str(&s);
                            for _ in 0..pad_right {
                                padded.push(pad_ch);
                            }
                        }
                        _ => {
                            if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                            } else {
                                padded.push_str(&s);
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                            }
                        }
                    }
                    s = alloc::borrow::Cow::Owned(padded);
                }
            } else {
                let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                let target_bytes = match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                    _ => target_len,
                };
                if encoded.len() < target_bytes {
                    let pad_count = target_bytes.saturating_sub(encoded.len());
                    let mut padded_bytes = Vec::with_capacity(encoded.len().saturating_add(pad_count));
                    match justification {
                        crate::schema::ir::TextJustification::Right => {
                            for _ in 0..pad_count {
                                padded_bytes.push(pad_byte);
                            }
                            padded_bytes.extend_from_slice(&encoded);
                        }
                        crate::schema::ir::TextJustification::Center => {
                            let pad_left = pad_count.div_ceil(2);
                            let pad_right = pad_count.checked_div(2).unwrap_or(0);
                            for _ in 0..pad_left {
                                padded_bytes.push(pad_byte);
                            }
                            padded_bytes.extend_from_slice(&encoded);
                            for _ in 0..pad_right {
                                padded_bytes.push(pad_byte);
                            }
                        }
                        _ => {
                            if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                for _ in 0..pad_count {
                                    padded_bytes.push(pad_byte);
                                }
                                padded_bytes.extend_from_slice(&encoded);
                            } else {
                                padded_bytes.extend_from_slice(&encoded);
                                for _ in 0..pad_count {
                                    padded_bytes.push(pad_byte);
                                }
                            }
                        }
                    }
                    self.writer.write_bytes(&padded_bytes)?;
                    return Ok(());
                }
            }
        }

        if props.length_kind == crate::schema::ir::LengthKind::Explicit {
            if let Some(target_len) = self.resolve_explicit_length(props)? {
                if is_chars {
                    let char_count = s.chars().count();
                    if char_count < target_len {
                        let pad_count = target_len.saturating_sub(char_count);
                        let mut padded = String::with_capacity(s.len().saturating_add(pad_count.saturating_mul(4)));
                        match justification {
                            crate::schema::ir::TextJustification::Right => {
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                            }
                            crate::schema::ir::TextJustification::Center => {
                                let pad_left = pad_count.div_ceil(2);
                                let pad_right = pad_count.checked_div(2).unwrap_or(0);
                                for _ in 0..pad_left {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                                for _ in 0..pad_right {
                                    padded.push(pad_ch);
                                }
                            }
                            _ => {
                                if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                    for _ in 0..pad_count {
                                        padded.push(pad_ch);
                                    }
                                    padded.push_str(&s);
                                } else {
                                    padded.push_str(&s);
                                    for _ in 0..pad_count {
                                        padded.push(pad_ch);
                                    }
                                }
                            }
                        }
                        let encoded = crate::encoding::encode_text_string(&padded, &props.encoding);
                        self.writer.write_bytes(&encoded)?;
                    } else if char_count > target_len {
                        let can_truncate = props.truncate_specified_length_string;
                        if !can_truncate {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                    char_count,
                                    target_len
                                ),
                            ));
                        }
                        if justification == crate::schema::ir::TextJustification::Center {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                    char_count,
                                    target_len
                                ),
                            ));
                        }
                        let truncated: String = match justification {
                            crate::schema::ir::TextJustification::Right => {
                                let skip = char_count.saturating_sub(target_len);
                                s.chars().skip(skip).collect()
                            }
                            _ => s.chars().take(target_len).collect(),
                        };
                        let encoded = crate::encoding::encode_text_string(&truncated, &props.encoding);
                        self.writer.write_bytes(&encoded)?;
                    } else {
                        let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                        self.writer.write_bytes(&encoded)?;
                    }
                } else {
                    let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                    let target_bytes = match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                        _ => target_len,
                    };
                    if encoded.len() < target_bytes {
                        let pad_count = target_bytes.saturating_sub(encoded.len());
                        match justification {
                            crate::schema::ir::TextJustification::Right => {
                                for _ in 0..pad_count {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                                self.writer.write_bytes(&encoded)?;
                            }
                            crate::schema::ir::TextJustification::Center => {
                                let pad_left = pad_count.div_ceil(2);
                                let pad_right = pad_count.checked_div(2).unwrap_or(0);
                                for _ in 0..pad_left {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                                self.writer.write_bytes(&encoded)?;
                                for _ in 0..pad_right {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                            }
                            _ => {
                                if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                    for _ in 0..pad_count {
                                        self.writer.write_bits(pad_byte as u64, 8)?;
                                    }
                                    self.writer.write_bytes(&encoded)?;
                                } else {
                                    self.writer.write_bytes(&encoded)?;
                                    for _ in 0..pad_count {
                                        self.writer.write_bits(pad_byte as u64, 8)?;
                                    }
                                }
                            }
                        }
                    } else if encoded.len() > target_bytes {
                        let can_truncate = props.truncate_specified_length_string;
                        if !can_truncate {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                    encoded.len(),
                                    target_bytes
                                ),
                            ));
                        }
                        if justification == crate::schema::ir::TextJustification::Center {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                    encoded.len(),
                                    target_bytes
                                ),
                            ));
                        }
                        let truncated_bytes = match justification {
                            crate::schema::ir::TextJustification::Right => {
                                let skip = encoded.len().saturating_sub(target_bytes);
                                encoded.get(skip..).unwrap_or(&[])
                            }
                            _ => encoded.get(..target_bytes).unwrap_or(&[]),
                        };
                        self.writer.write_bytes(truncated_bytes)?;
                    } else {
                        self.writer.write_bytes(&encoded)?;
                    }
                }
                return Ok(());
            }
        }

        if props.encoding == crate::encoding::EncodingKind::Utf8
            || (s.is_ascii()
                && matches!(
                    props.encoding,
                    crate::encoding::EncodingKind::Ascii
                        | crate::encoding::EncodingKind::Iso8859_1
                ))
        {
            self.writer.write_bytes(s.as_bytes())?;
            return Ok(());
        }

        let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
        self.writer.write_bytes(&encoded)?;
        Ok(())
    }

    /// Writes `s` in an encoding whose characters are `cb` bits wide (e.g. 5-bit packed, octal).
    ///
    /// Each character becomes one `cb`-bit group written in the writer's bit order. With an
    /// explicit length the text is padded with `textPadCharacter` or truncated (when allowed)
    ///
    /// A character with no code in the encoding raises an `UnmappableCharacterException`
    /// unparse error under `encodingErrorPolicy="error"`, otherwise it is written as the
    /// encoding's replacement code. Unmappable pad characters are written as code 0.
    pub(crate) fn unparse_sub_byte_text(
        &mut self,
        s: &str,
        val: &DfdlValue,
        props: &ResolvedProperties,
        cb: usize,
    ) -> DFDLResult<()> {
        let mut codes: Vec<u64> = Vec::new();
        for ch in s.chars() {
            let code = match crate::encoding::strict_sub_byte_code(ch, &props.encoding) {
                Some(c) => c,
                None if props.encoding_error_policy_error => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: UnmappableCharacterException: character '{}' (U+{:04X}) cannot be encoded in {}",
                            ch,
                            ch as u32,
                            props.encoding
                        ),
                    ));
                }
                None => crate::encoding::sub_byte_replacement_code(&props.encoding),
            };
            codes.push(code);
        }
        if props.length_kind == crate::schema::ir::LengthKind::Explicit {
            if let Some(len) = self.resolve_explicit_length(props)? {
                let target = match props.length_units {
                    crate::schema::ir::LengthUnits::Characters => len,
                    crate::schema::ir::LengthUnits::Bits => len.checked_div(cb).unwrap_or(0),
                    crate::schema::ir::LengthUnits::Bytes => {
                        len.saturating_mul(8).checked_div(cb).unwrap_or(0)
                    }
                };
                if codes.len() > target {
                    let can_truncate = matches!(val, DfdlValue::String(_))
                        && props.truncate_specified_length_string;
                    if !can_truncate {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                codes.len(),
                                target
                            ),
                        ));
                    }
                    if props.text_string_justification == crate::schema::ir::TextJustification::Center {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                codes.len(),
                                target
                            ),
                        ));
                    }
                    codes.truncate(target);
                } else {
                    let pad_ch = props.text_pad_char.chars().next().unwrap_or(' ');
                    let pad_code =
                        crate::encoding::strict_sub_byte_code(pad_ch, &props.encoding).unwrap_or(0);
                    let pad = alloc::vec![pad_code; target.saturating_sub(codes.len())];
                    if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                        codes.splice(0..0, pad);
                    } else {
                        codes.extend(pad);
                    }
                }
            }
        }
        for code in codes {
            self.writer.write_bits(code, cb)?;
        }
        Ok(())
    }
}
