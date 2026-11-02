//! Check digit layer extension (`checkDigit`).
//!
//! Computes modulo-10 sum check digits over ASCII digit sequences.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};
use dfdl_core::types::QName;

/// Computes the check digit over an ASCII data slice.
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

/// Layer implementation for check digit computation and validation.
#[derive(Default)]
pub struct CheckDigitLayer;

impl Layer for CheckDigitLayer {
    fn name(&self) -> &'static str {
        "checkDigit"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("checkdigit")
            || clean.eq_ignore_ascii_case("checkDigit")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let layer_len = ctx
            .variable_map
            .get_variable("length")
            .and_then(|v| match v {
                DfdlValue::Short(s) if *s > 0 => Some(*s as usize),
                DfdlValue::Int(i) if *i > 0 => Some(*i as usize),
                DfdlValue::Long(l) if *l > 0 => Some(*l as usize),
                DfdlValue::UnsignedShort(s) if *s > 0 => Some(*s as usize),
                DfdlValue::UnsignedInt(i) if *i > 0 => Some(*i as usize),
                _ => None,
            })
            .unwrap_or(10);

        if raw_bytes.len() < layer_len {
            let msg = alloc::format!(
                "Parse Error: Insufficient data for checkDigit layer: expected {} bytes",
                layer_len
            );
            return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
        }

        let cd = compute_check_digit(raw_bytes.get(..layer_len).unwrap_or(&[]));
        let var_qname = QName::with_namespace(
            "urn:org.apache.daffodil.layers.checkDigit",
            "checkDigit",
            None,
        );
        ctx.variable_map.set_variable_validated(&var_qname, DfdlValue::UnsignedShort(cd), true)?;

        Ok(LayerParseInput::InPlace {
            bit_limit: Some(layer_len.saturating_mul(8)),
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        let cd = compute_check_digit(buffer);
        let var_qname = QName::with_namespace(
            "urn:org.apache.daffodil.layers.checkDigit",
            "checkDigit",
            None,
        );
        ctx.variable_map.set_variable_validated(&var_qname, DfdlValue::UnsignedShort(cd), false)?;
        Ok(())
    }
}
