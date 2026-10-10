//! DFDL primitive simple types and scalar values.
//!
//! Aligned with DFDL 1.0 §5.1 simple types specification.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Primitive simple types defined by DFDL 1.0 §5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DfdlSimpleType {
    /// String scalar type (`xs:string`).
    String,
    /// 32-bit signed integer (`xs:int`).
    Int,
    /// 64-bit signed integer (`xs:long`).
    Long,
    /// 16-bit signed integer (`xs:short`).
    Short,
    /// 8-bit signed integer (`xs:byte`).
    Byte,
    /// 64-bit unsigned integer (`xs:unsignedLong`).
    UnsignedLong,
    /// 32-bit unsigned integer (`xs:unsignedInt`).
    UnsignedInt,
    /// 16-bit unsigned integer (`xs:unsignedShort`).
    UnsignedShort,
    /// 8-bit unsigned integer (`xs:unsignedByte`).
    UnsignedByte,
    /// Boolean type (`xs:boolean`).
    Boolean,
    /// Single-precision 32-bit IEEE float (`xs:float`).
    Float,
    /// Double-precision 64-bit IEEE float (`xs:double`).
    Double,
    /// Raw binary hex-encoded data (`xs:hexBinary`).
    HexBinary,
    /// Calendar date and time (`xs:dateTime`).
    DateTime,
    /// Calendar date (`xs:date`).
    Date,
    /// Calendar time (`xs:time`).
    Time,
    /// Arbitrary precision decimal (`xs:decimal`).
    Decimal,
}

impl DfdlSimpleType {
    /// Returns `true` if this primitive type is numeric according to DFDL 1.0 §5.1.
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Int
                | Self::Long
                | Self::Short
                | Self::Byte
                | Self::UnsignedLong
                | Self::UnsignedInt
                | Self::UnsignedShort
                | Self::UnsignedByte
                | Self::Float
                | Self::Double
                | Self::Decimal
        )
    }

    /// Returns the default zero/empty primitive value for this simple type.
    #[must_use]
    pub fn default_primitive_value(&self) -> DfdlValue {
        match self {
            Self::String => DfdlValue::String(alloc::string::String::new()),
            Self::Int => DfdlValue::Int(0),
            Self::Long => DfdlValue::Long(0),
            Self::Short => DfdlValue::Short(0),
            Self::Byte => DfdlValue::Byte(0),
            Self::UnsignedLong => DfdlValue::UnsignedLong(0),
            Self::UnsignedInt => DfdlValue::UnsignedInt(0),
            Self::UnsignedShort => DfdlValue::UnsignedShort(0),
            Self::UnsignedByte => DfdlValue::UnsignedByte(0),
            Self::Boolean => DfdlValue::Boolean(false),
            Self::Float => DfdlValue::Float(0.0),
            Self::Double => DfdlValue::Double(0.0),
            Self::HexBinary => DfdlValue::HexBinary(alloc::vec::Vec::new()),
            Self::Decimal => DfdlValue::Decimal(alloc::string::String::from("0")),
            Self::DateTime => DfdlValue::DateTime(alloc::string::String::from("1970-01-01T00:00:00")),
            Self::Date => DfdlValue::Date(alloc::string::String::from("1970-01-01")),
            Self::Time => DfdlValue::Time(alloc::string::String::from("00:00:00")),
        }
    }

    /// Coerces a logical value representation into this target simple type per XML Schema rules.
    #[must_use]
    pub fn coerce_value(&self, val: &DfdlValue) -> Option<DfdlValue> {
        match (self, val) {
            (Self::String, DfdlValue::String(_)) => Some(val.clone()),
            (Self::Int, DfdlValue::Int(_)) => Some(val.clone()),
            (Self::Long, DfdlValue::Long(_)) => Some(val.clone()),
            (Self::Short, DfdlValue::Short(_)) => Some(val.clone()),
            (Self::Byte, DfdlValue::Byte(_)) => Some(val.clone()),
            (Self::UnsignedLong, DfdlValue::UnsignedLong(_)) => Some(val.clone()),
            (Self::UnsignedInt, DfdlValue::UnsignedInt(_)) => Some(val.clone()),
            (Self::UnsignedShort, DfdlValue::UnsignedShort(_)) => Some(val.clone()),
            (Self::UnsignedByte, DfdlValue::UnsignedByte(_)) => Some(val.clone()),
            (Self::Boolean, DfdlValue::Boolean(_)) => Some(val.clone()),
            (Self::Float, DfdlValue::Float(_)) => Some(val.clone()),
            (Self::Double, DfdlValue::Double(_)) => Some(val.clone()),
            (Self::HexBinary, DfdlValue::HexBinary(_)) => Some(val.clone()),
            (Self::DateTime, DfdlValue::DateTime(_)) => Some(val.clone()),
            (Self::Date, DfdlValue::Date(_)) => Some(val.clone()),
            (Self::Time, DfdlValue::Time(_)) => Some(val.clone()),
            (Self::Decimal, DfdlValue::Decimal(_)) => Some(val.clone()),
            _ => {
                let s = alloc::string::ToString::to_string(val);
                let trimmed = s.trim();
                match self {
                    Self::Int => trimmed.parse::<i32>().ok().map(DfdlValue::Int),
                    Self::Long => trimmed.parse::<i64>().ok().map(DfdlValue::Long),
                    Self::Short => trimmed.parse::<i16>().ok().map(DfdlValue::Short),
                    Self::Byte => trimmed.parse::<i8>().ok().map(DfdlValue::Byte),
                    Self::UnsignedLong => trimmed.parse::<u64>().ok().map(DfdlValue::UnsignedLong),
                    Self::UnsignedInt => trimmed.parse::<u32>().ok().map(DfdlValue::UnsignedInt),
                    Self::UnsignedShort => trimmed.parse::<u16>().ok().map(DfdlValue::UnsignedShort),
                    Self::UnsignedByte => trimmed.parse::<u8>().ok().map(DfdlValue::UnsignedByte),
                    Self::Boolean => match trimmed {
                        "true" | "1" => Some(DfdlValue::Boolean(true)),
                        "false" | "0" => Some(DfdlValue::Boolean(false)),
                        _ => None,
                    },
                    Self::Float => trimmed.parse::<f32>().ok().map(DfdlValue::Float),
                    Self::Double => trimmed.parse::<f64>().ok().map(DfdlValue::Double),
                    Self::String | Self::HexBinary => {
                        Some(DfdlValue::String(alloc::string::String::from(trimmed)))
                    }
                    Self::DateTime => Some(DfdlValue::DateTime(alloc::string::String::from(trimmed))),
                    Self::Date => Some(DfdlValue::Date(alloc::string::String::from(trimmed))),
                    Self::Time => Some(DfdlValue::Time(alloc::string::String::from(trimmed))),
                    Self::Decimal => Some(DfdlValue::Decimal(alloc::string::String::from(trimmed))),
                }
            }
        }
    }

    /// Returns the standard DFDL / XML Schema type name for this primitive simple type.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Long => "long",
            Self::Short => "short",
            Self::Byte => "byte",
            Self::UnsignedLong => "unsignedLong",
            Self::UnsignedInt => "unsignedInt",
            Self::UnsignedShort => "unsignedShort",
            Self::UnsignedByte => "unsignedByte",
            Self::Boolean => "boolean",
            Self::Float => "float",
            Self::Double => "double",
            Self::HexBinary => "hexBinary",
            Self::DateTime => "dateTime",
            Self::Date => "date",
            Self::Time => "time",
            Self::Decimal => "decimal",
        }
    }
}

/// Typed DFDL scalar value representation.
#[derive(Debug, Clone, PartialEq)]
pub enum DfdlValue {
    /// Text string value.
    String(String),
    /// 32-bit signed integer.
    Int(i32),
    /// 64-bit signed integer.
    Long(i64),
    /// 16-bit signed integer.
    Short(i16),
    /// 8-bit signed integer.
    Byte(i8),
    /// 64-bit unsigned integer.
    UnsignedLong(u64),
    /// 32-bit unsigned integer.
    UnsignedInt(u32),
    /// 16-bit unsigned integer.
    UnsignedShort(u16),
    /// 8-bit unsigned integer.
    UnsignedByte(u8),
    /// Boolean value.
    Boolean(bool),
    /// 32-bit IEEE float.
    Float(f32),
    /// 64-bit IEEE double.
    Double(f64),
    /// Raw hex/binary bytes.
    HexBinary(Vec<u8>),
    /// Calendar date and time ISO string.
    DateTime(String),
    /// Calendar date ISO string.
    Date(String),
    /// Calendar time ISO string.
    Time(String),
    /// Decimal representation string.
    Decimal(String),
}

impl DfdlValue {
    /// Returns the corresponding primitive type for this scalar value.
    #[must_use]
    pub const fn simple_type(&self) -> DfdlSimpleType {
        match self {
            Self::String(_) => DfdlSimpleType::String,
            Self::Int(_) => DfdlSimpleType::Int,
            Self::Long(_) => DfdlSimpleType::Long,
            Self::Short(_) => DfdlSimpleType::Short,
            Self::Byte(_) => DfdlSimpleType::Byte,
            Self::UnsignedLong(_) => DfdlSimpleType::UnsignedLong,
            Self::UnsignedInt(_) => DfdlSimpleType::UnsignedInt,
            Self::UnsignedShort(_) => DfdlSimpleType::UnsignedShort,
            Self::UnsignedByte(_) => DfdlSimpleType::UnsignedByte,
            Self::Boolean(_) => DfdlSimpleType::Boolean,
            Self::Float(_) => DfdlSimpleType::Float,
            Self::Double(_) => DfdlSimpleType::Double,
            Self::HexBinary(_) => DfdlSimpleType::HexBinary,
            Self::DateTime(_) => DfdlSimpleType::DateTime,
            Self::Date(_) => DfdlSimpleType::Date,
            Self::Time(_) => DfdlSimpleType::Time,
            Self::Decimal(_) => DfdlSimpleType::Decimal,
        }
    }

    /// Returns the standard DFDL / XML Schema primitive type name of this scalar value.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.simple_type().name()
    }

    /// Converts numeric or string scalar value to `i128` if possible.
    #[must_use]
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            Self::Int(v) => Some(i128::from(*v)),
            Self::Long(v) => Some(i128::from(*v)),
            Self::Short(v) => Some(i128::from(*v)),
            Self::Byte(v) => Some(i128::from(*v)),
            Self::UnsignedLong(v) => Some(i128::from(*v)),
            Self::UnsignedInt(v) => Some(i128::from(*v)),
            Self::UnsignedShort(v) => Some(i128::from(*v)),
            Self::UnsignedByte(v) => Some(i128::from(*v)),
            Self::Decimal(s) | Self::String(s) => s.trim().parse::<i128>().ok(),
            _ => None,
        }
    }

    /// Returns the Effective Boolean Value (EBV) per XPath 2.0 §2.4.3.
    #[must_use]
    pub fn effective_boolean_value(&self) -> bool {
        match self {
            Self::Boolean(b) => *b,
            Self::Int(n) => *n != 0,
            Self::Long(n) => *n != 0,
            Self::Short(n) => *n != 0,
            Self::Byte(n) => *n != 0,
            Self::UnsignedLong(n) => *n != 0,
            Self::UnsignedInt(n) => *n != 0,
            Self::UnsignedShort(n) => *n != 0,
            Self::UnsignedByte(n) => *n != 0,
            Self::Float(f) => *f != 0.0 && !f.is_nan(),
            Self::Double(d) => *d != 0.0 && !d.is_nan(),
            Self::Decimal(s) => {
                let trimmed = s.trim();
                !trimmed.is_empty() && (trimmed.parse::<f64>() != Ok(0.0))
            }
            Self::String(s) => !s.is_empty(),
            _ => false,
        }
    }

    /// Returns true if the scalar value is negative.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        match self {
            Self::Int(v) => *v < 0,
            Self::Long(v) => *v < 0,
            Self::Short(v) => *v < 0,
            Self::Byte(v) => *v < 0,
            Self::Float(v) => *v < 0.0,
            Self::Double(v) => *v < 0.0,
            Self::Decimal(s) | Self::String(s) => s.trim().starts_with('-'),
            _ => false,
        }
    }
}

impl fmt::Display for DfdlValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => write!(f, "{}", s),
            Self::Int(v) => write!(f, "{}", v),
            Self::Long(v) => write!(f, "{}", v),
            Self::Short(v) => write!(f, "{}", v),
            Self::Byte(v) => write!(f, "{}", v),
            Self::UnsignedLong(v) => write!(f, "{}", v),
            Self::UnsignedInt(v) => write!(f, "{}", v),
            Self::UnsignedShort(v) => write!(f, "{}", v),
            Self::UnsignedByte(v) => write!(f, "{}", v),
            Self::Boolean(b) => write!(f, "{}", b),
            Self::Float(v) => write!(f, "{}", v),
            Self::Double(v) => write!(f, "{}", v),
            Self::HexBinary(b) => {
                for byte in b {
                    write!(f, "{:02X}", byte)?;
                }
                Ok(())
            }
            Self::DateTime(dt) => write!(f, "{}", dt),
            Self::Date(d) => write!(f, "{}", d),
            Self::Time(t) => write!(f, "{}", t),
            Self::Decimal(d) => write!(f, "{}", d),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_dfdl_value_types_and_display() {
        let val_int = DfdlValue::Int(42);
        assert_eq!(val_int.simple_type(), DfdlSimpleType::Int);
        assert_eq!(val_int.to_string(), "42");

        let val_hex = DfdlValue::HexBinary(Vec::from([0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(val_hex.simple_type(), DfdlSimpleType::HexBinary);
        assert_eq!(val_hex.to_string(), "DEADBEEF");
    }

    /// Verifies that [`DfdlSimpleType::is_numeric`] correctly classifies all 16 DFDL 1.0
    /// primitive types, distinguishing numeric types from strings, booleans, calendars, and binary.
    #[test]
    fn test_dfdl_simple_type_is_numeric() {
        // All numeric primitive types specified in DFDL 1.0 §5.1
        let numeric_types = [
            DfdlSimpleType::Int,
            DfdlSimpleType::Long,
            DfdlSimpleType::Short,
            DfdlSimpleType::Byte,
            DfdlSimpleType::UnsignedLong,
            DfdlSimpleType::UnsignedInt,
            DfdlSimpleType::UnsignedShort,
            DfdlSimpleType::UnsignedByte,
            DfdlSimpleType::Float,
            DfdlSimpleType::Double,
            DfdlSimpleType::Decimal,
        ];
        for t in numeric_types {
            assert!(t.is_numeric(), "Expected {:?} to be numeric", t);
        }

        // Non-numeric primitive types (must never undergo number normalization or numeric padding)
        let non_numeric_types = [
            DfdlSimpleType::String,
            DfdlSimpleType::Boolean,
            DfdlSimpleType::HexBinary,
            DfdlSimpleType::DateTime,
            DfdlSimpleType::Date,
            DfdlSimpleType::Time,
        ];
        for t in non_numeric_types {
            assert!(!t.is_numeric(), "Expected {:?} to be non-numeric", t);
        }
    }

    /// Verifies default primitive values, type names, coercions, EBV, and is_negative across all types.
    #[test]
    fn test_dfdl_value_coercions_ebv_and_defaults() {
        let all_types = [
            DfdlSimpleType::String,
            DfdlSimpleType::Int,
            DfdlSimpleType::Long,
            DfdlSimpleType::Short,
            DfdlSimpleType::Byte,
            DfdlSimpleType::UnsignedLong,
            DfdlSimpleType::UnsignedInt,
            DfdlSimpleType::UnsignedShort,
            DfdlSimpleType::UnsignedByte,
            DfdlSimpleType::Boolean,
            DfdlSimpleType::Float,
            DfdlSimpleType::Double,
            DfdlSimpleType::HexBinary,
            DfdlSimpleType::DateTime,
            DfdlSimpleType::Date,
            DfdlSimpleType::Time,
            DfdlSimpleType::Decimal,
        ];

        for t in all_types {
            let def = t.default_primitive_value();
            assert_eq!(def.simple_type(), t);
            assert_eq!(t.name(), def.type_name());
            assert!(!t.name().is_empty());
        }

        // Coerce string to numeric and boolean types
        let str_val = DfdlValue::String("123".into());
        assert_eq!(DfdlSimpleType::Int.coerce_value(&str_val), Some(DfdlValue::Int(123)));
        assert_eq!(DfdlSimpleType::Long.coerce_value(&str_val), Some(DfdlValue::Long(123)));
        assert_eq!(DfdlSimpleType::Short.coerce_value(&str_val), Some(DfdlValue::Short(123)));
        assert_eq!(DfdlSimpleType::Byte.coerce_value(&str_val), Some(DfdlValue::Byte(123)));
        assert_eq!(DfdlSimpleType::UnsignedLong.coerce_value(&str_val), Some(DfdlValue::UnsignedLong(123)));
        assert_eq!(DfdlSimpleType::UnsignedInt.coerce_value(&str_val), Some(DfdlValue::UnsignedInt(123)));
        assert_eq!(DfdlSimpleType::UnsignedShort.coerce_value(&str_val), Some(DfdlValue::UnsignedShort(123)));
        assert_eq!(DfdlSimpleType::UnsignedByte.coerce_value(&str_val), Some(DfdlValue::UnsignedByte(123)));
        assert_eq!(DfdlSimpleType::Float.coerce_value(&str_val), Some(DfdlValue::Float(123.0)));
        assert_eq!(DfdlSimpleType::Double.coerce_value(&str_val), Some(DfdlValue::Double(123.0)));

        assert_eq!(DfdlSimpleType::Boolean.coerce_value(&DfdlValue::String("true".into())), Some(DfdlValue::Boolean(true)));
        assert_eq!(DfdlSimpleType::Boolean.coerce_value(&DfdlValue::String("0".into())), Some(DfdlValue::Boolean(false)));
        assert_eq!(DfdlSimpleType::Boolean.coerce_value(&DfdlValue::String("invalid".into())), None);

        // as_i128 across integer types
        assert_eq!(DfdlValue::Short(10).as_i128(), Some(10));
        assert_eq!(DfdlValue::Byte(5).as_i128(), Some(5));
        assert_eq!(DfdlValue::Long(100).as_i128(), Some(100));
        assert_eq!(DfdlValue::UnsignedLong(100).as_i128(), Some(100));
        assert_eq!(DfdlValue::UnsignedInt(50).as_i128(), Some(50));
        assert_eq!(DfdlValue::UnsignedShort(20).as_i128(), Some(20));
        assert_eq!(DfdlValue::UnsignedByte(10).as_i128(), Some(10));
        assert_eq!(DfdlValue::Decimal("99".into()).as_i128(), Some(99));
        assert_eq!(DfdlValue::Boolean(true).as_i128(), None);

        // effective_boolean_value
        assert!(DfdlValue::Boolean(true).effective_boolean_value());
        assert!(!DfdlValue::Boolean(false).effective_boolean_value());
        assert!(DfdlValue::Long(1).effective_boolean_value());
        assert!(!DfdlValue::Long(0).effective_boolean_value());
        assert!(DfdlValue::Short(1).effective_boolean_value());
        assert!(DfdlValue::Byte(1).effective_boolean_value());
        assert!(DfdlValue::UnsignedLong(1).effective_boolean_value());
        assert!(DfdlValue::UnsignedInt(1).effective_boolean_value());
        assert!(DfdlValue::UnsignedShort(1).effective_boolean_value());
        assert!(DfdlValue::UnsignedByte(1).effective_boolean_value());
        assert!(DfdlValue::Float(12.34).effective_boolean_value());
        assert!(!DfdlValue::Float(0.0).effective_boolean_value());
        assert!(DfdlValue::Double(56.78).effective_boolean_value());
        assert!(!DfdlValue::Double(0.0).effective_boolean_value());
        assert!(DfdlValue::Decimal("12".into()).effective_boolean_value());
        assert!(!DfdlValue::Decimal("0".into()).effective_boolean_value());
        assert!(DfdlValue::String("non-empty".into()).effective_boolean_value());
        assert!(!DfdlValue::String("".into()).effective_boolean_value());

        // is_negative
        assert!(DfdlValue::Int(-5).is_negative());
        assert!(!DfdlValue::Int(5).is_negative());
        assert!(DfdlValue::Long(-50).is_negative());
        assert!(DfdlValue::Short(-10).is_negative());
        assert!(DfdlValue::Byte(-1).is_negative());
        assert!(DfdlValue::Float(-12.34).is_negative());
        assert!(DfdlValue::Double(-56.78).is_negative());
        assert!(DfdlValue::Decimal("-123".into()).is_negative());
        assert!(!DfdlValue::UnsignedLong(10).is_negative());

        // Display formatting
        assert_eq!(alloc::format!("{}", DfdlValue::Short(10)), "10");
        assert_eq!(alloc::format!("{}", DfdlValue::Byte(2)), "2");
        assert_eq!(alloc::format!("{}", DfdlValue::UnsignedLong(99)), "99");
        assert_eq!(alloc::format!("{}", DfdlValue::UnsignedInt(88)), "88");
        assert_eq!(alloc::format!("{}", DfdlValue::UnsignedShort(77)), "77");
        assert_eq!(alloc::format!("{}", DfdlValue::UnsignedByte(66)), "66");
        assert_eq!(alloc::format!("{}", DfdlValue::Boolean(true)), "true");
        assert_eq!(alloc::format!("{}", DfdlValue::Date("2026-01-01".into())), "2026-01-01");
        assert_eq!(alloc::format!("{}", DfdlValue::Time("12:00:00".into())), "12:00:00");
        assert_eq!(alloc::format!("{}", DfdlValue::DateTime("2026-01-01T12:00:00".into())), "2026-01-01T12:00:00");
    }
}
