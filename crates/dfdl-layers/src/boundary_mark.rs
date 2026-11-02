//! Boundary mark layer extension (`boundaryMark`).
//!
//! Delimits variable-length bitstreams using an explicit boundary marker string
//! without consuming or retaining trailing delimiters in the infoset.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::DFDLResult;
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Layer implementation for stream limiting by explicit boundary mark.
#[derive(Default)]
pub struct BoundaryMarkLayer;

impl Layer for BoundaryMarkLayer {
    fn name(&self) -> &'static str {
        "boundaryMark"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("boundarymark")
            || clean.eq_ignore_ascii_case("boundaryMark")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let boundary_mark = ctx
            .variable_map
            .get_variable("boundaryMark")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| String::from("//"));

        let mark_bytes = boundary_mark.as_bytes();
        let mut found_offset: Option<usize> = None;

        if raw_bytes.len() >= mark_bytes.len() && !mark_bytes.is_empty() {
            if let Some(pos) = raw_bytes.windows(mark_bytes.len()).position(|w| w == mark_bytes) {
                found_offset = Some(pos);
            }
        }

        if let Some(offset) = found_offset {
            Ok(LayerParseInput::InPlace {
                bit_limit: Some(offset.saturating_mul(8)),
                post_skip_bits: Some(mark_bytes.len().saturating_mul(8)),
            })
        } else {
            Ok(LayerParseInput::InPlace {
                bit_limit: None,
                post_skip_bits: None,
            })
        }
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        let boundary_mark = ctx
            .variable_map
            .get_variable("boundaryMark")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| String::from("//"));

        buffer.extend_from_slice(boundary_mark.as_bytes());
        Ok(())
    }
}
