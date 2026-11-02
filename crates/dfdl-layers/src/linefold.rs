//! RFC 2822 / RFC 5322 Internet Message Format (IMF) line folding layer (`lineFolded_IMF`).
//!
//! Provides transparent unfolding of wrapped header/body fields where CRLF or bare LF/CR
//! is followed by whitespace (SP or HTAB).

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::DFDLResult;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Unfolds input bytes according to RFC 2822 / RFC 5322 section 2.2.3.
///
/// Returns (unfolded_bytes, orig_offsets_mapping).
#[must_use]
pub fn unfold_imf(input: &[u8]) -> (Vec<u8>, Vec<usize>) {
    let mut unfolded = Vec::with_capacity(input.len());
    let mut orig_offsets = Vec::with_capacity(input.len().saturating_add(1));
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

/// Layer implementation for IMF line folding.
#[derive(Default)]
pub struct LineFoldedImfLayer;

impl Layer for LineFoldedImfLayer {
    fn name(&self) -> &'static str {
        "lineFolded_IMF"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("linefolded_imf")
            || clean.eq_ignore_ascii_case("lineFolded_IMF")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        _ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let (unfolded, orig_offsets) = unfold_imf(raw_bytes);
        Ok(LayerParseInput::Transformed {
            data: unfolded,
            orig_byte_offsets: Some(orig_offsets),
            orig_consumed_bytes: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        // Output passes through
        Ok(())
    }
}
