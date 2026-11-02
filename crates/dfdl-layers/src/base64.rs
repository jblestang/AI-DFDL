//! RFC 2045 MIME Base64 layer extension (`base64_MIME`).
//!
//! Provides Base64 encoding and decoding with automatic omission of MIME whitespace
//! and standard 76-character column wrapping.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::DFDLResult;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};

/// Decodes a MIME Base64-encoded byte slice according to RFC 2045 section 6.8.
#[must_use]
pub fn decode_base64_mime(input: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(input.len());
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
#[must_use]
pub fn encode_base64_mime(input: &[u8]) -> Vec<u8> {
    const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((input.len().saturating_mul(4) / 3).saturating_add(64));
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

/// Layer implementation for MIME Base64 encoding and decoding.
#[derive(Default)]
pub struct Base64MimeLayer;

impl Layer for Base64MimeLayer {
    fn name(&self) -> &'static str {
        "base64_MIME"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("base64_mime")
            || clean.eq_ignore_ascii_case("base64_MIME")
    }

    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        _ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let decoded = decode_base64_mime(raw_bytes);
        Ok(LayerParseInput::Transformed {
            data: decoded,
            orig_byte_offsets: None,
            orig_consumed_bytes: None,
        })
    }

    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        *buffer = encode_base64_mime(buffer);
        Ok(())
    }
}
