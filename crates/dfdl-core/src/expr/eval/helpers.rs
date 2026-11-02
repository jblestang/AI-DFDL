//! Helper conversion and mathematical functions for expression evaluation.

extern crate alloc;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::value::DfdlValue;

pub(crate) fn value_to_i64(val: &DfdlValue) -> i64 {
    match val {
        DfdlValue::Int(n) => *n as i64,
        DfdlValue::Long(n) => *n,
        DfdlValue::Short(n) => *n as i64,
        DfdlValue::Byte(n) => *n as i64,
        DfdlValue::UnsignedLong(n) => *n as i64,
        DfdlValue::UnsignedInt(n) => *n as i64,
        DfdlValue::UnsignedShort(n) => *n as i64,
        DfdlValue::UnsignedByte(n) => *n as i64,
        DfdlValue::Float(f) => *f as i64,
        DfdlValue::Double(f) => *f as i64,
        DfdlValue::String(s) | DfdlValue::Decimal(s) => s.trim().parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn validate_bitwise_operand(val: &DfdlValue, fn_name: &str) -> DFDLResult<(u64, bool)> {
    match val {
        DfdlValue::Long(n) => Ok((*n as u64, true)),
        DfdlValue::Int(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::Short(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::Byte(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::UnsignedLong(n) => Ok((*n, false)),
        DfdlValue::UnsignedInt(n) => Ok((*n as u64, false)),
        DfdlValue::UnsignedShort(n) => Ok((*n as u64, false)),
        DfdlValue::UnsignedByte(n) => Ok((*n as u64, false)),
        _ => {
            let msg = alloc::format!(
                "Schema Definition Error: Arguments to dfdlx:{} must be xs:unsignedLong or xs:long or a subtype of those",
                fn_name
            );
            Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
        }
    }
}

pub(crate) fn value_to_f64(val: &DfdlValue) -> f64 {
    match val {
        DfdlValue::Double(f) => *f,
        DfdlValue::Float(f) => *f as f64,
        DfdlValue::Int(n) => *n as f64,
        DfdlValue::Long(n) => *n as f64,
        DfdlValue::Short(n) => *n as f64,
        DfdlValue::Byte(n) => *n as f64,
        DfdlValue::UnsignedLong(n) => *n as f64,
        DfdlValue::UnsignedInt(n) => *n as f64,
        DfdlValue::UnsignedShort(n) => *n as f64,
        DfdlValue::UnsignedByte(n) => *n as f64,
        DfdlValue::String(s) | DfdlValue::Decimal(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

pub(crate) fn value_to_bool(val: &DfdlValue) -> bool {
    match val {
        DfdlValue::Boolean(b) => *b,
        DfdlValue::Int(n) => *n != 0,
        DfdlValue::Long(n) => *n != 0,
        DfdlValue::Short(n) => *n != 0,
        DfdlValue::Byte(n) => *n != 0,
        DfdlValue::UnsignedLong(n) => *n != 0,
        DfdlValue::UnsignedInt(n) => *n != 0,
        DfdlValue::UnsignedShort(n) => *n != 0,
        DfdlValue::UnsignedByte(n) => *n != 0,
        DfdlValue::Float(f) => *f != 0.0 && !f.is_nan(),
        DfdlValue::Double(f) => *f != 0.0 && !f.is_nan(),
        DfdlValue::String(s) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => false,
    }
}

/// Computes floor according to XPath 2.0 §6.4.3 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Preserves NaN, ±0.0, and ±INF.
/// - Handles all values |x| >= 2^52 without i64 saturation (since they have no fractional bits).
pub(crate) fn xpath_floor_f64(x: f64) -> f64 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    // IEEE 754 binary64 has 52 mantissa bits; values >= 2^52 are already exact integers.
    if x >= 4503599627370496.0 || x <= -4503599627370496.0 {
        return x;
    }
    let trunc = x as i64;
    if x >= 0.0 {
        trunc as f64
    } else if (trunc as f64) == x {
        x
    } else {
        (trunc.saturating_sub(1)) as f64
    }
}

/// Computes floor for f32 according to XPath 2.0 §6.4.3 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Preserves NaN, ±0.0, and ±INF.
/// - Handles all values |x| >= 2^23 without i64 saturation.
pub(crate) fn xpath_floor_f32(x: f32) -> f32 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    // IEEE 754 binary32 has 23 mantissa bits; values >= 2^23 are already exact integers.
    if x >= 8388608.0 || x <= -8388608.0 {
        return x;
    }
    let trunc = x as i64;
    if x >= 0.0 {
        trunc as f32
    } else if (trunc as f32) == x {
        x
    } else {
        (trunc.saturating_sub(1)) as f32
    }
}

/// Computes ceiling according to XPath 2.0 §6.4.2 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Preserves NaN, ±INF, and sign of zero (-0.0 for -0.5 < x <= -0.0).
/// - Handles all values |x| >= 2^52 without i64 saturation.
pub(crate) fn xpath_ceil_f64(x: f64) -> f64 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    if x >= 4503599627370496.0 || x <= -4503599627370496.0 {
        return x;
    }
    let trunc = x as i64;
    if x > 0.0 {
        if (trunc as f64) == x {
            x
        } else {
            (trunc.saturating_add(1)) as f64
        }
    } else if trunc == 0 {
        // XPath 2.0 §6.4.2: fn:ceiling(-0.5) returns -0.0
        -0.0
    } else {
        trunc as f64
    }
}

/// Computes ceiling for f32 according to XPath 2.0 §6.4.2 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Preserves NaN, ±INF, and sign of zero (-0.0 for -0.5 < x <= -0.0).
/// - Handles all values |x| >= 2^23 without i64 saturation.
pub(crate) fn xpath_ceil_f32(x: f32) -> f32 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    if x >= 8388608.0 || x <= -8388608.0 {
        return x;
    }
    let trunc = x as i64;
    if x > 0.0 {
        if (trunc as f32) == x {
            x
        } else {
            (trunc.saturating_add(1)) as f32
        }
    } else if trunc == 0 {
        -0.0
    } else {
        trunc as f32
    }
}

/// Computes round according to XPath 2.0 §6.4.4 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Ties are resolved towards positive infinity (e.g. -1.5 -> -1.0, 1.5 -> 2.0).
/// - For -0.5 <= x < 0.0, fn:round returns -0.0.
/// - Handles all values |x| >= 2^52 without i64 saturation.
pub(crate) fn xpath_round_f64(x: f64) -> f64 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    if x >= 4503599627370496.0 || x <= -4503599627370496.0 {
        return x;
    }
    // XPath 2.0 §6.4.4: fn:round(-0.5) returns -0.0
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    xpath_floor_f64(x + 0.5)
}

/// Computes round for f32 according to XPath 2.0 §6.4.4 (no_std compatible).
///
/// Compliant with W3C XPath and XQuery Functions and Operators 1.0/2.0:
/// - Ties are resolved towards positive infinity (e.g. -1.5 -> -1.0, 1.5 -> 2.0).
/// - For -0.5 <= x < 0.0, fn:round returns -0.0.
/// - Handles all values |x| >= 2^23 without i64 saturation.
pub(crate) fn xpath_round_f32(x: f32) -> f32 {
    if x.is_infinite() || x == 0.0 {
        return x;
    }
    if x >= 8388608.0 || x <= -8388608.0 {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    xpath_floor_f32(x + 0.5)
}

/// Computes round-half-to-even according to XPath 2.0 §6.4.5 (no_std compatible).
///
/// Implements Banker's rounding: if equidistant between two integers, round to the even one.
pub(crate) fn xpath_round_half_to_even_f64(num: f64, precision: i32) -> f64 {
    if num.is_infinite() || num == 0.0 {
        return num;
    }
    let mut factor = 1.0f64;
    if precision > 0 {
        for _ in 0..precision.min(15) {
            factor *= 10.0;
        }
    } else if precision < 0 {
        for _ in 0..precision.saturating_neg().min(15) {
            factor /= 10.0;
        }
    }
    let scaled = num * factor;
    if scaled >= 4503599627370496.0 || scaled <= -4503599627370496.0 {
        return num;
    }
    let floor_val = xpath_floor_f64(scaled);
    let diff = scaled - floor_val;
    let rounded = if (diff - 0.5).abs() < 1e-9 {
        let fl_int = floor_val as i64;
        if fl_int & 1 == 0 {
            floor_val
        } else {
            floor_val + 1.0
        }
    } else if diff > 0.5 {
        floor_val + 1.0
    } else {
        floor_val
    };
    rounded / factor
}

pub(crate) fn parse_date_components(s: &str) -> Option<(i64, i64, i64)> {
    let s = s.trim();
    let (is_neg, rest) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s)
    };
    let mut parts = rest.split('-');
    let year_str = parts.next()?;
    let month_str = parts.next()?;
    let day_rem = parts.next()?;
    let day_str = day_rem.split(['+', '-', 'Z', 'T']).next()?;
    let mut year: i64 = year_str.parse().ok()?;
    if is_neg {
        year = year.saturating_neg();
    }
    let month: i64 = month_str.parse().ok()?;
    let day: i64 = day_str.parse().ok()?;
    Some((year, month, day))
}

pub(crate) fn parse_time_components(s: &str) -> Option<(i64, i64, DfdlValue)> {
    let s = s.trim();
    let mut parts = s.split(':');
    let hours_str = parts.next()?;
    let minutes_str = parts.next()?;
    let sec_rem = parts.next()?;
    let sec_str = sec_rem.split(['+', '-', 'Z']).next()?;
    let hours: i64 = hours_str.parse().ok()?;
    let minutes: i64 = minutes_str.parse().ok()?;
    let sec_val = if let Some((whole, frac)) = sec_str.split_once('.') {
        let whole_num: u64 = whole.parse().ok()?;
        let dec_str = alloc::format!("{}.{}", whole_num, frac);
        DfdlValue::Decimal(dec_str)
    } else {
        let sec_num: u64 = sec_str.parse().ok()?;
        DfdlValue::Decimal(alloc::format!("{}", sec_num))
    };
    Some((hours, minutes, sec_val))
}

pub(crate) fn parse_datetime_components(s: &str) -> Option<(i64, i64, i64, i64, i64, DfdlValue)> {
    let s = s.trim();
    let (d_str, t_str) = s.split_once('T')?;
    let (y, m, d) = parse_date_components(d_str)?;
    let (hh, mm, ss) = parse_time_components(t_str)?;
    Some((y, m, d, hh, mm, ss))
}

pub(crate) fn extract_date_str<'a>(arg: &'a DfdlValue, target_kind: &str) -> DFDLResult<&'a str> {
    match arg {
        DfdlValue::Date(s) => {
            if target_kind != "Date" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Date cannot be converted to {}",
                        target_kind
                    ),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::DateTime(s) => {
            if target_kind != "DateTime" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: DateTime cannot be converted to {}",
                        target_kind
                    ),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::Time(s) => {
            if target_kind != "Time" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Time cannot be converted to {}",
                        target_kind
                    ),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::String(s) => {
            let s_trim = s.trim();
            if target_kind == "Date" && s_trim.contains('T') {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: DateTime cannot be converted to Date",
                ));
            }
            if target_kind == "Date" && !s_trim.contains('-') && s_trim.contains(':') {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Time cannot be converted to Date",
                ));
            }
            if target_kind == "Time"
                && s_trim.contains('-')
                && !s_trim.starts_with('-')
                && !s_trim.contains(':')
            {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Date cannot be converted to Time",
                ));
            }
            if target_kind == "DateTime" && !s_trim.contains('T') {
                if s_trim.contains('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Date cannot be converted to DateTime",
                    ));
                } else if s_trim.contains(':') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Time cannot be converted to DateTime",
                    ));
                }
            }
            Ok(s_trim)
        }
        other => Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            &alloc::format!(
                "Schema Definition Error: cannot convert {} to {}",
                other.type_name(),
                target_kind
            ),
        )),
    }
}
