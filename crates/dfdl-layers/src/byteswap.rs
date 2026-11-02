//! Byte swap layer extensions (`twoByteSwap`, `fourByteSwap`, `byteSwap`).
//!
//! Reverses the byte order of multi-byte words in physical bitstreams
//! according to Daffodil layer specifications.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Swaps adjacent byte pairs (16-bit word byte swap) in place.
pub fn swap_two_bytes(bytes: &mut [u8]) {
    let mut i = 0usize;
    while i.saturating_add(1) < bytes.len() {
        bytes.swap(i, i.saturating_add(1));
        i = i.saturating_add(2);
    }
}

/// Swaps 4-byte quads (32-bit word byte swap) in place.
pub fn swap_four_bytes(bytes: &mut [u8]) {
    let mut i = 0usize;
    while i.saturating_add(3) < bytes.len() {
        bytes.swap(i, i.saturating_add(3));
        bytes.swap(i.saturating_add(1), i.saturating_add(2));
        i = i.saturating_add(4);
    }
}

/// Layer implementation for 2-byte and 4-byte endianness swapping.
pub struct ByteSwapLayer {
    /// Number of bytes per word: 2 for `twoByteSwap`/`byteSwap`, 4 for `fourByteSwap`.
    pub word_size: usize,
}

impl Layer for ByteSwapLayer {
    fn name(&self) -> &'static str {
        if self.word_size == 4 {
            "fourByteSwap"
        } else {
            "twoByteSwap"
        }
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        if self.word_size == 4 {
            clean.eq_ignore_ascii_case("fourbyteswap")
                || clean.eq_ignore_ascii_case("fourByteSwap")
        } else {
            clean.eq_ignore_ascii_case("twobyteswap")
                || clean.eq_ignore_ascii_case("twoByteSwap")
                || clean.eq_ignore_ascii_case("byteswap")
                || clean.eq_ignore_ascii_case("byteSwap")
        }
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let req_words = ctx
            .variable_map
            .get_variable("requireLengthInWholeWords")
            .is_some_and(|v| match v {
                dfdl_core::infoset::value::DfdlValue::String(s) => {
                    s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("true")
                }
                _ => false,
            });

        if req_words && !raw_bytes.len().is_multiple_of(self.word_size) {
            let msg = alloc::format!(
                "Parse Error: Data length is not a multiple of {} for {} layer",
                self.word_size,
                self.name()
            );
            return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
        }

        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        let req_words = ctx
            .variable_map
            .get_variable("requireLengthInWholeWords")
            .is_some_and(|v| match v {
                dfdl_core::infoset::value::DfdlValue::String(s) => {
                    s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("true")
                }
                _ => false,
            });

        if req_words && !buffer.len().is_multiple_of(self.word_size) {
            let msg = alloc::format!(
                "Unparse Error: not a multiple of {} for {} layer",
                self.word_size,
                self.name()
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }

        if self.word_size == 4 {
            swap_four_bytes(buffer);
        } else {
            swap_two_bytes(buffer);
        }

        Ok(())
    }
}
