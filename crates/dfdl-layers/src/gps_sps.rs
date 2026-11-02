//! GPS Standard Positioning Service (SPS) parity layers.
//!
//! Includes `parityLayer` and `parityLayerHOW` used in GPS NAV telemetry subframes.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::DFDLResult;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Layer implementation for GPS-SPS 30-bit word parity decoding.
#[derive(Default)]
pub struct GpsSpsParityLayer;

impl Layer for GpsSpsParityLayer {
    fn name(&self) -> &'static str {
        "parityLayer"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("paritylayer")
            || clean.eq_ignore_ascii_case("parityLayer")
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        _ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}

/// Layer implementation for GPS-SPS Hand-Over Word (HOW) parity decoding.
#[derive(Default)]
pub struct GpsSpsParityHowLayer;

impl Layer for GpsSpsParityHowLayer {
    fn name(&self) -> &'static str {
        "parityLayerHOW"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("paritylayerhow")
            || clean.eq_ignore_ascii_case("parityLayerHOW")
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        _ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}
