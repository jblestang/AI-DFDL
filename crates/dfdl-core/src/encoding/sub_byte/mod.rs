//! Sub-byte and packed bit-level character encoding implementations.
//!
//! Conforms to DFDL Specification §11 for non-byte-aligned and packed representations.
//! Each sub-byte encoding is partitioned into its own dedicated module:
//! - [`bits`]: 1-bit binary representation (`X-DFDL-BITS`)
//! - [`base4`]: 2-bit base-4 representation (`X-DFDL-BASE4`)
//! - [`octal`]: 3-bit octal representation (`X-DFDL-OCTAL`)
//! - [`hex`]: 4-bit hexadecimal representation (`X-DFDL-HEX`)
//! - [`packed_5bit`]: 5-bit packed representation (`X-DFDL-5-BIT-PACKED`)
//! - [`packed_6bit`]: 6-bit US-ASCII packed representation (`X-DFDL-US-ASCII-6-BIT-PACKED`)
//! - [`icao`]: 6-bit ICAO Annex 10 representation (`X-DFDL-ICAO-6-BIT`)
//! - [`packed_7bit`]: 7-bit US-ASCII packed representation (`X-DFDL-US-ASCII-7-BIT-PACKED`)
//! - [`packed_8bit`]: 8-bit packed representation (`X-DFDL-8-BIT-PACKED`)
//! - [`packed_8bit_reverse`]: 8-bit inverted representation (`X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE`)
//! - [`dfi_alpha`]: Aviation DFI 746/747/336/769 alpha representation (`X-DFDL-6-BIT-DFI-746`)
//! - [`dfi_264`]: Tactical DFI-264-DUI-001 representation (`X-DFDL-6-BIT-DFI-264-DUI-001`)
//! - [`dfi_1661`]: Tactical DFI-1661-DUI-001 representation (`X-DFDL-5-BIT-DFI-1661-DUI-001`)

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::encoding::traits::Encoding;
use crate::error::DFDLResult;

pub mod base4;
pub mod bits;
pub mod dfi_1661;
pub mod dfi_264;
pub mod dfi_alpha;
pub mod hex;
pub mod icao;
pub mod octal;
pub mod packed_5bit;
pub mod packed_6bit;
pub mod packed_7bit;
pub mod packed_8bit;
pub mod packed_8bit_reverse;

pub use base4::Base4Encoding;
pub use bits::BitsEncoding;
pub use dfi_1661::Dfi1661Dui001Encoding;
pub use dfi_264::Dfi264Dui001Encoding;
pub use dfi_alpha::DfiAlphaEncoding;
pub use hex::HexEncoding;
pub use icao::IcaoEncoding;
pub use octal::OctalEncoding;
pub use packed_5bit::{Packed5BitEncoding, PACKED_5BIT_CHARS};
pub use packed_6bit::Packed6BitEncoding;
pub use packed_7bit::Packed7BitEncoding;
pub use packed_8bit::Packed8BitEncoding;
pub use packed_8bit_reverse::Packed8BitReverseEncoding;

/// Sub-byte encoding variant category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubByteVariant {
    /// 1-bit binary character representation ('0' or '1').
    Bits,
    /// 2-bit base-4 character representation ('0'..'3').
    Base4,
    /// 3-bit octal character representation ('0'..'7').
    Octal,
    /// 4-bit hexadecimal character representation ('0'..'F').
    Hex,
    /// 5-bit packed encoding: '0'..'7' and 'A'..'Z' without I and O.
    Packed5Bit,
    /// 6-bit ASCII encoding: 0x20..=0x5F mapped to 0..63.
    Packed6Bit,
    /// ICAO 6-bit encoding: uppercase letters, digits, and spaces.
    Icao,
    /// 7-bit packed ASCII encoding: 0..=127.
    Packed7Bit,
    /// 8-bit packed encoding: 0..=255.
    Packed8Bit,
    /// 8-bit packed inverted encoding: byte value is inverted (255 - code).
    Packed8BitReverse,
    /// DFI 746/747/336/769 aviation field character encoding.
    DfiAlpha {
        /// Number of bits per character code unit.
        bit_width: usize,
    },
    /// DFI-264-DUI-001 aviation field character encoding.
    Dfi264Dui001 {
        /// Number of bits per character code unit.
        bit_width: usize,
    },
    /// DFI-1661-DUI-001 aviation field character encoding.
    Dfi1661Dui001 {
        /// Number of bits per character code unit.
        bit_width: usize,
    },
}

/// Unified sub-byte and packed character encoding dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubByteEncoding {
    /// Specific sub-byte variant.
    pub variant: SubByteVariant,
}

impl SubByteEncoding {
    /// 1-bit binary instance.
    pub const BITS: Self = Self { variant: SubByteVariant::Bits };
    /// 2-bit base4 instance.
    pub const BASE4: Self = Self { variant: SubByteVariant::Base4 };
    /// 3-bit octal instance.
    pub const OCTAL: Self = Self { variant: SubByteVariant::Octal };
    /// 4-bit hex instance.
    pub const HEX: Self = Self { variant: SubByteVariant::Hex };
    /// 5-bit packed instance.
    pub const PACKED_5BIT: Self = Self { variant: SubByteVariant::Packed5Bit };
    /// 6-bit packed ASCII instance.
    pub const PACKED_6BIT: Self = Self { variant: SubByteVariant::Packed6Bit };
    /// 6-bit ICAO instance.
    pub const ICAO: Self = Self { variant: SubByteVariant::Icao };
    /// 7-bit packed ASCII instance.
    pub const PACKED_7BIT: Self = Self { variant: SubByteVariant::Packed7Bit };
    /// 8-bit packed instance.
    pub const PACKED_8BIT: Self = Self { variant: SubByteVariant::Packed8Bit };
    /// 8-bit packed reverse instance.
    pub const PACKED_8BIT_REVERSE: Self = Self { variant: SubByteVariant::Packed8BitReverse };
}

impl Encoding for SubByteEncoding {
    #[inline]
    fn name(&self) -> &str {
        match self.variant {
            SubByteVariant::Bits => "X-DFDL-BITS",
            SubByteVariant::Base4 => "X-DFDL-BASE4",
            SubByteVariant::Octal => "X-DFDL-OCTAL",
            SubByteVariant::Hex => "X-DFDL-HEX",
            SubByteVariant::Packed5Bit => "X-DFDL-5-BIT-PACKED",
            SubByteVariant::Packed6Bit => "X-DFDL-US-ASCII-6-BIT-PACKED",
            SubByteVariant::Icao => "X-DFDL-ICAO-6-BIT",
            SubByteVariant::Packed7Bit => "X-DFDL-US-ASCII-7-BIT-PACKED",
            SubByteVariant::Packed8Bit => "X-DFDL-8-BIT-PACKED",
            SubByteVariant::Packed8BitReverse => "X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE",
            SubByteVariant::DfiAlpha { .. } => "X-DFDL-6-BIT-DFI-746",
            SubByteVariant::Dfi264Dui001 { .. } => "X-DFDL-6-BIT-DFI-264-DUI-001",
            SubByteVariant::Dfi1661Dui001 { .. } => "X-DFDL-5-BIT-DFI-1661-DUI-001",
        }
    }

    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.decode_text_bytes(bytes),
            SubByteVariant::Base4 => Base4Encoding.decode_text_bytes(bytes),
            SubByteVariant::Octal => OctalEncoding.decode_text_bytes(bytes),
            SubByteVariant::Hex => HexEncoding.decode_text_bytes(bytes),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.decode_text_bytes(bytes),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.decode_text_bytes(bytes),
            SubByteVariant::Icao => IcaoEncoding.decode_text_bytes(bytes),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.decode_text_bytes(bytes),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.decode_text_bytes(bytes),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.decode_text_bytes(bytes),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.decode_text_bytes(bytes),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.decode_text_bytes(bytes),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.decode_text_bytes(bytes),
        }
    }

    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Base4 => Base4Encoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Octal => OctalEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Hex => HexEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Icao => IcaoEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.decode_text_bytes_with_offsets(bytes),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.decode_text_bytes_with_offsets(bytes),
        }
    }

    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.encode_text_string(text),
            SubByteVariant::Base4 => Base4Encoding.encode_text_string(text),
            SubByteVariant::Octal => OctalEncoding.encode_text_string(text),
            SubByteVariant::Hex => HexEncoding.encode_text_string(text),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.encode_text_string(text),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.encode_text_string(text),
            SubByteVariant::Icao => IcaoEncoding.encode_text_string(text),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.encode_text_string(text),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.encode_text_string(text),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.encode_text_string(text),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.encode_text_string(text),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.encode_text_string(text),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.encode_text_string(text),
        }
    }

    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.find_unmappable_char(text),
            SubByteVariant::Base4 => Base4Encoding.find_unmappable_char(text),
            SubByteVariant::Octal => OctalEncoding.find_unmappable_char(text),
            SubByteVariant::Hex => HexEncoding.find_unmappable_char(text),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.find_unmappable_char(text),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.find_unmappable_char(text),
            SubByteVariant::Icao => IcaoEncoding.find_unmappable_char(text),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.find_unmappable_char(text),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.find_unmappable_char(text),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.find_unmappable_char(text),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.find_unmappable_char(text),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.find_unmappable_char(text),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.find_unmappable_char(text),
        }
    }

    #[inline]
    fn char_bits(&self) -> Option<usize> {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.char_bits(),
            SubByteVariant::Base4 => Base4Encoding.char_bits(),
            SubByteVariant::Octal => OctalEncoding.char_bits(),
            SubByteVariant::Hex => HexEncoding.char_bits(),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.char_bits(),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.char_bits(),
            SubByteVariant::Icao => IcaoEncoding.char_bits(),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.char_bits(),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.char_bits(),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.char_bits(),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.char_bits(),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.char_bits(),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.char_bits(),
        }
    }

    #[inline]
    fn unit_bits(&self) -> usize {
        1
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        1
    }

    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.decode_sub_byte_char(code),
            SubByteVariant::Base4 => Base4Encoding.decode_sub_byte_char(code),
            SubByteVariant::Octal => OctalEncoding.decode_sub_byte_char(code),
            SubByteVariant::Hex => HexEncoding.decode_sub_byte_char(code),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.decode_sub_byte_char(code),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.decode_sub_byte_char(code),
            SubByteVariant::Icao => IcaoEncoding.decode_sub_byte_char(code),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.decode_sub_byte_char(code),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.decode_sub_byte_char(code),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.decode_sub_byte_char(code),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.decode_sub_byte_char(code),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.decode_sub_byte_char(code),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.decode_sub_byte_char(code),
        }
    }

    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Base4 => Base4Encoding.strict_sub_byte_code(ch),
            SubByteVariant::Octal => OctalEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Hex => HexEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Icao => IcaoEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.strict_sub_byte_code(ch),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.strict_sub_byte_code(ch),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.strict_sub_byte_code(ch),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.strict_sub_byte_code(ch),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.strict_sub_byte_code(ch),
        }
    }

    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        match self.variant {
            SubByteVariant::Bits => BitsEncoding.sub_byte_replacement_code(),
            SubByteVariant::Base4 => Base4Encoding.sub_byte_replacement_code(),
            SubByteVariant::Octal => OctalEncoding.sub_byte_replacement_code(),
            SubByteVariant::Hex => HexEncoding.sub_byte_replacement_code(),
            SubByteVariant::Packed5Bit => Packed5BitEncoding.sub_byte_replacement_code(),
            SubByteVariant::Packed6Bit => Packed6BitEncoding.sub_byte_replacement_code(),
            SubByteVariant::Icao => IcaoEncoding.sub_byte_replacement_code(),
            SubByteVariant::Packed7Bit => Packed7BitEncoding.sub_byte_replacement_code(),
            SubByteVariant::Packed8Bit => Packed8BitEncoding.sub_byte_replacement_code(),
            SubByteVariant::Packed8BitReverse => Packed8BitReverseEncoding.sub_byte_replacement_code(),
            SubByteVariant::DfiAlpha { bit_width } => DfiAlphaEncoding { bit_width }.sub_byte_replacement_code(),
            SubByteVariant::Dfi264Dui001 { bit_width } => Dfi264Dui001Encoding { bit_width }.sub_byte_replacement_code(),
            SubByteVariant::Dfi1661Dui001 { bit_width } => Dfi1661Dui001Encoding { bit_width }.sub_byte_replacement_code(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_individual_bits_encoding() {
        let enc = BitsEncoding;
        assert_eq!(enc.name(), "X-DFDL-BITS");
        assert_eq!(enc.char_bits(), Some(1));
        assert_eq!(enc.decode_sub_byte_char(0), '0');
        assert_eq!(enc.decode_sub_byte_char(1), '1');
        assert_eq!(enc.strict_sub_byte_code('0'), Some(0));
        assert_eq!(enc.strict_sub_byte_code('1'), Some(1));
        assert_eq!(enc.strict_sub_byte_code('2'), None);
        assert_eq!(enc.find_unmappable_char("0101"), None);
        assert_eq!(enc.find_unmappable_char("0120"), Some('2'));
    }

    #[test]
    fn test_individual_base4_encoding() {
        let enc = Base4Encoding;
        assert_eq!(enc.name(), "X-DFDL-BASE4");
        assert_eq!(enc.char_bits(), Some(2));
        assert_eq!(enc.decode_sub_byte_char(0), '0');
        assert_eq!(enc.decode_sub_byte_char(3), '3');
        assert_eq!(enc.strict_sub_byte_code('3'), Some(3));
        assert_eq!(enc.strict_sub_byte_code('4'), None);
    }

    #[test]
    fn test_individual_octal_encoding() {
        let enc = OctalEncoding;
        assert_eq!(enc.name(), "X-DFDL-OCTAL");
        assert_eq!(enc.char_bits(), Some(3));
        assert_eq!(enc.decode_sub_byte_char(7), '7');
        assert_eq!(enc.strict_sub_byte_code('7'), Some(7));
        assert_eq!(enc.strict_sub_byte_code('8'), None);
    }

    #[test]
    fn test_individual_hex_encoding() {
        let enc = HexEncoding;
        assert_eq!(enc.name(), "X-DFDL-HEX");
        assert_eq!(enc.char_bits(), Some(4));
        assert_eq!(enc.decode_sub_byte_char(10), 'A');
        assert_eq!(enc.decode_sub_byte_char(15), 'F');
        assert_eq!(enc.strict_sub_byte_code('f'), Some(15));
        assert_eq!(enc.strict_sub_byte_code('G'), None);
    }

    #[test]
    fn test_individual_packed_5bit_encoding() {
        let enc = Packed5BitEncoding;
        assert_eq!(enc.name(), "X-DFDL-5-BIT-PACKED");
        assert_eq!(enc.char_bits(), Some(5));
        assert_eq!(enc.decode_sub_byte_char(0), '0');
        assert_eq!(enc.decode_sub_byte_char(8), 'A');
        assert_eq!(enc.strict_sub_byte_code('O'), Some(0));
        assert_eq!(enc.strict_sub_byte_code('I'), Some(1));
        assert_eq!(enc.strict_sub_byte_code('Z'), Some(31));
        assert_eq!(enc.sub_byte_replacement_code(), 29);
    }

    #[test]
    fn test_individual_packed_6bit_encoding() {
        let enc = Packed6BitEncoding;
        assert_eq!(enc.name(), "X-DFDL-US-ASCII-6-BIT-PACKED");
        assert_eq!(enc.char_bits(), Some(6));
        assert_eq!(enc.decode_sub_byte_char(0x20), ' ');
        assert_eq!(enc.decode_sub_byte_char(1), 'A');
        assert_eq!(enc.strict_sub_byte_code(' '), Some(0x20));
        assert_eq!(enc.strict_sub_byte_code('A'), Some(0x01));
        assert_eq!(enc.strict_sub_byte_code('a'), None);
        assert_eq!(enc.sub_byte_replacement_code(), 0x1F);
    }

    #[test]
    fn test_individual_icao_encoding() {
        let enc = IcaoEncoding;
        assert_eq!(enc.name(), "X-DFDL-ICAO-6-BIT");
        assert_eq!(enc.char_bits(), Some(6));
        assert_eq!(enc.decode_sub_byte_char(1), 'A');
        assert_eq!(enc.decode_sub_byte_char(26), 'Z');
        assert_eq!(enc.decode_sub_byte_char(48), '0');
        assert_eq!(enc.decode_sub_byte_char(32), ' ');
        assert_eq!(enc.strict_sub_byte_code('A'), Some(1));
        assert_eq!(enc.strict_sub_byte_code(' '), Some(32));
        assert_eq!(enc.strict_sub_byte_code('.'), None);
        assert_eq!(enc.sub_byte_replacement_code(), 32);
    }

    #[test]
    fn test_individual_packed_7bit_encoding() {
        let enc = Packed7BitEncoding;
        assert_eq!(enc.name(), "X-DFDL-US-ASCII-7-BIT-PACKED");
        assert_eq!(enc.char_bits(), Some(7));
        assert_eq!(enc.decode_sub_byte_char(65), 'A');
        assert_eq!(enc.strict_sub_byte_code('A'), Some(65));
        assert_eq!(enc.strict_sub_byte_code('\u{A0}'), None);
        assert_eq!(enc.sub_byte_replacement_code(), 0x3F);
    }

    #[test]
    fn test_individual_packed_8bit_encoding() {
        let enc = Packed8BitEncoding;
        assert_eq!(enc.name(), "X-DFDL-8-BIT-PACKED");
        assert_eq!(enc.char_bits(), Some(8));
        assert_eq!(enc.decode_sub_byte_char(255), '\u{FF}');
        assert_eq!(enc.strict_sub_byte_code('\u{FF}'), Some(255));
        assert_eq!(enc.strict_sub_byte_code('\u{100}'), None);
    }

    #[test]
    fn test_individual_packed_8bit_reverse_encoding() {
        let enc = Packed8BitReverseEncoding;
        assert_eq!(enc.name(), "X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE");
        assert_eq!(enc.char_bits(), Some(8));
        assert_eq!(enc.decode_sub_byte_char(0), '\u{FF}');
        assert_eq!(enc.decode_sub_byte_char(255), '\0');
        assert_eq!(enc.strict_sub_byte_code('\u{FF}'), Some(0));
        assert_eq!(enc.strict_sub_byte_code('\0'), Some(255));
    }

    #[test]
    fn test_individual_dfi_encodings() {
        let alpha = DfiAlphaEncoding { bit_width: 6 };
        assert_eq!(alpha.name(), "X-DFDL-6-BIT-DFI-746");
        assert_eq!(alpha.char_bits(), Some(6));
        assert_eq!(alpha.decode_sub_byte_char(0), 'A');
        assert_eq!(alpha.decode_sub_byte_char(25), 'Z');
        assert_eq!(alpha.strict_sub_byte_code('A'), Some(0));
        assert_eq!(alpha.strict_sub_byte_code('0'), None);

        let dfi264 = Dfi264Dui001Encoding { bit_width: 6 };
        assert_eq!(dfi264.name(), "X-DFDL-6-BIT-DFI-264-DUI-001");
        assert_eq!(dfi264.char_bits(), Some(6));
        assert_eq!(dfi264.decode_sub_byte_char(0), '0');
        assert_eq!(dfi264.decode_sub_byte_char(10), ' ');
        assert_eq!(dfi264.decode_sub_byte_char(11), 'A');
        assert_eq!(dfi264.strict_sub_byte_code(' '), Some(10));
        assert_eq!(dfi264.strict_sub_byte_code('A'), Some(11));

        let dfi1661 = Dfi1661Dui001Encoding { bit_width: 5 };
        assert_eq!(dfi1661.name(), "X-DFDL-5-BIT-DFI-1661-DUI-001");
        assert_eq!(dfi1661.char_bits(), Some(5));
        assert_eq!(dfi1661.decode_sub_byte_char(0), '\u{00A0}');
        assert_eq!(dfi1661.decode_sub_byte_char(1), 'A');
        assert_eq!(dfi1661.strict_sub_byte_code('\u{00A0}'), Some(0));
        assert_eq!(dfi1661.strict_sub_byte_code('A'), Some(1));
    }
}

