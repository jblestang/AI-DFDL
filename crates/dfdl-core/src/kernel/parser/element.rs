//! Element and occurrence parsing logic for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::events::InfosetEvent;
use crate::infoset::{DfdlSimpleType, DfdlValue, InfosetBuilder};
use crate::io::traits::{ByteOrder, ByteSource};
use crate::schema::ir::{
    CompiledElement, CompiledTerm, OccursCountKind, Representation, ResolvedProperties, TermKind,
};
use crate::util::try_push;

use super::delimiters::split_delimiter_alternatives;
use super::numbers::{
    parse_flexible_f64, parse_flexible_int_i64, parse_flexible_uint_u64,
};
use super::{ParserEngine, PointOfUncertainty, PouKind, ValidationMode};

fn dfdl_value_type_name(val: &DfdlValue) -> &'static str {
    match val {
        DfdlValue::String(_) => "String",
        DfdlValue::Int(_) => "Int",
        DfdlValue::Long(_) => "Long",
        DfdlValue::Short(_) => "Short",
        DfdlValue::Byte(_) => "Byte",
        DfdlValue::UnsignedLong(_) => "UnsignedLong",
        DfdlValue::UnsignedInt(_) => "UnsignedInt",
        DfdlValue::UnsignedShort(_) => "UnsignedShort",
        DfdlValue::UnsignedByte(_) => "UnsignedByte",
        DfdlValue::Boolean(_) => "Boolean",
        DfdlValue::Float(_) => "Float",
        DfdlValue::Double(_) => "Double",
        DfdlValue::HexBinary(_) => "HexBinary",
        DfdlValue::DateTime(_) => "DateTime",
        DfdlValue::Date(_) => "Date",
        DfdlValue::Time(_) => "Time",
        DfdlValue::Decimal(_) => "Decimal",
    }
}

fn dfdl_simple_type_name(st: &DfdlSimpleType) -> &'static str {
    match st {
        DfdlSimpleType::String => "String",
        DfdlSimpleType::Int => "Int",
        DfdlSimpleType::Long => "Long",
        DfdlSimpleType::Short => "Short",
        DfdlSimpleType::Byte => "Byte",
        DfdlSimpleType::UnsignedLong => "UnsignedLong",
        DfdlSimpleType::UnsignedInt => "UnsignedInt",
        DfdlSimpleType::UnsignedShort => "UnsignedShort",
        DfdlSimpleType::UnsignedByte => "UnsignedByte",
        DfdlSimpleType::Boolean => "Boolean",
        DfdlSimpleType::Float => "Float",
        DfdlSimpleType::Double => "Double",
        DfdlSimpleType::HexBinary => "HexBinary",
        DfdlSimpleType::DateTime => "DateTime",
        DfdlSimpleType::Date => "Date",
        DfdlSimpleType::Time => "Time",
        DfdlSimpleType::Decimal => "Decimal",
    }
}

fn validate_expression_result_coercion(raw: &DfdlValue, st: &DfdlSimpleType) -> DFDLResult<()> {
    let raw_name = dfdl_value_type_name(raw);
    let target_name = dfdl_simple_type_name(st);
    if raw_name == target_name {
        return Ok(());
    }
    let compatible = matches!(
        (st, raw),
        (DfdlSimpleType::Long, DfdlValue::Int(_))
            | (DfdlSimpleType::Int, DfdlValue::Short(_))
            | (DfdlSimpleType::Int, DfdlValue::Byte(_))
            | (DfdlSimpleType::Short, DfdlValue::Byte(_))
            | (DfdlSimpleType::Decimal, DfdlValue::Double(_))
    );
    if compatible {
        return Ok(());
    }
    let msg = alloc::format!(
        "Schema Definition Error: The expression result type '{}' must be manually cast to '{}' when allowExpressionResultCoercion is false",
        raw_name, target_name
    );
    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
}

pub(crate) fn coerce_and_validate_ivc_value(
    raw_val: &DfdlValue,
    st: &DfdlSimpleType,
    props: &ResolvedProperties,
) -> DFDLResult<DfdlValue> {
    if let Some(val_num) = raw_val.as_i128() {
        let is_neg = raw_val.is_negative();
        let display_str = alloc::format!("{}", raw_val);
        return match st {
            DfdlSimpleType::Boolean => match val_num {
                1 => Ok(DfdlValue::Boolean(true)),
                0 => Ok(DfdlValue::Boolean(false)),
                _ => Ok(raw_val.clone()),
            },
            DfdlSimpleType::Int => {
                let i = i32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:int", display_str),
                    )
                })?;
                Ok(DfdlValue::Int(i))
            }
            DfdlSimpleType::Long => {
                let l = i64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:long", display_str),
                    )
                })?;
                Ok(DfdlValue::Long(l))
            }
            DfdlSimpleType::Short => {
                let s = i16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:short", display_str),
                    )
                })?;
                Ok(DfdlValue::Short(s))
            }
            DfdlSimpleType::Byte => {
                let b = i8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:byte", display_str),
                    )
                })?;
                Ok(DfdlValue::Byte(b))
            }
            DfdlSimpleType::UnsignedLong => {
                if is_neg {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedLong: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            display_str
                        ),
                    ));
                }
                let u = u64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedLong",
                            display_str
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedLong(u))
            }
            DfdlSimpleType::UnsignedInt => {
                if is_neg {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedInt: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            display_str
                        ),
                    ));
                }
                let u = u32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedInt",
                            display_str
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedInt(u))
            }
            DfdlSimpleType::UnsignedShort => {
                if is_neg {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedShort: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            display_str
                        ),
                    ));
                }
                let u = u16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedShort",
                            display_str
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedShort(u))
            }
            DfdlSimpleType::UnsignedByte => {
                if is_neg {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedByte: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            display_str
                        ),
                    ));
                }
                let u = u8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedByte",
                            display_str
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedByte(u))
            }
            DfdlSimpleType::Float => Ok(DfdlValue::Float(val_num as f32)),
            DfdlSimpleType::Double => Ok(DfdlValue::Double(val_num as f64)),
            DfdlSimpleType::Decimal => {
                if props.facets.min_inclusive.as_deref() == Some("0") && (val_num < 0 || is_neg) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", val_num),
                    ));
                }
                if props.facets.min_exclusive.as_deref() == Some("0") && (val_num <= 0 || is_neg) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Cannot convert '{}' to PositiveInteger", val_num),
                    ));
                }
                Ok(DfdlValue::Decimal(alloc::format!("{}", val_num)))
            }
            DfdlSimpleType::String => Ok(DfdlValue::String(alloc::format!("{}", raw_val))),
            DfdlSimpleType::HexBinary => {
                let s = alloc::format!("{}", raw_val);
                let clean = s.trim();
                if clean.len() % 2 == 0 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
                    let mut bytes = Vec::with_capacity(clean.len() / 2);
                    for i in (0..clean.len()).step_by(2) {
                        if let (Some(h1), Some(h2)) = (
                            clean.as_bytes().get(i).and_then(|&b| (b as char).to_digit(16)),
                            clean.as_bytes().get(i.saturating_add(1)).and_then(|&b| (b as char).to_digit(16)),
                        ) {
                            bytes.push(((h1 << 4) | h2) as u8);
                        }
                    }
                    Ok(DfdlValue::HexBinary(bytes))
                } else {
                    Ok(raw_val.clone())
                }
            }
            _ => Ok(raw_val.clone()),
        };
    }

    match (st, raw_val) {
        (DfdlSimpleType::Float, DfdlValue::Double(v)) => Ok(DfdlValue::Float(*v as f32)),
        (DfdlSimpleType::Double, DfdlValue::Float(v)) => Ok(DfdlValue::Double(*v as f64)),
        (DfdlSimpleType::Decimal, DfdlValue::Double(v)) => {
            if props.facets.min_inclusive.as_deref() == Some("0") && *v < 0.0 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", v),
                ));
            }
            Ok(DfdlValue::Decimal(alloc::format!("{}", v)))
        }
        (DfdlSimpleType::Decimal, DfdlValue::Long(v)) => {
            if props.facets.min_inclusive.as_deref() == Some("0") && *v < 0 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", v),
                ));
            }
            Ok(DfdlValue::Decimal(alloc::format!("{}", v)))
        }
        (DfdlSimpleType::Decimal, DfdlValue::Int(v)) => {
            if props.facets.min_inclusive.as_deref() == Some("0") && *v < 0 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", v),
                ));
            }
            Ok(DfdlValue::Decimal(alloc::format!("{}", v)))
        }
        (DfdlSimpleType::Float, DfdlValue::Decimal(s)) => parse_flexible_f64(s.trim())
            .map(|v| DfdlValue::Float(v as f32))
            .ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parsed value {} out of range for xs:float", s),
                )
            }),
        (DfdlSimpleType::Double, DfdlValue::Decimal(s)) => parse_flexible_f64(s.trim())
            .map(DfdlValue::Double)
            .ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parsed value {} out of range for xs:double", s),
                )
            }),
        (st, DfdlValue::String(s) | DfdlValue::Decimal(s)) => {
            let clean = s.trim();
            match st {
                DfdlSimpleType::Int => {
                    let v = parse_flexible_int_i64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:int", clean),
                        )
                    })?;
                    let i = i32::try_from(v).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:int", clean),
                        )
                    })?;
                    Ok(DfdlValue::Int(i))
                }
                DfdlSimpleType::Long => {
                    let v = parse_flexible_int_i64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:long", clean),
                        )
                    })?;
                    Ok(DfdlValue::Long(v))
                }
                DfdlSimpleType::Short => {
                    let v = parse_flexible_int_i64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:short", clean),
                        )
                    })?;
                    let s_val = i16::try_from(v).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:short", clean),
                        )
                    })?;
                    Ok(DfdlValue::Short(s_val))
                }
                DfdlSimpleType::Byte => {
                    let v = parse_flexible_int_i64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:byte", clean),
                        )
                    })?;
                    let b = i8::try_from(v).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:byte", clean),
                        )
                    })?;
                    Ok(DfdlValue::Byte(b))
                }
                DfdlSimpleType::UnsignedLong => {
                    if clean.starts_with('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedLong: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                                clean
                            ),
                        ));
                    }
                    let u = parse_flexible_uint_u64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedLong",
                                clean
                            ),
                        )
                    })?;
                    Ok(DfdlValue::UnsignedLong(u))
                }
                DfdlSimpleType::UnsignedInt => {
                    if clean.starts_with('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedInt: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                                clean
                            ),
                        ));
                    }
                    let u = parse_flexible_uint_u64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedInt",
                                clean
                            ),
                        )
                    })?;
                    let u32_val = u32::try_from(u).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedInt",
                                clean
                            ),
                        )
                    })?;
                    Ok(DfdlValue::UnsignedInt(u32_val))
                }
                DfdlSimpleType::UnsignedShort => {
                    if clean.starts_with('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedShort: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                                clean
                            ),
                        ));
                    }
                    let u = parse_flexible_uint_u64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedShort",
                                clean
                            ),
                        )
                    })?;
                    let u16_val = u16::try_from(u).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedShort",
                                clean
                            ),
                        )
                    })?;
                    Ok(DfdlValue::UnsignedShort(u16_val))
                }
                DfdlSimpleType::UnsignedByte => {
                    if clean.starts_with('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedByte: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                                clean
                            ),
                        ));
                    }
                    let u = parse_flexible_uint_u64(clean).ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedByte",
                                clean
                            ),
                        )
                    })?;
                    let u8_val = u8::try_from(u).map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:unsignedByte",
                                clean
                            ),
                        )
                    })?;
                    Ok(DfdlValue::UnsignedByte(u8_val))
                }
                DfdlSimpleType::Float => parse_flexible_f64(clean)
                    .map(|v| DfdlValue::Float(v as f32))
                    .ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:float", clean),
                        )
                    }),
                DfdlSimpleType::Double => parse_flexible_f64(clean)
                    .map(DfdlValue::Double)
                    .ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parsed value {} out of range for xs:float", clean),
                        )
                    }),
                DfdlSimpleType::Date => {
                    let year_str = if clean.starts_with('-') {
                        clean.get(1..).unwrap_or("").split('-').next().unwrap_or("")
                    } else {
                        clean.split('-').next().unwrap_or("")
                    };
                    if year_str.len() < 4 || year_str.parse::<u32>().is_err() || !clean.contains('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parse Error: Failed to parse xs:date from text: '{}'", clean),
                        ));
                    }
                    Ok(DfdlValue::Date(alloc::string::ToString::to_string(clean)))
                }
                DfdlSimpleType::Time => {
                    if !clean.contains(':') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parse Error: Failed to parse xs:time from text: '{}'", clean),
                        ));
                    }
                    Ok(DfdlValue::Time(alloc::string::ToString::to_string(clean)))
                }
                DfdlSimpleType::DateTime => {
                    if !clean.contains('-') || !clean.contains(':') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parse Error: Failed to parse xs:dateTime from text: '{}'", clean),
                        ));
                    }
                    Ok(DfdlValue::DateTime(alloc::string::ToString::to_string(clean)))
                }
                DfdlSimpleType::Decimal => {
                    if props.facets.min_inclusive.as_deref() == Some("0") && clean.starts_with('-') {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", clean),
                        ));
                    }
                    Ok(DfdlValue::Decimal(alloc::string::ToString::to_string(clean)))
                }
                DfdlSimpleType::String => {
                    Ok(DfdlValue::String(s.clone()))
                }
                DfdlSimpleType::HexBinary => {
                    if clean.len() % 2 == 0 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
                        let mut bytes = Vec::with_capacity(clean.len() / 2);
                        for i in (0..clean.len()).step_by(2) {
                            if let (Some(h1), Some(h2)) = (
                                clean.as_bytes().get(i).and_then(|&b| (b as char).to_digit(16)),
                                clean.as_bytes().get(i.saturating_add(1)).and_then(|&b| (b as char).to_digit(16)),
                            ) {
                                bytes.push(((h1 << 4) | h2) as u8);
                            }
                        }
                        Ok(DfdlValue::HexBinary(bytes))
                    } else {
                        Ok(raw_val.clone())
                    }
                }
                DfdlSimpleType::Boolean => match clean {
                    "true" | "1" => Ok(DfdlValue::Boolean(true)),
                    "false" | "0" => Ok(DfdlValue::Boolean(false)),
                    _ => Ok(raw_val.clone()),
                },
            }
        }
        _ => {
            if raw_val.is_negative() {
                if props.facets.min_inclusive.as_deref() == Some("0") {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Cannot convert '{}' to NonNegativeInteger", raw_val),
                    ));
                }
                match st {
                    DfdlSimpleType::UnsignedLong
                    | DfdlSimpleType::UnsignedInt
                    | DfdlSimpleType::UnsignedShort
                    | DfdlSimpleType::UnsignedByte => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parsed value {} out of range for xs:{:?}: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                                raw_val, st
                            ),
                        ));
                    }
                    _ => {}
                }
            }
            Ok(raw_val.clone())
        }
    }
}

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    pub(crate) fn parse_element(
        &mut self,
        term: &CompiledTerm,
        elem: &CompiledElement,
        builder: &mut InfosetBuilder,
    ) -> DFDLResult<()> {
        let parent_choice_initiated = self
            .pou_stack
            .iter()
            .rev()
            .find_map(|p| match p.kind {
                PouKind::Choice { initiated_content } => Some(initiated_content),
                _ => None,
            })
            .unwrap_or(false);
        let _ = self.parse_element_with_separators(
            term,
            elem,
            builder,
            None,
            crate::schema::ir::SeparatorPosition::Infix,
            crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty,
            0,
            0,
            true,
            parent_choice_initiated,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_element_with_separators(
        &mut self,
        term: &CompiledTerm,
        elem: &CompiledElement,
        builder: &mut InfosetBuilder,
        sep_opt: Option<&str>,
        sep_pos: crate::schema::ir::SeparatorPosition,
        sep_policy: crate::schema::ir::SeparatorSuppressionPolicy,
        mut total_element_count: usize,
        member_idx: usize,
        is_last_member: bool,
        parent_initiated_content: bool,
    ) -> DFDLResult<usize> {
        use crate::schema::ir::OccursCountKind;

        if term.properties.parse_unparse_policy == crate::schema::ir::ParseUnparsePolicy::UnparseOnly {
            let msg = alloc::format!(
                "Schema Definition Error: Cannot parse element '{}' without parse support (parseUnparsePolicy is 'unparseOnly')",
                elem.name.local_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        let max_occurs = match term.properties.occurs_count_kind {
            OccursCountKind::Expression => {
                if let Some(ref expr_str) = term.properties.occurs_count_expr {
                    let ast = crate::expr::parse_expr(expr_str)?;
                    let mut current_path = builder.current_path();
                    let clean_name = elem
                        .name
                        .local_name
                        .split(':')
                        .next_back()
                        .unwrap_or(&elem.name.local_name);
                    let last_seg = current_path
                        .segments()
                        .last()
                        .map(|s| s.split(':').next_back().unwrap_or(s));
                    if last_seg != Some(clean_name) {
                        let _ = current_path.try_push(clean_name);
                    }
                    let active_doc = builder.active_doc();
                    let mut ctx = crate::expr::ExprContext::with_variable_map(
                        Some(&active_doc),
                        &current_path,
                        &[],
                        Some(&self.variable_map),
                        self.budget,
                    )
                    .with_occurs_index(self.current_occurs_index)
                    .with_schema(self.schema)
                    .with_namespaces(&term.properties.in_scope_namespaces)
                    .with_enclosing_lengths(&self.enclosing_complex_elements);
                    let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                    match val {
                        DfdlValue::Int(v) if v >= 0 => v as usize,
                        DfdlValue::Long(v) if v >= 0 => v as usize,
                        DfdlValue::UnsignedLong(v) => v as usize,
                        DfdlValue::UnsignedInt(v) => v as usize,
                        DfdlValue::UnsignedShort(v) => v as usize,
                        DfdlValue::UnsignedByte(v) => v as usize,
                        DfdlValue::Short(v) if v >= 0 => v as usize,
                        DfdlValue::Byte(v) if v >= 0 => v as usize,
                        DfdlValue::String(ref s) => s.trim().parse::<usize>().map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "occursCount expression string must be a non-negative integer",
                            )
                        })?,
                        _ => {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "occursCount expression must evaluate to a non-negative integer",
                            ));
                        }
                    }
                } else {
                    elem.max_occurs.unwrap_or(1)
                }
            }
            // Under `occursCountKind="parsed"`, the parser continues looking for occurrences
            // until speculative parsing fails to find another one (DFDL §16.1.3).
            // maxOccurs is only enforced as a validation check when validation is enabled.
            // Strictly scalar elements (minOccurs=1 and maxOccurs=1) occur exactly once.
            OccursCountKind::Parsed => {
                if elem.min_occurs == 1 && elem.max_occurs == Some(1) {
                    1
                } else {
                    usize::MAX
                }
            }
            OccursCountKind::Implicit => elem.max_occurs.unwrap_or(usize::MAX),
            OccursCountKind::Fixed | OccursCountKind::StopValue => {
                elem.max_occurs.unwrap_or(usize::MAX)
            }
        };

        if term.properties.input_value_calc.is_some()
            || !self.schema.term_has_representation(term.id)
        {
            let target_occurs = match term.properties.occurs_count_kind {
                OccursCountKind::Expression => max_occurs,
                OccursCountKind::Fixed => elem.min_occurs.max(1),
                _ => {
                    if elem.max_occurs == Some(0) {
                        0
                    } else {
                        elem.min_occurs.max(1)
                    }
                }
            };
            let mut count: usize = 0;
            for _ in 0..target_occurs {
                let occurrences = builder.count_child_occurrences(&elem.name.local_name);
                self.current_occurs_index = occurrences.saturating_add(1);
                let builder_cp = builder.checkpoint();
                let reader_cp = self.reader.checkpoint();
                let vmap_cp = self.variable_map.clone();
                let val_err_cp = self.validation_errors.len();
                match self.parse_single_element_occurrence(term, elem, builder, false, false) {
                    Ok(()) => {
                        count = count.saturating_add(1);
                    }
                    Err(e) => {
                        self.reader.rollback(reader_cp)?;
                        builder.rollback(builder_cp);
                        self.variable_map = vmap_cp;
                        self.validation_errors.truncate(val_err_cp);
                        if count >= elem.min_occurs && e.kind != DFDLErrorKind::SchemaDefinition {
                            break;
                        }
                        return Err(e);
                    }
                }
            }
            self.current_occurs_index = 1;
            return Ok(0);
        }

        // Per DFDL v1.0 §16.1.4: When dfdl:occursCountKind is 'parsed', the number of occurrences
        // is determined solely through speculative parsing. There is no reliance on xs:minOccurs
        // to control the parsing loop. minOccurs/maxOccurs are checked as schema validation errors
        // after parsing when validation is enabled.
        let is_parsed = term.properties.occurs_count_kind == OccursCountKind::Parsed
            && !(elem.min_occurs == 1 && elem.max_occurs == Some(1));
        let min_occurs = if is_parsed { 0 } else { elem.min_occurs };
        let mut count = 0;
        let mut slots_processed: usize = 0;

        while count < max_occurs && slots_processed < max_occurs {
            let is_simple_repr = matches!(elem.type_ir, crate::schema::ir::CompiledType::Simple(_))
                && term.properties.input_value_calc.is_none();
            if (count > 0 || is_simple_repr) && count >= min_occurs && self.reader.is_eof() {
                if sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::Never
                    && slots_processed < max_occurs
                {
                    slots_processed = slots_processed.saturating_add(1);
                }
                break;
            }
            if let Some(bound) = self.schema.max_occurs_bounds {
                if count >= bound {
                    let msg = alloc::format!(
                        "Tunable Limit Exceeded Error: maxOccursBounds limit of {} exceeded for element '{}'",
                        bound,
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
            }
            let reader_cp = self.reader.checkpoint();
            let builder_cp = builder.checkpoint();
            let vmap_cp = self.variable_map.clone();
            let val_err_cp = self.validation_errors.len();
            let bo_cp = self.reader.bit_order();
            let delims_cp = self.in_scope_delimiters.clone();
            let terms_cp = self.in_scope_terminators.clone();
            let is_occurrence_pou = count >= min_occurs;
            if is_occurrence_pou {
                self.pou_stack.push(PointOfUncertainty {
                    kind: PouKind::Occurrence,
                    is_discriminated: false,
                });
            }
            let occurrences = builder.count_child_occurrences(&elem.name.local_name);
            self.current_occurs_index = occurrences.saturating_add(1);

            let mut elem_consumed_zero = false;
            let mut sep_consumed = false;
            let mut pre_elem_cp = reader_cp;
            let parse_res = (|| -> DFDLResult<()> {
                if let Some(sep) = sep_opt {
                    if !sep.is_empty() {
                        if sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                            || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                && (count > 0 || member_idx > 0 || total_element_count > 0))
                        {
                            if let Some(sep_len) = self.peek_delimiter_match_length(sep) {
                                if self.is_in_scope_terminator_longer(sep_len) {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch: in-scope terminator matches longer than separator",
                                    ));
                                }
                            }
                            self.match_literal_delimiter(sep).map_err(|e| {
                                DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!("elem prefix/infix sep for '{}' failed: {}", elem.name.local_name, e),
                                )
                            })?;
                            sep_consumed = true;
                        } else if sep_pos == crate::schema::ir::SeparatorPosition::Infix
                            && sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty
                            && term.properties.occurs_count_kind != OccursCountKind::Expression
                            && term.properties.occurs_count_kind != OccursCountKind::Fixed
                            && count == 0
                            && total_element_count == 0
                            && elem.min_occurs == 0
                        {
                            while !self.reader.is_eof() {
                                if let Some(sep_len) = self.peek_delimiter_match_length(sep) {
                                    if sep_len == 0 || self.is_in_scope_terminator_longer(sep_len) {
                                        break;
                                    }
                                    let _ = self.match_literal_delimiter(sep);
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }

                pre_elem_cp = self.reader.checkpoint();

                // Left framing per DFDL §12 grammar: LeadingAlignment = LeadingSkip AlignmentFill.
                if term.properties.leading_skip > 0 {
                    let skip_bits = match term.properties.alignment_units {
                        crate::schema::ir::AlignmentUnits::Bytes => {
                            term.properties.leading_skip.saturating_mul(8)
                        }
                        crate::schema::ir::AlignmentUnits::Bits => term.properties.leading_skip,
                    };
                    if skip_bits > 0 {
                        let _ = self.reader.read_bits(skip_bits)?;
                    }
                }

                let align_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.alignment.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.alignment,
                };
                if term.properties.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
                    && align_bits > 1
                {
                    let current_pos = self.reader.position().0;
                    let rem = current_pos % align_bits;
                    if rem > 0 {
                        let skip = align_bits - rem;
                        let _ = self.reader.read_bits(skip)?;
                    }
                }

                // Enforce bitOrder change only on byte boundary (§11.2)
                // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
                if term.properties.bit_order != self.reader.bit_order() {
                    let current_pos = self.reader.position().0;
                    let rem = current_pos % 8;
                    if rem != 0 {
                        let bit_in_byte_1based = rem.saturating_add(1);
                        let msg = alloc::format!(
                            "Schema Definition Error: Can only change bitOrder on a byte boundary. Bit position {} is not on a byte boundary",
                            bit_in_byte_1based
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    self.reader.set_bit_order(term.properties.bit_order);
                }
                let res = self.parse_single_element_occurrence(
                    term,
                    elem,
                    builder,
                    is_occurrence_pou,
                    parent_initiated_content,
                );
                let post_elem_cp = self.reader.checkpoint();
                if post_elem_cp == pre_elem_cp {
                    elem_consumed_zero = true;
                }
                res?;

                // Right framing: trailingSkip
                if term.properties.trailing_skip > 0 {
                    let skip_bits = match term.properties.alignment_units {
                        crate::schema::ir::AlignmentUnits::Bytes => {
                            term.properties.trailing_skip.saturating_mul(8)
                        }
                        crate::schema::ir::AlignmentUnits::Bits => term.properties.trailing_skip,
                    };
                    if skip_bits > 0 {
                        let _ = self.reader.read_bits(skip_bits)?;
                    }
                }

                if let Some(sep) = sep_opt {
                    if sep_pos == crate::schema::ir::SeparatorPosition::Postfix {
                        self.match_literal_delimiter(sep).map_err(|e| {
                            DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!(
                                    "elem postfix sep for '{}' failed: {}",
                                    elem.name.local_name,
                                    e
                                ),
                            )
                        })?;
                    }
                }
                Ok(())
            })();

            match parse_res {
                Ok(()) => {
                    if is_occurrence_pou {
                        self.pou_stack.pop();
                    }
                    let new_reader_cp = self.reader.checkpoint();
                    if elem_consumed_zero
                        && term.properties.empty_element_parse_policy
                            == crate::schema::ir::EmptyElementParsePolicy::TreatAsAbsent
                    {
                        if count < min_occurs {
                            let msg = alloc::format!(
                                "Parse Error: Empty element not allowed in this position for required element '{}'",
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                        } else {
                            // Under DFDL v1.0 §16.1, an optional or array element parsing to an empty
                            // representation with emptyElementParsePolicy="treatAsAbsent" is rolled back
                            // and treated as absent without creating an infoset item.
                            // For scalars (maxOccurs == 1), processing terminates immediately.
                            // For arrays (maxOccurs > 1 or unbounded), additional occurrences may follow
                            // separated by delimiters (e.g. under separatorSuppressionPolicy="anyEmpty").
                            builder.rollback(builder_cp);
                            self.variable_map = vmap_cp;
                            self.validation_errors.truncate(val_err_cp);
                            slots_processed = slots_processed.saturating_add(1);
                            total_element_count = total_element_count.saturating_add(1);
                            if elem.max_occurs == Some(1) || self.reader.is_eof() {
                                break;
                            } else {
                                continue;
                            }
                        }
                    }
                    if elem_consumed_zero
                        && count >= min_occurs
                        && sep_policy
                            == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                        && (is_last_member
                            || self.reader.is_eof()
                            || sep_opt.is_some_and(|sep| self.peek_only_separators_to_end(sep)))
                    {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Parse Error: Empty trailing optional element with trailingEmptyStrict separatorSuppressionPolicy",
                        ));
                    }
                    count = count.saturating_add(1);
                    slots_processed = slots_processed.saturating_add(1);
                    total_element_count = total_element_count.saturating_add(1);
                    let has_next_sep = sep_opt.is_some_and(|sep| self.peek_literal_delimiter(sep));
                    if (elem.max_occurs.is_none()
                        || term.properties.occurs_count_kind == OccursCountKind::Parsed
                        || term.properties.occurs_count_kind == OccursCountKind::Implicit)
                        && count >= min_occurs
                        && (self.reader.is_eof() || (new_reader_cp == reader_cp && !has_next_sep))
                    {
                        break;
                    }
                }
                Err(e) => {
                    let is_discriminated = if is_occurrence_pou {
                        self.pou_stack.pop().map(|p| p.is_discriminated).unwrap_or(false)
                    } else {
                        false
                    };
                    if e.kind == DFDLErrorKind::SchemaDefinition {
                        return Err(e);
                    }
                    if is_discriminated {
                        let msg = alloc::format!("Parse Error: Failed to populate {}: {}", elem.name, e);
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                    if sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                        && sep_consumed
                        && elem_consumed_zero
                        && count >= min_occurs
                    {
                        let is_trailing = is_last_member
                            || sep_opt.is_some_and(|sep| self.peek_only_separators_to_end(sep));
                        if sep_policy
                            == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                            && is_trailing
                        {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Parse Error: Empty trailing optional element with trailingEmptyStrict separatorSuppressionPolicy",
                            ));
                        }
                        if is_trailing
                            || sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty
                        {
                            self.reader.rollback(reader_cp)?;
                        } else {
                            self.reader.rollback(pre_elem_cp)?;
                            slots_processed = slots_processed.saturating_add(1);
                        }
                        self.reader.set_bit_order(bo_cp);
                        builder.rollback(builder_cp);
                        self.variable_map = vmap_cp;
                        self.validation_errors.truncate(val_err_cp);
                        self.current_occurs_index = 1;
                        break;
                    }

                    if elem_consumed_zero
                        && count >= min_occurs
                        && elem.max_occurs != Some(1)
                        && sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::Never
                    {
                        if let Some(sep) = sep_opt {
                            if !sep.is_empty() && self.peek_literal_delimiter(sep) {
                                builder.rollback(builder_cp);
                                self.variable_map = vmap_cp;
                                self.validation_errors.truncate(val_err_cp);
                                slots_processed = slots_processed.saturating_add(1);
                                let _ = self.match_literal_delimiter(sep);
                                continue;
                            }
                        }
                        if (sep_consumed || slots_processed > 0) && self.reader.is_eof() {
                            builder.rollback(builder_cp);
                            self.variable_map = vmap_cp;
                            self.validation_errors.truncate(val_err_cp);
                            slots_processed = slots_processed.saturating_add(1);
                            self.current_occurs_index = 1;
                            break;
                        }
                    }

                    if sep_consumed && count >= min_occurs {
                        let is_trailing = is_last_member
                            || self.reader.is_eof()
                            || sep_opt.is_some_and(|sep| self.peek_only_separators_to_end(sep));
                        if sep_policy
                            == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                            && is_trailing
                        {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Parse Error: Empty trailing optional element with trailingEmptyStrict separatorSuppressionPolicy",
                            ));
                        }

                        if elem_consumed_zero
                            && term.properties.occurs_count_kind == OccursCountKind::Implicit
                            && elem.max_occurs.is_some_and(|m| slots_processed < m)
                            && sep_opt.is_some_and(|sep| self.peek_literal_delimiter(sep))
                        {
                            self.reader.rollback(pre_elem_cp)?;
                            builder.rollback(builder_cp);
                            self.variable_map = vmap_cp;
                            self.validation_errors.truncate(val_err_cp);
                            self.reader.set_bit_order(bo_cp);
                            slots_processed = slots_processed.saturating_add(1);
                            continue;
                        }

                        if count == 0 {
                            if sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::Never
                                || (!is_trailing
                                    && (sep_policy
                                        == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                                        || sep_policy
                                            == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty))
                            {
                                self.reader.rollback(pre_elem_cp)?;
                            } else {
                                self.reader.rollback(reader_cp)?;
                            }
                            builder.rollback(builder_cp);
                            self.variable_map = vmap_cp;
                            self.validation_errors.truncate(val_err_cp);
                            self.reader.set_bit_order(bo_cp);
                            self.in_scope_delimiters = delims_cp.clone();
                            self.in_scope_terminators = terms_cp.clone();
                            slots_processed = slots_processed.saturating_add(1);
                            self.current_occurs_index = 1;
                            break;
                        } else {
                            self.reader.rollback(reader_cp)?;
                            builder.rollback(builder_cp);
                            self.variable_map = vmap_cp;
                            self.validation_errors.truncate(val_err_cp);
                            self.reader.set_bit_order(bo_cp);
                            self.in_scope_delimiters = delims_cp.clone();
                            self.in_scope_terminators = terms_cp.clone();
                            self.current_occurs_index = 1;
                            break;
                        }
                    }

                    self.reader.rollback(reader_cp)?;
                    builder.rollback(builder_cp);
                    self.variable_map = vmap_cp;
                    self.validation_errors.truncate(val_err_cp);
                    self.in_scope_delimiters = delims_cp;
                    self.in_scope_terminators = terms_cp;
                    self.current_occurs_index = 1;
                    if count >= min_occurs {
                        if count == 0 && elem.min_occurs == 0 && elem.max_occurs == Some(1) {
                            if let Some(sep) = sep_opt {
                                if !sep.is_empty()
                                    && sep_policy
                                        == crate::schema::ir::SeparatorSuppressionPolicy::Never
                                    && !(is_last_member && sep_pos == crate::schema::ir::SeparatorPosition::Infix)
                                    && !self.peek_literal_delimiter(sep)
                                {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Parse Error: Missing required separator for optional element with separatorSuppressionPolicy='never'",
                                    ));
                                }
                            }
                        }
                        break;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        self.current_occurs_index = 1;

        if !is_parsed && count < elem.min_occurs {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Array occurrences failed to meet minOccurs constraint",
            ));
        }

        if term.properties.occurs_count_kind == OccursCountKind::Parsed
            && self.validation_mode != ValidationMode::Off
        {
            if count < elem.min_occurs {
                let msg = alloc::format!(
                    "Validation Error: element '{}' occurs {} times, less than minOccurs {}",
                    elem.name.local_name, count, elem.min_occurs
                );
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
            if let Some(max) = elem.max_occurs {
                if count > max {
                    let msg = alloc::format!(
                        "Validation Error: element '{}' occurs {} times, exceeding maxOccurs {}",
                        elem.name.local_name, count, max
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
        }

        if let Some(sep) = sep_opt {
            if !sep.is_empty()
                && sep_policy == crate::schema::ir::SeparatorSuppressionPolicy::Never
                && term.properties.occurs_count_kind == OccursCountKind::Implicit
            {
                if let Some(max) = elem.max_occurs {
                    if max > 1 && slots_processed < max {
                        let msg = alloc::format!(
                            "Parse Error: All maxOccurs ({}) occurrences are required when separatorSuppressionPolicy is 'never', but only {} occurrences were found",
                            max, slots_processed
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                }
            }
        }

        Ok(count)
    }

    pub(crate) fn parse_single_unordered_element_occurrence(
        &mut self,
        term: &CompiledTerm,
        elem: &CompiledElement,
        builder: &mut InfosetBuilder,
        sep_opt: Option<&str>,
        sep_pos: crate::schema::ir::SeparatorPosition,
        total_element_count: usize,
    ) -> DFDLResult<()> {
        if let Some(sep) = sep_opt {
            if !sep.is_empty()
                && (sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                    || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                        && total_element_count > 0))
            {
                if let Some(sep_len) = self.peek_delimiter_match_length(sep) {
                    if self.is_in_scope_terminator_longer(sep_len) {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: in-scope terminator matches longer than separator",
                        ));
                    }
                }
                self.match_literal_delimiter(sep).map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "elem prefix/infix sep for '{}' failed: {}",
                            elem.name.local_name,
                            e
                        ),
                    )
                })?;
            }
        }

        // Left framing per DFDL §12 grammar: LeadingAlignment = LeadingSkip AlignmentFill.
        if term.properties.leading_skip > 0 {
            let skip_bits = match term.properties.alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => {
                    term.properties.leading_skip.saturating_mul(8)
                }
                crate::schema::ir::AlignmentUnits::Bits => term.properties.leading_skip,
            };
            if skip_bits > 0 {
                let _ = self.reader.read_bits(skip_bits)?;
            }
        }

        let align_bits = match term.properties.alignment_units {
            crate::schema::ir::AlignmentUnits::Bytes => {
                term.properties.alignment.saturating_mul(8)
            }
            crate::schema::ir::AlignmentUnits::Bits => term.properties.alignment,
        };
        if term.properties.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
            && align_bits > 1
        {
            let current_pos = self.reader.position().0;
            let rem = current_pos % align_bits;
            if rem > 0 {
                let skip = align_bits - rem;
                let _ = self.reader.read_bits(skip)?;
            }
        }

        // Enforce bitOrder change only on byte boundary (§11.2)
        // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
        if term.properties.bit_order != self.reader.bit_order() {
            let current_pos = self.reader.position().0;
            let rem = current_pos % 8;
            if rem != 0 {
                let bit_in_byte_1based = rem.saturating_add(1);
                let msg = alloc::format!(
                    "Schema Definition Error: Can only change bitOrder on a byte boundary. Bit position {} is not on a byte boundary",
                    bit_in_byte_1based
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            self.reader.set_bit_order(term.properties.bit_order);
        }

        let occurrences = builder.count_child_occurrences(&elem.name.local_name);
        self.current_occurs_index = occurrences.saturating_add(1);
        let res = self.parse_single_element_occurrence(term, elem, builder, false, false);
        self.current_occurs_index = 1;
        res?;

        if term.properties.trailing_skip > 0 {
            let skip_bits = match term.properties.alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => {
                    term.properties.trailing_skip.saturating_mul(8)
                }
                crate::schema::ir::AlignmentUnits::Bits => term.properties.trailing_skip,
            };
            if skip_bits > 0 {
                let _ = self.reader.read_bits(skip_bits)?;
            }
        }

        if let Some(sep) = sep_opt {
            if sep_pos == crate::schema::ir::SeparatorPosition::Postfix {
                self.match_literal_delimiter(sep).map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "elem postfix sep for '{}' failed: {}",
                            elem.name.local_name,
                            e
                        ),
                    )
                })?;
            }
        }

        Ok(())
    }

    pub(crate) fn parse_unordered_sequence(
        &mut self,
        _term: &CompiledTerm,
        seq: &crate::schema::ir::CompiledSequence,
        builder: &mut InfosetBuilder,
        sep_opt: Option<&str>,
        sep_pos: crate::schema::ir::SeparatorPosition,
        _sep_policy: crate::schema::ir::SeparatorSuppressionPolicy,
    ) -> DFDLResult<()> {
        let initial_child_count = builder.current_child_count();
        let mut counts = alloc::vec![0usize; seq.members.len()];
        let mut total_element_count: usize = 0;

        let mut schema_names = alloc::vec::Vec::with_capacity(seq.members.len());
        for &member_id in &seq.members {
            if let Some(member_term) = self.schema.get_term(member_id) {
                if let TermKind::Element(elem) = &member_term.kind {
                    schema_names.push(elem.name.local_name.clone());
                }
            }
        }

        loop {
            if self.reader.is_eof() {
                break;
            }
            let mut matched_any = false;
            for (idx, &member_id) in seq.members.iter().enumerate() {
                let member_term = match self.schema.get_term(member_id) {
                    Some(t) => t,
                    None => continue,
                };

                match &member_term.kind {
                    TermKind::Element(elem) => {
                        if !self.schema.term_has_representation(member_term.id) {
                            continue;
                        }
                        let is_array = elem.max_occurs.is_none() || elem.max_occurs > Some(1);
                        let max_occurs = if is_array
                            && member_term.properties.occurs_count_kind == OccursCountKind::Parsed
                        {
                            usize::MAX
                        } else {
                            elem.max_occurs.unwrap_or(usize::MAX)
                        };
                        let cur_count = counts.get(idx).copied().unwrap_or(0);
                        if cur_count >= max_occurs {
                            continue;
                        }

                        let reader_cp = self.reader.checkpoint();
                        let builder_cp = builder.checkpoint();
                        let vmap_cp = self.variable_map.clone();
                        let val_err_cp = self.validation_errors.len();

                        match self.parse_single_unordered_element_occurrence(
                            member_term,
                            elem,
                            builder,
                            sep_opt,
                            sep_pos,
                            total_element_count,
                        ) {
                            Ok(()) => {
                                let new_reader_cp = self.reader.checkpoint();
                                if new_reader_cp == reader_cp && cur_count >= elem.min_occurs {
                                    let _ = self.reader.rollback(reader_cp);
                                    builder.rollback(builder_cp);
                                    self.variable_map = vmap_cp;
                                    self.validation_errors.truncate(val_err_cp);
                                    continue;
                                }
                                if let Some(c) = counts.get_mut(idx) {
                                    *c = c.saturating_add(1);
                                }
                                total_element_count = total_element_count.saturating_add(1);
                                matched_any = true;
                                break;
                            }
                            Err(_) => {
                                let _ = self.reader.rollback(reader_cp);
                                builder.rollback(builder_cp);
                                self.variable_map = vmap_cp;
                                self.validation_errors.truncate(val_err_cp);
                            }
                        }
                    }
                    _ => {
                        let reader_cp = self.reader.checkpoint();
                        let builder_cp = builder.checkpoint();
                        let vmap_cp = self.variable_map.clone();
                        let val_err_cp = self.validation_errors.len();
                        let group_res = (|| -> DFDLResult<()> {
                            if let Some(sep) = sep_opt {
                                if !sep.is_empty()
                                    && (sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                                        || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                            && total_element_count > 0))
                                {
                                    self.match_literal_delimiter(sep)?;
                                }
                            }
                            self.parse_term(member_id, builder)?;
                            if let Some(sep) = sep_opt {
                                if !sep.is_empty()
                                    && sep_pos == crate::schema::ir::SeparatorPosition::Postfix
                                {
                                    self.match_literal_delimiter(sep)?;
                                }
                            }
                            Ok(())
                        })();
                        match group_res {
                            Ok(()) => {
                                if let Some(c) = counts.get_mut(idx) {
                                    *c = c.saturating_add(1);
                                }
                                total_element_count = total_element_count.saturating_add(1);
                                matched_any = true;
                                break;
                            }
                            Err(_) => {
                                let _ = self.reader.rollback(reader_cp);
                                builder.rollback(builder_cp);
                                self.variable_map = vmap_cp;
                                self.validation_errors.truncate(val_err_cp);
                            }
                        }
                    }
                }
            }

            if !matched_any {
                break;
            }
        }

        // Process any inputValueCalc elements in the sequence
        for (idx, &member_id) in seq.members.iter().enumerate() {
            let member_term = match self.schema.get_term(member_id) {
                Some(t) => t,
                None => continue,
            };
            if let TermKind::Element(elem) = &member_term.kind {
                if !self.schema.term_has_representation(member_term.id) {
                    let occurrences = builder.count_child_occurrences(&elem.name.local_name);
                    self.current_occurs_index = occurrences.saturating_add(1);
                    self.parse_single_element_occurrence(member_term, elem, builder, false, false)?;
                    self.current_occurs_index = 1;
                    if let Some(c) = counts.get_mut(idx) {
                        *c = 1;
                    }
                }
            }
        }

        // Validate minOccurs and maxOccurs constraints (§14.3 & §16.1.4)
        for (idx, &member_id) in seq.members.iter().enumerate() {
            let member_term = match self.schema.get_term(member_id) {
                Some(t) => t,
                None => continue,
            };
            if let TermKind::Element(elem) = &member_term.kind {
                let parsed_cnt = counts.get(idx).copied().unwrap_or(0);
                if parsed_cnt < elem.min_occurs {
                    let msg = alloc::format!(
                        "Unordered sequence member '{}' failed to meet minOccurs constraint: parsed {} < minOccurs {}",
                        elem.name.local_name, parsed_cnt, elem.min_occurs
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                if self.validation_mode != ValidationMode::Off {
                    if let Some(max_occ) = elem.max_occurs {
                        if parsed_cnt > max_occ {
                            let display_name = if let Some(ref pfx) = elem.name.prefix {
                                alloc::format!("{}:{}", pfx, elem.name.local_name)
                            } else {
                                elem.name.local_name.clone()
                            };
                            let msg = alloc::format!(
                                "Validation Error: Element '{}' failed check: occurrence count {} exceeds maxOccurs {}",
                                display_name,
                                parsed_cnt,
                                max_occ
                            );
                            let _ = crate::util::try_push(
                                &mut self.validation_errors,
                                DFDLError::new(DFDLErrorKind::Validation, &msg),
                            );
                        }
                    }
                }
            }
        }

        // Reorder children added in this sequence according to schema definition order (§14.3)
        let schema_name_refs: alloc::vec::Vec<&str> =
            schema_names.iter().map(|s| s.as_str()).collect();
        builder.reorder_children_from(initial_child_count, &schema_name_refs);

        Ok(())
    }

    pub(crate) fn parse_single_element_occurrence(
        &mut self,
        term: &CompiledTerm,
        elem: &CompiledElement,
        builder: &mut InfosetBuilder,
        is_occurrence_pou: bool,
        parent_initiated_content: bool,
    ) -> DFDLResult<()> {
        let saved_delim_enc =
            core::mem::replace(&mut self.delim_encoding, term.properties.encoding.clone());
        let saved_delim_case =
            core::mem::replace(&mut self.delim_ignore_case, term.properties.ignore_case);
        let res = self.parse_single_element_occurrence_inner(
            term,
            elem,
            builder,
            is_occurrence_pou,
            parent_initiated_content,
        );
        self.delim_encoding = saved_delim_enc;
        self.delim_ignore_case = saved_delim_case;
        res
    }

    fn parse_single_element_occurrence_inner(
        &mut self,
        term: &CompiledTerm,
        elem: &CompiledElement,
        builder: &mut InfosetBuilder,
        is_occurrence_pou: bool,
        parent_initiated_content: bool,
    ) -> DFDLResult<()> {
        let is_hidden = term.properties.is_hidden;

        self.evaluate_pattern_asserts(term, Some(&elem.name.local_name), builder)?;

        if term.properties.discriminator.is_some()
            && term.properties.discriminator_test_kind == crate::schema::ir::TestKind::Pattern
        {
            self.evaluate_discriminator(term, Some(&elem.name.local_name), builder)?;
        }

        if let Some(ref ivc_expr) = term.properties.input_value_calc {
            let ast = crate::expr::parse_expr(ivc_expr)?;
            let mut current_path = builder.current_path();
            let _ = current_path.try_push(&elem.name.local_name);
            let active_doc = builder.active_doc();
            let (variable_map, schema, budget, reader) = (
                &self.variable_map,
                self.schema,
                &mut *self.budget,
                &mut *self.reader,
            );
            let reader_cell = core::cell::RefCell::new(reader);
            let la_fn = |offset: usize, num_bits: usize| -> DFDLResult<u128> {
                reader_cell.borrow_mut().lookahead_bits(offset, num_bits)
            };
            let mut ctx = crate::expr::ExprContext::with_variable_map(
                Some(&active_doc),
                &current_path,
                &[],
                Some(variable_map),
                budget,
            )
            .with_occurs_index(self.current_occurs_index)
            .with_schema(schema)
            .with_namespaces(&term.properties.in_scope_namespaces)
            .with_lookahead(Some(&la_fn))
            .with_enclosing_lengths(&self.enclosing_complex_elements);
            let raw_val = crate::expr::eval_expr(&ast, &mut ctx).map_err(|e| {
                if e.kind == DFDLErrorKind::Parse {
                    e
                } else {
                    let msg = alloc::format!("Schema Definition Error: {}", e);
                    DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg)
                }
            })?;
            let value = match &elem.type_ir {
                crate::schema::ir::CompiledType::Simple(st) => {
                    if !self.allow_expression_result_coercion {
                        validate_expression_result_coercion(&raw_val, st)?;
                    }
                    coerce_and_validate_ivc_value(&raw_val, st, &term.properties)?
                }
                _ => raw_val,
            };
            if self.validation_mode != ValidationMode::Off {
                if let Err(e) = term.properties.facets.validate_value_detailed(&value) {
                    let msg_str = e.message.as_str();
                    let detail = msg_str.strip_prefix("Validation Error: ").unwrap_or(msg_str);
                    let msg = alloc::format!("Validation Error: element '{}' {}", elem.name.prefixed_name(), detail);
                    let _ = crate::util::try_push(&mut self.validation_errors, DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
            builder.push_event_with_hidden(
                InfosetEvent::SimpleValue {
                    name: elem.name.clone(),
                    value,
                },
                is_hidden,
            )?;
            self.execute_set_variables(term, builder, Some(&elem.name.local_name))?;
            self.evaluate_asserts(term, Some(&elem.name.local_name), builder)?;
            if term.properties.discriminator.is_some()
                && term.properties.discriminator_test_kind != crate::schema::ir::TestKind::Pattern
            {
                self.evaluate_discriminator(term, Some(&elem.name.local_name), builder)?;
            }
            return Ok(());
        }

        if elem.is_nillable {
            if let Some(ref nil_raw) = term.properties.nil_value {
                let alternatives = split_delimiter_alternatives(nil_raw);
                let nvdp = term.properties.nil_value_delimiter_policy;
                let expect_initiator = match nvdp {
                    crate::schema::ir::NilValueDelimiterPolicy::Both
                    | crate::schema::ir::NilValueDelimiterPolicy::Initiator => true,
                    crate::schema::ir::NilValueDelimiterPolicy::Terminator
                    | crate::schema::ir::NilValueDelimiterPolicy::None => false,
                };
                let expect_terminator = match nvdp {
                    crate::schema::ir::NilValueDelimiterPolicy::Both
                    | crate::schema::ir::NilValueDelimiterPolicy::Terminator => true,
                    crate::schema::ir::NilValueDelimiterPolicy::Initiator
                    | crate::schema::ir::NilValueDelimiterPolicy::None => false,
                };

                let nil_reader_cp = self.reader.checkpoint();
                let mut nil_matched = false;

                let mut try_nil = || -> DFDLResult<bool> {
                    if expect_initiator {
                        if let Some(ref init_raw) = term.properties.initiator {
                            let init = self.evaluate_property_str_at(
                                init_raw,
                                builder,
                                Some(&elem.name.local_name),
                            )?;
                            if !init.is_empty() {
                                self.match_literal_delimiter(&init)?;
                            }
                        }
                    }

                    if term.properties.representation == crate::schema::ir::Representation::Text {
                        let text_start_cp = self.reader.checkpoint();
                        let mut text_bytes = Vec::new();
                        let is_delimited = term.properties.length_kind == crate::schema::ir::LengthKind::Delimited;
                        if is_delimited {
                            while !self.reader.is_eof() {
                                if let Some(ref term_raw) = term.properties.terminator {
                                    if !term_raw.is_empty() {
                                        let term_str = self.evaluate_property_str_at(
                                            term_raw,
                                            builder,
                                            Some(&elem.name.local_name),
                                        )?;
                                        if !term_str.is_empty() && self.peek_literal_delimiter(&term_str) {
                                            break;
                                        }
                                    }
                                }
                                if self.peek_any_in_scope_delimiter() {
                                    break;
                                }
                                if let Ok(b) = self.reader.read_bits(8) {
                                    try_push(&mut text_bytes, b as u8)?;
                                } else {
                                    break;
                                }
                            }
                        } else if let Some(len) = self.evaluate_length_property(
                            &term.properties,
                            Some(&elem.name.local_name),
                            builder,
                        )? {
                            let byte_len = match term.properties.length_units {
                                crate::schema::ir::LengthUnits::Bits => len.saturating_add(7) / 8,
                                _ => len,
                            };
                            for _ in 0..byte_len {
                                if let Ok(b) = self.reader.read_bits(8) {
                                    try_push(&mut text_bytes, b as u8)?;
                                } else {
                                    break;
                                }
                            }
                        }

                        if let Ok(raw_str) = crate::encoding::decode_text_bytes(&text_bytes, &term.properties.encoding) {
                            let pad_char_str = if let crate::schema::ir::CompiledType::Simple(st) = elem.type_ir {
                                if st.is_numeric() {
                                    term.properties
                                        .text_number_pad_character
                                        .as_deref()
                                        .unwrap_or(&term.properties.text_pad_char)
                                } else if matches!(
                                    st,
                                    DfdlSimpleType::Date | DfdlSimpleType::Time | DfdlSimpleType::DateTime
                                ) {
                                    term.properties
                                        .text_calendar_pad_character
                                        .as_deref()
                                        .unwrap_or(&term.properties.text_pad_char)
                                } else if st == DfdlSimpleType::Boolean {
                                    term.properties
                                        .text_boolean_pad_character
                                        .as_deref()
                                        .unwrap_or(&term.properties.text_pad_char)
                                } else {
                                    &term.properties.text_pad_char
                                }
                            } else {
                                &term.properties.text_pad_char
                            };

                            let trimmed_str = match term.properties.text_trim_kind {
                                crate::schema::ir::TextTrimKind::None => raw_str.as_str(),
                                crate::schema::ir::TextTrimKind::Head => {
                                    raw_str.trim_start_matches(|c: char| pad_char_str.contains(c))
                                }
                                crate::schema::ir::TextTrimKind::Tail => {
                                    raw_str.trim_end_matches(|c: char| pad_char_str.contains(c))
                                }
                                crate::schema::ir::TextTrimKind::Both => {
                                    raw_str.trim_matches(|c: char| pad_char_str.contains(c))
                                }
                            };

                            let matches_target = |s: &str, target: &str| -> bool {
                                if term.properties.ignore_case {
                                    s.eq_ignore_ascii_case(target)
                                } else {
                                    s == target
                                }
                            };

                            for alt in &alternatives {
                                let nil_str = self.evaluate_property_str_at(
                                    alt,
                                    builder,
                                    Some(&elem.name.local_name),
                                )?;
                                let is_matched = match alt.as_str() {
                                    "%WSP;" => trimmed_str == " " || trimmed_str == "\t",
                                    "%WSP+;" => {
                                        !trimmed_str.is_empty()
                                            && trimmed_str.chars().all(|c| c == ' ' || c == '\t')
                                    }
                                    "%WSP*;" => {
                                        trimmed_str.chars().all(|c| c == ' ' || c == '\t')
                                    }
                                    "%ES;" => {
                                        if matches!(elem.type_ir, crate::schema::ir::CompiledType::Complex(_)) {
                                            let has_terminator = term.properties.terminator.as_deref().is_some_and(|t| !t.is_empty());
                                            if expect_terminator && has_terminator {
                                                trimmed_str.is_empty()
                                            } else if self.reader.is_eof() {
                                                true
                                            } else {
                                                self.peek_any_in_scope_delimiter()
                                            }
                                        } else {
                                            trimmed_str.is_empty()
                                        }
                                    }
                                    _ if nil_str.is_empty() => {
                                        if matches!(elem.type_ir, crate::schema::ir::CompiledType::Complex(_)) {
                                            let has_terminator = term.properties.terminator.as_deref().is_some_and(|t| !t.is_empty());
                                            if expect_terminator && has_terminator {
                                                trimmed_str.is_empty()
                                            } else if self.reader.is_eof() {
                                                true
                                            } else {
                                                self.peek_any_in_scope_delimiter()
                                            }
                                        } else {
                                            trimmed_str.is_empty()
                                        }
                                    }
                                    _ => {
                                        let unescaped =
                                            crate::expr::properties::decode_dfdl_character_entities(
                                                &nil_str,
                                            );
                                        let is_literal_char = term.properties.nil_kind
                                            == crate::schema::ir::NilKind::LiteralCharacter;
                                        if is_literal_char {
                                            let target = if !unescaped.is_empty() {
                                                &unescaped
                                            } else {
                                                &nil_str
                                            };
                                            !trimmed_str.is_empty()
                                                && trimmed_str.chars().all(|c| {
                                                    let mut buf = [0u8; 4];
                                                    let c_str = c.encode_utf8(&mut buf);
                                                    matches_target(c_str, target)
                                                })
                                        } else {
                                            matches_target(trimmed_str, &nil_str)
                                                || matches_target(trimmed_str, &unescaped)
                                                || (unescaped.trim().is_empty()
                                                    && trimmed_str.is_empty())
                                        }
                                    }
                                };
                                if is_matched {
                                    nil_matched = true;
                                    break;
                                }
                            }
                        }

                        if !nil_matched {
                            let _ = self.reader.rollback(text_start_cp);
                        }
                    } else {
                        for alt in &alternatives {
                            let nil_str = self.evaluate_property_str_at(
                                alt,
                                builder,
                                Some(&elem.name.local_name),
                            )?;
                            match term.properties.nil_kind {
                                crate::schema::ir::NilKind::LiteralCharacter => {
                                    if !nil_str.is_empty() {
                                        let explicit_len = self.evaluate_length_property(
                                            &term.properties,
                                            Some(&elem.name.local_name),
                                            builder,
                                        )?;
                                        let rep_count = explicit_len.unwrap_or(1);
                                        let mut expected_nil = String::new();
                                        for _ in 0..rep_count {
                                            expected_nil.push_str(&nil_str);
                                        }
                                        if self.match_literal_delimiter(&expected_nil).is_ok() {
                                            nil_matched = true;
                                            break;
                                        }
                                    }
                                }
                                crate::schema::ir::NilKind::LiteralValue
                                | crate::schema::ir::NilKind::LogicalValue => {
                                    if nil_str.is_empty() || nil_str == "%ES;" {
                                        let mut has_non_empty_term = false;
                                        if let Some(ref term_raw) = term.properties.terminator {
                                            if !term_raw.is_empty() {
                                                let term_eval = self.evaluate_property_str_at(
                                                    term_raw,
                                                    builder,
                                                    Some(&elem.name.local_name),
                                                )?;
                                                if !term_eval.is_empty() && term_eval != "%ES;" {
                                                    has_non_empty_term = true;
                                                    if self.peek_literal_delimiter(&term_eval) {
                                                        nil_matched = true;
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        if !has_non_empty_term {
                                            if self.reader.is_eof() {
                                                nil_matched = true;
                                                break;
                                            }
                                            let mut is_delim = false;
                                            for i in (0..self.in_scope_delimiters.len()).rev() {
                                                if let Ok(delim) = crate::util::get_checked(
                                                    &self.in_scope_delimiters,
                                                    i,
                                                ) {
                                                    let delim_clone = delim.clone();
                                                    if !delim_clone.text.is_empty()
                                                        && delim_clone.text != "%ES;"
                                                        && self.peek_in_scope_delimiter(&delim_clone)
                                                    {
                                                        is_delim = true;
                                                        break;
                                                    }
                                                }
                                            }
                                            if is_delim {
                                                nil_matched = true;
                                                break;
                                            }
                                        }
                                    } else if self.match_literal_delimiter(&nil_str).is_ok() {
                                        nil_matched = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if nil_matched && expect_terminator {
                        if let Some(ref term_raw) = term.properties.terminator {
                            let term_str = self.evaluate_property_str_at(
                                term_raw,
                                builder,
                                Some(&elem.name.local_name),
                            )?;
                            if !term_str.is_empty() {
                                self.match_literal_delimiter(&term_str).map_err(|e| {
                                    DFDLError::new(
                                        DFDLErrorKind::Parse,
                                        &alloc::format!("Terminator '{}' not found: {}", term_raw, e.message),
                                    )
                                })?;
                            }
                        }
                    }
                    Ok(nil_matched)
                };

                if try_nil().unwrap_or(false) {
                    builder.push_event_with_hidden(
                        InfosetEvent::StartElement {
                            name: elem.name.clone(),
                            is_nil: true,
                        },
                        is_hidden,
                    )?;
                    builder.push_event_with_hidden(
                        InfosetEvent::EndElement {
                            name: elem.name.clone(),
                        },
                        is_hidden,
                    )?;
                    self.evaluate_asserts(term, Some(&elem.name.local_name), builder)?;
                    if term.properties.discriminator.is_some()
                        && term.properties.discriminator_test_kind
                            != crate::schema::ir::TestKind::Pattern
                    {
                        self.evaluate_discriminator(term, Some(&elem.name.local_name), builder)?;
                    }
                    return Ok(());
                }
                let _ = self.reader.rollback(nil_reader_cp);
            }
        }

        if term.properties.representation == crate::schema::ir::Representation::Text
            && term.properties.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
            && term.properties.rep_type.is_none()
            && term.properties.rep_simple_type.is_none()
        {
            self.align_mandatory_text(&term.properties.encoding)?;
        }

        let evdp = term.properties.empty_value_delimiter_policy;
        let mut skipped_delimiters_for_empty = false;

        if let Some(ref init_raw) = term.properties.initiator {
            let init = self.evaluate_property_str_at(
                init_raw,
                builder,
                Some(&elem.name.local_name),
            )?;
            if !init.is_empty() {
                let init_matched = self.peek_literal_delimiter(&init);
                if init_matched {
                    self.align_mandatory_text(&term.properties.encoding)?;
                    self.match_literal_delimiter(&init)?;
                    self.on_initiator_matched(is_occurrence_pou, parent_initiated_content);
                } else if (evdp == crate::schema::ir::EmptyValueDelimiterPolicy::None
                    || evdp == crate::schema::ir::EmptyValueDelimiterPolicy::Terminator)
                    && (self.reader.is_eof() || self.peek_any_in_scope_delimiter())
                {
                    skipped_delimiters_for_empty = true;
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Initiator '{}' not found for element '{}'", init, elem.name.local_name),
                    ));
                }
            }
        }

        let res = match &elem.type_ir {
            crate::schema::ir::CompiledType::Simple(st) => {
                let simple_term = if let Some(ref term_raw) = term.properties.terminator {
                    let evaluated = self.evaluate_property_str_at(
                        term_raw,
                        builder,
                        Some(&elem.name.local_name),
                    )?;
                    if evaluated.is_empty() {
                        None
                    } else {
                        Some(evaluated)
                    }
                } else {
                    None
                };
                if let Some(ref t) = simple_term {
                    self.push_in_scope_terminator(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                    self.push_in_scope_delimiter(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                }
                let elem_pname = elem.name.prefixed_name();
                let simple_val_res = if (term.properties.rep_type.is_some()
                    || term.properties.rep_simple_type.is_some())
                    && !term.properties.facets.rep_values.is_empty()
                {
                    let rep_st = term.properties.rep_simple_type.unwrap_or(*st);
                    let mut rep_props = term.properties.clone();
                    rep_props.facets = crate::schema::ir::SimpleTypeFacets::default();
                    let wire_res = self.parse_simple_value(
                        rep_st,
                        Some(&elem_pname),
                        &rep_props,
                        builder,
                    )?;
                    let wire_str = alloc::format!("{}", wire_res);
                    if let Some((enum_str, _)) = term
                        .properties
                        .facets
                        .rep_values
                        .iter()
                        .find(|(_, r)| r == &wire_str)
                    {
                        Ok(DfdlValue::String(enum_str.clone()))
                    } else {
                        let msg = alloc::format!(
                            "Parse Error: Value '{}' not found in enumeration repValues",
                            wire_str
                        );
                        Err(DFDLError::new(DFDLErrorKind::Parse, &msg))
                    }
                } else {
                    self.parse_simple_value(
                        *st,
                        Some(&elem_pname),
                        &term.properties,
                        builder,
                    )
                };
                if simple_term.is_some() {
                    let _ = self.in_scope_terminators.pop();
                    let _ = self.in_scope_delimiters.pop();
                }
                let value = simple_val_res?;
                if (term.properties.rep_type.is_some() || term.properties.rep_simple_type.is_some())
                    && self.validation_mode != ValidationMode::Off
                    && term.properties.facets.has_facets()
                {
                    if let Err(e) = term.properties.facets.validate_value_detailed(&value) {
                        let msg_str = e.message.as_str();
                        let detail = msg_str.strip_prefix("Validation Error: ").unwrap_or(msg_str);
                        let msg = alloc::format!("Validation Error: element '{}' {}", elem.name.prefixed_name(), detail);
                        let _ = crate::util::try_push(&mut self.validation_errors, DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
                builder.push_event_with_hidden(
                    InfosetEvent::SimpleValue {
                        name: elem.name.clone(),
                        value,
                    },
                    is_hidden,
                )
            }
            crate::schema::ir::CompiledType::Complex(child_id) => {
                let complex_len = self.evaluate_length_property(
                    &term.properties,
                    Some(&elem.name.local_name),
                    builder,
                )?;
                if let Some(l) = complex_len {
                    let _ = try_push(
                        &mut self.enclosing_complex_elements,
                        (
                            elem.name.local_name.clone(),
                            l,
                            term.properties.length_units,
                            term.properties.encoding.clone(),
                        ),
                    );
                }
                let start_bit = self.reader.position().0;
                let expected_bits = match (term.properties.length_kind, complex_len) {
                    (
                        crate::schema::ir::LengthKind::Explicit
                        | crate::schema::ir::LengthKind::Prefixed
                        | crate::schema::ir::LengthKind::Pattern,
                        Some(l),
                    ) => {
                        let bits = match term.properties.length_units {
                            crate::schema::ir::LengthUnits::Bits => l,
                            crate::schema::ir::LengthUnits::Bytes => l.saturating_mul(8),
                            crate::schema::ir::LengthUnits::Characters => {
                                l.saturating_mul(crate::encoding::encoding_unit_bits(&term.properties.encoding))
                            }
                        };
                        Some(bits)
                    }
                    _ => None,
                };
                builder.push_event_with_hidden(
                    InfosetEvent::StartElement {
                        name: elem.name.clone(),
                        is_nil: false,
                    },
                    is_hidden,
                )?;
                let eval_term = if let Some(ref raw_term) = term.properties.terminator {
                    if raw_term.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_property_str_at(
                            raw_term,
                            builder,
                            Some(&elem.name.local_name),
                        )?;
                        if evaluated.is_empty() {
                            None
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                if let Some(ref t) = eval_term {
                    self.push_in_scope_terminator(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                    self.push_in_scope_delimiter(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                }
                let prev_bit_limit = self.reader.bit_limit();
                if let Some(exp_bits) = expected_bits {
                    let limit = start_bit.saturating_add(exp_bits);
                    self.reader.set_bit_limit(Some(match prev_bit_limit {
                        Some(l) => l.min(limit),
                        None => limit,
                    }));
                }
                let child_res = self.parse_term(*child_id, builder);
                if expected_bits.is_some() {
                    self.reader.set_bit_limit(prev_bit_limit);
                }
                if eval_term.is_some() {
                    let _ = self.in_scope_terminators.pop();
                    let _ = self.in_scope_delimiters.pop();
                }
                if complex_len.is_some() {
                    let _ = self.enclosing_complex_elements.pop();
                }
                if child_res.is_ok() {
                    if let Some(exp_bits) = expected_bits {
                        let current_bit = self.reader.position().0;
                        let target_bit = start_bit.saturating_add(exp_bits);
                        if current_bit < target_bit {
                            let to_skip = target_bit.saturating_sub(current_bit);
                            self.reader.skip_bits(to_skip)?;
                        } else if current_bit > target_bit {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Complex element child data exceeded explicit length",
                            ));
                        }
                    }
                    builder.push_event_with_hidden(
                        InfosetEvent::EndElement {
                            name: elem.name.clone(),
                        },
                        is_hidden,
                    )?;
                }
                child_res
            }
        };

        if res.is_ok() {
            if let Some(ref term_raw) = term.properties.terminator {
                let skip_term = skipped_delimiters_for_empty
                    || (evdp == crate::schema::ir::EmptyValueDelimiterPolicy::None
                        && (self.reader.is_eof() || self.peek_any_in_scope_delimiter()))
                    || (self.reader.is_eof() && term.properties.document_final_terminator_can_be_missing);
                if !skip_term {
                    let term_str = self.evaluate_property_str_at(
                        term_raw,
                        builder,
                        Some(&elem.name.local_name),
                    )?;
                    if !term_str.is_empty() {
                        self.align_mandatory_text(&term.properties.encoding)?;
                        self.match_literal_delimiter(&term_str).map_err(|e| {
                            DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Terminator '{}' not found: {}", term_raw, e.message),
                            )
                        })?;
                    }
                }
            }
            self.execute_set_variables(term, builder, Some(&elem.name.local_name))?;
            self.evaluate_asserts(term, Some(&elem.name.local_name), builder)?;
            if term.properties.discriminator.is_some()
                && term.properties.discriminator_test_kind != crate::schema::ir::TestKind::Pattern
            {
                self.evaluate_discriminator(term, Some(&elem.name.local_name), builder)?;
            }
        }
        res
    }

    pub(crate) fn evaluate_pattern_asserts(
        &mut self,
        term: &CompiledTerm,
        _elem_name: Option<&str>,
        _builder: &InfosetBuilder,
    ) -> DFDLResult<()> {
        for assert_item in &term.properties.asserts {
            if assert_item.test_kind != crate::schema::ir::TestKind::Pattern {
                continue;
            }
            let reader_cp = self.reader.checkpoint();
            let mut bytes = Vec::new();
            for _ in 0..1024 {
                if self.reader.is_eof() {
                    break;
                }
                if let Ok(b) = self.reader.read_bits(8) {
                    let _ = try_push(&mut bytes, b as u8);
                } else {
                    break;
                }
            }
            let _ = self.reader.rollback(reader_cp);
            let raw_text_peek =
                crate::encoding::decode_text_bytes(&bytes, &term.properties.encoding)
                    .unwrap_or_default();

            let anchored_pat = if let Some(stripped) = assert_item.test_expr.strip_prefix('^') {
                alloc::format!("^(?:{})", stripped)
            } else {
                alloc::format!("^(?:{})", assert_item.test_expr)
            };

            let is_true = if let Ok(re) = crate::pattern::DfdlRegex::new(&anchored_pat) {
                re.is_match(&raw_text_peek)
            } else if let Ok(re) = crate::pattern::DfdlRegex::new(&assert_item.test_expr) {
                re.is_match(&raw_text_peek)
            } else {
                false
            };

            if !is_true {
                if assert_item.failure_type == crate::schema::ir::FailureType::RecoverableError {
                    continue;
                }
                let msg = assert_item.message.clone().unwrap_or_else(|| {
                    alloc::format!("Parse Error: Assertion failed for pattern '{}'", assert_item.test_expr)
                });
                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate_discriminator(
        &mut self,
        term: &CompiledTerm,
        elem_name: Option<&str>,
        builder: &InfosetBuilder,
    ) -> DFDLResult<()> {
        if let Some(ref disc_expr) = term.properties.discriminator {
            let mut current_path = builder.current_path();
            if let Some(name) = elem_name {
                let clean_name = name.split(':').next_back().unwrap_or(name);
                let last_seg = current_path
                    .segments()
                    .last()
                    .map(|s| s.split(':').next_back().unwrap_or(s));
                if last_seg != Some(clean_name) {
                    let _ = current_path.try_push(clean_name);
                }
            }
            let active_doc = builder.active_doc();
            let mut ctx = crate::expr::ExprContext::with_variable_map(
                Some(&active_doc),
                &current_path,
                &[],
                Some(&self.variable_map),
                self.budget,
            )
            .with_occurs_index(self.current_occurs_index)
            .with_schema(self.schema)
            .with_namespaces(&term.properties.in_scope_namespaces)
            .with_enclosing_lengths(&self.enclosing_complex_elements);

            let is_true = match term.properties.discriminator_test_kind {
                crate::schema::ir::TestKind::Expression => {
                    if disc_expr.trim().is_empty() {
                        true
                    } else {
                        let ast = crate::expr::parse_expr(disc_expr)?;
                        match crate::expr::eval_expr(&ast, &mut ctx) {
                            Ok(val) => val.effective_boolean_value(),
                            Err(e) if e.kind == DFDLErrorKind::SchemaDefinition && !e.message.as_str().contains("does not exist") => {
                                return Err(e);
                            }
                            Err(_) => false,
                        }
                    }
                }
                crate::schema::ir::TestKind::Pattern => {
                    if disc_expr.trim().is_empty() {
                        true
                    } else {
                        let text_to_match = if let Some(elem) = active_doc
                            .find_element_with_context(&current_path, self.current_occurs_index)
                        {
                            match &elem.state {
                                crate::infoset::state::ElementState::Value(v) => match v {
                                    DfdlValue::String(s) => s.clone(),
                                    _ => alloc::format!("{}", v),
                                },
                                _ => String::new(),
                            }
                        } else {
                            String::new()
                        };

                        let reader_cp = self.reader.checkpoint();
                        let mut bytes = Vec::new();
                        for _ in 0..1024 {
                            if self.reader.is_eof() {
                                break;
                            }
                            if let Ok(b) = self.reader.read_bits(8) {
                                let _ = try_push(&mut bytes, b as u8);
                            } else {
                                break;
                            }
                        }
                        let _ = self.reader.rollback(reader_cp);
                        let raw_text_peek =
                            crate::encoding::decode_text_bytes(&bytes, &term.properties.encoding)
                                .unwrap_or_default();

                        let anchored_pat = if let Some(stripped) = disc_expr.strip_prefix('^') {
                            alloc::format!("^(?:{})", stripped)
                        } else {
                            alloc::format!("^(?:{})", disc_expr)
                        };

                        if let Ok(re) = crate::pattern::DfdlRegex::new(&anchored_pat) {
                            re.is_match(&raw_text_peek)
                                || (!text_to_match.is_empty() && re.is_match(&text_to_match))
                        } else if let Ok(re) = crate::pattern::DfdlRegex::new(disc_expr) {
                            re.is_match(&raw_text_peek)
                                || (!text_to_match.is_empty() && re.is_match(&text_to_match))
                        } else {
                            false
                        }
                    }
                }
            };

            if !is_true {
                let msg = if let Some(ref m_str) = term.properties.discriminator_message {
                    if m_str.starts_with('{') && m_str.ends_with('}') {
                        if let Ok(ast) = crate::expr::parse_expr(m_str) {
                            if let Ok(eval_val) = crate::expr::eval_expr(&ast, &mut ctx) {
                                alloc::format!("{}", eval_val)
                            } else {
                                m_str.clone()
                            }
                        } else {
                            m_str.clone()
                        }
                    } else {
                        m_str.clone()
                    }
                } else {
                    alloc::format!("Discriminator failed for '{}'", disc_expr)
                };
                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
            } else if let Some(pou) = self.pou_stack.iter_mut().rev().find(|p| !p.is_discriminated) {
                pou.is_discriminated = true;
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate_asserts(
        &mut self,
        term: &CompiledTerm,
        elem_name: Option<&str>,
        builder: &InfosetBuilder,
    ) -> DFDLResult<()> {
        for assert_item in &term.properties.asserts {
            let mut current_path = builder.current_path();
            if let Some(name) = elem_name {
                let clean_name = name.split(':').next_back().unwrap_or(name);
                let last_seg = current_path
                    .segments()
                    .last()
                    .map(|s| s.split(':').next_back().unwrap_or(s));
                if last_seg != Some(clean_name) {
                    let _ = current_path.try_push(clean_name);
                }
            }
            let active_doc = builder.active_doc();
            let mut ctx = crate::expr::ExprContext::with_variable_map(
                Some(&active_doc),
                &current_path,
                &[],
                Some(&self.variable_map),
                self.budget,
            )
            .with_occurs_index(self.current_occurs_index)
            .with_schema(self.schema)
            .with_namespaces(&term.properties.in_scope_namespaces)
            .with_enclosing_lengths(&self.enclosing_complex_elements);

            if assert_item.test_kind == crate::schema::ir::TestKind::Pattern {
                continue;
            }

            let ast = crate::expr::parse_expr(&assert_item.test_expr)?;
            let val = crate::expr::eval_expr(&ast, &mut ctx)?;
            let is_true = val.effective_boolean_value();

            if !is_true {
                let msg = if let Some(ref m_str) = assert_item.message {
                    if m_str.starts_with('{') && m_str.ends_with('}') {
                        if let Ok(ast) = crate::expr::parse_expr(m_str) {
                            if let Ok(eval_val) = crate::expr::eval_expr(&ast, &mut ctx) {
                                alloc::format!("{}", eval_val)
                            } else {
                                m_str.clone()
                            }
                        } else {
                            m_str.clone()
                        }
                    } else {
                        m_str.clone()
                    }
                } else {
                    alloc::string::String::from("dfdl:assert evaluation failed")
                };
                if assert_item.failure_type == crate::schema::ir::FailureType::RecoverableError {
                    continue;
                }
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate_length_property(
        &mut self,
        props: &ResolvedProperties,
        elem_name: Option<&str>,
        builder: &InfosetBuilder,
    ) -> DFDLResult<Option<usize>> {
        if props.length_kind == crate::schema::ir::LengthKind::Prefixed {
            let ptype = props
                .prefix_length_type
                .as_deref()
                .unwrap_or("xs:unsignedShort");
            let parts: Vec<&str> = ptype.split(':').collect();
            let len_part = parts.get(2).copied().unwrap_or("");
            // DFDL §12.3.4: Chained/nested prefixLengthType descriptors are encoded as `@clean_nested,rep,len,units,min,max@`.
            // When present, recursively read the nested prefix length first to determine the length of the current prefix.
            let nested_prefix_len = if len_part.starts_with('@') && len_part.ends_with('@') && len_part.len() >= 2 {
                let inner = &len_part[1..len_part.len() - 1];
                let nparts: Vec<&str> = inner.split(',').collect();
                let n_rep = nparts.get(1).copied().unwrap_or("binary");
                let n_len: usize = nparts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
                let n_units = nparts.get(3).copied().unwrap_or("bytes");
                let n_is_text = n_rep.eq_ignore_ascii_case("text");
                let n_bits = if n_units.eq_ignore_ascii_case("bits") {
                    n_len
                } else {
                    n_len.saturating_mul(8)
                };
                let n_val = if n_is_text {
                    let byte_count = n_bits.div_ceil(8);
                    let mut bvec = Vec::new();
                    for _ in 0..byte_count {
                        let b = self.read_binary_bits(8)? as u8;
                        try_push(&mut bvec, b)?;
                    }
                    let s = crate::encoding::decode_text_bytes(&bvec, &props.encoding).unwrap_or_default();
                    s.trim().parse::<usize>().unwrap_or(0)
                } else {
                    self.read_binary_bits(n_bits)? as usize
                };
                Some(n_val)
            } else {
                None
            };
            let (is_text, bits) = if parts.len() >= 4 {
                let rep = parts.get(1).copied().unwrap_or("binary");
                let units = parts.get(3).copied().unwrap_or("bytes");
                let is_txt = rep.eq_ignore_ascii_case("text");
                let num_len: usize = if let Some(n_val) = nested_prefix_len {
                    n_val
                } else {
                    parts
                        .get(2)
                        .filter(|s| !s.is_empty())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| {
                            let clean_ptype = parts.first().copied().unwrap_or(ptype);
                            match clean_ptype {
                                "byte" | "unsignedByte" => 1,
                                "short" | "unsignedShort" => 2,
                                "int" | "unsignedInt" => 4,
                                "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                                _ => 2,
                            }
                        })
                };
                let b = if units.eq_ignore_ascii_case("bits") {
                    num_len
                } else {
                    num_len.saturating_mul(8)
                };
                (is_txt, b)
            } else {
                let clean_ptype = parts.last().copied().unwrap_or(ptype);
                let bits = match clean_ptype {
                    "byte" | "unsignedByte" => 8,
                    "short" | "unsignedShort" => 16,
                    "int" | "unsignedInt" => 32,
                    "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 64,
                    _ => 16,
                };
                (false, bits)
            };

            let pad_char = parts.get(6).copied().unwrap_or("");
            let len = if is_text {
                let byte_count = bits.div_ceil(8);
                let mut bytes = Vec::new();
                for _ in 0..byte_count {
                    let b = self.read_binary_bits(8)? as u8;
                    try_push(&mut bytes, b)?;
                }
                let s =
                    crate::encoding::decode_text_bytes(&bytes, &props.encoding).unwrap_or_default();
                let trimmed = if !pad_char.is_empty() {
                    s.trim_matches(|c: char| c.is_whitespace() || pad_char.contains(c))
                } else {
                    s.trim()
                };
                if let Ok(v) = trimmed.parse::<i64>() {
                    if v < 0 {
                        let msg = alloc::format!(
                            "Runtime Schema Definition Error: Prefixed length result ({v}) must be non-negative"
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    v as usize
                } else if trimmed.starts_with('-') {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: Prefixed length result ({trimmed}) must be non-negative"
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                } else {
                    s.trim_matches(|c: char| !c.is_ascii_digit()).parse::<usize>().unwrap_or(0)
                }
            } else {
                let prefix_val = self.read_binary_bits(bits)?;
                match props.byte_order {
                    ByteOrder::BigEndian => prefix_val as usize,
                    ByteOrder::LittleEndian => match bits {
                        16 => (prefix_val as u16).swap_bytes() as usize,
                        24 => {
                            let b0 = (prefix_val & 0xFF) as usize;
                            let b1 = ((prefix_val >> 8) & 0xFF) as usize;
                            let b2 = ((prefix_val >> 16) & 0xFF) as usize;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (prefix_val as u32).swap_bytes() as usize,
                        64 => prefix_val.swap_bytes() as usize,
                        _ => prefix_val as usize,
                    },
                }
            };
            let min_inc = parts.get(4).copied().unwrap_or("");
            let max_inc = parts.get(5).copied().unwrap_or("");
            if !max_inc.is_empty() {
                if let Ok(max) = max_inc.parse::<usize>() {
                    if len > max {
                        let name = elem_name.unwrap_or("element");
                        let msg = alloc::format!("Parse Error: failed check {name} ({len}) facet maxInclusive ({max})");
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                }
            }
            if !min_inc.is_empty() {
                if let Ok(min) = min_inc.parse::<usize>() {
                    if len < min {
                        let name = elem_name.unwrap_or("element");
                        let msg = alloc::format!("Parse Error: failed check {name} ({len}) facet minInclusive ({min})");
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                }
            }

            let effective_len = if props.prefix_includes_prefix_length {
                let prefix_units = match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => bits,
                    crate::schema::ir::LengthUnits::Bytes
                    | crate::schema::ir::LengthUnits::Characters => bits.div_ceil(8),
                };
                if len < prefix_units {
                    let diff = (len as i64) - (prefix_units as i64);
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: Prefixed length result ({len}) after dfdl:prefixIncludesPrefixLength adjustment is negative ({diff}), must be non-negative"
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                len - prefix_units
            } else {
                len
            };
            return Ok(Some(effective_len));
        }

        if props.length_kind == crate::schema::ir::LengthKind::Pattern {
            if let Some(ref lp_str) = props.length_pattern {
                let reader_cp = self.reader.checkpoint();
                let sub_byte_bits = crate::encoding::encoding_char_bits(&props.encoding);
                let mut bytes = Vec::new();
                let mut sub_byte_text = String::new();
                for _ in 0..4096 {
                    if self.reader.is_eof() {
                        break;
                    }
                    if let Some(cb) = sub_byte_bits {
                        match self.reader.read_bits(cb) {
                            Ok(code) => sub_byte_text
                                .push(crate::encoding::decode_sub_byte_char(code, &props.encoding)),
                            Err(_) => break,
                        }
                    } else if let Ok(b) = self.reader.read_bits(8) {
                        let _ = try_push(&mut bytes, b as u8);
                    } else {
                        break;
                    }
                }
                let _ = self.reader.rollback(reader_cp);
                // DFDL §12.3.4 (lengthKind="pattern"):
                // Peek ahead and decode text according to the element's encoding while tracking
                // byte offsets so character positions accurately map to source bytes or bits.
                let (text_peek, offsets) = if sub_byte_bits.is_some() {
                    (sub_byte_text, Vec::new())
                } else {
                    crate::encoding::decode_text_bytes_with_offsets(&bytes, &props.encoding)
                };
                if let Ok(re) = crate::pattern::DfdlRegex::new(lp_str) {
                    if let Some(m_end) = re.match_prefix_len(&text_peek) {
                        let chars = text_peek.get(..m_end).map_or(0, |t| t.chars().count());
                        let result_len = match sub_byte_bits {
                            // Sub-byte text (e.g. 5-bit, 6-bit, 7-bit): length is in bits, characters, or rounded bytes.
                            Some(cb) => match props.length_units {
                                crate::schema::ir::LengthUnits::Bits => chars.saturating_mul(cb),
                                crate::schema::ir::LengthUnits::Bytes => chars.saturating_mul(cb).div_ceil(8),
                                crate::schema::ir::LengthUnits::Characters => chars,
                            },
                            // Byte-aligned and multi-byte text (UTF-8, UTF-16, UTF-32, ISO-8859-*, ASCII, EBCDIC):
                            None => match props.length_units {
                                crate::schema::ir::LengthUnits::Characters => chars,
                                crate::schema::ir::LengthUnits::Bytes => {
                                    offsets.get(chars).copied().unwrap_or(chars)
                                }
                                crate::schema::ir::LengthUnits::Bits => {
                                    offsets.get(chars).copied().unwrap_or(chars).saturating_mul(8)
                                }
                            },
                        };
                        return Ok(Some(result_len));
                    }
                }
                // DFDL 1.0 §12.3.4: If the regular expression does not match at the current position,
                // the length of the element is zero.
                return Ok(Some(0));
            }
        }

        if props.length_kind != crate::schema::ir::LengthKind::Explicit {
            if props.length_kind == crate::schema::ir::LengthKind::Implicit
                && props.representation == crate::schema::ir::Representation::Text
                && props.length.is_some()
            {
                return Ok(props.length);
            }
            return Ok(None);
        }

        if let Some(ref expr_str) = props.length_expr {
            let ast = crate::expr::parse_expr(expr_str)?;
            let mut current_path = builder.current_path();
            if let Some(name) = elem_name {
                let clean_name = name.split(':').next_back().unwrap_or(name);
                let last_seg = current_path.segments().last().map(|s| s.as_str());
                if last_seg != Some(clean_name) {
                    let _ = current_path.try_push(clean_name);
                }
            }
            let active_doc = builder.active_doc();
            let mut ctx = crate::expr::ExprContext::with_variable_map(
                Some(&active_doc),
                &current_path,
                &[],
                Some(&self.variable_map),
                self.budget,
            )
            .with_occurs_index(self.current_occurs_index)
            .with_schema(self.schema)
            .with_namespaces(&props.in_scope_namespaces)
            .with_enclosing_lengths(&self.enclosing_complex_elements);
            let val = crate::expr::eval_expr(&ast, &mut ctx)?;
            let len = match val {
                DfdlValue::Int(v) if v >= 0 => v as usize,
                DfdlValue::Long(v) if v >= 0 => v as usize,
                DfdlValue::UnsignedInt(v) => v as usize,
                DfdlValue::UnsignedLong(v) => v as usize,
                DfdlValue::Short(v) if v >= 0 => v as usize,
                DfdlValue::Byte(v) if v >= 0 => v as usize,
                DfdlValue::UnsignedShort(v) => v as usize,
                DfdlValue::UnsignedByte(v) => v as usize,
                DfdlValue::String(ref s) => s.trim().parse::<usize>().unwrap_or(0),
                _ => 0,
            };
            Ok(Some(len))
        } else {
            Ok(props.length)
        }
    }

    pub(crate) fn parse_simple_value(
        &mut self,
        simple_type: DfdlSimpleType,
        elem_name: Option<&str>,
        props: &ResolvedProperties,
        builder: &InfosetBuilder,
    ) -> DFDLResult<DfdlValue> {
        let mut local_props;
        let props = if (props.encoding.starts_with('{') && props.encoding.ends_with('}'))
            || props.byte_order_expr.is_some()
            || props.text_boolean_true_rep.as_ref().is_some_and(|s| s.starts_with('{') && s.ends_with('}'))
            || props.text_boolean_false_rep.as_ref().is_some_and(|s| s.starts_with('{') && s.ends_with('}'))
            || props.text_standard_exponent_rep.as_ref().is_some_and(|s| s.starts_with('{') && s.ends_with('}'))
        {
            local_props = props.clone();
            if let Some(ref bo_expr) = props.byte_order_expr {
                let bo = self.evaluate_property_str_at(bo_expr, builder, elem_name)?;
                local_props.byte_order = crate::expr::properties::parse_byte_order(&bo)?;
            }
            if props.encoding.starts_with('{') && props.encoding.ends_with('}') {
                let dyn_encoding = self.evaluate_property_str_at(&props.encoding, builder, elem_name)?;
                let dyn_upper = dyn_encoding.to_ascii_uppercase();
                if dyn_upper.contains("6-BIT")
                    || dyn_upper.contains("5-BIT")
                    || dyn_upper.contains("7-BIT")
                    || dyn_upper.contains("BIT-PACKED")
                {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: Only encodings with byte-sized code units can be computed via expressions. The encoding '{}' must be specified as a literal encoding name in the DFDL schema.",
                        dyn_encoding
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                local_props.encoding = dyn_encoding;
            }
            if let Some(ref rep) = props.text_boolean_true_rep {
                if rep.starts_with('{') && rep.ends_with('}') {
                    local_props.text_boolean_true_rep =
                        Some(self.evaluate_property_str_at(rep, builder, elem_name)?);
                }
            }
            if let Some(ref rep) = props.text_boolean_false_rep {
                if rep.starts_with('{') && rep.ends_with('}') {
                    local_props.text_boolean_false_rep =
                        Some(self.evaluate_property_str_at(rep, builder, elem_name)?);
                }
            }
            if let Some(ref exp) = props.text_standard_exponent_rep {
                if exp.starts_with('{') && exp.ends_with('}') {
                    local_props.text_standard_exponent_rep =
                        Some(self.evaluate_property_str_at(exp, builder, elem_name)?);
                }
            }
            &local_props
        } else {
            props
        };

        let dynamic_len = self.evaluate_length_property(props, elem_name, builder)?;
        let rep = if simple_type == DfdlSimpleType::String {
            Representation::Text
        } else if simple_type == DfdlSimpleType::HexBinary {
            // DFDL 1.0 §12.3 & §13.7: xs:hexBinary is raw binary data and must always
            // use binary representation even if representation="text" is inherited.
            Representation::Binary
        } else {
            props.representation
        };

        let val = match rep {
            Representation::Binary => self.parse_binary_value(simple_type, props, dynamic_len),
            Representation::Text => self.parse_text_value(simple_type, props, dynamic_len, builder, elem_name),
        }?;
        if props.string_as_xml {
            if let DfdlValue::String(ref s) = val {
                validate_string_as_xml(s)?;
            }
        }
        if self.validation_mode != ValidationMode::Off && props.facets.has_facets() {
            if let Err(e) = props.facets.validate_value_detailed(&val) {
                let name = elem_name.unwrap_or("element");
                let msg_str = e.message.as_str();
                let detail = msg_str.strip_prefix("Validation Error: ").unwrap_or(msg_str);
                let msg = alloc::format!("Validation Error: element '{}' {}", name, detail);
                let _ = crate::util::try_push(&mut self.validation_errors, DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }
        Ok(val)
    }
}

/// Validates that a string is well-formed XML for `dfdlx:runtimeProperties="stringAsXml=true"`.
fn validate_string_as_xml(s: &str) -> DFDLResult<()> {
    let mut tag_stack: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    let mut element_count: usize = 0;
    let tokenizer = xmlparser::Tokenizer::from(s);
    for token_res in tokenizer {
        let token = token_res.map_err(|e| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!("Parse Error: XML parsing error: {}", e),
            )
        })?;
        match token {
            xmlparser::Token::ElementStart { local, .. } => {
                element_count = element_count.saturating_add(1);
                tag_stack.push(alloc::string::String::from(local.as_str()));
            }
            xmlparser::Token::Attribute { value, .. } => {
                check_xml_entities(value.as_str())?;
            }
            xmlparser::Token::Text { text } => {
                check_xml_entities(text.as_str())?;
            }
            xmlparser::Token::ElementEnd { end, .. } => match end {
                xmlparser::ElementEnd::Open => {}
                xmlparser::ElementEnd::Empty => {
                    tag_stack.pop();
                }
                xmlparser::ElementEnd::Close(_, local) => {
                    if let Some(top) = tag_stack.pop() {
                        if top != local.as_str() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!(
                                    "Parse Error: Mismatched closing tag: expected <{}>, found </{}>",
                                    top,
                                    local.as_str()
                                ),
                            ));
                        }
                    } else {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parse Error: Unexpected closing tag </{}>",
                                local.as_str()
                            ),
                        ));
                    }
                }
            },
            _ => {}
        }
    }
    if !tag_stack.is_empty() {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: XML document ended with unclosed tags",
        ));
    }
    if element_count == 0 {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: XML document must contain at least one element",
        ));
    }
    Ok(())
}

fn check_xml_entities(text: &str) -> DFDLResult<()> {
    let mut rest = text;
    while let Some(amp_pos) = rest.find('&') {
        if let Some(after_amp) = rest.get(amp_pos.saturating_add(1)..) {
            if let Some(semi_pos) = after_amp.find(';') {
                if let Some(ent) = after_amp.get(..semi_pos) {
                    if ent != "amp"
                        && ent != "lt"
                        && ent != "gt"
                        && ent != "quot"
                        && ent != "apos"
                        && !ent.starts_with('#')
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!("Parse Error: Undeclared general entity \"{}\"", ent),
                        ));
                    }
                }
                rest = after_amp.get(semi_pos.saturating_add(1)..).unwrap_or("");
            } else {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Parse Error: Unterminated entity reference",
                ));
            }
        } else {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_string_as_xml_valid() {
        let xml = "<p:root xmlns:p=\"urn:payload\"><field foo=\"bar\">hello &amp; world</field></p:root>";
        assert!(validate_string_as_xml(xml).is_ok());
    }

    #[test]
    fn test_validate_string_as_xml_unclosed_attr() {
        let xml = "<field foo=\"unclosed attr>invalid xml</field>";
        let res = validate_string_as_xml(xml);
        assert!(res.is_err());
        let err = alloc::format!("{}", res.unwrap_err());
        assert!(err.contains("XML parsing error"));
    }

    #[test]
    fn test_validate_string_as_xml_undeclared_entity() {
        let xml = "<p:root><field>&name;</field></p:root>";
        let res = validate_string_as_xml(xml);
        assert!(res.is_err());
        let err = alloc::format!("{}", res.unwrap_err());
        assert!(err.contains("Undeclared general entity \"name\""));
    }

    #[test]
    fn test_validate_string_as_xml_mismatched_tag() {
        let xml = "<root><field>value</wrong></root>";
        let res = validate_string_as_xml(xml);
        assert!(res.is_err());
        let err = alloc::format!("{}", res.unwrap_err());
        assert!(err.contains("Mismatched closing tag"));
    }
}
