//! Utility and evaluation functions for DFDL property lowering and type coercions.

use alloc::string::String;

use dfdl_core::expr::variables::VariableMap;
use dfdl_core::infoset::value::{DfdlSimpleType, DfdlValue};
use dfdl_core::schema::ir::CompiledType;
use dfdl_core::types::QName;

pub(crate) fn compute_implicit_alignment(
    compiled_type: &CompiledType,
    representation: dfdl_core::schema::ir::Representation,
    encoding: &dfdl_core::encoding::EncodingKind,
    alignment_units: dfdl_core::schema::ir::AlignmentUnits,
) -> usize {
    use dfdl_core::schema::ir::{AlignmentUnits, Representation};

    let is_numeric = matches!(
        compiled_type,
        CompiledType::Simple(st) if st.is_numeric()
    );
    if is_numeric && representation == Representation::Text {
        return if alignment_units == AlignmentUnits::Bits { 8 } else { 1 };
    }

    let is_string = matches!(compiled_type, CompiledType::Simple(DfdlSimpleType::String));
    if representation == Representation::Text || is_string {
        if alignment_units == AlignmentUnits::Bits {
            encoding.mandatory_alignment_bits()
        } else {
            match encoding {
                dfdl_core::encoding::EncodingKind::Utf16Be
                | dfdl_core::encoding::EncodingKind::Utf16Le => 2,
                dfdl_core::encoding::EncodingKind::Utf32Be
                | dfdl_core::encoding::EncodingKind::Utf32Le => 4,
                _ => 1,
            }
        }
    } else {
        match compiled_type {
            CompiledType::Simple(st) => match st {
                DfdlSimpleType::Boolean => 1,
                DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
                DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => {
                    if alignment_units == AlignmentUnits::Bits {
                        16
                    } else {
                        2
                    }
                }
                DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt | DfdlSimpleType::Float => {
                    if alignment_units == AlignmentUnits::Bits {
                        32
                    } else {
                        4
                    }
                }
                DfdlSimpleType::Long | DfdlSimpleType::UnsignedLong | DfdlSimpleType::Double => {
                    if alignment_units == AlignmentUnits::Bits {
                        64
                    } else {
                        8
                    }
                }
                DfdlSimpleType::HexBinary | DfdlSimpleType::Decimal => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
                DfdlSimpleType::Date | DfdlSimpleType::Time | DfdlSimpleType::DateTime => {
                    if alignment_units == AlignmentUnits::Bits {
                        32
                    } else {
                        4
                    }
                }
                DfdlSimpleType::String => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
            },
            CompiledType::Complex(_) => {
                if alignment_units == AlignmentUnits::Bits {
                    8
                } else {
                    1
                }
            }
        }
    }
}

pub(crate) fn coerce_dfdl_value(val: &DfdlValue, simple_type: DfdlSimpleType) -> Option<DfdlValue> {
    let s = alloc::string::ToString::to_string(val);
    let trimmed = s.trim();

    match simple_type {
        DfdlSimpleType::Int => trimmed.parse::<i32>().ok().map(DfdlValue::Int),
        DfdlSimpleType::Long => trimmed.parse::<i64>().ok().map(DfdlValue::Long),
        DfdlSimpleType::Short => trimmed.parse::<i16>().ok().map(DfdlValue::Short),
        DfdlSimpleType::Byte => trimmed.parse::<i8>().ok().map(DfdlValue::Byte),
        DfdlSimpleType::UnsignedLong => trimmed.parse::<u64>().ok().map(DfdlValue::UnsignedLong),
        DfdlSimpleType::UnsignedInt => trimmed.parse::<u32>().ok().map(DfdlValue::UnsignedInt),
        DfdlSimpleType::UnsignedShort => trimmed.parse::<u16>().ok().map(DfdlValue::UnsignedShort),
        DfdlSimpleType::UnsignedByte => trimmed.parse::<u8>().ok().map(DfdlValue::UnsignedByte),
        DfdlSimpleType::Boolean => match trimmed {
            "true" | "1" => Some(DfdlValue::Boolean(true)),
            "false" | "0" => Some(DfdlValue::Boolean(false)),
            _ => None,
        },
        DfdlSimpleType::Float => trimmed.parse::<f32>().ok().map(DfdlValue::Float),
        DfdlSimpleType::Double => trimmed.parse::<f64>().ok().map(DfdlValue::Double),
        DfdlSimpleType::String | DfdlSimpleType::HexBinary => {
            Some(DfdlValue::String(String::from(trimmed)))
        }
        DfdlSimpleType::DateTime => Some(DfdlValue::DateTime(String::from(trimmed))),
        DfdlSimpleType::Date => Some(DfdlValue::Date(String::from(trimmed))),
        DfdlSimpleType::Time => Some(DfdlValue::Time(String::from(trimmed))),
        DfdlSimpleType::Decimal => Some(DfdlValue::Decimal(String::from(trimmed))),
    }
}

pub(crate) fn parse_default_value(
    val_str: &str,
    simple_type: DfdlSimpleType,
    vmap: Option<&VariableMap>,
) -> Option<DfdlValue> {
    let trimmed = val_str.trim();

    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
        if let Ok(ast) = dfdl_core::expr::parse_expr(trimmed) {
            let mut budget = dfdl_core::limits::WorkBudget::new(1000);
            let root_path = dfdl_core::types::InfosetPath::root();
            let mut ctx = dfdl_core::expr::ExprContext::with_variable_map(
                None,
                &root_path,
                &[],
                vmap,
                &mut budget,
            );
            if let Ok(val) = dfdl_core::expr::eval_expr(&ast, &mut ctx) {
                if let Some(coerced) = coerce_dfdl_value(&val, simple_type) {
                    return Some(coerced);
                }
            }
        }
        return None;
    }

    let fallback_val = DfdlValue::String(String::from(trimmed));
    coerce_dfdl_value(&fallback_val, simple_type)
}

/// Returns whether the given layer transform name is a supported/registered DFDL layer.
///
/// Supported layers include official Daffodil standard extension layers
/// (byte swapping, line folding, base64, AIS payload armoring, IPv4 checksum,
/// check digit, boundary mark, gzip) and test suite layers (bomb out layer, simple test layers,
/// all types layer).
#[must_use]
pub fn is_known_layer(layer_name: &str) -> bool {
    let parsed = QName::parse(layer_name);
    let clean = parsed.local_name.as_str();
    dfdl_core::kernel::layer::is_known_layer(clean)
}
