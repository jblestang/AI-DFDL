//! Syntactic anomaly injector for DFDL payloads.
//!
//! Injects physical layer, framing, length, and bitstream defects:
//! - Truncation (premature EOF)
//! - Trailing garbage / framing overflow
//! - Header / prefix length mismatches
//! - Framing delimiter corruption (initiator, terminator, separator)
//! - Bitstream misalignment (unaligned bit shifts)
//! - Encoding character code unit violations

use crate::prng::FastPrng;

/// Specific category of syntactic mutation to apply to a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxAnomaly {
    /// Payload truncated by `cut_bytes` octets (causing premature EOF).
    Truncated {
        /// Number of trailing octets to discard.
        cut_bytes: usize,
    },
    /// Unexpected trailing bytes appended beyond schema boundaries.
    TrailingGarbage {
        /// Number of arbitrary octets to append.
        garbage_bytes: usize,
    },
    /// Declared length header or prefix modified to disagree with actual payload length.
    DeclaredLengthMismatch {
        /// Byte offset in payload where length field is located.
        field_offset: usize,
        /// Field width in bytes (e.g. 2 for u16).
        field_width: usize,
        /// Additive distortion delta (+/-).
        delta: i32,
    },
    /// Framing initiator or terminator sequence corrupted in the stream.
    CorruptedFraming {
        /// Byte offset of the framing sequence to corrupt.
        offset: usize,
    },
    /// Injects fractional bit shift (1 to 7 bits) desynchronizing bit-level fields.
    BitMisaligned {
        /// Number of extra bits to shift (1..=7).
        extra_bits: u8,
    },
    /// Injects invalid bytes violating the target character encoding.
    EncodingViolation {
        /// Offset where invalid bytes should be injected.
        offset: usize,
    },
}

/// Applies syntactic mutations to binary payloads.
pub struct SyntaxAnomalyInjector;

impl SyntaxAnomalyInjector {
    /// Applies a specific syntactic anomaly to a valid payload byte buffer.
    pub fn apply(payload: &[u8], anomaly: &SyntaxAnomaly, prng: &mut FastPrng) -> Vec<u8> {
        let mut corrupted = payload.to_vec();

        match anomaly {
            SyntaxAnomaly::Truncated { cut_bytes } => {
                let cut = (*cut_bytes).min(corrupted.len().saturating_sub(1)).max(1);
                corrupted.truncate(corrupted.len().saturating_sub(cut));
            }
            SyntaxAnomaly::TrailingGarbage { garbage_bytes } => {
                let count = (*garbage_bytes).max(1).min(1024);
                let mut garbage = vec![0u8; count];
                prng.fill_bytes(&mut garbage);
                corrupted.extend_from_slice(&garbage);
            }
            SyntaxAnomaly::DeclaredLengthMismatch {
                field_offset,
                field_width,
                delta,
            } => {
                if corrupted.len() >= field_offset.saturating_add(*field_width) {
                    match field_width {
                        1 => {
                            if let Some(b) = corrupted.get_mut(*field_offset) {
                                *b = (*b as i16).saturating_add(*delta as i16) as u8;
                            }
                        }
                        2 => {
                            if let Some(slice) = corrupted.get_mut(*field_offset..field_offset.saturating_add(2)) {
                                let orig = u16::from_be_bytes([slice[0], slice[1]]);
                                let distorted = (orig as i32).saturating_add(*delta).max(0) as u16;
                                slice.copy_from_slice(&distorted.to_be_bytes());
                            }
                        }
                        4 => {
                            if let Some(slice) = corrupted.get_mut(*field_offset..field_offset.saturating_add(4)) {
                                let orig = u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]);
                                let distorted = (orig as i64).saturating_add(*delta as i64).max(0) as u32;
                                slice.copy_from_slice(&distorted.to_be_bytes());
                            }
                        }
                        _ => {}
                    }
                }
            }
            SyntaxAnomaly::CorruptedFraming { offset } => {
                if let Some(b) = corrupted.get_mut(*offset) {
                    *b ^= 0xFF; // Invert framing byte
                }
            }
            SyntaxAnomaly::BitMisaligned { extra_bits } => {
                let shift = (*extra_bits % 8).max(1);
                let mut shifted = Vec::with_capacity(corrupted.len().saturating_add(1));
                let mut carry = 0u8;
                for &b in &corrupted {
                    let new_byte = (carry << (8 - shift)) | (b >> shift);
                    shifted.push(new_byte);
                    carry = b & ((1 << shift) - 1);
                }
                shifted.push(carry << (8 - shift));
                corrupted = shifted;
            }
            SyntaxAnomaly::EncodingViolation { offset } => {
                if let Some(b) = corrupted.get_mut(*offset) {
                    *b = 0xFF; // Non-ASCII / illegal code point
                }
            }
        }

        corrupted
    }
}
