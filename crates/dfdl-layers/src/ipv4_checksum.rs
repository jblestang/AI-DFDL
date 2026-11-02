//! RFC 791 IPv4 Header Checksum layer extension (`IPv4Checksum`).
//!
//! Computes the 16-bit one's complement sum of the 20-byte IPv4 header
//! and validates/injects it into the header stream and infoset variables.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};
use dfdl_core::types::QName;

/// Computes the RFC 791 IPv4 header checksum over a 20-byte slice.
#[must_use]
pub fn compute_ipv4_checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
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
    while (sum >> 16) > 0 {
        sum = (sum & 0xffff).saturating_add(sum >> 16);
    }
    !(sum as u16)
}

/// Layer implementation for IPv4 header checksum calculation and validation.
#[derive(Default)]
pub struct IPv4ChecksumLayer;

impl Layer for IPv4ChecksumLayer {
    fn name(&self) -> &'static str {
        "IPv4Checksum"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("ipv4checksum")
            || clean.eq_ignore_ascii_case("IPv4Checksum")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        if raw_bytes.len() < 20 {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                "Parse Error: Insufficient data for IPv4 layer (expected 20 bytes)",
            ));
        }

        let chk = compute_ipv4_checksum(raw_bytes.get(..20).unwrap_or(&[]));
        let var_qname = QName::with_namespace(
            "urn:org.apache.daffodil.layers.IPv4Checksum",
            "IPv4Checksum",
            None,
        );
        ctx.variable_map.set_variable_validated(&var_qname, DfdlValue::UnsignedShort(chk), true)?;

        Ok(LayerParseInput::InPlace {
            bit_limit: Some(160),
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        if buffer.len() >= 20 {
            let chk = compute_ipv4_checksum(buffer.get(..20).unwrap_or(&[]));
            let var_qname = QName::with_namespace(
                "urn:org.apache.daffodil.layers.IPv4Checksum",
                "IPv4Checksum",
                None,
            );
            ctx.variable_map.set_variable_validated(&var_qname, DfdlValue::UnsignedShort(chk), false)?;

            let be_bytes = chk.to_be_bytes();
            if let Some(b10) = buffer.get_mut(10) {
                *b10 = *be_bytes.first().unwrap_or(&0);
            }
            if let Some(b11) = buffer.get_mut(11) {
                *b11 = *be_bytes.get(1).unwrap_or(&0);
            }
        }
        Ok(())
    }
}
