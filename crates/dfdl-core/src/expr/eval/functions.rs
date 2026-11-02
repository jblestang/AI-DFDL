//! Standard XPath and DFDL function evaluation.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::state::ElementState;
use crate::infoset::value::DfdlValue;
use crate::types::{InfosetPath, QName};
use crate::util::get_checked;
use super::context::ExprContext;
use super::helpers::*;
use super::navigation::path_node_exists;

pub(crate) fn eval_fn_call(name: &QName, args: &[DfdlValue], ctx: &ExprContext) -> DFDLResult<DfdlValue> {
    macro_rules! cast_to_int {
        ($arg:expr, $type_name:expr, $target_type:ty, $variant:ident) => {{
            let err = || {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("cannot convert value to xs:{}", $type_name),
                )
            };
            let err_str = |s: &str| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Cannot convert '{}' from String to Long on element (cannot convert to xs:{})", s, $type_name),
                )
            };
            let res: $target_type = match $arg {
                DfdlValue::Int(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Long(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Short(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Byte(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedLong(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedInt(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedShort(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedByte(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Float(f) => {
                    if !f.is_finite() {
                        return Err(err());
                    }
                    let f64_val = *f as f64;
                    if f64_val < (<$target_type>::MIN as f64)
                        || f64_val > (<$target_type>::MAX as f64)
                    {
                        return Err(err());
                    }
                    f64_val as $target_type
                }
                DfdlValue::Double(f) => {
                    if !f.is_finite() {
                        return Err(err());
                    }
                    if *f < (<$target_type>::MIN as f64) || *f > (<$target_type>::MAX as f64) {
                        return Err(err());
                    }
                    *f as $target_type
                }
                DfdlValue::Boolean(b) => {
                    let v: i32 = if *b { 1 } else { 0 };
                    v.try_into().map_err(|_| err())?
                }
                DfdlValue::String(s) => s.trim().parse::<$target_type>().map_err(|_| err_str(s))?,
                DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if let Ok(v) = trimmed.parse::<$target_type>() {
                        v
                    } else if let Ok(f) = trimmed.parse::<f64>() {
                        if f.is_finite()
                            && f >= (<$target_type>::MIN as f64)
                            && f <= (<$target_type>::MAX as f64)
                        {
                            f as $target_type
                        } else {
                            return Err(err_str(s));
                        }
                    } else {
                        return Err(err_str(s));
                    }
                }
                _ => return Err(err()),
            };
            Ok(DfdlValue::$variant(res))
        }};
    }

    match name.local_name.as_str() {
        "error" => {
            let msg = if let Some(first) = args.first() {
                alloc::format!("fn:error(): {}", first)
            } else {
                alloc::string::String::from("fn:error() raised by expression")
            };
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
        "valueLength" | "contentLength" => {
            let val = get_checked(args, 0)?;
            let units = args
                .get(1)
                .map(|u| format!("{}", u))
                .unwrap_or_else(|| String::from("bytes"));
            let byte_len = match val {
                DfdlValue::String(s) => s.len(),
                DfdlValue::HexBinary(b) => b.len(),
                DfdlValue::Int(_) | DfdlValue::Short(_) => 4,
                DfdlValue::Long(_) | DfdlValue::Double(_) => 8,
                DfdlValue::Byte(_) => 1,
                _ => format!("{}", val).len(),
            };
            match units.to_lowercase().as_str() {
                "bits" => Ok(DfdlValue::Long((byte_len.saturating_mul(8)) as i64)),
                "characters" | "bytes" => Ok(DfdlValue::Long(byte_len as i64)),
                _ => Ok(DfdlValue::Long(byte_len as i64)),
            }
        }
        "concat" => {
            let mut res = String::new();
            for arg in args {
                let arg_str = alloc::format!("{}", arg);
                res.push_str(&arg_str);
            }
            Ok(DfdlValue::String(res))
        }
        "string-length" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::Long(s.chars().count() as i64))
        }
        "decodeDFDLEntities" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::String(
                crate::expr::properties::decode_dfdl_character_entities(&s),
            ))
        }
        "encodeDFDLEntities" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::String(
                crate::expr::properties::encode_dfdl_character_entities(&s),
            ))
        }
        "substring" => {
            let str_arg = get_checked(args, 0)?;
            let start_arg = get_checked(args, 1)?;
            let len_arg = args.get(2);

            let s = format!("{}", str_arg);
            let chars: Vec<char> = s.chars().collect();

            let raw_start = match start_arg {
                DfdlValue::Double(d) => {
                    if d.is_nan() || *d == f64::INFINITY {
                        return Ok(DfdlValue::String(String::new()));
                    }
                    if *d == f64::NEG_INFINITY {
                        i64::MIN
                    } else {
                        *d as i64
                    }
                }
                DfdlValue::Float(f) => {
                    if f.is_nan() || *f == f32::INFINITY {
                        return Ok(DfdlValue::String(String::new()));
                    }
                    if *f == f32::NEG_INFINITY {
                        i64::MIN
                    } else {
                        *f as i64
                    }
                }
                DfdlValue::Long(n) => *n,
                DfdlValue::Int(n) => *n as i64,
                _ => 1,
            };

            let start_idx = (raw_start.max(1).saturating_sub(1)) as usize;
            if start_idx >= chars.len() {
                return Ok(DfdlValue::String(String::new()));
            }

            let avail = chars.len().saturating_sub(start_idx);
            let len = if let Some(la) = len_arg {
                match la {
                    DfdlValue::Double(d) => {
                        if d.is_nan() || *d <= 0.0 {
                            0
                        } else {
                            (*d as usize).min(avail)
                        }
                    }
                    DfdlValue::Float(f) => {
                        if f.is_nan() || *f <= 0.0 {
                            0
                        } else {
                            (*f as usize).min(avail)
                        }
                    }
                    DfdlValue::Long(n) => ((*n).max(0) as usize).min(avail),
                    DfdlValue::Int(n) => ((*n).max(0) as usize).min(avail),
                    _ => avail,
                }
            } else {
                avail
            };

            let sub: String = chars.iter().skip(start_idx).take(len).collect();
            Ok(DfdlValue::String(sub))
        }
        "substring-before" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str_arg);
            let s2 = alloc::format!("{}", sub_arg);
            if s2.is_empty() {
                return Ok(DfdlValue::String(String::new()));
            }
            if let Some(pos) = s1.find(&s2) {
                let res = s1.get(..pos).unwrap_or("").to_string();
                Ok(DfdlValue::String(res))
            } else {
                Ok(DfdlValue::String(String::new()))
            }
        }
        "substring-after" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str_arg);
            let s2 = alloc::format!("{}", sub_arg);
            if s2.is_empty() {
                return Ok(DfdlValue::String(s1));
            }
            if let Some(pos) = s1.find(&s2) {
                let after_pos = pos.saturating_add(s2.len());
                let res = s1.get(after_pos..).unwrap_or("").to_string();
                Ok(DfdlValue::String(res))
            } else {
                Ok(DfdlValue::String(String::new()))
            }
        }
        "contains" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.contains(&s2)))
        }
        "starts-with" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.starts_with(&s2)))
        }
        "ends-with" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.ends_with(&s2)))
        }
        "upper-case" => {
            let str_arg = get_checked(args, 0)?;
            let s = format!("{}", str_arg);
            Ok(DfdlValue::String(s.to_uppercase()))
        }
        "lower-case" => {
            let str_arg = get_checked(args, 0)?;
            let s = format!("{}", str_arg);
            Ok(DfdlValue::String(s.to_lowercase()))
        }
        "abs" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Int(n) => Ok(DfdlValue::Int(n.abs())),
                DfdlValue::Long(n) => Ok(DfdlValue::Long(n.abs())),
                DfdlValue::Float(f) => Ok(DfdlValue::Float(if *f < 0.0 { -*f } else { *f })),
                DfdlValue::Double(f) => Ok(DfdlValue::Double(if *f < 0.0 { -*f } else { *f })),
                _ => Ok(num_arg.clone()),
            }
        }
        "ceiling" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot ceiling NaN",
                        ));
                    }
                    Ok(DfdlValue::Float(xpath_ceil_f32(*f)))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot ceiling NaN",
                        ));
                    }
                    Ok(DfdlValue::Double(xpath_ceil_f64(*f)))
                }
                DfdlValue::Decimal(s) => {
                    let f = s.trim().parse::<f64>().unwrap_or(0.0);
                    let c = xpath_ceil_f64(f);
                    let s_out = if (c as i64 as f64) == c {
                        alloc::format!("{}", c as i64)
                    } else {
                        alloc::format!("{}", c)
                    };
                    Ok(DfdlValue::Decimal(s_out))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "floor" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot floor NaN",
                        ));
                    }
                    Ok(DfdlValue::Float(xpath_floor_f32(*f)))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot floor NaN",
                        ));
                    }
                    Ok(DfdlValue::Double(xpath_floor_f64(*f)))
                }
                DfdlValue::Decimal(s) => {
                    let f = s.trim().parse::<f64>().unwrap_or(0.0);
                    let fl = xpath_floor_f64(f);
                    let s_out = if (fl as i64 as f64) == fl {
                        alloc::format!("{}", fl as i64)
                    } else {
                        alloc::format!("{}", fl)
                    };
                    Ok(DfdlValue::Decimal(s_out))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "round" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot round NaN",
                        ));
                    }
                    Ok(DfdlValue::Float(xpath_round_f32(*f)))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot round NaN",
                        ));
                    }
                    Ok(DfdlValue::Double(xpath_round_f64(*f)))
                }
                DfdlValue::Decimal(s) => {
                    let f = s.trim().parse::<f64>().unwrap_or(0.0);
                    let r = xpath_round_f64(f);
                    let s_out = if (r as i64 as f64) == r {
                        alloc::format!("{}", r as i64)
                    } else {
                        alloc::format!("{}", r)
                    };
                    Ok(DfdlValue::Decimal(s_out))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "matches" => {
            let str_arg = get_checked(args, 0)?;
            let pat_arg = get_checked(args, 1)?;
            let s = format!("{}", str_arg);
            let p = format!("{}", pat_arg);
            Ok(DfdlValue::Boolean(s.contains(&p)))
        }
        "exists" => {
            if let Some(arg) = args.first() {
                let path_str = alloc::format!("{}", arg);
                let path = InfosetPath::parse(&path_str);
                let exists = path_node_exists(&path, ctx).unwrap_or(false);
                Ok(DfdlValue::Boolean(exists))
            } else {
                Ok(DfdlValue::Boolean(false))
            }
        }
        "checkConstraints" => {
            let target_val = if let Some(arg) = args.first() {
                arg.clone()
            } else if let Some(val) = ctx
                .doc
                .and_then(|doc| doc.find_element_with_context(ctx.current_path, ctx.occurs_index))
                .and_then(|elem| match &elem.state {
                    ElementState::Value(val) => Some(val.clone()),
                    _ => None,
                })
            {
                val
            } else {
                let elem_name = ctx
                    .current_path
                    .last_step()
                    .map(|s| s.raw_target())
                    .unwrap_or("");
                let msg = alloc::format!(
                    "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            };
            let target_path = ctx.current_path;
            if let Some(sch) = ctx.schema {
                let term_opt = sch.find_term_by_path(target_path).or_else(|| {
                    target_path
                        .last_step()
                        .map(|s| s.local_name())
                        .and_then(|last_name| {
                            sch.terms
                                .iter()
                                .find(|t| t.name.local_name.as_str() == last_name)
                        })
                });
                if let Some(term) = term_opt {
                    let mut is_valid = term.properties.facets.validate_value(&target_val);
                    if is_valid {
                        if let crate::schema::ir::TermKind::Element(ref elem) = term.kind {
                            if let crate::schema::ir::CompiledType::Simple(ref st) = elem.type_ir {
                                if let Some(v) = target_val.as_i128() {
                                    let is_neg = target_val.is_negative();
                                    match st {
                                        crate::infoset::DfdlSimpleType::Int => {
                                            is_valid = i32::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Long => {
                                            is_valid = i64::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Short => {
                                            is_valid = i16::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Byte => {
                                            is_valid = i8::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedInt => {
                                            is_valid = !is_neg && u32::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedShort => {
                                            is_valid = !is_neg && u16::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedByte => {
                                            is_valid = !is_neg && u8::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedLong => {
                                            is_valid = !is_neg && u64::try_from(v).is_ok();
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    return Ok(DfdlValue::Boolean(is_valid));
                }
            }
            Ok(DfdlValue::Boolean(true))
        }
        "double" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Double(d) => Ok(DfdlValue::Double(*d)),
                DfdlValue::Float(f) => Ok(DfdlValue::Double(*f as f64)),
                DfdlValue::Int(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Long(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Short(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Boolean(b) => Ok(DfdlValue::Double(if *b { 1.0 } else { 0.0 })),
                DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if trimmed.eq_ignore_ascii_case("nan") && trimmed != "NaN" {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:double",
                                s
                            ),
                        ));
                    }
                    if (trimmed.eq_ignore_ascii_case("inf") && trimmed != "INF")
                        || (trimmed.eq_ignore_ascii_case("-inf") && trimmed != "-INF")
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:double",
                                s
                            ),
                        ));
                    }
                    let val = trimmed.parse::<f64>().map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:double",
                                s
                            ),
                        )
                    })?;
                    Ok(DfdlValue::Double(val))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "cannot convert value to xs:double",
                )),
            }
        }
        "int" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "int", i32, Int)
        }
        "long" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "long", i64, Long)
        }
        "decimal" | "integer" | "nonNegativeInteger" => {
            let arg = get_checked(args, 0)?;
            let s = alloc::format!("{}", arg);
            Ok(DfdlValue::Decimal(s))
        }
        "short" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "short", i16, Short)
        }
        "byte" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "byte", i8, Byte)
        }
        "unsignedByte" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedByte", u8, UnsignedByte)
        }
        "unsignedShort" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedShort", u16, UnsignedShort)
        }
        "unsignedInt" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedInt", u32, UnsignedInt)
        }
        "unsignedLong" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedLong", u64, UnsignedLong)
        }
        "float" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Float(f) => Ok(DfdlValue::Float(*f)),
                DfdlValue::Double(f) => Ok(DfdlValue::Float(*f as f32)),
                DfdlValue::Int(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Long(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Short(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Boolean(b) => Ok(DfdlValue::Float(if *b { 1.0 } else { 0.0 })),
                DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if trimmed.eq_ignore_ascii_case("nan") && trimmed != "NaN" {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:float",
                                s
                            ),
                        ));
                    }
                    if (trimmed.eq_ignore_ascii_case("inf") && trimmed != "INF")
                        || (trimmed.eq_ignore_ascii_case("-inf") && trimmed != "-INF")
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:float",
                                s
                            ),
                        ));
                    }
                    let val = trimmed.parse::<f32>().map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: cannot convert '{}' to xs:float",
                                s
                            ),
                        )
                    })?;
                    Ok(DfdlValue::Float(val))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "cannot convert value to xs:float",
                )),
            }
        }
        "string" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::String(alloc::format!("{}", arg)))
        }
        "compare" => {
            let str1 = get_checked(args, 0)?;
            let str2 = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str1);
            let s2 = alloc::format!("{}", str2);
            let cmp = match s1.cmp(&s2) {
                core::cmp::Ordering::Less => -1,
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 1,
            };
            Ok(DfdlValue::Int(cmp))
        }
        "true" => Ok(DfdlValue::Boolean(true)),
        "false" => Ok(DfdlValue::Boolean(false)),
        "date" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Date(alloc::format!("{}", arg)))
        }
        "dateTime" => {
            let arg = get_checked(args, 0)?;
            let s = alloc::format!("{}", arg);
            if let Some((_y, m, d, hh, mm, ss)) = parse_datetime_components(&s) {
                let sec_valid = match ss {
                    DfdlValue::Decimal(ref dec_s) => {
                        let whole_sec: u64 =
                            dec_s.split('.').next().unwrap_or("0").parse().unwrap_or(99);
                        whole_sec <= 60
                    }
                    _ => true,
                };
                if !(1..=12).contains(&m)
                    || !(1..=31).contains(&d)
                    || !(0..=24).contains(&hh)
                    || !(0..=59).contains(&mm)
                    || !sec_valid
                {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: xs:dateTime from string '{}'", s),
                    ));
                }
                Ok(DfdlValue::DateTime(s))
            } else {
                Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: xs:dateTime from string '{}'", s),
                ))
            }
        }
        "time" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Time(alloc::format!("{}", arg)))
        }
        "year-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (y, _, _) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(y))
        }
        "month-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (_, m, _) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(m))
        }
        "day-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (_, _, d) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(d))
        }
        "year-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (y, _, _, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(y))
        }
        "month-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, m, _, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(m))
        }
        "day-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, d, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(d))
        }
        "hours-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, hh, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(hh))
        }
        "minutes-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, _, mm, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(mm))
        }
        "seconds-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, _, _, ss) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(ss)
        }
        "hours-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (hh, _, _) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(DfdlValue::Long(hh))
        }
        "minutes-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (_, mm, _) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(DfdlValue::Long(mm))
        }
        "seconds-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (_, _, ss) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(ss)
        }
        "lookAhead" => {
            let offset = if let Ok(offset_val) = get_checked(args, 0) {
                if let Some(n) = offset_val.as_i128() {
                    if n < 0 {
                        let msg = alloc::format!(
                            "Schema Definition Error: out of range for xs:unsignedInt: {}",
                            n
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    n as usize
                } else {
                    0
                }
            } else {
                0
            };
            let bitsize = if let Ok(bitsize_val) = get_checked(args, 1) {
                if let Some(n) = bitsize_val.as_i128() {
                    if n < 0 {
                        let msg = alloc::format!(
                            "Schema Definition Error: out of range for xs:unsignedInt: {}",
                            n
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    n as usize
                } else {
                    0
                }
            } else {
                0
            };
            let total_dist = offset.saturating_add(bitsize);
            if total_dist > 512 {
                let msg = alloc::format!(
                    "Schema Definition Error: Look-ahead distance of {} bits exceeds implementation defined limit of 512 bits",
                    total_dist
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if let Some(la_fn) = ctx.lookahead_fn {
                let val = la_fn(offset, bitsize)?;
                if val <= (u64::MAX as u128) {
                    Ok(DfdlValue::UnsignedLong(val as u64))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("{}", val)))
                }
            } else {
                Ok(DfdlValue::Long(0))
            }
        }
        "currentPosition" => Ok(DfdlValue::Long(0)),
        "occursIndex" => Ok(DfdlValue::Long(ctx.occurs_index as i64)),
        "count" => {
            if let Some(arg) = args.first() {
                match arg {
                    DfdlValue::String(s) if s.is_empty() => Ok(DfdlValue::Long(0)),
                    _ => Ok(DfdlValue::Long(1)),
                }
            } else {
                Ok(DfdlValue::Long(0))
            }
        }
        "boolean" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(*b)),
                DfdlValue::Int(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Long(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Short(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Float(f) => Ok(DfdlValue::Boolean(*f != 0.0 && !f.is_nan())),
                DfdlValue::Double(f) => Ok(DfdlValue::Boolean(*f != 0.0 && !f.is_nan())),
                DfdlValue::String(s) => match s.as_str() {
                    "true" | "1" => Ok(DfdlValue::Boolean(true)),
                    "false" | "0" => Ok(DfdlValue::Boolean(false)),
                    _ => {
                        let msg = format!(
                            "Cannot convert '{}' to xs:boolean: Must be one of 0, 1, true, or false",
                            s
                        );
                        Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
                    }
                },
                _ => {
                    let msg =
                        "Cannot convert value to xs:boolean: Must be one of 0, 1, true, or false";
                    Err(DFDLError::new(DFDLErrorKind::ExpressionError, msg))
                }
            }
        }
        "not" => {
            let arg = get_checked(args, 0)?;
            let b = match arg {
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
            };
            Ok(DfdlValue::Boolean(!b))
        }
        "replace" => {
            if name.prefix.as_deref().is_some()
                && name.prefix.as_deref() != Some("fn")
                && name.prefix.as_deref() != Some("dfdl")
                && name.prefix.as_deref() != Some("jgsu")
            {
                let pfx = name.prefix.as_deref().unwrap_or("");
                let msg = alloc::format!(
                    "Schema Definition Error: Unsupported function: {}:replace",
                    pfx
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if args.len() < 3 {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: The {} function requires 3 argument(s).",
                        name
                    ),
                ));
            }
            let target = format!("{}", get_checked(args, 0)?);
            let pattern = format!("{}", get_checked(args, 1)?);
            let replacement = format!("{}", get_checked(args, 2)?);
            Ok(DfdlValue::String(target.replace(&pattern, &replacement)))
        }
        "sayHello" => Ok(DfdlValue::String(String::from("Hello"))),
        "addBoxed" | "addPrimitive" => {
            let arg1 = get_checked(args, 0)?;
            let arg2 = get_checked(args, 1)?;
            let parse_num = |val: &DfdlValue| -> DFDLResult<i64> {
                match val {
                    DfdlValue::Int(n) => Ok(*n as i64),
                    DfdlValue::Long(n) => Ok(*n),
                    DfdlValue::Short(n) => Ok(*n as i64),
                    DfdlValue::Byte(n) => Ok(*n as i64),
                    DfdlValue::UnsignedLong(n) => Ok(*n as i64),
                    DfdlValue::UnsignedInt(n) => Ok(*n as i64),
                    DfdlValue::UnsignedShort(n) => Ok(*n as i64),
                    DfdlValue::UnsignedByte(n) => Ok(*n as i64),
                    DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                        s.trim().parse::<i64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::ExpressionError,
                                &format!("Cannot convert '{}' from String type to numeric", s),
                            )
                        })
                    }
                    _ => Err(DFDLError::new_static(
                        DFDLErrorKind::ExpressionError,
                        "Type mismatch in add function",
                    )),
                }
            };
            let n1 = parse_num(arg1)?;
            let n2 = parse_num(arg2)?;
            Ok(DfdlValue::Int(n1.saturating_add(n2) as i32))
        }
        "bitAnd" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitAnd")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitAnd")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitAnd must match in signedness",
                ));
            }
            let res = v1 & v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitOr" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitOr")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitOr")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitOr must match in signedness",
                ));
            }
            let res = v1 | v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitXor" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitXor")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitXor")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitXor must match in signedness",
                ));
            }
            let res = v1 ^ v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitNot" => {
            // The complement keeps the operand's type: for unsigned subtypes the result must
            // stay within the type's width (e.g. ~1 as xs:unsignedByte is 254, not 2^64-2).
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "bitNot")?;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(!*n),
                DfdlValue::Short(n) => DfdlValue::Short(!*n),
                DfdlValue::Int(n) => DfdlValue::Int(!*n),
                DfdlValue::Long(n) => DfdlValue::Long(!*n),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(!*n),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(!*n),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(!*n),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(!*n),
                other => other.clone(),
            })
        }
        "leftShift" => {
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "leftShift")?;
            let shift_arg = get_checked(args, 1)?;
            let count = value_to_i64(shift_arg);
            let bit_width: i64 = match arg {
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => 8,
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => 16,
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => 32,
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => 64,
                _ => 64,
            };
            if count < 0 || count >= bit_width {
                let msg = alloc::format!(
                    "Schema Definition Error: Shift count must be between 0 and {} (less than {}), but was {}",
                    bit_width.saturating_sub(1), bit_width, count
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let c = count as u32;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(((*n as u8).wrapping_shl(c)) as i8),
                DfdlValue::Short(n) => DfdlValue::Short(((*n as u16).wrapping_shl(c)) as i16),
                DfdlValue::Int(n) => DfdlValue::Int(((*n as u32).wrapping_shl(c)) as i32),
                DfdlValue::Long(n) => DfdlValue::Long(((*n as u64).wrapping_shl(c)) as i64),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(n.wrapping_shl(c)),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(n.wrapping_shl(c)),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(n.wrapping_shl(c)),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(n.wrapping_shl(c)),
                other => other.clone(),
            })
        }
        "rightShift" => {
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "rightShift")?;
            let shift_arg = get_checked(args, 1)?;
            let count = value_to_i64(shift_arg);
            let bit_width: i64 = match arg {
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => 8,
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => 16,
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => 32,
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => 64,
                _ => 64,
            };
            if count < 0 || count >= bit_width {
                let msg = alloc::format!(
                    "Schema Definition Error: Shift count must be between 0 and {} (less than {}), but was {}",
                    bit_width.saturating_sub(1), bit_width, count
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let c = count as u32;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(n.wrapping_shr(c)),
                DfdlValue::Short(n) => DfdlValue::Short(n.wrapping_shr(c)),
                DfdlValue::Int(n) => DfdlValue::Int(n.wrapping_shr(c)),
                DfdlValue::Long(n) => DfdlValue::Long(n.wrapping_shr(c)),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(n.wrapping_shr(c)),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(n.wrapping_shr(c)),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(n.wrapping_shr(c)),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(n.wrapping_shr(c)),
                other => other.clone(),
            })
        }
        "primByteFunc" | "boxedByteFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Byte(n as i8))
        }
        "primByteArrayFunc" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::String(format!("{}", arg)))
        }
        "primShortFunc" | "boxedShortFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Short(n as i16))
        }
        "primLongFunc" | "boxedLongFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Long(n))
        }
        "primDoubleFunc" | "boxedDoubleFunc" => {
            let f = value_to_f64(get_checked(args, 0)?);
            Ok(DfdlValue::Double(f))
        }
        "primFloatFunc" | "boxedFloatFunc" => {
            let f = value_to_f64(get_checked(args, 0)?);
            Ok(DfdlValue::Float(f as f32))
        }
        "primBooleanFunc" | "boxedBooleanFunc" => {
            let b = value_to_bool(get_checked(args, 0)?);
            Ok(DfdlValue::Boolean(b))
        }
        "javaBigDecimalFunc" | "javaBigIntegerFunc" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Decimal(format!("{}", arg)))
        }
        "checkRangeInclusive" | "checkRangeExclusive" => {
            let a0 = get_checked(args, 0)?;
            let a1 = get_checked(args, 1)?;
            let a2 = get_checked(args, 2)?;
            if !a0.simple_type().is_numeric()
                || !a1.simple_type().is_numeric()
                || !a2.simple_type().is_numeric()
            {
                let msg = alloc::format!(
                    "Schema Definition Error: Cannot call dfdl:{} with non-numeric types: {}, {}, {}",
                    name.local_name,
                    a0.type_name(),
                    a1.type_name(),
                    a2.type_name()
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let val = value_to_f64(a0);
            let min = value_to_f64(a1);
            let max = value_to_f64(a2);
            if name.local_name == "checkRangeInclusive" {
                Ok(DfdlValue::Boolean(val >= min && val <= max))
            } else {
                Ok(DfdlValue::Boolean(val > min && val < max))
            }
        }
        "hexBinary" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::HexBinary(b) => Ok(DfdlValue::HexBinary(b.clone())),
                DfdlValue::Byte(b) => Ok(DfdlValue::HexBinary(alloc::vec![*b as u8])),
                DfdlValue::UnsignedByte(b) => Ok(DfdlValue::HexBinary(alloc::vec![*b])),
                DfdlValue::Short(s) => Ok(DfdlValue::HexBinary(s.to_be_bytes().to_vec())),
                DfdlValue::UnsignedShort(s) => Ok(DfdlValue::HexBinary(s.to_be_bytes().to_vec())),
                _ if arg.as_i128().is_some() => {
                    let n = arg.as_i128().unwrap_or(0);
                    let bytes = if (-128..=127).contains(&n) {
                        alloc::vec![n as i8 as u8]
                    } else if (-32768..=32767).contains(&n) {
                        (n as i16).to_be_bytes().to_vec()
                    } else if (-2147483648..=2147483647).contains(&n) {
                        (n as i32).to_be_bytes().to_vec()
                    } else if (i64::MIN as i128..=i64::MAX as i128).contains(&n) {
                        (n as i64).to_be_bytes().to_vec()
                    } else {
                        n.to_be_bytes().to_vec()
                    };
                    Ok(DfdlValue::HexBinary(bytes))
                }
                _ => {
                    let s = format!("{}", arg);
                    let clean = s.trim();
                    if clean.len() % 2 != 0 {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parse Error: xs:hexBinary string must have an even number of characters: '{}'",
                                clean
                            ),
                        ));
                    }
                    for ch in clean.chars() {
                        if !ch.is_ascii_hexdigit() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!(
                                    "Parse Error: Hex character must be [0-9a-fA-F], found '{}'",
                                    ch
                                ),
                            ));
                        }
                    }
                    let mut bytes = Vec::new();
                    for i in (0..clean.len()).step_by(2) {
                        let end = i.saturating_add(2).min(clean.len());
                        if let Some(sub) = clean.get(i..end) {
                            let b = u8::from_str_radix(sub, 16).map_err(|_| {
                                DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!("Parse Error: Invalid hex byte '{}'", sub),
                                )
                            })?;
                            bytes.push(b);
                        }
                    }
                    Ok(DfdlValue::HexBinary(bytes))
                }
            }
        }
        "round-half-to-even" | "roundHalfToEven" => {
            let num = value_to_f64(get_checked(args, 0)?);
            if num.is_nan() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Parse Error: Cannot round NaN",
                ));
            }
            let precision = args.get(1).map(value_to_f64).unwrap_or(0.0) as i32;
            let result = xpath_round_half_to_even_f64(num, precision);
            Ok(DfdlValue::Double(result))
        }
        "trace" => {
            if name.prefix.as_deref() == Some("fn") {
                let msg = "Schema Definition Error: Unsupported function: fn:trace";
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, msg));
            }
            let val = get_checked(args, 0)?;
            Ok(val.clone())
        }
        _ => {
            let func_name = if let Some(ref pfx) = name.prefix {
                alloc::format!("{}:{}", pfx, name.local_name)
            } else {
                name.local_name.clone()
            };
            let msg = alloc::format!(
                "Schema Definition Error: Unsupported function: {}",
                func_name
            );
            Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
        }
    }
}
