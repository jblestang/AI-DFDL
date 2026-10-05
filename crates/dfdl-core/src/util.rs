//! Panic-free utilities: checked arithmetic, checked slicing, and fallible collection operations.
//!
//! Enforces Section B panic-free contract provisions.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};

/// Safely attempts to index a slice without panicking.
///
/// # Examples
/// ```rust
/// use dfdl_core::util::get_checked;
/// let data = [10, 20, 30];
/// assert_eq!(get_checked(&data, 1), Ok(&20));
/// assert!(get_checked(&data, 5).is_err());
/// ```
#[inline]
pub fn get_checked<T>(slice: &[T], index: usize) -> DFDLResult<&T> {
    slice.get(index).ok_or_else(|| {
        DFDLError::new(
            DFDLErrorKind::Parse,
            "Index out of bounds in safe slice lookup",
        )
    })
}

/// Safely attempts to slice a range without panicking.
///
/// # Examples
/// ```rust
/// use dfdl_core::util::get_slice_checked;
/// let data = [1, 2, 3, 4, 5];
/// assert_eq!(get_slice_checked(&data, 1, 3), Ok(&[2, 3, 4][..]));
/// assert!(get_slice_checked(&data, 3, 10).is_err());
/// ```
#[inline]
pub fn get_slice_checked<T>(slice: &[T], start: usize, len: usize) -> DFDLResult<&[T]> {
    let end = start.checked_add(len).ok_or_else(|| {
        DFDLError::new(
            DFDLErrorKind::ImplementationLimit,
            "Integer overflow calculating slice range",
        )
    })?;

    if end <= slice.len() {
        match slice.get(start..end) {
            Some(sub) => Ok(sub),
            None => Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Slice range out of bounds",
            )),
        }
    } else {
        Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Requested slice range exceeds buffer length",
        ))
    }
}

/// Pushes an item into a `Vec` using fallible `try_reserve`.
///
/// Returns `DFDLErrorKind::ImplementationLimit` on allocation failure.
#[inline]
pub fn try_push<T>(vec: &mut Vec<T>, item: T) -> DFDLResult<()> {
    vec.try_reserve(1).map_err(|_| {
        DFDLError::new(
            DFDLErrorKind::ImplementationLimit,
            "Allocation failure pushing element into vector",
        )
    })?;
    vec.push(item);
    Ok(())
}

/// Extends a `Vec` from a slice using fallible `try_reserve`.
#[inline]
pub fn try_extend_from_slice<T: Clone>(vec: &mut Vec<T>, items: &[T]) -> DFDLResult<()> {
    vec.try_reserve(items.len()).map_err(|_| {
        DFDLError::new(
            DFDLErrorKind::ImplementationLimit,
            "Allocation failure extending vector from slice",
        )
    })?;
    vec.extend_from_slice(items);
    Ok(())
}

/// Checked integer addition helper returning a typed `DFDLError` on overflow.
#[inline]
pub const fn checked_add_usize(a: usize, b: usize) -> DFDLResult<usize> {
    match a.checked_add(b) {
        Some(val) => Ok(val),
        None => Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Integer overflow in addition",
        )),
    }
}

/// Checked integer multiplication helper returning a typed `DFDLError` on overflow.
#[inline]
pub const fn checked_mul_usize(a: usize, b: usize) -> DFDLResult<usize> {
    match a.checked_mul(b) {
        Some(val) => Ok(val),
        None => Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Integer overflow in multiplication",
        )),
    }
}

/// Checked integer division helper protecting against zero division and overflow.
#[inline]
pub const fn checked_div_usize(a: usize, b: usize) -> DFDLResult<usize> {
    if b == 0 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Division by zero in integer arithmetic",
        ));
    }
    match a.checked_div(b) {
        Some(val) => Ok(val),
        None => Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Integer overflow in division",
        )),
    }
}

/// Decodes IBM Comp-3 Packed Decimal bytes into an integer value according to DFDL §13.7.1.1.
/// If `sign_codes_opt` is provided, it must be four whitespace-separated hex characters:
/// positive, negative, unsigned, zero.
pub fn decode_packed_decimal_with_signs(
    bytes: &[u8],
    sign_codes_opt: Option<&str>,
) -> DFDLResult<i64> {
    if bytes.is_empty() {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Cannot decode empty packed decimal bytes",
        ));
    }

    let mut result: i64 = 0;
    let len = bytes.len();

    let (pos_code, neg_code, unsigned_code, zero_code) = if let Some(sc) = sign_codes_opt {
        let parts: Vec<&str> = sc.split_whitespace().collect();
        if let (Some(&p_str), Some(&n_str), Some(&u_str), Some(&z_str)) = (
            parts.first(),
            parts.get(1),
            parts.get(2),
            parts.get(3),
        ) {
            let p = u8::from_str_radix(p_str, 16).unwrap_or(0x0C);
            let n = u8::from_str_radix(n_str, 16).unwrap_or(0x0D);
            let u = u8::from_str_radix(u_str, 16).unwrap_or(0x0F);
            let z = u8::from_str_radix(z_str, 16).unwrap_or(0x0C);
            (Some(p), Some(n), Some(u), Some(z))
        } else {
            (None, None, None, None)
        }
    } else {
        (None, None, None, None)
    };

    for (idx, &byte) in bytes.iter().enumerate() {
        let high_nibble = (byte >> 4) & 0x0F;
        let low_nibble = byte & 0x0F;

        if idx.saturating_add(1) < len {
            if high_nibble > 9 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Invalid high nibble in packed decimal",
                ));
            }
            if low_nibble > 9 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Invalid low nibble in packed decimal",
                ));
            }
            result = result
                .checked_mul(10)
                .and_then(|r| r.checked_add(high_nibble as i64))
                .and_then(|r| r.checked_mul(10))
                .and_then(|r| r.checked_add(low_nibble as i64))
                .ok_or_else(|| DFDLError::arithmetic_overflow("Packed decimal integer overflow"))?;
        } else {
            if high_nibble > 9 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Invalid high nibble in packed decimal sign byte",
                ));
            }
            result = result
                .checked_mul(10)
                .and_then(|r| r.checked_add(high_nibble as i64))
                .ok_or_else(|| DFDLError::arithmetic_overflow("Packed decimal integer overflow"))?;

            let is_negative = if let (Some(p), Some(n), Some(u), Some(z)) =
                (pos_code, neg_code, unsigned_code, zero_code)
            {
                if low_nibble == n {
                    true
                } else if low_nibble == p || low_nibble == u || low_nibble == z {
                    false
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Parse Error: Invalid sign nibble in packed decimal",
                    ));
                }
            } else {
                match low_nibble {
                    0x0D | 0x0B => true,
                    0x0C | 0x0A | 0x0E | 0x0F => false,
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Invalid sign nibble in packed decimal",
                        ));
                    }
                }
            };
            if is_negative {
                result = result.wrapping_neg();
            }
        }
    }

    Ok(result)
}

/// Decodes IBM Comp-3 Packed Decimal bytes into an integer value according to DFDL §13.7.
pub fn decode_packed_decimal(bytes: &[u8]) -> DFDLResult<i64> {
    decode_packed_decimal_with_signs(bytes, None)
}

/// Encodes an unsigned integer into BCD (Binary Coded Decimal) bytes according to DFDL §13.7.1.4.
pub fn encode_bcd(value: u64, min_bytes: Option<usize>) -> DFDLResult<Vec<u8>> {
    let mut digits_str = alloc::format!("{}", value);
    if digits_str.len() % 2 != 0 {
        digits_str.insert(0, '0');
    }
    let mut bytes = Vec::new();
    let chars: Vec<char> = digits_str.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let d1 = chars
            .get(i)
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as u8;
        let d2 = chars
            .get(i.saturating_add(1))
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as u8;
        bytes.push((d1 << 4) | d2);
        i = i.saturating_add(2);
    }
    if let Some(target_len) = min_bytes {
        if bytes.len() > target_len {
            return Err(DFDLError::new(
                DFDLErrorKind::Unparse,
                &alloc::format!(
                    "Unparse Error: Value {} ({} bytes in BCD) exceeds specified length of {} bytes",
                    value,
                    bytes.len(),
                    target_len
                ),
            ));
        }
        if bytes.len() < target_len {
            let pad = target_len.saturating_sub(bytes.len());
            let mut padded = alloc::vec![0u8; pad];
            padded.extend_from_slice(&bytes);
            return Ok(padded);
        }
    }
    Ok(bytes)
}

/// Encodes an integer value into IBM Comp-3 Packed Decimal bytes according to DFDL §13.7.
pub fn encode_packed_decimal(value: i64) -> Vec<u8> {
    let is_neg = value < 0;
    let abs_val = value.unsigned_abs();
    let mut digits_str = alloc::format!("{}", abs_val);

    if digits_str.len() % 2 == 0 {
        digits_str.insert(0, '0');
    }

    let mut bytes = Vec::new();
    let chars: Vec<char> = digits_str.chars().collect();
    let num_digits = chars.len();

    let mut i = 0usize;
    while i < num_digits.saturating_sub(1) {
        let d1 = chars.get(i).and_then(|c| c.to_digit(10)).unwrap_or(0) as u8;
        let d2 = chars
            .get(i.saturating_add(1))
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as u8;
        bytes.push((d1 << 4) | d2);
        i = i.saturating_add(2);
    }

    let last_digit = chars.last().and_then(|c| c.to_digit(10)).unwrap_or(0) as u8;
    let sign_nibble: u8 = if is_neg { 0x0D } else { 0x0C };
    bytes.push((last_digit << 4) | sign_nibble);

    bytes
}

/// Decodes an integer or decimal from IBM 4690 Packed Decimal format according to DFDL §13.7.1.3.
pub fn decode_ibm4690_packed(bytes: &[u8]) -> DFDLResult<i64> {
    if bytes.is_empty() {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Insufficient binary data for IBM 4690 packed decimal",
        ));
    }

    let mut is_negative = false;
    let mut result = 0i64;
    let mut saw_digits = false;

    for &byte in bytes {
        let nibbles = [(byte >> 4) & 0x0F, byte & 0x0F];
        for &nibble in &nibbles {
            if nibble == 0x0F {
                // Leading 0xF (or 0xF before digits) is padding
                if saw_digits {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Unexpected 0xF pad nibble after digits in IBM 4690 packed decimal",
                    ));
                }
            } else if nibble == 0x0D {
                // Sign nibble
                if saw_digits || is_negative {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Unexpected 0xD sign nibble in IBM 4690 packed decimal",
                    ));
                }
                is_negative = true;
            } else if nibble <= 9 {
                saw_digits = true;
                result = result
                    .checked_mul(10)
                    .and_then(|r| r.checked_add(nibble as i64))
                    .ok_or_else(|| DFDLError::arithmetic_overflow("IBM 4690 packed decimal overflow"))?;
            } else {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Invalid nibble in IBM 4690 packed decimal",
                ));
            }
        }
    }

    if is_negative {
        result = result.wrapping_neg();
    }

    Ok(result)
}

/// Encodes an integer value into IBM 4690 Packed Decimal format according to DFDL §13.7.1.3.
pub fn encode_ibm4690_packed(value: i64) -> Vec<u8> {
    use alloc::string::String;
    let is_neg = value < 0;
    let abs_val = value.unsigned_abs();
    let digits_str = alloc::format!("{}", abs_val);

    let mut nibble_str = String::new();
    if is_neg {
        nibble_str.push('D');
        nibble_str.push_str(&digits_str);
        if !nibble_str.len().is_multiple_of(2) {
            nibble_str.insert(0, 'F');
        }
    } else {
        nibble_str.push_str(&digits_str);
        if !nibble_str.len().is_multiple_of(2) {
            nibble_str.insert(0, 'F');
        }
    }

    let mut bytes = Vec::new();
    let chars: Vec<char> = nibble_str.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let n1 = chars.get(i).and_then(|c| c.to_digit(16)).unwrap_or(0) as u8;
        let n2 = chars
            .get(i.saturating_add(1))
            .and_then(|c| c.to_digit(16))
            .unwrap_or(0) as u8;
        bytes.push((n1 << 4) | n2);
        i = i.saturating_add(2);
    }
    bytes
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_checked_indexing_and_slicing() {
        let arr = [10, 20, 30, 40, 50];
        assert_eq!(get_checked(&arr, 2), Ok(&30));
        assert!(get_checked(&arr, 10).is_err());

        assert_eq!(get_slice_checked(&arr, 1, 3), Ok(&[20, 30, 40][..]));
        assert!(get_slice_checked(&arr, 3, 5).is_err());
    }

    #[test]
    fn test_fallible_vec_push() {
        let mut v = Vec::new();
        try_push(&mut v, 100).unwrap();
        try_push(&mut v, 200).unwrap();
        assert_eq!(v, [100, 200]);
    }

    #[test]
    fn test_checked_arithmetic_helpers() {
        assert_eq!(checked_add_usize(10, 20), Ok(30));
        assert!(checked_add_usize(usize::MAX, 1).is_err());

        assert_eq!(checked_div_usize(20, 4), Ok(5));
        assert!(checked_div_usize(20, 0).is_err());
    }

    #[test]
    fn test_packed_decimal_codecs_roundtrip() {
        // Positive packed decimal: +12345 -> [0x12, 0x34, 0x5C]
        let pos_encoded = encode_packed_decimal(12345);
        assert_eq!(pos_encoded, vec![0x12, 0x34, 0x5C]);
        let pos_decoded = decode_packed_decimal(&pos_encoded).unwrap();
        assert_eq!(pos_decoded, 12345);

        // Negative packed decimal: -12345 -> [0x12, 0x34, 0x5D]
        let neg_encoded = encode_packed_decimal(-12345);
        assert_eq!(neg_encoded, vec![0x12, 0x34, 0x5D]);
        let neg_decoded = decode_packed_decimal(&neg_encoded).unwrap();
        assert_eq!(neg_decoded, -12345);

        // Zero packed decimal: 0 -> [0x0C]
        let zero_encoded = encode_packed_decimal(0);
        assert_eq!(zero_encoded, vec![0x0C]);
        assert_eq!(decode_packed_decimal(&zero_encoded).unwrap(), 0);

        // Even number of digits (e.g. 1234): padded with leading zero -> [0x01, 0x23, 0x4C]
        let even_encoded = encode_packed_decimal(1234);
        assert_eq!(even_encoded, vec![0x01, 0x23, 0x4C]);
        assert_eq!(decode_packed_decimal(&even_encoded).unwrap(), 1234);

        // Unsigned alternative sign nibble 0x0F
        assert_eq!(decode_packed_decimal(&[0x12, 0x34, 0x5F]).unwrap(), 12345);

        // Negative alternative sign nibble 0x0B
        assert_eq!(decode_packed_decimal(&[0x12, 0x34, 0x5B]).unwrap(), -12345);

        // Invalid BCD digit in data byte (0x1A) returns error
        assert!(decode_packed_decimal(&[0x1A, 0x34, 0x5C]).is_err());
    }

    #[test]
    fn test_ibm4690_packed_codecs_roundtrip() {
        // Negative IBM 4690 packed decimal: D123 -> -123
        let neg_bytes = [0xD1, 0x23];
        assert_eq!(decode_ibm4690_packed(&neg_bytes).unwrap(), -123);

        // Positive even-digit: 1234 -> 1234
        let pos_even = [0x12, 0x34];
        assert_eq!(decode_ibm4690_packed(&pos_even).unwrap(), 1234);

        // Positive odd-digit with 0xF padding: F123 -> 123
        let pos_odd = [0xF1, 0x23];
        assert_eq!(decode_ibm4690_packed(&pos_odd).unwrap(), 123);

        // Roundtrip encoding
        let enc_neg = encode_ibm4690_packed(-123);
        assert_eq!(decode_ibm4690_packed(&enc_neg).unwrap(), -123);
        let enc_pos = encode_ibm4690_packed(1234);
        assert_eq!(decode_ibm4690_packed(&enc_pos).unwrap(), 1234);
    }

    #[test]
    fn test_encode_bcd() {
        // Single digit (odd count) -> prepends leading 0 -> [0x03]
        assert_eq!(encode_bcd(3, None).unwrap(), vec![0x03]);
        // 3 digits (odd count) -> prepends leading 0 -> [0x01, 0x23]
        assert_eq!(encode_bcd(123, None).unwrap(), vec![0x01, 0x23]);
        // 2 digits (even count) -> [0x12]
        assert_eq!(encode_bcd(12, None).unwrap(), vec![0x12]);
        // With min_bytes padding: 11 with min_bytes=4 -> [0x00, 0x00, 0x00, 0x11]
        assert_eq!(
            encode_bcd(11, Some(4)).unwrap(),
            vec![0x00, 0x00, 0x00, 0x11]
        );
        // Exceeding min_bytes returns error
        assert!(encode_bcd(12345, Some(2)).is_err());
    }

    #[test]
    fn test_decode_packed_decimal_with_custom_signs() {
        // With explicit sign codes "C D F C":
        // 0x12, 0x3C is positive 123
        assert_eq!(
            decode_packed_decimal_with_signs(&[0x12, 0x3C], Some("C D F C")).unwrap(),
            123
        );
        // 0x12, 0x3D is negative -123
        assert_eq!(
            decode_packed_decimal_with_signs(&[0x12, 0x3D], Some("C D F C")).unwrap(),
            -123
        );
        // 0x12, 0x3B with "C D F C" should fail because 'B' is not an allowed sign code
        assert!(decode_packed_decimal_with_signs(&[0x12, 0x3B], Some("C D F C")).is_err());
    }
}
