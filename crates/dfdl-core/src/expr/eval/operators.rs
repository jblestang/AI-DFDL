//! Evaluation of unary, binary, and relational operators.

extern crate alloc;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::expr::ast::{BinaryOp, UnaryOp};
use crate::infoset::value::DfdlValue;
use super::helpers::{value_to_f64, value_to_i64};

pub(crate) fn eval_unary(op: UnaryOp, val: DfdlValue) -> DFDLResult<DfdlValue> {
    match op {
        UnaryOp::Plus => match val {
            DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_)
            | DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_) => Ok(val),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Invalid numeric type for unary plus",
            )),
        },
        UnaryOp::Negate => match val {
            DfdlValue::Int(n) => n
                .checked_neg()
                .map(DfdlValue::Int)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Int negation overflow")),
            DfdlValue::Long(n) => n
                .checked_neg()
                .map(DfdlValue::Long)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Long negation overflow")),
            DfdlValue::Short(n) => n
                .checked_neg()
                .map(DfdlValue::Short)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Short negation overflow")),
            DfdlValue::Byte(n) => n
                .checked_neg()
                .map(DfdlValue::Byte)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Byte negation overflow")),
            DfdlValue::UnsignedLong(n) => {
                if n == 9_223_372_036_854_775_808 {
                    Ok(DfdlValue::Long(i64::MIN))
                } else if n <= i64::MAX as u64 {
                    Ok(DfdlValue::Long((n as i64).wrapping_neg()))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("-{}", n)))
                }
            }
            DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Long(i64::from(n).wrapping_neg())),
            DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Int(i32::from(n).wrapping_neg())),
            DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Int(i32::from(n).wrapping_neg())),
            DfdlValue::Decimal(s) => {
                if let Some(stripped) = s.strip_prefix('-') {
                    Ok(DfdlValue::Decimal(alloc::string::ToString::to_string(
                        stripped,
                    )))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("-{}", s)))
                }
            }
            DfdlValue::Float(f) => Ok(DfdlValue::Float(-f)),
            DfdlValue::Double(f) => Ok(DfdlValue::Double(-f)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Invalid numeric type for negation",
            )),
        },
        UnaryOp::Not => match val {
            DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(!b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Expected boolean for 'not' operator",
            )),
        },
    }
}

pub(crate) fn coerce_numeric(val: DfdlValue) -> DfdlValue {
    match val {
        DfdlValue::Int(n) => DfdlValue::Long(n as i64),
        DfdlValue::Short(n) => DfdlValue::Long(n as i64),
        DfdlValue::Byte(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedInt(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedShort(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedByte(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedLong(n) => DfdlValue::Long(n as i64),
        DfdlValue::Float(f) => DfdlValue::Double(f as f64),
        DfdlValue::String(ref s) => {
            let clean = s.trim();
            if let Ok(i) = clean.parse::<i64>() {
                DfdlValue::Long(i)
            } else if let Ok(f) = clean.parse::<f64>() {
                DfdlValue::Double(f)
            } else {
                val
            }
        }
        _ => val,
    }
}

pub(crate) fn eval_binary(op: BinaryOp, left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    let is_arith = matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::IDiv
            | BinaryOp::Mod
    );
    if is_arith {
        let op_sym = match op {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "div",
            BinaryOp::IDiv => "idiv",
            BinaryOp::Mod => "mod",
            _ => "",
        };
        if matches!(left, DfdlValue::String(_) | DfdlValue::Boolean(_))
            || matches!(right, DfdlValue::String(_) | DfdlValue::Boolean(_))
        {
            let bad_type =
                if matches!(left, DfdlValue::String(_)) || matches!(right, DfdlValue::String(_)) {
                    "string"
                } else {
                    "boolean"
                };
            let msg = alloc::format!(
                "Schema Definition Error: operator '{}' requires numeric operands, but received {}",
                op_sym,
                bad_type
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
    }
    let left = coerce_numeric(left);
    let right = coerce_numeric(right);
    match op {
        BinaryOp::Add => eval_add(left, right),
        BinaryOp::Sub => eval_sub(left, right),
        BinaryOp::Mul => eval_mul(left, right),
        BinaryOp::Div => eval_div(left, right),
        BinaryOp::IDiv => eval_idiv(left, right),
        BinaryOp::Mod => eval_mod(left, right),
        BinaryOp::Eq => Ok(DfdlValue::Boolean(left == right)),
        BinaryOp::Ne => Ok(DfdlValue::Boolean(left != right)),
        BinaryOp::Lt => eval_cmp("lt", left, right, |ord| ord.is_lt()),
        BinaryOp::Le => eval_cmp("le", left, right, |ord| ord.is_le()),
        BinaryOp::Gt => eval_cmp("gt", left, right, |ord| ord.is_gt()),
        BinaryOp::Ge => eval_cmp("ge", left, right, |ord| ord.is_ge()),
        BinaryOp::And => match (left, right) {
            (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(a && b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Logical AND requires boolean operands",
            )),
        },
        BinaryOp::Or => match (left, right) {
            (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(a || b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Logical OR requires boolean operands",
            )),
        },
    }
}

pub(crate) fn eval_add(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) | (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a + d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a + f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a + b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_add(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long addition overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_add(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int addition overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) | (DfdlValue::Long(b), DfdlValue::Int(a)) => (a
            as i64)
            .checked_add(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Addition overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a + b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 + b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a + b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a + b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 + b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a + b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in addition",
        )),
    }
}

pub(crate) fn eval_sub(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a - d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a - f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a - b)))
            }
        }
        (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let b = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(d - b))
            } else if let DfdlValue::Float(f) = other {
                let b = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(f - b))
            } else {
                let a = value_to_f64(other);
                let b = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a - b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_sub(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long subtraction overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_sub(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int subtraction overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) => (a as i64)
            .checked_sub(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Subtraction overflow")),
        (DfdlValue::Long(a), DfdlValue::Int(b)) => a
            .checked_sub(b as i64)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Subtraction overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a - b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 - b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a - b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a - b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 - b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a - b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in subtraction",
        )),
    }
}

pub(crate) fn eval_mul(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) | (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a * d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a * f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a * b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_mul(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long multiplication overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_mul(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int multiplication overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) | (DfdlValue::Long(b), DfdlValue::Int(a)) => (a
            as i64)
            .checked_mul(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Multiplication overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a * b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 * b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a * b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a * b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 * b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a * b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in multiplication",
        )),
    }
}

pub(crate) fn eval_div(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        // Floating point operations return IEEE 754 results (INF, -INF, NaN) on div by zero
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a / b)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a / b)),
        (DfdlValue::Double(a), DfdlValue::Float(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Float(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Int(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Double(a), DfdlValue::Int(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 / b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a / b as f32)),
        (DfdlValue::Int(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 / b)),
        (DfdlValue::Float(a), DfdlValue::Int(b)) => Ok(DfdlValue::Float(a / b as f32)),

        // Decimal operations
        (DfdlValue::Decimal(ref s), ref other) => {
            let b = value_to_f64(other);
            if let DfdlValue::Double(_) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a / b))
            } else if let DfdlValue::Float(_) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a / b as f32))
            } else if b == 0.0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a / b)))
            }
        }
        (ref other, DfdlValue::Decimal(ref s)) => {
            let b = s.trim().parse::<f64>().unwrap_or(0.0);
            if let DfdlValue::Double(d) = other {
                Ok(DfdlValue::Double(d / b))
            } else if let DfdlValue::Float(f) = other {
                Ok(DfdlValue::Float(f / b as f32))
            } else if b == 0.0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else {
                let a = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a / b)))
            }
        }

        // Integer divisions
        (DfdlValue::Long(a), DfdlValue::Long(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) = (a.checked_rem(b), a.checked_div(b)) {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Int(a), DfdlValue::Int(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) = (a.checked_rem(b), a.checked_div(b)) {
                if rem == 0 {
                    Ok(DfdlValue::Int(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Int(a), DfdlValue::Long(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) =
                ((a as i64).checked_rem(b), (a as i64).checked_div(b))
            {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Int(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) =
                (a.checked_rem(b as i64), a.checked_div(b as i64))
            {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in division",
        )),
    }
}

pub(crate) fn eval_idiv(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    if !left.simple_type().is_numeric() || !right.simple_type().is_numeric() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in idiv",
        ));
    }

    let is_float = matches!(left, DfdlValue::Double(_) | DfdlValue::Float(_))
        || matches!(right, DfdlValue::Double(_) | DfdlValue::Float(_));

    if is_float {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if a.is_nan() || b.is_nan() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division (idiv) operand is NaN",
            ));
        }
        if a.is_infinite() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division (idiv) operand is Infinity",
            ));
        }
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division by zero",
            ));
        }
        if b.is_infinite() {
            return Ok(DfdlValue::Long(0));
        }
        return Ok(DfdlValue::Long((a / b) as i64));
    }

    let is_decimal =
        matches!(left, DfdlValue::Decimal(_)) || matches!(right, DfdlValue::Decimal(_));
    if is_decimal {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division by zero",
            ));
        }
        return Ok(DfdlValue::Long((a / b) as i64));
    }

    let b = value_to_i64(&right);
    if b == 0 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: integer division by zero",
        ));
    }
    let a = value_to_i64(&left);
    let res = a
        .checked_div(b)
        .ok_or_else(|| DFDLError::arithmetic_overflow("Long idiv overflow"))?;
    if matches!(left, DfdlValue::Int(_)) && matches!(right, DfdlValue::Int(_)) {
        if let Ok(res_i32) = i32::try_from(res) {
            return Ok(DfdlValue::Int(res_i32));
        }
    }
    Ok(DfdlValue::Long(res))
}

pub(crate) fn eval_mod(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    if !left.simple_type().is_numeric() || !right.simple_type().is_numeric() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in modulo",
        ));
    }

    if matches!(left, DfdlValue::Double(_)) || matches!(right, DfdlValue::Double(_)) {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        return Ok(DfdlValue::Double(a % b));
    }

    if matches!(left, DfdlValue::Float(_)) || matches!(right, DfdlValue::Float(_)) {
        let a = value_to_f64(&left) as f32;
        let b = value_to_f64(&right) as f32;
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        return Ok(DfdlValue::Float(a % b));
    }

    if matches!(left, DfdlValue::Decimal(_)) || matches!(right, DfdlValue::Decimal(_)) {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        let rem = a % b;
        return Ok(DfdlValue::Decimal(alloc::format!("{}", rem)));
    }

    let b = value_to_i64(&right);
    if b == 0 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "integer division by zero: modulo zero NaN",
        ));
    }
    let a = value_to_i64(&left);
    let rem = a.checked_rem(b).unwrap_or(0);
    if matches!(left, DfdlValue::Int(_)) && matches!(right, DfdlValue::Int(_)) {
        return Ok(DfdlValue::Int(rem as i32));
    }
    Ok(DfdlValue::Long(rem))
}

pub(crate) fn eval_cmp<F: Fn(core::cmp::Ordering) -> bool>(
    op_name: &str,
    left: DfdlValue,
    right: DfdlValue,
    pred: F,
) -> DFDLResult<DfdlValue> {
    if matches!(left, DfdlValue::HexBinary(_)) || matches!(right, DfdlValue::HexBinary(_)) {
        let msg = alloc::format!(
            "Schema Definition Error: Unsupported operation '{}' on type hexBinary",
            op_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    match (&left, &right) {
        (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(pred(a.cmp(b)))),
        (DfdlValue::Boolean(_), _) | (_, DfdlValue::Boolean(_)) => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Cannot compare boolean with non-boolean value",
        )),
        (DfdlValue::String(a), DfdlValue::String(b)) => Ok(DfdlValue::Boolean(pred(a.cmp(b)))),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => match a.partial_cmp(b) {
            Some(ord) => Ok(DfdlValue::Boolean(pred(ord))),
            None => Ok(DfdlValue::Boolean(false)),
        },
        (DfdlValue::Float(_), _)
        | (_, DfdlValue::Float(_))
        | (DfdlValue::Double(_), _)
        | (_, DfdlValue::Double(_))
        | (DfdlValue::Decimal(_), _)
        | (_, DfdlValue::Decimal(_)) => {
            let a = value_to_f64(&left);
            let b = value_to_f64(&right);
            match a.partial_cmp(&b) {
                Some(ord) => Ok(DfdlValue::Boolean(pred(ord))),
                None => Ok(DfdlValue::Boolean(false)),
            }
        }
        _ => {
            let a = value_to_i64(&left);
            let b = value_to_i64(&right);
            Ok(DfdlValue::Boolean(pred(a.cmp(&b))))
        }
    }
}
