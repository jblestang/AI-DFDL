//! DFDL Layer Transform Implementations.
//!
//! Provides algorithmic transforms for Daffodil layer extensions (§dfdlx:layer),
//! including RFC 791 IPv4 header checksum computation and check digit verification.
//!
//! Layers wrap physical sequences to perform stream transformations, checksum
//! injection/validation, or stream filtering.

extern crate alloc;

/// Computes the RFC 791 IPv4 header checksum over a 20-byte slice.
///
/// Under RFC 791 §3.1, the checksum field is the 16-bit one's complement of
/// the one's complement sum of all 16-bit words in the header. For purposes
/// of computing the checksum, the checksum field (bytes 10-11) is treated as zero.
///
/// # Arguments
///
/// * `bytes` - Header byte slice of at least 20 bytes.
///
/// # Returns
///
/// The calculated 16-bit big-endian checksum value.
#[must_use]
pub fn compute_ipv4_checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    // Iterate over ten 16-bit words forming the 20-byte IPv4 header.
    for i in 0..10usize {
        let w = if i == 5 {
            // Checksum field itself at bytes 10-11 is treated as zero during calculation.
            0u16
        } else {
            let idx0 = i.saturating_mul(2);
            let idx1 = idx0.saturating_add(1);
            let b0 = bytes.get(idx0).copied().unwrap_or(0);
            let b1 = bytes.get(idx1).copied().unwrap_or(0);
            u16::from_be_bytes([b0, b1])
        };
        sum = sum.saturating_add(w as u32);
    }
    // Fold 32-bit sum into 16-bit one's complement sum by adding carry bits.
    while (sum >> 16) > 0 {
        sum = (sum & 0xffff).saturating_add(sum >> 16);
    }
    // Return one's complement.
    !(sum as u16)
}

/// Computes the check digit over an ASCII data slice.
///
/// The check digit is the least-significant digit (modulo 10) of the sum
/// of all ASCII digit characters ('0'..='9') present in the input slice.
///
/// # Arguments
///
/// * `bytes` - Slice of raw bytes to inspect for ASCII digits.
///
/// # Returns
///
/// Calculated check digit in range `0..=9`.
#[must_use]
pub fn compute_check_digit(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for &b in bytes {
        if b.is_ascii_digit() {
            sum = sum.saturating_add(b.saturating_sub(b'0') as u32);
        }
    }
    (sum % 10) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_checksum_rfc791() {
        // Sample IPv4 header bytes from RFC 791 / Wikipedia example:
        // Version 4, IHL 5, DSCP/ECN 0, Total Length 115, Ident 0, Flags 2, Offset 0, TTL 64, Proto 17
        let header = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00,
            0x40, 0x11, 0x00, 0x00, 0xc0, 0xa8, 0x00, 0x01,
            0xc0, 0xa8, 0x00, 0xc7,
        ];
        let chk = compute_ipv4_checksum(&header);
        assert_eq!(chk, 0xb861);
        assert_eq!(chk, 47201);
    }

    #[test]
    fn test_check_digit_calculation() {
        // "2021-09-25" -> digits 2+0+2+1+0+9+2+5 = 21 -> 21 % 10 = 1
        let text = b"2021-09-25";
        let cd = compute_check_digit(text);
        assert_eq!(cd, 1);
    }
}
