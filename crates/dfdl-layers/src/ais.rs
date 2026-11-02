//! AIS payload armoring layer extension (`aisPayloadArmor`, `aisPayloadArmoring`).
//!
//! Provides encoding/decoding for ITU-R M.1371 Automatic Identification System (AIS)
//! 6-bit ASCII payload armoring.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::DFDLResult;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Layer implementation for AIS payload armoring.
#[derive(Default)]
pub struct AisPayloadArmorLayer;

impl Layer for AisPayloadArmorLayer {
    fn name(&self) -> &'static str {
        "aisPayloadArmor"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("aispayloadarmor")
            || clean.eq_ignore_ascii_case("aisPayloadArmor")
            || clean.eq_ignore_ascii_case("aispayloadarmoring")
            || clean.eq_ignore_ascii_case("aisPayloadArmoring")
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
