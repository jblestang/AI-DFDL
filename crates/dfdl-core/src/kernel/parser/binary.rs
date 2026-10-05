//! Binary decoding and parsing logic for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::{DfdlSimpleType, DfdlValue};
use crate::io::traits::{BitOrder, ByteOrder, ByteSource};
use crate::schema::ir::ResolvedProperties;
use crate::util::try_push;

use super::ParserEngine;

pub(crate) fn sign_extend(val: u64, bits: usize) -> i64 {
    // A 1-bit signed integer has no room for a sign bit and is read as unsigned.
    if bits <= 1 || bits >= 64 {
        val as i64
    } else {
        let mask = 1u64 << (bits.saturating_sub(1));
        if (val & mask) != 0 {
            (val | (!0u64 << bits)) as i64
        } else {
            val as i64
        }
    }
}

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    pub(crate) fn read_binary_bits(&mut self, bits: usize) -> DFDLResult<u64> {
        if bits == 0 {
            return Ok(0);
        }
        if bits <= 64 {
            if self.reader.is_eof() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::Parse,
                    "Insufficient binary data for primitive scalar",
                ));
            }
            self.reader.read_bits(bits).map_err(|_| {
                DFDLError::new_static(
                    DFDLErrorKind::Parse,
                    "Insufficient binary data for primitive scalar",
                )
            })
        } else {
            let mut val = 0u64;
            let mut remaining = bits;
            while remaining > 0 {
                let chunk = remaining.min(64);
                if self.reader.is_eof() {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::Parse,
                        "Insufficient binary data for primitive scalar",
                    ));
                }
                let chunk_val = self.reader.read_bits(chunk).map_err(|_| {
                    DFDLError::new_static(
                        DFDLErrorKind::Parse,
                        "Insufficient binary data for primitive scalar",
                    )
                })?;
                val = if chunk >= 64 {
                    chunk_val
                } else {
                    (val << chunk) | chunk_val
                };
                remaining = remaining.saturating_sub(chunk);
            }
            Ok(val)
        }
    }

    pub(crate) fn format_virtual_decimal(val: i64, scale: i32) -> String {
        if scale <= 0 {
            return alloc::format!("{}", val);
        }
        let is_neg = val < 0;
        let abs_val = val.unsigned_abs();
        let s = alloc::format!("{}", abs_val);
        let scale_usize = scale as usize;
        let formatted = if s.len() <= scale_usize {
            let mut out = String::from("0.");
            for _ in 0..(scale_usize - s.len()) {
                out.push('0');
            }
            out.push_str(&s);
            out
        } else {
            let split_pos = s.len() - scale_usize;
            alloc::format!("{}.{}", &s[..split_pos], &s[split_pos..])
        };
        if is_neg {
            alloc::format!("-{}", formatted)
        } else {
            formatted
        }
    }

    pub(crate) fn parse_binary_value(
        &mut self,
        simple_type: DfdlSimpleType,
        props: &ResolvedProperties,
        dynamic_len: Option<usize>,
    ) -> DFDLResult<DfdlValue> {
        let explicit_len = dynamic_len.unwrap_or(0);
        let requested_bits_opt = dynamic_len.map(|len| match props.length_units {
            crate::schema::ir::LengthUnits::Bits => len,
            crate::schema::ir::LengthUnits::Bytes | crate::schema::ir::LengthUnits::Characters => {
                len.saturating_mul(8)
            }
        });

        if props.binary_number_rep == crate::schema::ir::BinaryNumberRep::Binary {
            if let Some(bits) = requested_bits_opt {
                let is_unsigned_int = matches!(
                    simple_type,
                    DfdlSimpleType::UnsignedByte
                        | DfdlSimpleType::UnsignedShort
                        | DfdlSimpleType::UnsignedInt
                        | DfdlSimpleType::UnsignedLong
                );
                let is_signed_int = matches!(
                    simple_type,
                    DfdlSimpleType::Byte
                        | DfdlSimpleType::Short
                        | DfdlSimpleType::Int
                        | DfdlSimpleType::Long
                );

                if is_unsigned_int && bits == 0 {
                    let msg = alloc::format!(
                        "Schema Definition Error: unsigned binary integer: minimum length is 1 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                if is_signed_int
                    && bits < 2
                    && (bits == 0 || self.schema.disallow_signed_integer_length_1bit)
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: signed binary integer: minimum length is 2 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                let (min_bits, max_bits) = match simple_type {
                    DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => (1, 8),
                    DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => (1, 16),
                    DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt => (1, 32),
                    DfdlSimpleType::Long | DfdlSimpleType::UnsignedLong => (1, 64),
                    _ => (0, 0),
                };
                if max_bits > 0 && (bits < min_bits || bits > max_bits) {
                    let msg = alloc::format!(
                        "Schema Definition Error: Length in bits {} out of range. Expected between {} and {} for {:?}",
                        bits,
                        min_bits,
                        max_bits,
                        simple_type
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let calc_bits =
            |default_bits: usize| -> usize { requested_bits_opt.unwrap_or(default_bits) };

        match props.binary_number_rep {
            crate::schema::ir::BinaryNumberRep::Packed
            | crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                let mut bytes = Vec::new();
                if let Some(l) = dynamic_len {
                    let len_bytes = match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => l.saturating_add(7) / 8,
                        _ => l,
                    };
                    for _ in 0..len_bytes {
                        let b = self.read_binary_bits(8)? as u8;
                        try_push(&mut bytes, b)?;
                    }
                } else {
                    while !self.reader.is_eof() {
                        let mut matched_in_scope = false;
                        if let Some(ref term) = props.terminator {
                            if !term.is_empty() && self.peek_literal_delimiter(term) {
                                matched_in_scope = true;
                            }
                        }
                        if !matched_in_scope {
                            for i in (0..self.in_scope_delimiters.len()).rev() {
                                if let Ok(delim) =
                                    crate::util::get_checked(&self.in_scope_delimiters, i)
                                {
                                    let delim_clone = delim.clone();
                                    if !delim_clone.is_empty()
                                        && self.peek_literal_delimiter(&delim_clone)
                                    {
                                        matched_in_scope = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if matched_in_scope {
                            break;
                        }
                        if let Ok(b) = self.read_binary_bits(8) {
                            try_push(&mut bytes, b as u8)?;
                        } else {
                            break;
                        }
                    }
                }
                let val_i64 = match props.binary_number_rep {
                    crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                        crate::util::decode_ibm4690_packed(&bytes)?
                    }
                    _ => crate::util::decode_packed_decimal_with_signs(
                        &bytes,
                        props.binary_packed_sign_codes.as_deref(),
                    )?,
                };
                if val_i64 < 0 && !props.decimal_signed {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Parse Error: Packed binary data is negative, but dfdl:decimalSigned is 'no'",
                    ));
                }
                return match simple_type {
                    DfdlSimpleType::Int => {
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:int", val_i64),
                            ));
                        }
                        Ok(DfdlValue::Int(val_i64 as i32))
                    }
                    DfdlSimpleType::Short => {
                        if !(i16::MIN as i64..=i16::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:short", val_i64),
                            ));
                        }
                        Ok(DfdlValue::Short(val_i64 as i16))
                    }
                    DfdlSimpleType::Byte => {
                        if !(i8::MIN as i64..=i8::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:byte", val_i64),
                            ));
                        }
                        Ok(DfdlValue::Byte(val_i64 as i8))
                    }
                    DfdlSimpleType::UnsignedLong => {
                        if val_i64 < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} is negative, out of range for type xs:unsignedLong", val_i64),
                            ));
                        }
                        Ok(DfdlValue::UnsignedLong(val_i64 as u64))
                    }
                    DfdlSimpleType::UnsignedInt => {
                        if !(0..=u32::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedInt", val_i64),
                            ));
                        }
                        Ok(DfdlValue::UnsignedInt(val_i64 as u32))
                    }
                    DfdlSimpleType::UnsignedShort => {
                        if !(0..=u16::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedShort", val_i64),
                            ));
                        }
                        Ok(DfdlValue::UnsignedShort(val_i64 as u16))
                    }
                    DfdlSimpleType::UnsignedByte => {
                        if !(0..=u8::MAX as i64).contains(&val_i64) {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedByte", val_i64),
                            ));
                        }
                        Ok(DfdlValue::UnsignedByte(val_i64 as u8))
                    }
                    DfdlSimpleType::Decimal | DfdlSimpleType::String => {
                        let formatted = Self::format_virtual_decimal(
                            val_i64,
                            props.binary_decimal_virtual_point,
                        );
                        Ok(DfdlValue::Decimal(formatted))
                    }
                    _ => Ok(DfdlValue::Long(val_i64)),
                };
            }
            crate::schema::ir::BinaryNumberRep::Bcd => {
                let mut bytes = Vec::new();
                if let Some(l) = dynamic_len {
                    let len_bytes = match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => l.saturating_add(7) / 8,
                        _ => l,
                    };
                    for _ in 0..len_bytes {
                        let b = self.read_binary_bits(8)? as u8;
                        try_push(&mut bytes, b)?;
                    }
                } else {
                    while !self.reader.is_eof() {
                        let mut matched_in_scope = false;
                        for i in (0..self.in_scope_delimiters.len()).rev() {
                            if let Ok(delim) =
                                crate::util::get_checked(&self.in_scope_delimiters, i)
                            {
                                let delim_clone = delim.clone();
                                if !delim_clone.is_empty()
                                    && self.peek_literal_delimiter(&delim_clone)
                                {
                                    matched_in_scope = true;
                                    break;
                                }
                            }
                        }
                        if matched_in_scope {
                            break;
                        }
                        if let Ok(b) = self.read_binary_bits(8) {
                            try_push(&mut bytes, b as u8)?;
                        } else {
                            break;
                        }
                    }
                }
                let mut val: u64 = 0;
                for &b in &bytes {
                    let high = (b >> 4) & 0x0F;
                    let low = b & 0x0F;
                    if high > 9 || low > 9 {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "not an allowed type for bcd",
                        ));
                    }
                    val = val
                        .saturating_mul(100)
                        .saturating_add(high as u64 * 10)
                        .saturating_add(low as u64);
                }
                return match simple_type {
                    DfdlSimpleType::Int => {
                        if val > i32::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:int", val),
                            ));
                        }
                        Ok(DfdlValue::Int(val as i32))
                    }
                    DfdlSimpleType::Short => {
                        if val > i16::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:short", val),
                            ));
                        }
                        Ok(DfdlValue::Short(val as i16))
                    }
                    DfdlSimpleType::Byte => {
                        if val > i8::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:byte", val),
                            ));
                        }
                        Ok(DfdlValue::Byte(val as i8))
                    }
                    DfdlSimpleType::UnsignedLong => {
                        Ok(DfdlValue::UnsignedLong(val))
                    }
                    DfdlSimpleType::UnsignedInt => {
                        if val > u32::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedInt", val),
                            ));
                        }
                        Ok(DfdlValue::UnsignedInt(val as u32))
                    }
                    DfdlSimpleType::UnsignedShort => {
                        if val > u16::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedShort", val),
                            ));
                        }
                        Ok(DfdlValue::UnsignedShort(val as u16))
                    }
                    DfdlSimpleType::UnsignedByte => {
                        if val > u8::MAX as u64 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!("Parse Error: Value {} out of range for type xs:unsignedByte", val),
                            ));
                        }
                        Ok(DfdlValue::UnsignedByte(val as u8))
                    }
                    DfdlSimpleType::Decimal | DfdlSimpleType::String => {
                        let formatted = Self::format_virtual_decimal(
                            val as i64,
                            props.binary_decimal_virtual_point,
                        );
                        Ok(DfdlValue::Decimal(formatted))
                    }
                    _ => Ok(DfdlValue::Long(val as i64)),
                };
            }
            crate::schema::ir::BinaryNumberRep::Binary => {}
        }

        match simple_type {
            DfdlSimpleType::Int => {
                let bits = calc_bits(32);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_u32 = val_u64 as u32;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u32,
                    ByteOrder::LittleEndian => val_u32.swap_bytes(),
                };
                let val_i32 = sign_extend(ordered as u64, bits) as i32;
                Ok(DfdlValue::Int(val_i32))
            }
            DfdlSimpleType::Long => {
                let bits = calc_bits(64);
                let val_u64 = self.read_binary_bits(bits)?;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u64,
                    ByteOrder::LittleEndian => val_u64.swap_bytes(),
                };
                let val_i64 = sign_extend(ordered, bits);
                Ok(DfdlValue::Long(val_i64))
            }
            DfdlSimpleType::Short => {
                let bits = calc_bits(16);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_u16 = val_u64 as u16;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u16,
                    ByteOrder::LittleEndian => val_u16.swap_bytes(),
                };
                let val_i16 = sign_extend(ordered as u64, bits) as i16;
                Ok(DfdlValue::Short(val_i16))
            }
            DfdlSimpleType::Byte => {
                let bits = calc_bits(8);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_i8 = sign_extend(val_u64, bits) as i8;
                Ok(DfdlValue::Byte(val_i8))
            }
            DfdlSimpleType::UnsignedInt => {
                let bits = calc_bits(32);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_u32 = val_u64 as u32;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u32,
                    ByteOrder::LittleEndian => val_u32.swap_bytes(),
                };
                Ok(DfdlValue::UnsignedInt(ordered))
            }
            DfdlSimpleType::UnsignedLong => {
                let bits = calc_bits(64);
                let val_u64 = self.read_binary_bits(bits)?;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u64,
                    ByteOrder::LittleEndian => val_u64.swap_bytes(),
                };
                Ok(DfdlValue::UnsignedLong(ordered))
            }
            DfdlSimpleType::UnsignedShort => {
                let bits = calc_bits(16);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_u16 = val_u64 as u16;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u16,
                    ByteOrder::LittleEndian => val_u16.swap_bytes(),
                };
                Ok(DfdlValue::UnsignedShort(ordered))
            }
            DfdlSimpleType::UnsignedByte => {
                let bits = calc_bits(8);
                let val_u64 = self.read_binary_bits(bits)?;
                Ok(DfdlValue::UnsignedByte(val_u64 as u8))
            }
            DfdlSimpleType::Boolean => {
                let bits = calc_bits(32);
                let raw_u64 = self.read_binary_bits(bits)?;
                let ordered = match bits {
                    16 => match props.byte_order {
                        ByteOrder::BigEndian => (raw_u64 as u16) as u64,
                        ByteOrder::LittleEndian => (raw_u64 as u16).swap_bytes() as u64,
                    },
                    32 => match props.byte_order {
                        ByteOrder::BigEndian => (raw_u64 as u32) as u64,
                        ByteOrder::LittleEndian => (raw_u64 as u32).swap_bytes() as u64,
                    },
                    64 => match props.byte_order {
                        ByteOrder::BigEndian => raw_u64,
                        ByteOrder::LittleEndian => raw_u64.swap_bytes(),
                    },
                    _ => raw_u64,
                };
                let matches_rep = |rep: i64| -> bool {
                    if rep >= 0 {
                        ordered == (rep as u64)
                    } else {
                        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
                        ordered == ((rep as i128) as u64 & mask)
                    }
                };
                let is_true = match (props.binary_boolean_true_rep, props.binary_boolean_false_rep) {
                    (crate::schema::ir::BinaryBooleanRep::Value(t), crate::schema::ir::BinaryBooleanRep::Value(f)) => {
                        if matches_rep(t) {
                            true
                        } else if matches_rep(f) {
                            false
                        } else {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!(
                                    "Parse Error: Binary boolean value {} matches neither binaryBooleanTrueRep ({}) nor binaryBooleanFalseRep ({})",
                                    ordered, t, f
                                ),
                            ));
                        }
                    }
                    (crate::schema::ir::BinaryBooleanRep::Value(t), _) => {
                        matches_rep(t)
                    }
                    (_, crate::schema::ir::BinaryBooleanRep::Value(f)) => {
                        !matches_rep(f)
                    }
                    _ => ordered != 0,
                };
                Ok(DfdlValue::Boolean(is_true))
            }
            DfdlSimpleType::Float => {
                let bits = calc_bits(32);
                let val_u64 = self.read_binary_bits(bits)?;
                let val_u32 = val_u64 as u32;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u32,
                    ByteOrder::LittleEndian => val_u32.swap_bytes(),
                };
                Ok(DfdlValue::Float(f32::from_bits(ordered)))
            }
            DfdlSimpleType::Double => {
                let bits = calc_bits(64);
                let val_u64 = self.read_binary_bits(bits)?;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u64,
                    ByteOrder::LittleEndian => val_u64.swap_bytes(),
                };
                Ok(DfdlValue::Double(f64::from_bits(ordered)))
            }
            DfdlSimpleType::HexBinary => {
                let mut bytes = Vec::new();
                if props.length_units == crate::schema::ir::LengthUnits::Bits {
                    if let Some(mut bits_left) = dynamic_len {
                        while bits_left > 0 {
                            let chunk = bits_left.min(8);
                            let val = self.read_binary_bits(chunk)? as u8;
                            let byte_val = if chunk < 8
                                && props.bit_order == BitOrder::MostSignificantBitFirst
                            {
                                val << (8 - chunk)
                            } else {
                                val
                            };
                            try_push(&mut bytes, byte_val)?;
                            bits_left = bits_left.saturating_sub(chunk);
                        }
                    } else {
                        while !self.reader.is_eof() {
                            if let Ok(b) = self.read_binary_bits(8) {
                                try_push(&mut bytes, b as u8)?;
                            } else {
                                break;
                            }
                        }
                    }
                } else if let Some(len) = dynamic_len {
                    for _ in 0..len {
                        let b = self.read_binary_bits(8)? as u8;
                        try_push(&mut bytes, b)?;
                    }
                } else {
                    while !self.reader.is_eof() {
                        let mut matched_in_scope = false;
                        for i in (0..self.in_scope_delimiters.len()).rev() {
                            if let Ok(delim) =
                                crate::util::get_checked(&self.in_scope_delimiters, i)
                            {
                                let delim_clone = delim.clone();
                                if !delim_clone.is_empty()
                                    && self.peek_literal_delimiter(&delim_clone)
                                {
                                    matched_in_scope = true;
                                    break;
                                }
                            }
                        }
                        if matched_in_scope {
                            break;
                        }
                        if let Ok(b) = self.read_binary_bits(8) {
                            try_push(&mut bytes, b as u8)?;
                        } else {
                            break;
                        }
                    }
                }
                if let Some(max_len) = self.schema.max_hex_binary_length_in_bytes {
                    if bytes.len() > max_len {
                        let msg = alloc::format!(
                            "Parse Error: xs:hexBinary length ({}) exceeds maximum allowed length of {} bytes",
                            bytes.len(), max_len
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                }
                Ok(DfdlValue::HexBinary(bytes))
            }
            DfdlSimpleType::String => {
                let enc_upper = props.encoding.to_ascii_uppercase();
                if enc_upper.starts_with("X-DFDL-") {
                    let char_bits = if enc_upper.contains("BITS") {
                        1
                    } else if enc_upper.contains("BASE4") {
                        2
                    } else if enc_upper.contains("OCTAL") || enc_upper.contains("3-BIT") {
                        3
                    } else if enc_upper.contains("HEX") {
                        4
                    } else if enc_upper.contains("6-BIT") {
                        6
                    } else {
                        8
                    };
                    let mut s = String::new();
                    if let Some(l) = dynamic_len {
                        let num_chars = match props.length_units {
                            crate::schema::ir::LengthUnits::Bits => l / char_bits,
                            _ => l,
                        };
                        for _ in 0..num_chars {
                            let val = self.read_binary_bits(char_bits)?;
                            let ch = crate::encoding::decode_sub_byte_char(val, &props.encoding);
                            s.push(ch);
                        }
                    } else {
                        while !self.reader.is_eof() {
                            if let Ok(val) = self.read_binary_bits(char_bits) {
                                let ch = crate::encoding::decode_sub_byte_char(val, &props.encoding);
                                s.push(ch);
                            } else {
                                break;
                            }
                        }
                    }
                    Ok(DfdlValue::String(s))
                } else {
                    let mut bytes = Vec::new();
                    if props.length_units == crate::schema::ir::LengthUnits::Characters
                        && props.encoding.to_ascii_uppercase().contains("UTF-8")
                    {
                        for _ in 0..explicit_len {
                            let lead = self.read_binary_bits(8)? as u8;
                            try_push(&mut bytes, lead)?;
                            let extra = if lead & 0x80 == 0 {
                                0
                            } else if lead & 0xE0 == 0xC0 {
                                1
                            } else if lead & 0xF0 == 0xE0 {
                                2
                            } else if lead & 0xF8 == 0xF0 {
                                3
                            } else {
                                0
                            };
                            for _ in 0..extra {
                                let b = self.read_binary_bits(8)? as u8;
                                try_push(&mut bytes, b)?;
                            }
                        }
                    } else if props.length_units == crate::schema::ir::LengthUnits::Bits {
                        let mut bits_left = explicit_len;
                        while bits_left > 0 {
                            let chunk = bits_left.min(8);
                            let val = self.read_binary_bits(chunk)? as u8;
                            let byte_val = if chunk < 8
                                && props.bit_order == BitOrder::MostSignificantBitFirst
                            {
                                val << (8 - chunk)
                            } else {
                                val
                            };
                            try_push(&mut bytes, byte_val)?;
                            bits_left = bits_left.saturating_sub(chunk);
                        }
                    } else {
                        for _ in 0..explicit_len {
                            let b = self.read_binary_bits(8)? as u8;
                            try_push(&mut bytes, b)?;
                        }
                    }
                    let s = crate::encoding::decode_text_bytes(&bytes, &props.encoding)?;
                    Ok(DfdlValue::String(s))
                }
            }
            DfdlSimpleType::Decimal => {
                let bits = calc_bits(32);
                let val_u64 = self.read_binary_bits(bits)?;
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => val_u64,
                    ByteOrder::LittleEndian => match bits {
                        16 => (val_u64 as u16).swap_bytes() as u64,
                        24 => {
                            let b0 = val_u64 & 0xFF;
                            let b1 = (val_u64 >> 8) & 0xFF;
                            let b2 = (val_u64 >> 16) & 0xFF;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (val_u64 as u32).swap_bytes() as u64,
                        64 => val_u64.swap_bytes(),
                        _ => val_u64,
                    },
                };
                let val_i64 = if props.decimal_signed {
                    sign_extend(ordered, bits)
                } else {
                    ordered as i64
                };
                let formatted =
                    Self::format_virtual_decimal(val_i64, props.binary_decimal_virtual_point);
                Ok(DfdlValue::Decimal(formatted))
            }
            DfdlSimpleType::DateTime | DfdlSimpleType::Date | DfdlSimpleType::Time => {
                let mut bytes = Vec::new();
                if let Some(l) = dynamic_len {
                    let len_bytes = match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => l.saturating_add(7) / 8,
                        _ => l,
                    };
                    for _ in 0..len_bytes {
                        let b = self.read_binary_bits(8)? as u8;
                        try_push(&mut bytes, b)?;
                    }
                } else {
                    while !self.reader.is_eof() {
                        let mut matched_in_scope = false;
                        if let Some(ref term) = props.terminator {
                            if !term.is_empty() && self.peek_literal_delimiter(term) {
                                matched_in_scope = true;
                            }
                        }
                        if !matched_in_scope {
                            for i in (0..self.in_scope_delimiters.len()).rev() {
                                if let Ok(delim) =
                                    crate::util::get_checked(&self.in_scope_delimiters, i)
                                {
                                    let delim_clone = delim.clone();
                                    if !delim_clone.is_empty()
                                        && self.peek_literal_delimiter(&delim_clone)
                                    {
                                        matched_in_scope = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if matched_in_scope {
                            break;
                        }
                        if let Ok(b) = self.read_binary_bits(8) {
                            try_push(&mut bytes, b as u8)?;
                        } else {
                            break;
                        }
                    }
                }
                let s = match props.binary_calendar_rep {
                    crate::schema::ir::BinaryCalendarRep::Packed => {
                        let mut digits = String::new();
                        for (i, &b) in bytes.iter().enumerate() {
                            let high = (b >> 4) & 0x0F;
                            let low = b & 0x0F;
                            if i + 1 == bytes.len() {
                                if low == 0x0D || low == 0x0B {
                                    let type_name = match simple_type {
                                        DfdlSimpleType::DateTime => "xs:dateTime",
                                        DfdlSimpleType::Date => "xs:date",
                                        DfdlSimpleType::Time => "xs:time",
                                        _ => "calendar",
                                    };
                                    let msg = alloc::format!(
                                        "Parse Error: Unable to parse {} from negative packed number",
                                        type_name
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                                }
                                if high <= 9 {
                                    digits.push(
                                        core::char::from_digit(high as u32, 10).unwrap_or('0'),
                                    );
                                }
                            } else {
                                if high <= 9 {
                                    digits.push(
                                        core::char::from_digit(high as u32, 10).unwrap_or('0'),
                                    );
                                }
                                if low <= 9 {
                                    digits.push(
                                        core::char::from_digit(low as u32, 10).unwrap_or('0'),
                                    );
                                }
                            }
                        }
                        Self::parse_calendar_from_digits(
                            &digits,
                            props.calendar_pattern.as_deref(),
                            simple_type,
                        )?
                    }
                    crate::schema::ir::BinaryCalendarRep::Bcd
                    | crate::schema::ir::BinaryCalendarRep::Ibm4690Packed => {
                        let mut digits = String::new();
                        for &b in &bytes {
                            let high = (b >> 4) & 0x0F;
                            let low = b & 0x0F;
                            if high > 9 {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Parse Error: Invalid high nibble",
                                ));
                            }
                            if low > 9 {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Parse Error: Invalid low nibble",
                                ));
                            }
                            digits.push(core::char::from_digit(high as u32, 10).unwrap_or('0'));
                            digits.push(core::char::from_digit(low as u32, 10).unwrap_or('0'));
                        }
                        Self::parse_calendar_from_digits(
                            &digits,
                            props.calendar_pattern.as_deref(),
                            simple_type,
                        )?
                    }
                    crate::schema::ir::BinaryCalendarRep::BinarySeconds
                    | crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds => {
                        Self::parse_binary_seconds_or_millis(&bytes, props, simple_type)?
                    }
                };
                match simple_type {
                    DfdlSimpleType::DateTime => Ok(DfdlValue::DateTime(s)),
                    DfdlSimpleType::Date => Ok(DfdlValue::Date(s)),
                    _ => Ok(DfdlValue::Time(s)),
                }
            }
        }
    }

}
