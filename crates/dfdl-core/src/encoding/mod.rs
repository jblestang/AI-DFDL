//! DFDL Character Encoding Engine.
//!
//! Conforms to DFDL 1.0 Specification §11 (`dfdl:encoding`).
//!
//! Provides the [`Encoding`] trait, standard and sub-byte encoding handlers,
//! and the [`EncodingKind`] enum for typed character set representation.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;

use crate::error::DFDLResult;

pub mod ascii;
pub mod custom;
pub mod ebcdic;
pub mod iso8859_1;
pub mod iso8859_13;
pub mod sub_byte;
pub mod traits;
pub mod utf16;
pub mod utf32;
pub mod utf8;

pub use ascii::AsciiEncoding;
pub use custom::CustomEncoding;
pub use ebcdic::EbcdicEncoding;
pub use iso8859_1::Iso8859_1Encoding;
pub use iso8859_13::Iso8859_13Encoding;
pub use sub_byte::{
    Base4Encoding, BitsEncoding, Dfi1661Dui001Encoding, Dfi264Dui001Encoding, DfiAlphaEncoding,
    HexEncoding, IcaoEncoding, OctalEncoding, Packed5BitEncoding, Packed6BitEncoding,
    Packed7BitEncoding, Packed8BitEncoding, Packed8BitReverseEncoding, SubByteEncoding,
    SubByteVariant, PACKED_5BIT_CHARS,
};
pub use traits::Encoding;
pub use utf16::Utf16Encoding;
pub use utf32::Utf32Encoding;
pub use utf8::{utf8_lossy_with_offsets, Utf8Encoding};

/// Static singleton for UTF-16 Big-Endian encoding.
pub static UTF_16_BE: Utf16Encoding = Utf16Encoding::BE;
/// Static singleton for UTF-16 Little-Endian encoding.
pub static UTF_16_LE: Utf16Encoding = Utf16Encoding::LE;
/// Static singleton for UTF-32 Big-Endian encoding.
pub static UTF_32_BE: Utf32Encoding = Utf32Encoding::BE;
/// Static singleton for UTF-32 Little-Endian encoding.
pub static UTF_32_LE: Utf32Encoding = Utf32Encoding::LE;

/// Case-insensitive substring matching without heap allocation.
#[inline]
pub(crate) fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return true;
    }
    if h.len() < n.len() {
        return false;
    }
    h.windows(n.len()).any(|window| window.eq_ignore_ascii_case(n))
}

/// Case-insensitive prefix matching without heap allocation.
#[inline]
pub(crate) fn starts_with_ascii_ci(haystack: &str, prefix: &str) -> bool {
    let h = haystack.trim().as_bytes();
    let p = prefix.as_bytes();
    if h.len() < p.len() {
        return false;
    }
    h.get(..p.len()).is_some_and(|w| w.eq_ignore_ascii_case(p))
}

/// DFDL character encoding enumeration.
///
/// Enumerates all standard, packed, sub-byte, and custom DFDL encodings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EncodingKind {
    /// UTF-8 variable-length encoding.
    Utf8,
    /// UTF-16 Big-Endian encoding.
    Utf16Be,
    /// UTF-16 Little-Endian encoding.
    Utf16Le,
    /// UTF-32 Big-Endian encoding.
    Utf32Be,
    /// UTF-32 Little-Endian encoding.
    Utf32Le,
    /// US-ASCII 7-bit character encoding.
    Ascii,
    /// ISO-8859-1 (Latin-1) 8-bit character encoding.
    Iso8859_1,
    /// ISO-8859-13 (Latin-7) 8-bit character encoding.
    Iso8859_13,
    /// EBCDIC (IBM037 / CP037) character encoding.
    Ebcdic,
    /// 1-bit binary character encoding ('0' or '1').
    Bits,
    /// 2-bit base4 character encoding ('0'..'3').
    Base4,
    /// 3-bit octal character encoding ('0'..'7').
    Octal,
    /// 4-bit hexadecimal character encoding ('0'..'F').
    Hex,
    /// 5-bit packed character encoding.
    Packed5Bit,
    /// 6-bit packed ASCII character encoding.
    Packed6Bit,
    /// 6-bit ICAO character encoding.
    Icao,
    /// 7-bit packed ASCII character encoding.
    Packed7Bit,
    /// 8-bit packed character encoding.
    Packed8Bit,
    /// 8-bit packed reverse (bitwise inverted) character encoding.
    Packed8BitReverse,
    /// DFI 746/747/336/769 aviation field character encoding.
    DfiAlpha {
        /// Number of bits per code unit.
        bit_width: usize,
    },
    /// DFI-264-DUI-001 aviation field character encoding.
    Dfi264Dui001 {
        /// Number of bits per code unit.
        bit_width: usize,
    },
    /// DFI-1661-DUI-001 aviation field character encoding.
    Dfi1661Dui001 {
        /// Number of bits per code unit.
        bit_width: usize,
    },
    /// Custom or user-defined DFDL encoding.
    Custom(String),
}

impl Default for EncodingKind {
    #[inline]
    fn default() -> Self {
        Self::Utf8
    }
}

impl EncodingKind {
    /// Parses an encoding name string into an [`EncodingKind`].
    #[must_use]
    pub fn parse_name(encoding: &str) -> Self {
        let enc = encoding.trim();
        if enc.eq_ignore_ascii_case("UTF-8") || enc.eq_ignore_ascii_case("UTF8") {
            return Self::Utf8;
        }
        if enc.eq_ignore_ascii_case("ASCII")
            || enc.eq_ignore_ascii_case("US-ASCII")
            || enc.eq_ignore_ascii_case("ASCII-7")
            || enc.eq_ignore_ascii_case("ASCII7")
        {
            return Self::Ascii;
        }
        if enc.eq_ignore_ascii_case("ISO-8859-1")
            || enc.eq_ignore_ascii_case("ISO8859-1")
            || enc.eq_ignore_ascii_case("ISO-8859_1")
            || enc.eq_ignore_ascii_case("LATIN1")
            || enc.eq_ignore_ascii_case("LATIN-1")
            || enc.eq_ignore_ascii_case("CP1252")
            || enc.eq_ignore_ascii_case("WINDOWS-1252")
        {
            return Self::Iso8859_1;
        }
        if enc.eq_ignore_ascii_case("ISO-8859-13")
            || enc.eq_ignore_ascii_case("LATIN7")
            || enc.eq_ignore_ascii_case("LATIN-7")
        {
            return Self::Iso8859_13;
        }
        if enc.eq_ignore_ascii_case("UTF-16")
            || enc.eq_ignore_ascii_case("UTF-16BE")
            || enc.eq_ignore_ascii_case("UTF16BE")
            || enc.eq_ignore_ascii_case("UCS-2")
            || enc.eq_ignore_ascii_case("UCS-2BE")
        {
            return Self::Utf16Be;
        }
        if enc.eq_ignore_ascii_case("UTF-16LE")
            || enc.eq_ignore_ascii_case("UTF16LE")
            || enc.eq_ignore_ascii_case("UCS-2LE")
        {
            return Self::Utf16Le;
        }
        if enc.eq_ignore_ascii_case("UTF-32")
            || enc.eq_ignore_ascii_case("UTF-32BE")
            || enc.eq_ignore_ascii_case("UTF32BE")
            || enc.eq_ignore_ascii_case("UCS-4")
            || enc.eq_ignore_ascii_case("UCS-4BE")
        {
            return Self::Utf32Be;
        }
        if enc.eq_ignore_ascii_case("UTF-32LE")
            || enc.eq_ignore_ascii_case("UTF32LE")
            || enc.eq_ignore_ascii_case("UCS-4LE")
        {
            return Self::Utf32Le;
        }
        if starts_with_ascii_ci(enc, "EBCDIC")
            || enc.eq_ignore_ascii_case("IBM037")
            || enc.eq_ignore_ascii_case("CP037")
        {
            return Self::Ebcdic;
        }
        if contains_ascii_ci(enc, "8-BIT-PACKED-LSB-FIRST-REVERSE")
            || (contains_ascii_ci(enc, "8-BIT-PACKED") && contains_ascii_ci(enc, "REVERSE"))
        {
            return Self::Packed8BitReverse;
        }
        if contains_ascii_ci(enc, "8-BIT-PACKED") || contains_ascii_ci(enc, "ISO-88591-8-BIT-PACKED") {
            return Self::Packed8Bit;
        }
        if contains_ascii_ci(enc, "7-BIT") || contains_ascii_ci(enc, "ASCII-7-BIT-PACKED") {
            return Self::Packed7Bit;
        }

        let bit_width = if contains_ascii_ci(enc, "1-BIT") || contains_ascii_ci(enc, "BITS") {
            1
        } else if contains_ascii_ci(enc, "2-BIT") || contains_ascii_ci(enc, "BASE4") {
            2
        } else if contains_ascii_ci(enc, "3-BIT") || contains_ascii_ci(enc, "OCTAL") {
            3
        } else if contains_ascii_ci(enc, "4-BIT") || contains_ascii_ci(enc, "HEX") {
            4
        } else if contains_ascii_ci(enc, "5-BIT") {
            5
        } else if contains_ascii_ci(enc, "6-BIT") || contains_ascii_ci(enc, "6BIT") || contains_ascii_ci(enc, "ICAO") {
            6
        } else if contains_ascii_ci(enc, "7-BIT") {
            7
        } else if contains_ascii_ci(enc, "8-BIT") {
            8
        } else {
            6
        };

        if contains_ascii_ci(enc, "DFI-746")
            || contains_ascii_ci(enc, "DFI-747")
            || contains_ascii_ci(enc, "DFI-336")
            || contains_ascii_ci(enc, "DFI-769")
        {
            return Self::DfiAlpha { bit_width };
        }
        if contains_ascii_ci(enc, "DFI-264-DUI-001") || contains_ascii_ci(enc, "DFI-264.2") {
            return Self::Dfi264Dui001 { bit_width };
        }
        if contains_ascii_ci(enc, "DFI-1661-DUI-001") {
            let bw = if contains_ascii_ci(enc, "5-BIT") { 5 } else { bit_width };
            return Self::Dfi1661Dui001 { bit_width: bw };
        }
        if contains_ascii_ci(enc, "ICAO") {
            return Self::Icao;
        }
        if contains_ascii_ci(enc, "6-BIT") || contains_ascii_ci(enc, "6BIT") {
            return Self::Packed6Bit;
        }
        if contains_ascii_ci(enc, "5-BIT") {
            return Self::Packed5Bit;
        }
        if contains_ascii_ci(enc, "HEX") || contains_ascii_ci(enc, "4-BIT") {
            return Self::Hex;
        }
        if contains_ascii_ci(enc, "OCTAL") || contains_ascii_ci(enc, "3-BIT") {
            return Self::Octal;
        }
        if contains_ascii_ci(enc, "BASE4") || contains_ascii_ci(enc, "2-BIT") {
            return Self::Base4;
        }
        if contains_ascii_ci(enc, "BITS") || contains_ascii_ci(enc, "1-BIT") {
            return Self::Bits;
        }
        Self::Custom(enc.to_string())
    }

    /// Returns the canonical encoding name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Be => "UTF-16BE",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf32Be => "UTF-32BE",
            Self::Utf32Le => "UTF-32LE",
            Self::Ascii => "US-ASCII",
            Self::Iso8859_1 => "ISO-8859-1",
            Self::Iso8859_13 => "ISO-8859-13",
            Self::Ebcdic => "IBM037",
            Self::Bits => "X-DFDL-BITS",
            Self::Base4 => "X-DFDL-BASE4",
            Self::Octal => "X-DFDL-OCTAL",
            Self::Hex => "X-DFDL-HEX",
            Self::Packed5Bit => "X-DFDL-5-BIT-PACKED",
            Self::Packed6Bit => "X-DFDL-US-ASCII-6-BIT-PACKED",
            Self::Icao => "X-DFDL-ICAO-6-BIT",
            Self::Packed7Bit => "X-DFDL-US-ASCII-7-BIT-PACKED",
            Self::Packed8Bit => "X-DFDL-8-BIT-PACKED",
            Self::Packed8BitReverse => "X-DFDL-ISO-8859-1-8-BIT-PACKED-LSB-FIRST-REVERSE",
            Self::DfiAlpha { .. } => "X-DFDL-6-BIT-DFI-746",
            Self::Dfi264Dui001 { .. } => "X-DFDL-6-BIT-DFI-264-DUI-001",
            Self::Dfi1661Dui001 { .. } => "X-DFDL-5-BIT-DFI-1661-DUI-001",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Returns the fixed character width in bits, or `None` for variable-width encodings.
    #[inline]
    #[must_use]
    pub fn char_bits(&self) -> Option<usize> {
        Encoding::char_bits(self)
    }

    /// Returns the alignment/code unit size in bits.
    #[inline]
    #[must_use]
    pub fn unit_bits(&self) -> usize {
        Encoding::unit_bits(self)
    }

    /// Returns the mandatory alignment in bits (MTA).
    #[inline]
    #[must_use]
    pub fn mandatory_alignment_bits(&self) -> usize {
        Encoding::mandatory_alignment_bits(self)
    }

    /// Returns whether this encoding has variable-width characters.
    #[inline]
    #[must_use]
    pub fn is_variable_width(&self) -> bool {
        Encoding::is_variable_width(self)
    }

    /// Returns whether this is a sub-byte encoding (< 8 bits per character).
    #[inline]
    #[must_use]
    pub fn is_sub_byte(&self) -> bool {
        self.char_bits().is_some_and(|cb| cb < 8)
    }

    /// Decodes raw bytes into a String.
    #[inline]
    pub fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        Encoding::decode_text_bytes(self, bytes)
    }

    /// Encodes a text string into bytes according to this encoding.
    #[inline]
    #[must_use]
    pub fn encode_text_string(&self, s: &str) -> Vec<u8> {
        Encoding::encode_text_string(self, s)
    }
}

impl AsRef<str> for EncodingKind {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl core::fmt::Display for EncodingKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for EncodingKind {
    type Err = core::convert::Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse_name(s))
    }
}

impl From<&str> for EncodingKind {
    #[inline]
    fn from(s: &str) -> Self {
        Self::parse_name(s)
    }
}

impl From<String> for EncodingKind {
    #[inline]
    fn from(s: String) -> Self {
        Self::parse_name(&s)
    }
}

impl From<&String> for EncodingKind {
    #[inline]
    fn from(s: &String) -> Self {
        Self::parse_name(s.as_str())
    }
}

impl Encoding for EncodingKind {
    #[inline]
    fn name(&self) -> &str {
        self.as_str()
    }

    #[inline]
    fn decode_text_bytes(&self, bytes: &[u8]) -> DFDLResult<String> {
        match self {
            Self::Utf8 => Utf8Encoding.decode_text_bytes(bytes),
            Self::Utf16Be => Utf16Encoding::BE.decode_text_bytes(bytes),
            Self::Utf16Le => Utf16Encoding::LE.decode_text_bytes(bytes),
            Self::Utf32Be => Utf32Encoding::BE.decode_text_bytes(bytes),
            Self::Utf32Le => Utf32Encoding::LE.decode_text_bytes(bytes),
            Self::Ascii => AsciiEncoding.decode_text_bytes(bytes),
            Self::Iso8859_1 => Iso8859_1Encoding.decode_text_bytes(bytes),
            Self::Iso8859_13 => Iso8859_13Encoding.decode_text_bytes(bytes),
            Self::Ebcdic => EbcdicEncoding.decode_text_bytes(bytes),
            Self::Bits => SubByteEncoding::BITS.decode_text_bytes(bytes),
            Self::Base4 => SubByteEncoding::BASE4.decode_text_bytes(bytes),
            Self::Octal => SubByteEncoding::OCTAL.decode_text_bytes(bytes),
            Self::Hex => SubByteEncoding::HEX.decode_text_bytes(bytes),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.decode_text_bytes(bytes),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.decode_text_bytes(bytes),
            Self::Icao => SubByteEncoding::ICAO.decode_text_bytes(bytes),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.decode_text_bytes(bytes),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.decode_text_bytes(bytes),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.decode_text_bytes(bytes),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.decode_text_bytes(bytes),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.decode_text_bytes(bytes),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.decode_text_bytes(bytes),
            Self::Custom(name) => CustomEncoding::new(name.clone()).decode_text_bytes(bytes),
        }
    }

    #[inline]
    fn decode_text_bytes_with_offsets(&self, bytes: &[u8]) -> (String, Vec<usize>) {
        match self {
            Self::Utf8 => Utf8Encoding.decode_text_bytes_with_offsets(bytes),
            Self::Utf16Be => Utf16Encoding::BE.decode_text_bytes_with_offsets(bytes),
            Self::Utf16Le => Utf16Encoding::LE.decode_text_bytes_with_offsets(bytes),
            Self::Utf32Be => Utf32Encoding::BE.decode_text_bytes_with_offsets(bytes),
            Self::Utf32Le => Utf32Encoding::LE.decode_text_bytes_with_offsets(bytes),
            Self::Ascii => AsciiEncoding.decode_text_bytes_with_offsets(bytes),
            Self::Iso8859_1 => Iso8859_1Encoding.decode_text_bytes_with_offsets(bytes),
            Self::Iso8859_13 => Iso8859_13Encoding.decode_text_bytes_with_offsets(bytes),
            Self::Ebcdic => EbcdicEncoding.decode_text_bytes_with_offsets(bytes),
            Self::Bits => SubByteEncoding::BITS.decode_text_bytes_with_offsets(bytes),
            Self::Base4 => SubByteEncoding::BASE4.decode_text_bytes_with_offsets(bytes),
            Self::Octal => SubByteEncoding::OCTAL.decode_text_bytes_with_offsets(bytes),
            Self::Hex => SubByteEncoding::HEX.decode_text_bytes_with_offsets(bytes),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.decode_text_bytes_with_offsets(bytes),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.decode_text_bytes_with_offsets(bytes),
            Self::Icao => SubByteEncoding::ICAO.decode_text_bytes_with_offsets(bytes),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.decode_text_bytes_with_offsets(bytes),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.decode_text_bytes_with_offsets(bytes),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.decode_text_bytes_with_offsets(bytes),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.decode_text_bytes_with_offsets(bytes),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.decode_text_bytes_with_offsets(bytes),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.decode_text_bytes_with_offsets(bytes),
            Self::Custom(name) => CustomEncoding::new(name.clone()).decode_text_bytes_with_offsets(bytes),
        }
    }

    #[inline]
    fn encode_text_string(&self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => Utf8Encoding.encode_text_string(text),
            Self::Utf16Be => Utf16Encoding::BE.encode_text_string(text),
            Self::Utf16Le => Utf16Encoding::LE.encode_text_string(text),
            Self::Utf32Be => Utf32Encoding::BE.encode_text_string(text),
            Self::Utf32Le => Utf32Encoding::LE.encode_text_string(text),
            Self::Ascii => AsciiEncoding.encode_text_string(text),
            Self::Iso8859_1 => Iso8859_1Encoding.encode_text_string(text),
            Self::Iso8859_13 => Iso8859_13Encoding.encode_text_string(text),
            Self::Ebcdic => EbcdicEncoding.encode_text_string(text),
            Self::Bits => SubByteEncoding::BITS.encode_text_string(text),
            Self::Base4 => SubByteEncoding::BASE4.encode_text_string(text),
            Self::Octal => SubByteEncoding::OCTAL.encode_text_string(text),
            Self::Hex => SubByteEncoding::HEX.encode_text_string(text),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.encode_text_string(text),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.encode_text_string(text),
            Self::Icao => SubByteEncoding::ICAO.encode_text_string(text),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.encode_text_string(text),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.encode_text_string(text),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.encode_text_string(text),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.encode_text_string(text),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.encode_text_string(text),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.encode_text_string(text),
            Self::Custom(name) => CustomEncoding::new(name.clone()).encode_text_string(text),
        }
    }

    #[inline]
    fn find_unmappable_char(&self, text: &str) -> Option<char> {
        match self {
            Self::Utf8 => Utf8Encoding.find_unmappable_char(text),
            Self::Utf16Be => Utf16Encoding::BE.find_unmappable_char(text),
            Self::Utf16Le => Utf16Encoding::LE.find_unmappable_char(text),
            Self::Utf32Be => Utf32Encoding::BE.find_unmappable_char(text),
            Self::Utf32Le => Utf32Encoding::LE.find_unmappable_char(text),
            Self::Ascii => AsciiEncoding.find_unmappable_char(text),
            Self::Iso8859_1 => Iso8859_1Encoding.find_unmappable_char(text),
            Self::Iso8859_13 => Iso8859_13Encoding.find_unmappable_char(text),
            Self::Ebcdic => EbcdicEncoding.find_unmappable_char(text),
            Self::Bits => SubByteEncoding::BITS.find_unmappable_char(text),
            Self::Base4 => SubByteEncoding::BASE4.find_unmappable_char(text),
            Self::Octal => SubByteEncoding::OCTAL.find_unmappable_char(text),
            Self::Hex => SubByteEncoding::HEX.find_unmappable_char(text),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.find_unmappable_char(text),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.find_unmappable_char(text),
            Self::Icao => SubByteEncoding::ICAO.find_unmappable_char(text),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.find_unmappable_char(text),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.find_unmappable_char(text),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.find_unmappable_char(text),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.find_unmappable_char(text),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.find_unmappable_char(text),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.find_unmappable_char(text),
            Self::Custom(name) => CustomEncoding::new(name.clone()).find_unmappable_char(text),
        }
    }

    #[inline]
    fn has_malformed_input(&self, bytes: &[u8]) -> bool {
        match self {
            Self::Utf8 => Utf8Encoding.has_malformed_input(bytes),
            _ => false,
        }
    }

    #[inline]
    fn char_bits(&self) -> Option<usize> {
        match self {
            Self::Bits => Some(1),
            Self::Base4 => Some(2),
            Self::Octal => Some(3),
            Self::Hex => Some(4),
            Self::Packed5Bit => Some(5),
            Self::Packed6Bit | Self::Icao => Some(6),
            Self::DfiAlpha { bit_width }
            | Self::Dfi264Dui001 { bit_width }
            | Self::Dfi1661Dui001 { bit_width } => Some(*bit_width),
            Self::Packed7Bit => Some(7),
            Self::Packed8Bit | Self::Packed8BitReverse => Some(8),
            _ => None,
        }
    }

    #[inline]
    fn unit_bits(&self) -> usize {
        match self {
            Self::Utf16Be | Self::Utf16Le => 16,
            Self::Utf32Be | Self::Utf32Le => 32,
            Self::Bits
            | Self::Base4
            | Self::Octal
            | Self::Hex
            | Self::Packed5Bit
            | Self::Packed6Bit
            | Self::Icao
            | Self::Packed7Bit
            | Self::Packed8Bit
            | Self::Packed8BitReverse
            | Self::DfiAlpha { .. }
            | Self::Dfi264Dui001 { .. }
            | Self::Dfi1661Dui001 { .. } => 1,
            _ => 8,
        }
    }

    #[inline]
    fn mandatory_alignment_bits(&self) -> usize {
        match self {
            Self::Bits
            | Self::Base4
            | Self::Octal
            | Self::Hex
            | Self::Packed5Bit
            | Self::Packed6Bit
            | Self::Icao
            | Self::Packed7Bit
            | Self::Packed8Bit
            | Self::Packed8BitReverse
            | Self::DfiAlpha { .. }
            | Self::Dfi264Dui001 { .. }
            | Self::Dfi1661Dui001 { .. } => 1,
            _ => 8,
        }
    }

    #[inline]
    fn is_variable_width(&self) -> bool {
        match self {
            Self::Utf8 => true,
            Self::Custom(ref s) => {
                let lower = s.to_ascii_lowercase();
                let norm = lower.replace(['-', '_'], "");
                norm == "utf8"
                    || norm.starts_with("gb18030")
                    || norm.starts_with("shiftjis")
                    || norm.starts_with("euc")
            }
            _ => false,
        }
    }

    #[inline]
    fn decode_sub_byte_char(&self, code: u64) -> char {
        match self {
            Self::Bits => SubByteEncoding::BITS.decode_sub_byte_char(code),
            Self::Base4 => SubByteEncoding::BASE4.decode_sub_byte_char(code),
            Self::Octal => SubByteEncoding::OCTAL.decode_sub_byte_char(code),
            Self::Hex => SubByteEncoding::HEX.decode_sub_byte_char(code),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.decode_sub_byte_char(code),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.decode_sub_byte_char(code),
            Self::Icao => SubByteEncoding::ICAO.decode_sub_byte_char(code),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.decode_sub_byte_char(code),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.decode_sub_byte_char(code),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.decode_sub_byte_char(code),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.decode_sub_byte_char(code),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.decode_sub_byte_char(code),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.decode_sub_byte_char(code),
            _ => core::char::from_u32(code as u32).unwrap_or('?'),
        }
    }

    #[inline]
    fn strict_sub_byte_code(&self, ch: char) -> Option<u64> {
        match self {
            Self::Bits => SubByteEncoding::BITS.strict_sub_byte_code(ch),
            Self::Base4 => SubByteEncoding::BASE4.strict_sub_byte_code(ch),
            Self::Octal => SubByteEncoding::OCTAL.strict_sub_byte_code(ch),
            Self::Hex => SubByteEncoding::HEX.strict_sub_byte_code(ch),
            Self::Packed5Bit => SubByteEncoding::PACKED_5BIT.strict_sub_byte_code(ch),
            Self::Packed6Bit => SubByteEncoding::PACKED_6BIT.strict_sub_byte_code(ch),
            Self::Icao => SubByteEncoding::ICAO.strict_sub_byte_code(ch),
            Self::Packed7Bit => SubByteEncoding::PACKED_7BIT.strict_sub_byte_code(ch),
            Self::Packed8Bit => SubByteEncoding::PACKED_8BIT.strict_sub_byte_code(ch),
            Self::Packed8BitReverse => SubByteEncoding::PACKED_8BIT_REVERSE.strict_sub_byte_code(ch),
            Self::DfiAlpha { bit_width } => SubByteEncoding { variant: SubByteVariant::DfiAlpha { bit_width: *bit_width } }.strict_sub_byte_code(ch),
            Self::Dfi264Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi264Dui001 { bit_width: *bit_width } }.strict_sub_byte_code(ch),
            Self::Dfi1661Dui001 { bit_width } => SubByteEncoding { variant: SubByteVariant::Dfi1661Dui001 { bit_width: *bit_width } }.strict_sub_byte_code(ch),
            _ => self.char_bits().map(|_| ch as u64),
        }
    }

    #[inline]
    fn sub_byte_replacement_code(&self) -> u64 {
        match self {
            Self::Packed5Bit => 29,
            Self::Icao => 32,
            Self::Packed6Bit => 0x1F,
            Self::Packed7Bit => 0x3F,
            _ => 0,
        }
    }
}

/// Conversion trait for types that can be resolved into an [`EncodingKind`].
pub trait IntoEncoding {
    /// Converts `self` into an [`EncodingKind`].
    fn into_encoding(self) -> EncodingKind;
}

impl IntoEncoding for EncodingKind {
    #[inline]
    fn into_encoding(self) -> EncodingKind {
        self
    }
}

impl IntoEncoding for &EncodingKind {
    #[inline]
    fn into_encoding(self) -> EncodingKind {
        self.clone()
    }
}

impl IntoEncoding for &str {
    #[inline]
    fn into_encoding(self) -> EncodingKind {
        EncodingKind::parse_name(self)
    }
}

impl IntoEncoding for &String {
    #[inline]
    fn into_encoding(self) -> EncodingKind {
        EncodingKind::parse_name(self.as_str())
    }
}

/// Decodes text bytes into a Unicode [`String`] according to the given encoding.
///
/// # Errors
/// Returns [`crate::error::DFDLError`] if decoding fails under strict error policy.
#[inline]
pub fn decode_text_bytes(bytes: &[u8], encoding: impl IntoEncoding) -> DFDLResult<String> {
    encoding.into_encoding().decode_text_bytes(bytes)
}

/// Encodes a Unicode string into raw bytes according to the given encoding.
#[inline]
pub fn encode_text_string(text: &str, encoding: impl IntoEncoding) -> Vec<u8> {
    encoding.into_encoding().encode_text_string(text)
}

/// Returns the first character of `text` that `encoding` cannot represent, if any.
#[must_use]
#[inline]
pub fn find_unmappable_char(text: &str, encoding: impl IntoEncoding) -> Option<char> {
    encoding.into_encoding().find_unmappable_char(text)
}

/// Decodes text bytes into a Unicode string while tracking source byte offsets.
#[must_use]
#[inline]
pub fn decode_text_bytes_with_offsets(bytes: &[u8], encoding: impl IntoEncoding) -> (String, Vec<usize>) {
    encoding.into_encoding().decode_text_bytes_with_offsets(bytes)
}

/// Returns `true` when `bytes` are not valid for `encoding`.
#[must_use]
#[inline]
pub fn has_malformed_input(bytes: &[u8], encoding: impl IntoEncoding) -> bool {
    encoding.into_encoding().has_malformed_input(bytes)
}

/// Returns the number of bits per character code unit if the encoding uses sub-byte widths.
#[must_use]
#[inline]
pub fn encoding_char_bits(encoding: impl IntoEncoding) -> Option<usize> {
    encoding.into_encoding().char_bits()
}

/// Returns the mandatory character alignment / unit width in bits for the given encoding.
#[must_use]
#[inline]
pub fn encoding_unit_bits(encoding: impl IntoEncoding) -> usize {
    encoding.into_encoding().unit_bits()
}

/// Returns the mandatory text alignment in bits for a given character set encoding.
#[must_use]
#[inline]
pub fn encoding_mandatory_alignment_bits(encoding: impl IntoEncoding) -> usize {
    encoding.into_encoding().mandatory_alignment_bits()
}

/// Decodes an integer code point into a character for sub-byte encodings.
#[must_use]
#[inline]
pub fn decode_sub_byte_char(code: u64, encoding: impl IntoEncoding) -> char {
    encoding.into_encoding().decode_sub_byte_char(code)
}

/// Strictly maps a character to its code in a sub-byte encoding (`None` if unmappable).
#[must_use]
#[inline]
pub fn strict_sub_byte_code(ch: char, encoding: impl IntoEncoding) -> Option<u64> {
    encoding.into_encoding().strict_sub_byte_code(ch)
}

/// Code used in place of an unmappable character (encodingErrorPolicy `replace`).
#[must_use]
#[inline]
pub fn sub_byte_replacement_code(encoding: impl IntoEncoding) -> u64 {
    encoding.into_encoding().sub_byte_replacement_code()
}

/// Encodes a character into its integer code point and bit length for sub-byte encodings.
#[must_use]
#[inline]
pub fn encode_sub_byte_char(ch: char, encoding: impl IntoEncoding) -> Option<(u64, usize)> {
    encoding.into_encoding().encode_sub_byte_char(ch)
}

/// Returns true if the encoding is a variable-width character encoding (such as UTF-8).
#[must_use]
#[inline]
pub fn is_variable_width_encoding(encoding: impl IntoEncoding) -> bool {
    encoding.into_encoding().is_variable_width()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_encoding_kind_parsing_and_traits() {
        assert_eq!(EncodingKind::parse_name("UTF-8"), EncodingKind::Utf8);
        assert_eq!(EncodingKind::parse_name("US-ASCII"), EncodingKind::Ascii);
        assert_eq!(EncodingKind::parse_name("ISO-8859-1"), EncodingKind::Iso8859_1);
        assert_eq!(EncodingKind::parse_name("UTF-16BE"), EncodingKind::Utf16Be);
        assert_eq!(EncodingKind::parse_name("UTF-16LE"), EncodingKind::Utf16Le);
        assert_eq!(EncodingKind::parse_name("UTF-32BE"), EncodingKind::Utf32Be);
        assert_eq!(EncodingKind::parse_name("UTF-32LE"), EncodingKind::Utf32Le);
        assert_eq!(EncodingKind::parse_name("IBM037"), EncodingKind::Ebcdic);
        assert_eq!(EncodingKind::parse_name("X-DFDL-BITS"), EncodingKind::Bits);
        assert_eq!(EncodingKind::parse_name("X-DFDL-5-BIT-PACKED"), EncodingKind::Packed5Bit);
        assert_eq!(EncodingKind::parse_name("CUSTOM_ENC"), EncodingKind::Custom("CUSTOM_ENC".into()));

        let utf8 = EncodingKind::Utf8;
        assert_eq!(utf8.unit_bits(), 8);
        assert_eq!(utf8.mandatory_alignment_bits(), 8);
        assert!(utf8.is_variable_width());
        assert_eq!(utf8.char_bits(), None);

        let sub = EncodingKind::Bits;
        assert_eq!(sub.unit_bits(), 1);
        assert_eq!(sub.mandatory_alignment_bits(), 1);
        assert_eq!(sub.char_bits(), Some(1));
    }

    #[test]
    fn test_packed_5bit_mapping() {
        let enc = "X-DFDL-5-BIT-PACKED";
        assert_eq!(decode_sub_byte_char(0, enc), '0');
        assert_eq!(decode_sub_byte_char(8, enc), 'A');
        assert_eq!(decode_sub_byte_char(16, enc), 'J');
        assert_eq!(decode_sub_byte_char(31, enc), 'Z');
        assert_eq!(encode_sub_byte_char('K', enc), Some((17, 5)));
        assert_eq!(encode_sub_byte_char('O', enc), Some((0, 5)));
        assert_eq!(encode_sub_byte_char('i', enc), Some((1, 5)));
        assert_eq!(encode_sub_byte_char('!', enc), Some((29, 5)));
        for code in 0..32u64 {
            let ch = decode_sub_byte_char(code, enc);
            assert_eq!(encode_sub_byte_char(ch, enc), Some((code, 5)));
        }
    }

    #[test]
    fn test_sub_byte_strict_and_replacement() {
        assert_eq!(strict_sub_byte_code('\u{A3}', "X-DFDL-US-ASCII-7-BIT-PACKED"), None);
        assert_eq!(strict_sub_byte_code('B', "X-DFDL-US-ASCII-7-BIT-PACKED"), Some(0x42));
        assert_eq!(strict_sub_byte_code('9', "X-DFDL-OCTAL-LSBF"), None);
        assert_eq!(strict_sub_byte_code('7', "X-DFDL-OCTAL-LSBF"), Some(7));
        assert_eq!(strict_sub_byte_code('g', "X-DFDL-HEX-LSBF"), None);
        assert_eq!(strict_sub_byte_code('a', "X-DFDL-US-ASCII-6-BIT-PACKED"), None);
        assert_eq!(strict_sub_byte_code('!', "X-DFDL-5-BIT-PACKED-LSBF"), None);
        assert_eq!(sub_byte_replacement_code("X-DFDL-US-ASCII-7-BIT-PACKED"), 0x3F);
        assert_eq!(sub_byte_replacement_code("X-DFDL-US-ASCII-6-BIT-PACKED"), 0x1F);
        assert_eq!(sub_byte_replacement_code("X-DFDL-OCTAL-LSBF"), 0);
        assert_eq!(
            encode_sub_byte_char('\u{A3}', "X-DFDL-US-ASCII-7-BIT-PACKED"),
            Some((0x3F, 7))
        );
    }

    #[test]
    fn test_packed_6bit_table_round_trip() {
        let enc = "X-DFDL-US-ASCII-6-BIT-PACKED";
        assert_eq!(decode_sub_byte_char(1, enc), 'A');
        assert_eq!(decode_sub_byte_char(0x1F, enc), '_');
        assert_eq!(decode_sub_byte_char(0x20, enc), ' ');
        for code in 0..64u64 {
            let ch = decode_sub_byte_char(code, enc);
            assert_eq!(strict_sub_byte_code(ch, enc), Some(code));
        }
    }

    #[test]
    fn test_find_unmappable_char_byte_encodings() {
        assert_eq!(find_unmappable_char("a\u{A3}", "US-ASCII"), Some('\u{A3}'));
        assert_eq!(find_unmappable_char("a\u{A3}", "ISO-8859-1"), None);
        assert_eq!(find_unmappable_char("a\u{20AC}", "ISO-8859-1"), Some('\u{20AC}'));
        assert_eq!(find_unmappable_char("a\u{20AC}", "UTF-8"), None);
        assert_eq!(find_unmappable_char("a", "IBM037"), None);
        assert_eq!(find_unmappable_char("\u{20AC}", "IBM037"), Some('\u{20AC}'));
        assert_eq!(encode_text_string("\u{20AC}", "ISO-8859-1"), [b'?']);
        assert_eq!(encode_text_string("\u{A3}", "US-ASCII"), [b'?']);
    }

    #[test]
    fn test_iso_8859_13_roundtrip() {
        let text = "Ą1234567";
        let encoded = encode_text_string(text, "ISO-8859-13");
        assert_eq!(encoded, [0xC0, b'1', b'2', b'3', b'4', b'5', b'6', b'7']);
        let decoded = decode_text_bytes(&encoded, "ISO-8859-13").unwrap();
        assert_eq!(decoded, text);
        assert_eq!(find_unmappable_char(text, "ISO-8859-13"), None);
        assert_eq!(find_unmappable_char("€", "ISO-8859-13"), Some('€'));
    }

    #[test]
    fn test_utf8_lossy_with_offsets() {
        let valid = b"hello";
        let (s, off) = decode_text_bytes_with_offsets(valid, "UTF-8");
        assert_eq!(s, "hello");
        assert_eq!(off, vec![0, 1, 2, 3, 4, 5]);

        let invalid = b"hello\xFFworld";
        assert!(has_malformed_input(invalid, "UTF-8"));
        let (s2, off2) = decode_text_bytes_with_offsets(invalid, "UTF-8");
        assert_eq!(s2, "hello\u{FFFD}world");
        assert_eq!(off2, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    }
}
