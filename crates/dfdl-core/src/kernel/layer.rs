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

/// Unfolds input bytes according to RFC 2822 / RFC 5322 section 2.2.3.
///
/// In the Internet Message Format (IMF), header fields that exceed line length
/// limits are folded by inserting CRLF before whitespace (SP or HTAB).
/// Unfolding reverses this process by removing any line break sequence (`\r\n`,
/// bare `\n`, or bare `\r`) that is immediately followed by a whitespace character
/// (`b' '` or `b'\t'`). The following whitespace character is preserved.
///
/// # Arguments
///
/// * `input` - Slice of raw bytes possibly containing folded lines.
///
/// # Returns
///
/// A tuple containing:
/// 1. The unfolded byte vector.
/// 2. An `orig_offsets` vector mapping each index `i` in the unfolded vector
///    to its starting byte offset in `input`. The vector has length `unfolded.len() + 1`,
///    with `orig_offsets[unfolded.len()]` containing `input.len()`.
#[must_use]
pub fn unfold_imf(input: &[u8]) -> (alloc::vec::Vec<u8>, alloc::vec::Vec<usize>) {
    let mut unfolded = alloc::vec::Vec::with_capacity(input.len());
    let mut orig_offsets = alloc::vec::Vec::with_capacity(input.len().saturating_add(1));
    let mut i = 0;
    while i < input.len() {
        let b = match input.get(i).copied() {
            Some(byte) => byte,
            None => break,
        };
        if b == b'\r' {
            if input.get(i.saturating_add(1)).copied() == Some(b'\n') {
                // CRLF
                if matches!(input.get(i.saturating_add(2)).copied(), Some(b' ' | b'\t')) {
                    // CRLF followed by WSP: skip CRLF
                    i = i.saturating_add(2);
                    continue;
                }
            } else if matches!(input.get(i.saturating_add(1)).copied(), Some(b' ' | b'\t')) {
                // Bare CR followed by WSP: skip CR
                i = i.saturating_add(1);
                continue;
            }
        } else if b == b'\n' && matches!(input.get(i.saturating_add(1)).copied(), Some(b' ' | b'\t')) {
            // Bare LF followed by WSP: skip LF
            i = i.saturating_add(1);
            continue;
        }
        orig_offsets.push(i);
        unfolded.push(b);
        i = i.saturating_add(1);
    }
    orig_offsets.push(input.len());
    (unfolded, orig_offsets)
}

/// Decodes a MIME Base64-encoded byte slice according to RFC 2045 section 6.8.
///
/// RFC 2045 specifies that any characters outside the Base64 alphabet (specifically
/// whitespace such as `\r`, `\n`, ` `, and `\t`) must be completely ignored.
/// The standard Base64 alphabet consists of `A-Z`, `a-z`, `0-9`, `+`, `/`, with `=`
/// used for terminal padding.
///
/// # Arguments
///
/// * `input` - Raw ASCII byte slice containing Base64 data and possible MIME whitespace.
///
/// # Returns
///
/// The decoded binary bytes in an allocated vector.
#[must_use]
pub fn decode_base64_mime(input: &[u8]) -> alloc::vec::Vec<u8> {
    let mut decoded = alloc::vec::Vec::with_capacity(input.len());
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &b in input {
        let val = match b {
            b'A'..=b'Z' => (b.saturating_sub(b'A')) as u32,
            b'a'..=b'z' => (b.saturating_sub(b'a').saturating_add(26)) as u32,
            b'0'..=b'9' => (b.saturating_sub(b'0').saturating_add(52)) as u32,
            b'+' => 62,
            b'/' => 63,
            b'=' => continue,
            _ => continue,
        };

        buf = (buf << 6) | val;
        bits = bits.saturating_add(6);

        if bits >= 8 {
            bits = bits.saturating_sub(8);
            let byte = ((buf >> bits) & 0xff) as u8;
            decoded.push(byte);
        }
    }

    decoded
}

/// Encodes raw bytes into MIME Base64 according to RFC 2045 section 6.8.
///
/// Encodes binary data into standard Base64 characters (`A-Z`, `a-z`, `0-9`, `+`, `/`),
/// with `=` padding as required. Every 76 characters, a CRLF line break (`\r\n`)
/// is inserted per RFC 2045 §6.8 line wrapping rules.
///
/// # Arguments
///
/// * `input` - Raw binary byte slice to encode.
///
/// # Returns
///
/// Base64 ASCII byte vector.
#[must_use]
pub fn encode_base64_mime(input: &[u8]) -> alloc::vec::Vec<u8> {
    const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = alloc::vec::Vec::with_capacity((input.len().saturating_mul(4) / 3).saturating_add(64));
    let mut col: usize = 0;

    for chunk in input.chunks(3) {
        if col >= 76 {
            out.push(b'\r');
            out.push(b'\n');
            col = 0;
        }
        match chunk.len() {
            3 => {
                if let (Some(&c0), Some(&c1), Some(&c2)) = (chunk.first(), chunk.get(1), chunk.get(2)) {
                    let n = ((c0 as u32) << 16) | ((c1 as u32) << 8) | (c2 as u32);
                    if let (Some(&b0), Some(&b1), Some(&b2), Some(&b3)) = (
                        B64_CHARS.get(((n >> 18) & 0x3f) as usize),
                        B64_CHARS.get(((n >> 12) & 0x3f) as usize),
                        B64_CHARS.get(((n >> 6) & 0x3f) as usize),
                        B64_CHARS.get((n & 0x3f) as usize),
                    ) {
                        out.push(b0);
                        out.push(b1);
                        out.push(b2);
                        out.push(b3);
                        col = col.saturating_add(4);
                    }
                }
            }
            2 => {
                if let (Some(&c0), Some(&c1)) = (chunk.first(), chunk.get(1)) {
                    let n = ((c0 as u32) << 16) | ((c1 as u32) << 8);
                    if let (Some(&b0), Some(&b1), Some(&b2)) = (
                        B64_CHARS.get(((n >> 18) & 0x3f) as usize),
                        B64_CHARS.get(((n >> 12) & 0x3f) as usize),
                        B64_CHARS.get(((n >> 6) & 0x3f) as usize),
                    ) {
                        out.push(b0);
                        out.push(b1);
                        out.push(b2);
                        out.push(b'=');
                        col = col.saturating_add(4);
                    }
                }
            }
            1 => {
                if let Some(&c0) = chunk.first() {
                    let n = (c0 as u32) << 16;
                    if let (Some(&b0), Some(&b1)) = (
                        B64_CHARS.get(((n >> 18) & 0x3f) as usize),
                        B64_CHARS.get(((n >> 12) & 0x3f) as usize),
                    ) {
                        out.push(b0);
                        out.push(b1);
                        out.push(b'=');
                        out.push(b'=');
                        col = col.saturating_add(4);
                    }
                }
            }
            _ => {}
        }
    }

    out
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

    #[test]
    fn test_unfold_imf_rfc2822() {
        // Test basic unfolding with CRLF + space and CRLF + tab.
        let folded = b"Subject: This is a very long\r\n subject line that was\r\n\tfolded.\r\nNext: Header\r\n";
        let (unfolded, offsets) = unfold_imf(folded);
        let expected = b"Subject: This is a very long subject line that was\tfolded.\r\nNext: Header\r\n";
        assert_eq!(unfolded.as_slice(), expected.as_slice());
        assert_eq!(offsets.len(), unfolded.len() + 1);
        assert_eq!(offsets.last().copied(), Some(folded.len()));
    }

    #[test]
    fn test_base64_mime_roundtrip() {
        // Test RFC 2045 Base64 decoding and encoding.
        let plain = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit.";
        let encoded = encode_base64_mime(plain);
        let decoded = decode_base64_mime(&encoded);
        assert_eq!(decoded.as_slice(), plain.as_slice());

        // Test with embedded MIME CRLF line breaks and whitespace.
        let b64_with_ws = b"TG9yZW0gaXBzdW0g\r\nZG9sb3Igc2l0IGFt\r\nZXQsIGNvbnNlY3Rl\r\ndHVyIGFkaXBpc2Np\r\nbmcgZWxpdC4=";
        let decoded_ws = decode_base64_mime(b64_with_ws);
        assert_eq!(decoded_ws.as_slice(), plain.as_slice());
    }
}
