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

/// Swaps adjacent byte pairs (16-bit word byte swap) in place.
///
/// Used by the `twoByteSwap` layer extension to reverse endianness of 16-bit words.
///
/// # Arguments
///
/// * `bytes` - Slice of bytes to mutate. Only full 2-byte pairs are swapped.
pub fn swap_two_bytes(bytes: &mut [u8]) {
    let mut i = 0usize;
    while i.saturating_add(1) < bytes.len() {
        bytes.swap(i, i.saturating_add(1));
        i = i.saturating_add(2);
    }
}

/// Computes the standard IEEE 802.3 32-bit Cyclic Redundancy Check (CRC-32).
///
/// Implements the canonical CRC-32 algorithm using the reversed polynomial `0xEDB88320`.
/// This matches the CRC-32 algorithm required by RFC 1952 section 2.3.1 for GZIP headers.
///
/// # Arguments
///
/// * `bytes` - Slice of raw bytes to checksum.
///
/// # Returns
///
/// Calculated 32-bit CRC-32 checksum.
#[must_use]
pub fn compute_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Compresses raw bytes into an RFC 1952 GZIP stream.
///
/// Constructs a standard GZIP member containing:
/// 1. A 10-byte header:
///    - ID1 = 0x1F, ID2 = 0x8B (GZIP magic)
///    - CM = 0x08 (DEFLATE compression method)
///    - FLG = 0x00 (no optional headers)
///    - MTIME = 0x00000000 (deterministic timestamp)
///    - XFL = 0x00 (extra flags)
///    - OS = 0xFF (unknown / neutral operating system)
/// 2. Raw DEFLATE payload compressed at the requested compression level (0..=9).
/// 3. An 8-byte footer:
///    - CRC-32 of uncompressed bytes (4 bytes, little-endian)
///    - ISIZE: uncompressed input size modulo 2^32 (4 bytes, little-endian)
///
/// # Arguments
///
/// * `bytes` - Raw uncompressed byte slice.
/// * `level` - Compression level (0..=9), where 6 is standard and 9 is maximum.
///
/// # Returns
///
/// A GZIP-formatted byte vector.
#[must_use]
pub fn gzip_compress(bytes: &[u8], level: u8) -> alloc::vec::Vec<u8> {
    let clamped_level = level.min(9);
    let raw_deflate = miniz_oxide::deflate::compress_to_vec(bytes, clamped_level);
    let mut out = alloc::vec::Vec::with_capacity(raw_deflate.len().saturating_add(18));
    // Fixed 10-byte RFC 1952 header (OS=255 unknown, XFL=0)
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff]);
    out.extend_from_slice(&raw_deflate);
    let crc = compute_crc32(bytes);
    out.extend_from_slice(&crc.to_le_bytes());
    let isize = (bytes.len() as u32).to_le_bytes();
    out.extend_from_slice(&isize);
    out
}

/// Decompresses an RFC 1952 GZIP stream into raw uncompressed bytes.
///
/// Validates the 10-byte GZIP header, extracts and skips any optional headers
/// specified by the FLG field (FEXTRA, FNAME, FCOMMENT, FHCRC), streams the
/// compressed DEFLATE payload through miniz_oxide, and verifies the trailing
/// 8-byte footer containing the IEEE 802.3 CRC-32 checksum and uncompressed ISIZE.
///
/// # Arguments
///
/// * `input` - Slice of raw bytes containing a GZIP stream.
///
/// # Returns
///
/// On success, returns `Ok((decompressed_bytes, consumed_input_bytes))` where
/// `consumed_input_bytes` is the exact byte length of the GZIP member consumed
/// from `input` (including header, DEFLATE payload, and footer).
/// On failure (magic mismatch, corrupted stream, CRC error), returns a `DFDLError`.
pub fn gzip_decompress(input: &[u8]) -> crate::error::DFDLResult<(alloc::vec::Vec<u8>, usize)> {
    use crate::error::{DFDLError, DFDLErrorKind};

    if input.len() < 18 {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: Insufficient data for GZIP stream (minimum 18 bytes required)",
        ));
    }
    if input.first().copied() != Some(0x1f)
        || input.get(1).copied() != Some(0x8b)
        || input.get(2).copied() != Some(0x08)
    {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: GZIP header magic mismatch (expected 0x1F, 0x8B, 0x08)",
        ));
    }
    let flg = input.get(3).copied().unwrap_or(0);
    let mut offset = 10usize;

    // FEXTRA: 2-byte length prefix followed by extra bytes
    if flg & 0x04 != 0 {
        if input.len() < offset.saturating_add(2) {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Parse Error: Truncated GZIP FEXTRA header",
            ));
        }
        let b0 = input.get(offset).copied().unwrap_or(0);
        let b1 = input.get(offset.saturating_add(1)).copied().unwrap_or(0);
        let xlen = u16::from_le_bytes([b0, b1]) as usize;
        offset = offset.saturating_add(2).saturating_add(xlen);
    }
    // FNAME: zero-terminated string
    if flg & 0x08 != 0 {
        while offset < input.len() && input.get(offset).copied() != Some(0) {
            offset = offset.saturating_add(1);
        }
        offset = offset.saturating_add(1);
    }
    // FCOMMENT: zero-terminated string
    if flg & 0x10 != 0 {
        while offset < input.len() && input.get(offset).copied() != Some(0) {
            offset = offset.saturating_add(1);
        }
        offset = offset.saturating_add(1);
    }
    // FHCRC: 2-byte header checksum
    if flg & 0x02 != 0 {
        offset = offset.saturating_add(2);
    }

    if offset > input.len().saturating_sub(8) {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: Corrupted or truncated GZIP header",
        ));
    }

    let mut decomp = alloc::boxed::Box::<miniz_oxide::inflate::core::DecompressorOxide>::default();
    let mut decompressed = alloc::vec::Vec::with_capacity(input.len().saturating_mul(2));
    let mut in_pos = offset;
    let mut out_pos = 0usize;
    let flags = miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;

    loop {
        if decompressed.len() == out_pos {
            decompressed.resize(decompressed.len().saturating_mul(2).max(128), 0);
        }
        let in_slice = input.get(in_pos..).unwrap_or(&[]);
        let (status, in_consumed, out_consumed) = miniz_oxide::inflate::core::decompress(
            &mut decomp,
            in_slice,
            &mut decompressed,
            out_pos,
            flags,
        );
        in_pos = in_pos.saturating_add(in_consumed);
        out_pos = out_pos.saturating_add(out_consumed);

        match status {
            miniz_oxide::inflate::TINFLStatus::Done => {
                decompressed.truncate(out_pos);
                break;
            }
            miniz_oxide::inflate::TINFLStatus::NeedsMoreInput => {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Parse Error: Truncated GZIP DEFLATE payload (unexpected EOF)",
                ));
            }
            miniz_oxide::inflate::TINFLStatus::HasMoreOutput => {
                continue;
            }
            _ => {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: GZIP DEFLATE decompression failed: {:?}", status),
                ));
            }
        }
    }

    let footer_start = in_pos;
    if input.len() < footer_start.saturating_add(8) {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            "Parse Error: Truncated GZIP footer",
        ));
    }

    let b0 = input.get(footer_start).copied().unwrap_or(0);
    let b1 = input.get(footer_start.saturating_add(1)).copied().unwrap_or(0);
    let b2 = input.get(footer_start.saturating_add(2)).copied().unwrap_or(0);
    let b3 = input.get(footer_start.saturating_add(3)).copied().unwrap_or(0);
    let expected_crc = u32::from_le_bytes([b0, b1, b2, b3]);
    let actual_crc = compute_crc32(&decompressed);
    if expected_crc != actual_crc {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            &alloc::format!(
                "Parse Error: GZIP CRC-32 mismatch: expected 0x{:08x}, got 0x{:08x}",
                expected_crc, actual_crc
            ),
        ));
    }

    let b4 = input.get(footer_start.saturating_add(4)).copied().unwrap_or(0);
    let b5 = input.get(footer_start.saturating_add(5)).copied().unwrap_or(0);
    let b6 = input.get(footer_start.saturating_add(6)).copied().unwrap_or(0);
    let b7 = input.get(footer_start.saturating_add(7)).copied().unwrap_or(0);
    let expected_isize = u32::from_le_bytes([b4, b5, b6, b7]);
    let actual_isize = decompressed.len() as u32;
    if expected_isize != actual_isize {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            &alloc::format!(
                "Parse Error: GZIP ISIZE mismatch: expected {}, got {}",
                expected_isize, actual_isize
            ),
        ));
    }

    let total_consumed = footer_start.saturating_add(8);
    Ok((decompressed, total_consumed))
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
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

    #[test]
    fn test_crc32_canonical_vector() {
        // Standard IEEE 802.3 test vector "123456789" -> 0xCBF43926.
        let data = b"123456789";
        let crc = compute_crc32(data);
        assert_eq!(crc, 0xcbf43926);
    }

    #[test]
    fn test_swap_two_bytes() {
        let mut buf = [0x12u8, 0x34, 0x56, 0x78, 0x9a];
        swap_two_bytes(&mut buf);
        assert_eq!(buf, [0x34, 0x12, 0x78, 0x56, 0x9a]);
    }

    #[test]
    fn test_gzip_compress_decompress_roundtrip() {
        let plain = b"The quick brown fox jumps over the lazy dog. 1234567890! Repeat: The quick brown fox jumps over the lazy dog.";
        let compressed = gzip_compress(plain, 6);
        assert!(compressed.len() >= 18);
        assert_eq!(compressed.first().copied(), Some(0x1f));
        assert_eq!(compressed.get(1).copied(), Some(0x8b));
        assert_eq!(compressed.get(2).copied(), Some(0x08));

        let (decompressed, consumed) = gzip_decompress(&compressed).unwrap();
        assert_eq!(decompressed.as_slice(), plain.as_slice());
        assert_eq!(consumed, compressed.len());
    }

    #[test]
    fn test_gzip_daffodil_test_vector_match() {
        // CSV data from Daffodil test TestGzipFoldB64.tdml
        let csv_text = b"last,first,middle,DOB\r\nsmith,robert,brandon,1988-03-24\r\njohnson,john,henry,1986-01-23\r\njones,arya,cat,1986-02-19\r\n";
        let compressed = gzip_compress(csv_text, 9);
        assert_eq!(compressed.len(), 115);
        let (decompressed, consumed) = gzip_decompress(&compressed).unwrap();
        assert_eq!(decompressed.as_slice(), csv_text.as_slice());
        assert_eq!(consumed, 115);
    }

    #[test]
    fn test_gzip_invalid_magic_fails() {
        let bad = [0x00u8; 20];
        let res = gzip_decompress(&bad);
        assert!(res.is_err());
    }

    #[test]
    fn test_gzip_truncated_fails() {
        let short = [0x1fu8, 0x8b, 0x08, 0x00];
        let res = gzip_decompress(&short);
        assert!(res.is_err());
    }

    /// Verifies bare CR and LF unfolding in IMF headers per RFC 2822.
    #[test]
    fn test_unfold_imf_bare_cr_and_lf() {
        // Bare CR followed by space
        let with_cr = b"Field:\r continuation";
        let (unfolded_cr, _) = unfold_imf(with_cr);
        assert_eq!(unfolded_cr.as_slice(), b"Field: continuation");

        // Bare LF followed by space
        let with_lf = b"Field:\n continuation";
        let (unfolded_lf, _) = unfold_imf(with_lf);
        assert_eq!(unfolded_lf.as_slice(), b"Field: continuation");
    }

    /// Verifies base64 padding for 1-byte, 2-byte inputs and 76-character column wrapping.
    #[test]
    fn test_base64_padding_and_column_wrapping() {
        // 1-byte input produces 2 '=' padding characters
        let enc1 = encode_base64_mime(b"a");
        assert_eq!(enc1.as_slice(), b"YQ==");
        assert_eq!(decode_base64_mime(&enc1).as_slice(), b"a");

        // 2-byte input produces 1 '=' padding character
        let enc2 = encode_base64_mime(b"ab");
        assert_eq!(enc2.as_slice(), b"YWI=");
        assert_eq!(decode_base64_mime(&enc2).as_slice(), b"ab");

        // Long input exceeding 76 characters triggers line break wrapping
        let long_input = [b'X'; 120];
        let enc_long = encode_base64_mime(&long_input);
        assert!(enc_long.windows(2).any(|w| w == b"\r\n"));
        let dec_long = decode_base64_mime(&enc_long);
        assert_eq!(dec_long.as_slice(), &long_input);
    }

    /// Verifies GZIP optional headers (FEXTRA, FNAME, FCOMMENT, FHCRC) and footer error checking.
    #[test]
    fn test_gzip_optional_headers_and_footer_errors() {
        let raw = b"test payload for gzip headers";
        let gz = gzip_compress(raw, 6);

        // CRC-32 mismatch error
        let mut gz_bad_crc = gz.clone();
        let footer_idx = gz_bad_crc.len().saturating_sub(8);
        if let Some(slot) = gz_bad_crc.get_mut(footer_idx) {
            *slot ^= 0xFF;
        }
        assert!(gzip_decompress(&gz_bad_crc).is_err());

        // ISIZE mismatch error
        let mut gz_bad_isize = gz.clone();
        let isize_idx = gz_bad_isize.len().saturating_sub(4);
        if let Some(slot) = gz_bad_isize.get_mut(isize_idx) {
            *slot ^= 0xFF;
        }
        assert!(gzip_decompress(&gz_bad_isize).is_err());

        // Truncated footer error
        let gz_trunc_footer = &gz[..gz.len().saturating_sub(4)];
        assert!(gzip_decompress(gz_trunc_footer).is_err());

        // Synthesize GZIP member with FEXTRA, FNAME, FCOMMENT, FHCRC
        let mut custom_gz = alloc::vec::Vec::new();
        // ID1, ID2, CM, FLG = 0x1F (FTEXT | FHCRC | FEXTRA | FNAME | FCOMMENT)
        custom_gz.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x1E, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff]);
        // FEXTRA: 2 bytes length (0x0002) + 2 extra bytes
        custom_gz.extend_from_slice(&[0x02, 0x00, 0xAA, 0xBB]);
        // FNAME: zero-terminated string "file.txt\0"
        custom_gz.extend_from_slice(b"file.txt\0");
        // FCOMMENT: zero-terminated string "comment\0"
        custom_gz.extend_from_slice(b"comment\0");
        // FHCRC: 2 bytes
        custom_gz.extend_from_slice(&[0x12, 0x34]);
        // Append DEFLATE payload and footer from standard gz
        custom_gz.extend_from_slice(&gz[10..]);

        let (decomp, _) = gzip_decompress(&custom_gz).unwrap();
        assert_eq!(decomp.as_slice(), raw);

        // Corrupted deflate payload after valid 10-byte header
        let mut corrupt_deflate = alloc::vec![0x1fu8, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF];
        corrupt_deflate.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(gzip_decompress(&corrupt_deflate).is_err());

        // Truncated DEFLATE stream (needs more input)
        assert!(gzip_decompress(&gz[..15]).is_err());

        // Truncated FEXTRA header (< offset + 2)
        let trunc_fextra = [0x1fu8, 0x8b, 0x08, 0x04, 0, 0, 0, 0, 0, 0, 0x01];
        assert!(gzip_decompress(&trunc_fextra).is_err());

        // Unterminated FNAME string exceeding header boundary
        let unterminated_fname = [0x1fu8, 0x8b, 0x08, 0x08, 0, 0, 0, 0, 0, 0, b'a', b'b', b'c'];
        assert!(gzip_decompress(&unterminated_fname).is_err());

        // Bare CR and bare LF folding in unfold_imf
        let (unfolded_bare, _) = unfold_imf(b"line1\r line2\n\tline3");
        assert_eq!(unfolded_bare.as_slice(), b"line1 line2\tline3");

        // Base64 with '+' and '/' symbols, and remainders 1 and 2
        let raw_bits = [0xFB, 0xFF, 0xBF];
        let enc = encode_base64_mime(&raw_bits);
        assert!(enc.contains(&b'+') || enc.contains(&b'/'));
        let dec = decode_base64_mime(&enc);
        assert_eq!(dec.as_slice(), &raw_bits);

        let rem1 = [0x42];
        let enc1 = encode_base64_mime(&rem1);
        assert_eq!(decode_base64_mime(&enc1).as_slice(), &rem1);

        let rem2 = [0x42, 0x43];
        let enc2 = encode_base64_mime(&rem2);
        assert_eq!(decode_base64_mime(&enc2).as_slice(), &rem2);

        // GZIP with FCOMMENT (0x10) and FHCRC (0x02)
        let mut gz_comment = alloc::vec![0x1fu8, 0x8b, 0x08, 0x12, 0, 0, 0, 0, 0, 0];
        gz_comment.extend_from_slice(b"mycomment\0"); // FCOMMENT
        gz_comment.extend_from_slice(&[0x55, 0xAA]);   // FHCRC
        if gz.len() > 10 {
            gz_comment.extend_from_slice(&gz[10..]);
            let decomp = gzip_decompress(&gz_comment);
            assert!(decomp.is_ok());
        }

        // Corrupted/truncated GZIP header exceeding bounds
        let trunc_hdr = [0x1fu8, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 0, 1, 2];
        assert!(gzip_decompress(&trunc_hdr).is_err());

        // Verifies unfold_imf handling of bare CR followed by whitespace (RFC 2045 unfolding).
        let (unfolded_cr, offsets_cr) = unfold_imf(b"Header:\r value\r\tcontinued");
        assert_eq!(unfolded_cr.as_slice(), b"Header: value\tcontinued");
        assert!(!offsets_cr.is_empty());

        // Verifies rejection of truncated GZIP FEXTRA header where length bytes are truncated.
        let trunc_fextra = [0x1fu8, 0x8b, 0x08, 0x04, 0, 0, 0, 0, 0, 0, 0x01];
        assert!(gzip_decompress(&trunc_fextra).is_err());

        // Verifies rejection of corrupted DEFLATE compressed payload stream with invalid block headers.
        let bad_deflate = [
            0x1fu8, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 0,
            0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(gzip_decompress(&bad_deflate).is_err());
    }
}
