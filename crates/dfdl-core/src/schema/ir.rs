//! Compiled Schema Intermediate Representation (IR).
//!
//! Provides immutable, executable schema graph components independent of XML DOM structures.
//! Aligned with DFDL 1.0 §§5–8 semantic model.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::value::{DfdlSimpleType, DfdlValue};
use crate::io::traits::{BitOrder, ByteOrder};
use crate::types::QName;

/// Stable internal identifier for executable schema graph nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

/// DFDL Representation property (textual vs binary physical representation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Representation {
    /// Text physical representation (ASCII, UTF-8, etc.).
    #[default]
    Text,
    /// Binary physical representation (IEEE float, binary int, BCD).
    Binary,
}

/// DFDL binaryNumberRep property (binary, packed, bcd, ibm4690Packed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BinaryNumberRep {
    /// Standard binary integer/float.
    #[default]
    Binary,
    /// IBM Comp-3 Packed decimal.
    Packed,
    /// Binary Coded Decimal.
    Bcd,
    /// IBM 4690 Packed decimal.
    Ibm4690Packed,
}

/// DFDL LengthKind property defining length calculation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LengthKind {
    /// Explicit length specified by `dfdl:length` property or constant.
    #[default]
    Explicit,
    /// Implicit length derived from primitive type or element structure.
    Implicit,
    /// Prefixed length read from a preceding prefix element.
    Prefixed,
    /// Expression length evaluated dynamically at runtime.
    Expression,
    /// Delimited length scanned up to initiator/terminator/separator.
    Delimited,
    /// Pattern length specified by matching `dfdl:lengthPattern` regex.
    Pattern,
    /// EndOfParent length scanned up to parent element boundary.
    EndOfParent,
}

/// Position of sequence member separator (§14.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SeparatorPosition {
    /// Infix separator between elements.
    #[default]
    Infix,
    /// Prefix separator before each element.
    Prefix,
    /// Postfix separator after each element.
    Postfix,
}

/// Separator suppression policy for empty elements (§14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SeparatorSuppressionPolicy {
    /// Never suppress separators.
    #[default]
    Never,
    /// Suppress trailing empty element separators.
    TrailingEmpty,
    /// Suppress trailing empty element separators strictly.
    TrailingEmptyStrict,
    /// Suppress any empty element separators.
    AnyEmpty,
}

/// DFDL parse/unparse policy (`dfdl:parseUnparsePolicy` or `dfdlx:parseUnparsePolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParseUnparsePolicy {
    /// Element supports both parsing and unparsing.
    #[default]
    Both,
    /// Element supports parsing only.
    ParseOnly,
    /// Element supports unparsing only.
    UnparseOnly,
}

/// DFDL SequenceKind property defining member ordering strategy (§14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SequenceKind {
    /// Ordered sequence where members must appear in schema definition order.
    #[default]
    Ordered,
    /// Unordered sequence where members may appear in data in arbitrary order.
    Unordered,
}

/// DFDL OccursCountKind property defining array repetition strategy (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OccursCountKind {
    /// Fixed occurrences given by `maxOccurs` / `minOccurs`.
    Fixed,
    /// Implicit occurrences up to maxOccurs or unbounded (§16.1).
    #[default]
    Implicit,
    /// Parsed occurrences up to stream failure, EOF, or delimiter.
    Parsed,
    /// Expression-driven occurrences evaluated dynamically via `dfdl:occursCount`.
    Expression,
    /// Occurrences parse until a stop value is encountered.
    StopValue,
}

/// DFDL NilKind property defining nil representation strategy (§13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NilKind {
    /// Nil value represented by literal value or character.
    #[default]
    LiteralValue,
    /// Nil value represented by literal character.
    LiteralCharacter,
    /// Nil value represented by logical scalar value.
    LogicalValue,
}

/// DFDL NilValueDelimiterPolicy property defining delimiters for nil values (§13.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NilValueDelimiterPolicy {
    /// Both initiator and terminator must be present for nil representation.
    #[default]
    Both,
    /// Initiator must be present; terminator is omitted for nil representation.
    Initiator,
    /// Terminator must be present; initiator is omitted for nil representation.
    Terminator,
    /// Neither initiator nor terminator is present for nil representation.
    None,
}

/// DFDL LengthUnits property (§12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LengthUnits {
    /// Length specified in bytes.
    #[default]
    Bytes,
    /// Length specified in bits.
    Bits,
    /// Length specified in characters.
    Characters,
}

/// DFDL AlignmentUnits property (§12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlignmentUnits {
    /// Alignment boundary specified in bytes (8 bits per unit).
    #[default]
    Bytes,
    /// Alignment boundary specified in bits.
    Bits,
}

/// DFDL AlignmentKind property (`dfdlx:alignmentKind` extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlignmentKind {
    /// Automatic alignment as defined by the DFDL specification.
    #[default]
    Automatic,
    /// Manual alignment disabling automatic alignment skipping.
    Manual,
}

/// DFDL TextTrimKind property (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextTrimKind {
    /// No text trimming performed.
    #[default]
    None,
    /// Trim leading pad characters.
    Head,
    /// Trim trailing pad characters.
    Tail,
    /// Trim both leading and trailing pad characters.
    Both,
}

/// DFDL textPadKind property (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextPadKind {
    /// No padding is performed.
    #[default]
    None,
    /// Pad character is used to pad the representation.
    PadChar,
}

/// DFDL escapeKind property (§13.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EscapeKind {
    /// Single escape character escaping.
    #[default]
    EscapeCharacter,
    /// Block delimiters escaping.
    EscapeBlock,
}

/// DFDL generateEscapeBlock property (§13.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GenerateEscapeBlock {
    /// Escape block generated only when needed.
    #[default]
    WhenNeeded,
    /// Escape block generated always.
    Always,
}

/// DFDL emptyValueDelimiterPolicy property (§12.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EmptyValueDelimiterPolicy {
    /// Neither initiator nor terminator is present when empty.
    None,
    /// Initiator is present; terminator is not present when empty.
    Initiator,
    /// Terminator is present; initiator is not present when empty.
    Terminator,
    /// Both initiator and terminator are present when empty.
    #[default]
    Both,
}

/// An in-scope delimiter paired with its case sensitivity and encoding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InScopeDelimiter {
    /// Literal delimiter text or expression pattern.
    pub text: String,
    /// Case sensitivity (`dfdl:ignoreCase`).
    pub ignore_case: bool,
    /// Encoding under which delimiter characters are encoded.
    pub encoding: String,
}

/// Compiled DFDL escapeScheme definition (§13.2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CompiledEscapeScheme {
    /// Escape kind (escapeCharacter or escapeBlock).
    pub escape_kind: EscapeKind,
    /// Single escape character.
    pub escape_character: Option<String>,
    /// Single escape escape character.
    pub escape_escape_character: Option<String>,
    /// Escape block start string.
    pub escape_block_start: Option<String>,
    /// Escape block end string.
    pub escape_block_end: Option<String>,
    /// Extra characters requiring escaping.
    pub extra_escaped_characters: Vec<char>,
    /// When to generate escape block.
    pub generate_escape_block: GenerateEscapeBlock,
}

impl CompiledEscapeScheme {
    /// Applies this escape scheme to `text` using provided evaluated escape characters and delimiters.
    pub fn escape_text(
        &self,
        text: &str,
        eval_ec: Option<&str>,
        eval_eec: Option<&str>,
        eval_bs: Option<&str>,
        eval_be: Option<&str>,
        delimiters: &[&str],
    ) -> String {
        use alloc::string::ToString;
        match self.escape_kind {
            EscapeKind::EscapeCharacter => {
                let escape_char = match eval_ec.or(self.escape_character.as_deref()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return text.to_string(),
                };
                let escape_escape = eval_eec
                    .or(self.escape_escape_character.as_deref())
                    .filter(|s| !s.is_empty());
                let extra_escaped = &self.extra_escaped_characters;

                let mut sorted_delims: Vec<&str> = delimiters
                    .iter()
                    .copied()
                    .filter(|s| !s.is_empty())
                    .collect();
                sorted_delims.sort_by_key(|a| core::cmp::Reverse(a.len()));
                sorted_delims.dedup();

                let mut result = String::with_capacity(text.len().saturating_mul(2));
                let mut rest = text;

                while !rest.is_empty() {
                    if let Some(ee) = escape_escape {
                        if rest.starts_with(ee) {
                            result.push_str(ee);
                            result.push_str(ee);
                            let advance = ee.len();
                            rest = &rest[advance..];
                            continue;
                        }
                    }

                    if rest.starts_with(escape_char) {
                        let esc_prefix = escape_escape.unwrap_or(escape_char);
                        result.push_str(esc_prefix);
                        result.push_str(escape_char);
                        let advance = escape_char.len();
                        rest = &rest[advance..];
                        continue;
                    }

                    let mut matched_delim = None;
                    for &d in &sorted_delims {
                        if rest.starts_with(d) {
                            matched_delim = Some(d);
                            break;
                        }
                    }
                    if let Some(d) = matched_delim {
                        result.push_str(escape_char);
                        result.push_str(d);
                        let advance = d.len();
                        rest = &rest[advance..];
                        continue;
                    }

                    let first_ch = match rest.chars().next() {
                        Some(c) => c,
                        None => break,
                    };
                    if extra_escaped.contains(&first_ch) {
                        result.push_str(escape_char);
                        result.push(first_ch);
                        let advance = first_ch.len_utf8();
                        rest = &rest[advance..];
                        continue;
                    }

                    result.push(first_ch);
                    let advance = first_ch.len_utf8();
                    rest = &rest[advance..];
                }
                result
            }
            EscapeKind::EscapeBlock => {
                let block_start = match eval_bs.or(self.escape_block_start.as_deref()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return text.to_string(),
                };
                let block_end = match eval_be.or(self.escape_block_end.as_deref()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return text.to_string(),
                };
                let escape_escape = eval_eec
                    .or(self.escape_escape_character.as_deref())
                    .filter(|s| !s.is_empty());
                let extra_escaped = &self.extra_escaped_characters;

                let needs_block = match self.generate_escape_block {
                    GenerateEscapeBlock::Always => true,
                    GenerateEscapeBlock::WhenNeeded => {
                        let contains_delim = delimiters
                            .iter()
                            .any(|d| !d.is_empty() && text.contains(d));
                        let starts_with_start = text.starts_with(block_start);
                        let contains_end = text.contains(block_end);
                        let contains_extra = text.chars().any(|c| extra_escaped.contains(&c));
                        contains_delim || starts_with_start || contains_end || contains_extra
                    }
                };

                if !needs_block {
                    return text.to_string();
                }

                let mut result = String::with_capacity(
                    text.len()
                        .saturating_add(block_start.len())
                        .saturating_add(block_end.len())
                        .saturating_add(8),
                );
                result.push_str(block_start);

                let mut rest = text;
                while !rest.is_empty() {
                    if let Some(ee) = escape_escape {
                        if rest.starts_with(ee) {
                            result.push_str(ee);
                            result.push_str(ee);
                            let advance = ee.len();
                            rest = &rest[advance..];
                            continue;
                        }
                    }

                    if rest.starts_with(block_end) {
                        let esc_prefix = escape_escape.unwrap_or(block_end);
                        result.push_str(esc_prefix);
                        result.push_str(block_end);
                        let advance = block_end.len();
                        rest = &rest[advance..];
                        continue;
                    }

                    let first_ch = match rest.chars().next() {
                        Some(c) => c,
                        None => break,
                    };
                    result.push(first_ch);
                    let advance = first_ch.len_utf8();
                    rest = &rest[advance..];
                }
                result.push_str(block_end);
                result
            }
        }
    }
}

/// DFDL text justification property (§13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextJustification {
    /// Default or no justification specified.
    #[default]
    None,
    /// Left justified (pad on right, truncate from right).
    Left,
    /// Right justified (pad on left, truncate from left).
    Right,
    /// Center justified (pad evenly, truncate evenly).
    Center,
}

/// DFDL textNumberRoundingMode property (§13.7.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextNumberRoundingMode {
    /// Round towards positive infinity.
    RoundCeiling,
    /// Round towards negative infinity.
    RoundFloor,
    /// Round towards zero (truncation).
    RoundDown,
    /// Round away from zero.
    RoundUp,
    /// Round to nearest neighbour, or even if equidistant (default).
    #[default]
    RoundHalfEven,
    /// Round to nearest neighbour, or down if equidistant.
    RoundHalfDown,
    /// Round to nearest neighbour, or up if equidistant.
    RoundHalfUp,
    /// Error if rounding is required.
    RoundUnnecessary,
}

/// Kind of DFDL assertion or discriminator test (§15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestKind {
    /// Test expression in DFDL XPath syntax `{ ... }` (default).
    #[default]
    Expression,
    /// Test pattern in regular expression syntax.
    Pattern,
}

/// DFDL failureType property for assertions (§7.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailureType {
    /// Assertion failure is a fatal processing error.
    #[default]
    ProcessingError,
    /// Assertion failure is a recoverable error (diagnostic/validation).
    RecoverableError,
}

/// Compiled DFDL assertion declaration (§15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAssert {
    /// Kind of test (`Expression` or `Pattern`).
    pub test_kind: TestKind,
    /// Test expression or pattern string.
    pub test_expr: String,
    /// Optional failure diagnostic message string.
    pub message: Option<String>,
    /// Failure type (`ProcessingError` or `RecoverableError`).
    pub failure_type: FailureType,
}

/// DFDL textNumberCheckPolicy property (§13.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextNumberCheckPolicy {
    /// Lax number parsing tolerates loose formatting, hex prefixes, loose commas.
    #[default]
    Lax,
    /// Strict number parsing requires exact ICU DecimalFormat pattern matching.
    Strict,
}

/// DFDL textNumberRep property defining text representation kind (§13.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextNumberRep {
    /// Standard numbers formatted with textNumberPattern (§13.7.1).
    #[default]
    Standard,
    /// Zoned decimal numbers with overpunched signs (§13.7.2).
    Zoned,
}

/// DFDL textZonedSignStyle property defining overpunch code style (§13.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextZonedSignStyle {
    /// Standard ASCII overpunch characters ('p'..='y').
    #[default]
    AsciiStandard,
    /// Translated EBCDIC overpunch characters ('}'/'{' and 'J'..'R'/'A'..'I').
    AsciiTranslatedEbcdic,
    /// CA Realia modified overpunch characters (' ', '!'..=')').
    AsciiCaRealiaModified,
    /// Tandem modified overpunch characters (0x80..=0x89).
    AsciiTandemModified,
}

/// DFDL calendarCheckPolicy property controlling calendar date/time rollover (§13.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CalendarCheckPolicy {
    /// Strict verification of days, months, and hours.
    Strict,
    /// Lax calendar parsing with date/time rollover normalization.
    #[default]
    Lax,
}

/// DFDL calendarFirstDayOfWeek property defining the first day of the week (§13.14.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CalendarFirstDayOfWeek {
    /// Sunday is the first day of the week (default in DFDL).
    #[default]
    Sunday,
    /// Monday is the first day of the week.
    Monday,
    /// Tuesday is the first day of the week.
    Tuesday,
    /// Wednesday is the first day of the week.
    Wednesday,
    /// Thursday is the first day of the week.
    Thursday,
    /// Friday is the first day of the week.
    Friday,
    /// Saturday is the first day of the week.
    Saturday,
}

/// DFDL binaryCalendarRep property defining binary date/time serialization (§13.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BinaryCalendarRep {
    /// 32-bit signed integer count of seconds since binaryCalendarEpoch (§13.11).
    #[default]
    BinarySeconds,
    /// 64-bit signed integer count of milliseconds since binaryCalendarEpoch (§13.11).
    BinaryMilliseconds,
    /// Binary Coded Decimal format.
    Bcd,
    /// Packed decimal format.
    Packed,
    /// IBM 4690 packed decimal format.
    Ibm4690Packed,
}

/// DFDL emptyElementParsePolicy property (§9.4, §13.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EmptyElementParsePolicy {
    /// When empty representation is parsed, element is created with empty value.
    #[default]
    TreatAsEmpty,
    /// When empty representation is parsed, optional element is absent; required element without default is an error.
    TreatAsAbsent,
}

/// Strongly-typed descriptor for `dfdl:prefixLengthType` (§12.3.4).
///
/// Encapsulates the resolved physical representation, width, units, facets,
/// and padding character of a prefix length element instead of ad-hoc string formatting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrefixLengthDescriptor {
    /// Type or element name reference (e.g. "xs:unsignedShort", "threeByteTextInt").
    pub name: String,
    /// Physical representation (Binary or Text).
    pub representation: Representation,
    /// Explicit length if defined on simple type or facet.
    pub length: Option<usize>,
    /// Units of length (Bits, Bytes, Characters).
    pub length_units: LengthUnits,
    /// Optional minInclusive facet constraint.
    pub min_inclusive: Option<i64>,
    /// Optional maxInclusive facet constraint.
    pub max_inclusive: Option<i64>,
    /// Optional padding character for text numbers.
    pub pad_char: Option<char>,
    /// Nested prefix length descriptor for chained prefix lengths (§12.3.4).
    pub nested: Option<Box<PrefixLengthDescriptor>>,
}

impl PrefixLengthDescriptor {
    /// Returns the prefix length in bits.
    #[inline]
    #[must_use]
    pub fn prefix_bits(&self) -> usize {
        let num_len = self.length.unwrap_or_else(|| {
            let clean = self.name.split(':').next_back().unwrap_or(&self.name);
            match clean {
                "byte" | "unsignedByte" => 1,
                "short" | "unsignedShort" => 2,
                "int" | "unsignedInt" => 4,
                "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                _ => 2,
            }
        });
        if self.length_units == LengthUnits::Bits {
            num_len
        } else {
            num_len.saturating_mul(8)
        }
    }

    /// Returns the prefix length in whole bytes (rounded up).
    #[inline]
    #[must_use]
    pub fn prefix_bytes(&self) -> usize {
        self.prefix_bits().div_ceil(8)
    }

    /// Returns whether the prefix length is text-represented.
    #[inline]
    #[must_use]
    pub fn is_text(&self) -> bool {
        self.representation == Representation::Text
    }

    /// Deserializes a descriptor string into a strongly-typed [`PrefixLengthDescriptor`].
    ///
    /// Accepts both legacy colon-delimited string representations (e.g.
    /// `name:rep:len:units:min:max:pad` or `@nested@`) and simple primitive type names.
    #[must_use]
    pub fn from_legacy_desc(desc: &str) -> Self {
        if desc.starts_with('@') && desc.ends_with('@') && desc.len() >= 2 {
            let inner = &desc[1..desc.len().saturating_sub(1)];
            let nparts: Vec<&str> = inner.split(',').collect();
            let name = nparts.first().copied().unwrap_or("").to_string();
            let rep = if nparts.get(1).copied().unwrap_or("binary").eq_ignore_ascii_case("text") {
                Representation::Text
            } else {
                Representation::Binary
            };
            let length = nparts.get(2).and_then(|s| s.parse::<usize>().ok());
            let units = if nparts.get(3).copied().unwrap_or("bytes").eq_ignore_ascii_case("bits") {
                LengthUnits::Bits
            } else {
                LengthUnits::Bytes
            };
            let min_inc = nparts.get(4).and_then(|s| s.parse::<i64>().ok());
            let max_inc = nparts.get(5).and_then(|s| s.parse::<i64>().ok());
            return Self {
                name,
                representation: rep,
                length,
                length_units: units,
                min_inclusive: min_inc,
                max_inclusive: max_inc,
                pad_char: None,
                nested: None,
            };
        }

        let parts: Vec<&str> = desc.split(':').collect();
        if parts.len() >= 4 {
            let name = parts.first().copied().unwrap_or("").to_string();
            let rep = if parts.get(1).copied().unwrap_or("binary").eq_ignore_ascii_case("text") {
                Representation::Text
            } else {
                Representation::Binary
            };
            let len_part = parts.get(2).copied().unwrap_or("");
            let (length, nested) = if len_part.starts_with('@') && len_part.ends_with('@') && len_part.len() >= 2 {
                (None, Some(Box::new(Self::from_legacy_desc(len_part))))
            } else {
                (len_part.parse::<usize>().ok(), None)
            };
            let units = if parts.get(3).copied().unwrap_or("bytes").eq_ignore_ascii_case("bits") {
                LengthUnits::Bits
            } else {
                LengthUnits::Bytes
            };
            let min_inc = parts.get(4).and_then(|s| s.parse::<i64>().ok());
            let max_inc = parts.get(5).and_then(|s| s.parse::<i64>().ok());
            let pad_char = parts.get(6).and_then(|s| s.chars().next());
            Self {
                name,
                representation: rep,
                length,
                length_units: units,
                min_inclusive: min_inc,
                max_inclusive: max_inc,
                pad_char,
                nested,
            }
        } else {
            let clean = parts.last().copied().unwrap_or(desc);
            let byte_len = match clean {
                "byte" | "unsignedByte" => 1,
                "short" | "unsignedShort" => 2,
                "int" | "unsignedInt" => 4,
                "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                _ => 2,
            };
            Self {
                name: desc.to_string(),
                representation: Representation::Binary,
                length: Some(byte_len),
                length_units: LengthUnits::Bytes,
                min_inclusive: None,
                max_inclusive: None,
                pad_char: None,
                nested: None,
            }
        }
    }

    /// Serializes this descriptor to the legacy colon-delimited string format.
    #[must_use]
    pub fn to_legacy_desc(&self) -> String {
        let rep_str = if self.is_text() { "text" } else { "binary" };
        let units_str = match self.length_units {
            LengthUnits::Bits => "bits",
            LengthUnits::Bytes => "bytes",
            LengthUnits::Characters => "characters",
        };
        let len_str = if let Some(ref n) = self.nested {
            let n_rep = if n.is_text() { "text" } else { "binary" };
            let n_units = match n.length_units {
                LengthUnits::Bits => "bits",
                LengthUnits::Bytes => "bytes",
                LengthUnits::Characters => "characters",
            };
            let n_len = n.length.map(|l| alloc::format!("{l}")).unwrap_or_else(|| "1".to_string());
            let n_min = n.min_inclusive.map(|v| alloc::format!("{v}")).unwrap_or_default();
            let n_max = n.max_inclusive.map(|v| alloc::format!("{v}")).unwrap_or_default();
            alloc::format!("@{},{},{},{},{},{}@", n.name, n_rep, n_len, n_units, n_min, n_max)
        } else if let Some(l) = self.length {
            alloc::format!("{l}")
        } else {
            String::new()
        };
        let min_str = self.min_inclusive.map(|v| alloc::format!("{v}")).unwrap_or_default();
        let max_str = self.max_inclusive.map(|v| alloc::format!("{v}")).unwrap_or_default();
        let pad_str = self.pad_char.map(|c| alloc::format!("{c}")).unwrap_or_default();
        alloc::format!("{}:{}:{}:{}:{}:{}:{}", self.name, rep_str, len_str, units_str, min_str, max_str, pad_str)
    }
}

/// DFDL property value that is either a static constant known at schema compile time,
/// or a pre-compiled dynamic XPath expression (`{ expr }`) evaluated against the infoset at runtime.
///
/// Pre-parsing expressions into [`ExprAst`] at schema compilation time eliminates
/// runtime string scanning, `starts_with('{')` pattern matching, and runtime Pest parsing in hot paths.
#[derive(Debug, Clone, PartialEq)]
pub enum DfdlProp<T> {
    /// Static constant property value.
    Constant(T),
    /// Pre-compiled dynamic expression evaluated with runtime infoset context.
    Expression {
        /// Original raw expression string (including curly braces).
        raw: String,
        /// Pre-compiled Abstract Syntax Tree.
        ast: Box<crate::expr::ast::ExprAst>,
    },
}

impl<T> DfdlProp<T> {
    /// Creates a static constant property.
    #[inline]
    pub const fn constant(val: T) -> Self {
        Self::Constant(val)
    }

    /// Creates a pre-compiled dynamic expression property.
    #[inline]
    pub fn expression(raw: impl Into<String>, ast: crate::expr::ast::ExprAst) -> Self {
        Self::Expression {
            raw: raw.into(),
            ast: Box::new(ast),
        }
    }

    /// Returns `true` if the property is a static constant.
    #[inline]
    pub const fn is_constant(&self) -> bool {
        matches!(self, Self::Constant(_))
    }

    /// Returns `true` if the property is a dynamic expression.
    #[inline]
    pub const fn is_expression(&self) -> bool {
        matches!(self, Self::Expression { .. })
    }

    /// Returns a reference to the constant value, or `None` if dynamic.
    #[inline]
    pub const fn as_constant(&self) -> Option<&T> {
        match self {
            Self::Constant(ref val) => Some(val),
            Self::Expression { .. } => None,
        }
    }

    /// Returns a mutable reference to the constant value, or `None` if dynamic.
    #[inline]
    pub fn as_constant_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Constant(ref mut val) => Some(val),
            Self::Expression { .. } => None,
        }
    }

    /// Returns the pre-compiled AST if this property is dynamic.
    #[inline]
    pub fn expr_ast(&self) -> Option<&crate::expr::ast::ExprAst> {
        match self {
            Self::Constant(_) => None,
            Self::Expression { ast, .. } => Some(ast.as_ref()),
        }
    }

    /// Returns the raw expression string if this property is dynamic.
    #[inline]
    pub fn raw_expr(&self) -> Option<&str> {
        match self {
            Self::Constant(_) => None,
            Self::Expression { raw, .. } => Some(raw.as_str()),
        }
    }

    /// Parses a raw property string at schema compile time into either a constant or a pre-compiled expression.
    ///
    /// - If `raw` matches `{ expr }` (and is not an escaped `{{`), the expression body is parsed
    ///   into an [`ExprAst`] immediately using [`crate::expr::parse_expr`].
    ///   If the expression evaluates to a constant literal at compile time, it is folded to `Constant(T)`.
    /// - If `raw` starts with `{{`, it is treated as an escaped literal `{` and passed to `convert_const`.
    /// - Otherwise, `convert_const` converts the static string into `T`.
    pub fn parse_with<F>(raw: &str, convert_const: F) -> DFDLResult<Self>
    where
        F: FnOnce(&str) -> DFDLResult<T>,
    {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
            let expr_body = trimmed
                .get(1..trimmed.len().saturating_sub(1))
                .unwrap_or("")
                .trim();
            let ast = crate::expr::parse_expr(expr_body)?;
            Ok(Self::Expression {
                raw: String::from(raw),
                ast: Box::new(ast),
            })
        } else if let Some(stripped) = trimmed.strip_prefix("{{") {
            let unescaped = alloc::format!("{{{stripped}");
            let val = convert_const(&unescaped)?;
            Ok(Self::Constant(val))
        } else {
            let val = convert_const(raw)?;
            Ok(Self::Constant(val))
        }
    }

    /// Maps the constant value using function `f`, preserving expressions unchanged.
    pub fn map_constant<U, F>(self, f: F) -> DfdlProp<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Constant(val) => DfdlProp::Constant(f(val)),
            Self::Expression { raw, ast } => DfdlProp::Expression { raw, ast },
        }
    }
}

impl<T: Default> Default for DfdlProp<T> {
    #[inline]
    fn default() -> Self {
        Self::Constant(T::default())
    }
}

impl<T> From<T> for DfdlProp<T> {
    #[inline]
    fn from(val: T) -> Self {
        Self::Constant(val)
    }
}

impl DfdlProp<String> {
    /// Parses a string property into either a constant string or a pre-compiled expression.
    pub fn parse_str(raw: &str) -> DFDLResult<Self> {
        Self::parse_with(raw, |s| Ok(String::from(s)))
    }

    /// Returns the constant string or raw expression representation.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Constant(s) => s.as_str(),
            Self::Expression { raw, .. } => raw.as_str(),
        }
    }
}

/// Resolved DFDL physical format properties bound to a schema term.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProperties {
    /// Physical representation mode.
    pub representation: Representation,
    /// Byte ordering mode.
    pub byte_order: ByteOrder,
    /// Optional dynamic byte order expression string if calculated at runtime.
    pub byte_order_expr: Option<String>,
    /// Pre-compiled byte order property (`Constant(ByteOrder)` or pre-compiled `Expression`).
    pub byte_order_prop: DfdlProp<ByteOrder>,
    /// Bit ordering mode.
    pub bit_order: BitOrder,
    /// Length strategy.
    pub length_kind: LengthKind,
    /// Explicit length in length units if known at compile time.
    pub length: Option<usize>,
    /// Optional dynamic length expression string if length is calculated at runtime.
    pub length_expr: Option<String>,
    /// Pre-compiled length property (`Constant(usize)` or pre-compiled `Expression`).
    pub length_prop: Option<DfdlProp<usize>>,
    /// Optional prefix length type descriptor if lengthKind="prefixed" (`dfdl:prefixLengthType`).
    pub prefix_length_type: Option<PrefixLengthDescriptor>,
    /// Whether prefix length includes prefix length itself (`dfdl:prefixIncludesPrefixLength`).
    pub prefix_includes_prefix_length: bool,
    /// Optional length pattern regex string if lengthKind="pattern" (`dfdl:lengthPattern`).
    pub length_pattern: Option<String>,
    /// Length units (Bytes, Bits, or Characters).
    pub length_units: LengthUnits,
    /// Byte or bit alignment required before parsing term.
    pub alignment: usize,
    /// Alignment units (Bytes or Bits).
    pub alignment_units: AlignmentUnits,
    /// Alignment kind (Automatic or Manual).
    pub alignment_kind: AlignmentKind,
    /// Number of bytes or bits to skip before alignment and initiator (`dfdl:leadingSkip`).
    pub leading_skip: usize,
    /// Number of bytes or bits to skip after terminator (`dfdl:trailingSkip`).
    pub trailing_skip: usize,
    /// Character encoding for textual scalars.
    pub encoding: String,
    /// Pre-compiled character encoding property (`Constant(String)` or pre-compiled `Expression`).
    pub encoding_prop: DfdlProp<String>,
    /// Sequence separator string if specified.
    pub separator: Option<String>,
    /// Pre-compiled sequence separator property if specified.
    pub separator_prop: Option<DfdlProp<String>>,
    /// Separator position strategy (Infix, Prefix, Postfix).
    pub separator_position: SeparatorPosition,
    /// Separator suppression policy for empty elements.
    pub separator_suppression_policy: SeparatorSuppressionPolicy,
    /// Sequence ordering strategy (Ordered or Unordered) (§14.3).
    pub sequence_kind: SequenceKind,
    /// Optional initiator string.
    pub initiator: Option<String>,
    /// Pre-compiled initiator property if specified.
    pub initiator_prop: Option<DfdlProp<String>>,
    /// Initiated content policy for sequence/choice.
    pub initiated_content: bool,
    /// Choice length strategy (Implicit or Explicit) (`dfdl:choiceLengthKind`).
    pub choice_length_kind: LengthKind,
    /// Choice explicit length in bytes (DFDL §15.1.2: `dfdl:choiceLength`).
    pub choice_length: Option<usize>,
    /// Optional terminator string.
    pub terminator: Option<String>,
    /// Pre-compiled terminator property if specified.
    pub terminator_prop: Option<DfdlProp<String>>,
    /// Occurs count strategy (Fixed, Parsed, Expression, StopValue).
    pub occurs_count_kind: OccursCountKind,
    /// Optional occurs count expression string if specified.
    pub occurs_count_expr: Option<String>,
    /// Pre-compiled occurs count property (`Constant(usize)` or pre-compiled `Expression`).
    pub occurs_count_prop: Option<DfdlProp<usize>>,
    /// Optional choice branch discriminator expression string if specified.
    pub discriminator: Option<String>,
    /// Test kind for discriminator (Expression or Pattern).
    pub discriminator_test_kind: TestKind,
    /// Optional failure message for discriminator.
    pub discriminator_message: Option<String>,
    /// Nil value representation strategy (LiteralValue, LiteralCharacter, LogicalValue).
    pub nil_kind: NilKind,
    /// Optional nil value string if specified.
    pub nil_value: Option<String>,
    /// Pre-compiled nil value property.
    pub nil_value_prop: Option<DfdlProp<String>>,
    /// Delimiter policy for nil representation (`dfdl:nilValueDelimiterPolicy`).
    pub nil_value_delimiter_policy: NilValueDelimiterPolicy,
    /// Text trimming strategy (None, Head, Tail, Both).
    pub text_trim_kind: TextTrimKind,
    /// Single-character text padding character string (default space `" "`).
    pub text_pad_char: String,
    /// Fill byte value for binary bitstream padding (default `0x00`).
    pub fill_byte: u8,
    /// Whether `dfdl:fillByte` was explicitly defined (default `true` for hand-built IR);
    /// unparsing that needs fill with an undefined property is a Schema Definition Error.
    pub fill_byte_defined: bool,
    /// Raw unparsed `dfdl:fillByte` string for validation against dynamic encoding.
    pub fill_byte_raw: Option<String>,
    /// Optional dynamic input value calculation expression string (`dfdl:inputValueCalc`).
    pub input_value_calc: Option<String>,
    /// Optional dynamic output value calculation expression string (`dfdl:outputValueCalc`).
    pub output_value_calc: Option<String>,
    /// Assertions attached to this term.
    pub asserts: Vec<CompiledAssert>,
    /// Optional hidden group reference string (`dfdl:hiddenGroupRef`).
    pub hidden_group_ref: Option<String>,
    /// Indicates if term is inside a hidden group and suppressed from public infoset (§14.2).
    pub is_hidden: bool,
    /// DFDL setVariable statements attached to this term (`(var_name, value_expr)`).
    pub set_variables: Vec<(QName, String)>,
    /// DFDL newVariableInstance statements attached to this term (`(var_name, default_value_expr)`).
    pub new_variable_instances: Vec<(QName, Option<String>)>,
    /// Text number check policy (Lax or Strict).
    pub text_number_check_policy: TextNumberCheckPolicy,
    /// Optional text number pattern (ICU DecimalFormat pattern string).
    pub text_number_pattern: Option<String>,
    /// Text standard decimal separator string (default `"."`).
    pub text_standard_decimal_separator: String,
    /// Pre-compiled decimal separator property.
    pub text_standard_decimal_separator_prop: DfdlProp<String>,
    /// Text standard grouping separator string (default `","`).
    pub text_standard_grouping_separator: String,
    /// Pre-compiled grouping separator property.
    pub text_standard_grouping_separator_prop: DfdlProp<String>,
    /// DFDL truncateSpecifiedLengthString property (default `false`).
    pub truncate_specified_length_string: bool,
    /// DFDL textStringJustification property (§13.2, default `Left`).
    pub text_string_justification: TextJustification,
    /// DFDL textNumberJustification property (§13.2, default `Right`).
    pub text_number_justification: TextJustification,
    /// DFDL textBooleanJustification property (§13.2, default `Left`).
    pub text_boolean_justification: TextJustification,
    /// DFDL textCalendarJustification property (§13.2, default `Left`).
    pub text_calendar_justification: TextJustification,
    /// DFDL textNumberRoundingMode property (§13.7.1.4, default `RoundHalfEven`).
    pub text_number_rounding_mode: TextNumberRoundingMode,
    /// DFDL textNumberRounding is `explicit` (§13.7.1.4): mode and increment come from properties
    /// rather than from the pattern. Default `false` (`pattern`).
    pub text_number_rounding_explicit: bool,
    /// DFDL textNumberRoundingIncrement as a decimal string (only used when rounding is explicit).
    pub text_number_rounding_increment: Option<String>,
    /// DFDL encodingErrorPolicy is `error` (default `false`, i.e. `replace`).
    pub encoding_error_policy_error: bool,
    /// Whether `encodingErrorPolicy` is explicitly defined in the property chain (default `true`
    /// so hand-built properties are never reported as missing).
    pub encoding_error_policy_defined: bool,
    /// DFDL binaryDecimalVirtualPoint property (default 0).
    pub binary_decimal_virtual_point: i32,
    /// DFDL calendarPattern property (`dfdl:calendarPattern`).
    pub calendar_pattern: Option<String>,
    /// Pre-compiled calendar pattern property.
    pub calendar_pattern_prop: Option<DfdlProp<String>>,
    /// DFDL calendarLanguage property (`dfdl:calendarLanguage`).
    pub calendar_language: Option<String>,
    /// Pre-compiled calendar language property.
    pub calendar_language_prop: Option<DfdlProp<String>>,
    /// DFDL calendarTimeZone property (`dfdl:calendarTimeZone`).
    pub calendar_time_zone: Option<String>,
    /// Pre-compiled calendar time zone property.
    pub calendar_time_zone_prop: Option<DfdlProp<String>>,
    /// DFDL binaryNumberRep property (binary, packed, bcd, ibm4690Packed).
    pub binary_number_rep: BinaryNumberRep,
    /// DFDL binaryPackedSignCodes property (`dfdl:binaryPackedSignCodes`).
    pub binary_packed_sign_codes: Option<String>,
    /// DFDL binaryCalendarRep property (binarySeconds, binaryMilliseconds, bcd, packed, ibm4690Packed).
    pub binary_calendar_rep: BinaryCalendarRep,
    /// Number base for textual integer representation (2, 8, 10, 16; default 10).
    pub text_standard_base: u32,
    /// XSD Simple Type Restriction Facets.
    pub facets: SimpleTypeFacets,
    /// Optional choice dispatch key expression string (`dfdl:choiceDispatchKey`) (§15).
    pub choice_dispatch_key: Option<String>,
    /// Optional choice branch key string (`dfdl:choiceBranchKey`) (§15).
    pub choice_branch_key: Option<String>,
    /// Optional choice branch key ranges string (`dfdlx:choiceBranchKeyRanges`).
    pub choice_branch_key_ranges: Option<String>,
    /// DFDL emptyElementParsePolicy property (treatAsEmpty or treatAsAbsent) (§9.4).
    pub empty_element_parse_policy: EmptyElementParsePolicy,
    /// DFDL binaryCalendarEpoch property (`dfdl:binaryCalendarEpoch`) (§13.11).
    pub binary_calendar_epoch: Option<String>,
    /// DFDL parse/unparse policy (`dfdl:parseUnparsePolicy` or `dfdlx:parseUnparsePolicy`).
    pub parse_unparse_policy: ParseUnparsePolicy,
    /// DFDL outputNewLine property (`dfdl:outputNewLine`).
    pub output_new_line: Option<String>,
    /// Pre-compiled output newline property.
    pub output_new_line_prop: Option<DfdlProp<String>>,
    /// DFDL textStandardNaNRep property (`dfdl:textStandardNaNRep`).
    pub text_standard_nan_rep: Option<String>,
    /// DFDL textStandardInfinityRep property (`dfdl:textStandardInfinityRep`).
    pub text_standard_infinity_rep: Option<String>,
    /// DFDL textStandardZeroRep property (`dfdl:textStandardZeroRep`).
    pub text_standard_zero_rep: Option<String>,
    /// DFDL textNumberPadCharacter property (`dfdl:textNumberPadCharacter`).
    pub text_number_pad_character: Option<String>,
    /// DFDL textCalendarPadCharacter property (`dfdl:textCalendarPadCharacter`).
    pub text_calendar_pad_character: Option<String>,
    /// DFDL textStandardExponentRep property (`dfdl:textStandardExponentRep`).
    pub text_standard_exponent_rep: Option<String>,
    /// Pre-compiled exponent rep property.
    pub text_standard_exponent_rep_prop: Option<DfdlProp<String>>,
    /// DFDL ignoreCase property (`dfdl:ignoreCase`).
    pub ignore_case: bool,
    /// DFDL textBooleanTrueRep property (`dfdl:textBooleanTrueRep`).
    pub text_boolean_true_rep: Option<String>,
    /// Pre-compiled boolean true rep property.
    pub text_boolean_true_rep_prop: Option<DfdlProp<String>>,
    /// DFDL textBooleanFalseRep property (`dfdl:textBooleanFalseRep`).
    pub text_boolean_false_rep: Option<String>,
    /// Pre-compiled boolean false rep property.
    pub text_boolean_false_rep_prop: Option<DfdlProp<String>>,
    /// DFDL textBooleanPadCharacter property (`dfdl:textBooleanPadCharacter`).
    pub text_boolean_pad_character: Option<String>,
    /// DFDL binaryBooleanTrueRep property (`dfdl:binaryBooleanTrueRep`).
    pub binary_boolean_true_rep: BinaryBooleanRep,
    /// DFDL binaryBooleanFalseRep property (`dfdl:binaryBooleanFalseRep`).
    pub binary_boolean_false_rep: BinaryBooleanRep,
    /// DFDL documentFinalTerminatorCanBeMissing property (`dfdl:documentFinalTerminatorCanBeMissing`).
    pub document_final_terminator_can_be_missing: bool,
    /// DFDL decimalSigned property (`dfdl:decimalSigned`), default true (§13.7.2).
    pub decimal_signed: bool,
    /// DFDL textNumberRep property (`dfdl:textNumberRep`), default standard (§13.7.2).
    pub text_number_rep: TextNumberRep,
    /// DFDL textZonedSignStyle property (`dfdl:textZonedSignStyle`), default asciiStandard (§13.7.3).
    pub text_zoned_sign_style: TextZonedSignStyle,
    /// DFDL calendarCheckPolicy property (`dfdl:calendarCheckPolicy`), default lax (§13.11).
    pub calendar_check_policy: CalendarCheckPolicy,
    /// DFDL calendarFirstDayOfWeek property (`dfdl:calendarFirstDayOfWeek`), default Sunday (§13.14.2).
    pub calendar_first_day_of_week: CalendarFirstDayOfWeek,
    /// DFDL textPadKind property (`dfdl:textPadKind`), default none (§13.2).
    pub text_pad_kind: TextPadKind,
    /// DFDL textOutputMinLength property (`dfdl:textOutputMinLength`), default 0 (§13.7.1.3).
    pub text_output_min_length: usize,
    /// Compiled DFDL escapeScheme definition if bound to this term (§13.2.1).
    pub escape_scheme: Option<CompiledEscapeScheme>,
    /// DFDL extension repType (`dfdlx:repType`).
    pub rep_type: Option<String>,
    /// Resolved representation simple type for `dfdlx:repType`.
    pub rep_simple_type: Option<DfdlSimpleType>,
    /// DFDL extension layer transform name (`dfdlx:layer` or `dfdlx:layerTransform`).
    pub layer: Option<String>,
    /// In-scope XML namespace prefix-to-URI bindings active for this term (§23.1).
    pub in_scope_namespaces: Vec<(String, String)>,
    /// Whether this string element represents XML content (`dfdlx:runtimeProperties="stringAsXml=true"`).
    pub string_as_xml: bool,
    /// DFDL emptyValueDelimiterPolicy property (`dfdl:emptyValueDelimiterPolicy`), default `both` (§12.3.4).
    pub empty_value_delimiter_policy: EmptyValueDelimiterPolicy,
}

impl ResolvedProperties {
    /// Returns the active text justification for the given DFDL value (§13.2).
    #[inline]
    #[must_use]
    pub fn text_justification_for_value(&self, val: &crate::infoset::value::DfdlValue) -> TextJustification {
        match val {
            crate::infoset::value::DfdlValue::Int(_)
            | crate::infoset::value::DfdlValue::Long(_)
            | crate::infoset::value::DfdlValue::Short(_)
            | crate::infoset::value::DfdlValue::Byte(_)
            | crate::infoset::value::DfdlValue::UnsignedInt(_)
            | crate::infoset::value::DfdlValue::UnsignedLong(_)
            | crate::infoset::value::DfdlValue::UnsignedShort(_)
            | crate::infoset::value::DfdlValue::UnsignedByte(_)
            | crate::infoset::value::DfdlValue::Float(_)
            | crate::infoset::value::DfdlValue::Double(_)
            | crate::infoset::value::DfdlValue::Decimal(_) => self.text_number_justification,
            crate::infoset::value::DfdlValue::Boolean(_) => self.text_boolean_justification,
            crate::infoset::value::DfdlValue::DateTime(_)
            | crate::infoset::value::DfdlValue::Date(_)
            | crate::infoset::value::DfdlValue::Time(_) => self.text_calendar_justification,
            _ => self.text_string_justification,
        }
    }
}

/// DFDL binaryBooleanTrueRep and binaryBooleanFalseRep representations (§13.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinaryBooleanRep {
    /// Property is not specified.
    #[default]
    NotSpecified,
    /// Property is specified as empty string `""` (meaning any other value).
    Empty,
    /// Property is specified with an explicit integer value.
    Value(i64),
}

/// XSD Simple Type Facet Restrictions (§5.2, §15.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleTypeFacets {
    /// xs:minInclusive value string.
    pub min_inclusive: Option<String>,
    /// xs:maxInclusive value string.
    pub max_inclusive: Option<String>,
    /// xs:minExclusive value string.
    pub min_exclusive: Option<String>,
    /// xs:maxExclusive value string.
    pub max_exclusive: Option<String>,
    /// xs:pattern regular expression string.
    pub pattern: Option<String>,
    /// xs:enumeration allowed value strings.
    pub enumeration: Vec<String>,
    /// xs:minLength constraint.
    pub min_length: Option<usize>,
    /// xs:maxLength constraint.
    pub max_length: Option<usize>,
    /// xs:length constraint.
    pub length: Option<usize>,
    /// xs:totalDigits constraint.
    pub total_digits: Option<usize>,
    /// xs:fractionDigits constraint.
    pub fraction_digits: Option<usize>,
    /// DFDL extension repValues mapping: (logical_value, canonical_rep_value).
    pub rep_values: Vec<(String, String)>,
}

impl SimpleTypeFacets {
    /// Checks whether `val` satisfies all defined facets.
    #[must_use]
    pub fn validate_value(&self, val: &DfdlValue) -> bool {
        self.validate_value_detailed(val).is_ok()
    }

    /// Checks whether `val` satisfies all defined facets and returns a detailed `DFDLError` if invalid.
    pub fn validate_value_detailed(&self, val: &DfdlValue) -> DFDLResult<()> {
        let pua_str;
        let str_rep = match val {
            DfdlValue::String(s) => {
                pua_str = crate::util::remap_raw_chars_to_pua(s);
                &pua_str
            }
            _ => {
                pua_str = alloc::format!("{}", val);
                &pua_str
            }
        };

        if let Some(ref pattern) = self.pattern {
            let anchored_pattern = alloc::format!("^(?:{})$", pattern);
            // W3C XML Schema 1.0 Part 2 §4.3.4 (Pattern Facet):
            // The pattern facet uses the XML Schema regular expression language, which
            // matches the entire literal representation and does not support lookarounds.
            let re_res = regex::Regex::new(&anchored_pattern)
                .or_else(|_| regex::Regex::new(pattern));
            match re_res {
                Ok(re) => {
                    if !re.is_match(str_rep) {
                        let msg = alloc::format!(
                            "Validation Error: failed facet checks due to: facet pattern match failed for {}",
                            pattern
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
                Err(_) => {
                    let msg = alloc::format!(
                        "Validation Error: failed facet checks due to invalid facet pattern: {}",
                        pattern
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
        }

        if !self.enumeration.is_empty() {
            let clean_val = str_rep.trim();
            if !self.enumeration.iter().any(|e| e.trim() == clean_val) {
                let msg = alloc::format!(
                    "Validation Error: failed facet checks due to: facet enumeration(s) ({}) - Value '{}' is not facet-valid",
                    self.enumeration.join(", "),
                    clean_val
                );
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }

        let actual_len = match val {
            DfdlValue::HexBinary(b) => b.len(),
            DfdlValue::String(s) => s.chars().count(),
            _ => str_rep.chars().count(),
        };

        if let Some(expected_len) = self.length {
            if actual_len != expected_len {
                let msg = alloc::format!(
                    "Validation Error: failed facet checks due to: facet length ({})",
                    expected_len
                );
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }

        if let Some(min_len) = self.min_length {
            if actual_len < min_len {
                let msg = alloc::format!("Validation Error: failed facet checks due to: facet minLength ({})", min_len);
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }

        if let Some(max_len) = self.max_length {
            if actual_len > max_len {
                let msg = alloc::format!("Validation Error: failed facet checks due to: facet maxLength ({})", max_len);
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }

        if let Some(total_digits) = self.total_digits {
            let count = if let Some(dot_idx) = str_rep.find('.') {
                let int_part = str_rep
                    .get(..dot_idx)
                    .unwrap_or("")
                    .trim_start_matches('-')
                    .trim_start_matches('+')
                    .trim_start_matches('0');
                let frac_part = str_rep
                    .get(dot_idx.saturating_add(1)..)
                    .unwrap_or("")
                    .trim_end_matches('0');
                let int_count = int_part.chars().filter(|c| c.is_ascii_digit()).count();
                let frac_count = frac_part.chars().filter(|c| c.is_ascii_digit()).count();
                let total = int_count.saturating_add(frac_count);
                if total == 0 {
                    1
                } else {
                    total
                }
            } else {
                let digits_only: String = str_rep
                    .trim_start_matches('-')
                    .trim_start_matches('+')
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                let clean_digits = digits_only.trim_start_matches('0');
                if clean_digits.is_empty() && !digits_only.is_empty() {
                    1
                } else {
                    clean_digits.len()
                }
            };
            if count > total_digits {
                let msg = alloc::format!(
                    "Validation Error: failed facet checks due to: facet totalDigits ({}) - total digits has been limited to {}, exceeded by value '{}'",
                    total_digits,
                    total_digits,
                    str_rep
                );
                return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
            }
        }

        if let Some(fraction_digits) = self.fraction_digits {
            if let Some(dot_idx) = str_rep.find('.') {
                let frac_start = dot_idx.saturating_add(1);
                let frac_part = str_rep
                    .get(frac_start..)
                    .unwrap_or("")
                    .trim_end_matches('0');
                let frac_count = frac_part.chars().filter(|c| c.is_ascii_digit()).count();
                if frac_count > fraction_digits {
                    let msg = alloc::format!(
                        "Validation Error: failed facet checks due to: facet fractionDigits ({}) exceeded by value '{}'",
                        fraction_digits,
                        str_rep
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
        }

        let num_opt = match val {
            DfdlValue::Int(n) => Some(*n as f64),
            DfdlValue::Long(n) => Some(*n as f64),
            DfdlValue::Short(n) => Some(*n as f64),
            DfdlValue::Byte(n) => Some(*n as f64),
            DfdlValue::UnsignedLong(n) => Some(*n as f64),
            DfdlValue::UnsignedInt(n) => Some(*n as f64),
            DfdlValue::UnsignedShort(n) => Some(*n as f64),
            DfdlValue::UnsignedByte(n) => Some(*n as f64),
            DfdlValue::Float(f) => Some(*f as f64),
            DfdlValue::Double(d) => Some(*d),
            DfdlValue::Decimal(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        };

        if let Some(num) = num_opt {
            let parse_bound = |s: &str| -> Option<f64> {
                let trimmed = s.trim();
                let clean = trimmed.strip_prefix('+').unwrap_or(trimmed);
                clean.parse::<f64>().ok()
            };

            if let Some(ref min_inc) = self.min_inclusive {
                if let Some(bound) = parse_bound(min_inc) {
                    if !matches!(
                        num.partial_cmp(&bound),
                        Some(core::cmp::Ordering::Greater | core::cmp::Ordering::Equal)
                    ) {
                        let msg = alloc::format!("Validation Error: failed facet checks due to: facet minInclusive ({}) - Value '{}' is not facet-valid with respect to minInclusive '{}'", min_inc, num, min_inc);
                        return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
            }
            if let Some(ref max_inc) = self.max_inclusive {
                if let Some(bound) = parse_bound(max_inc) {
                    if !matches!(
                        num.partial_cmp(&bound),
                        Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal)
                    ) {
                        let msg = alloc::format!("Validation Error: failed facet checks due to: facet maxInclusive ({}) - Value '{}' is not facet-valid with respect to maxInclusive '{}'", max_inc, num, max_inc);
                        return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
            }
            if let Some(ref min_exc) = self.min_exclusive {
                if let Some(bound) = parse_bound(min_exc) {
                    if !matches!(num.partial_cmp(&bound), Some(core::cmp::Ordering::Greater)) {
                        let msg = alloc::format!("Validation Error: failed facet checks due to: facet minExclusive ({}) - Value '{}' is not facet-valid with respect to minExclusive '{}'", min_exc, num, min_exc);
                        return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
            }
            if let Some(ref max_exc) = self.max_exclusive {
                if let Some(bound) = parse_bound(max_exc) {
                    if !matches!(num.partial_cmp(&bound), Some(core::cmp::Ordering::Less)) {
                        let msg = alloc::format!("Validation Error: failed facet checks due to: facet maxExclusive ({}) - Value '{}' is not facet-valid with respect to maxExclusive '{}'", max_exc, num, max_exc);
                        return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                    }
                }
            }
        } else if matches!(
            val,
            DfdlValue::String(_) | DfdlValue::DateTime(_) | DfdlValue::Date(_) | DfdlValue::Time(_)
        ) {
            let val_str = str_rep.trim();
            if let Some(ref min_inc) = self.min_inclusive {
                if val_str < min_inc.trim() {
                    let msg = alloc::format!("Validation Error: failed facet checks due to: facet minInclusive ({}) - Value '{}' is not facet-valid", min_inc, val_str);
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
            if let Some(ref max_inc) = self.max_inclusive {
                if val_str > max_inc.trim() {
                    let msg = alloc::format!("Validation Error: failed facet checks due to: facet maxInclusive ({}) - Value '{}' is not facet-valid", max_inc, val_str);
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
            if let Some(ref min_exc) = self.min_exclusive {
                if val_str <= min_exc.trim() {
                    let msg = alloc::format!("Validation Error: failed facet checks due to: facet minExclusive ({}) - Value '{}' is not facet-valid", min_exc, val_str);
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
            if let Some(ref max_exc) = self.max_exclusive {
                if val_str >= max_exc.trim() {
                    let msg = alloc::format!("Validation Error: failed facet checks due to: facet maxExclusive ({}) - Value '{}' is not facet-valid", max_exc, val_str);
                    return Err(DFDLError::new(DFDLErrorKind::Validation, &msg));
                }
            }
        }

        Ok(())
    }

    /// Returns `true` if any facets are defined.
    #[must_use]
    pub fn has_facets(&self) -> bool {
        self.min_inclusive.is_some()
            || self.max_inclusive.is_some()
            || self.min_exclusive.is_some()
            || self.max_exclusive.is_some()
            || self.pattern.is_some()
            || !self.enumeration.is_empty()
            || self.min_length.is_some()
            || self.max_length.is_some()
            || self.total_digits.is_some()
            || self.fraction_digits.is_some()
    }
}

impl Default for ResolvedProperties {
    fn default() -> Self {
        Self {
            representation: Representation::default(),
            byte_order: ByteOrder::default(),
            byte_order_expr: None,
            byte_order_prop: DfdlProp::constant(ByteOrder::default()),
            bit_order: BitOrder::default(),
            length_kind: LengthKind::default(),
            length: None,
            length_expr: None,
            length_prop: None,
            prefix_length_type: None,
            prefix_includes_prefix_length: false,
            length_pattern: None,
            length_units: LengthUnits::default(),
            alignment: 1,
            alignment_units: AlignmentUnits::default(),
            alignment_kind: AlignmentKind::default(),
            leading_skip: 0,
            trailing_skip: 0,
            encoding: String::from("UTF-8"),
            encoding_prop: DfdlProp::constant(String::from("UTF-8")),
            separator: None,
            separator_prop: None,
            separator_position: SeparatorPosition::default(),
            separator_suppression_policy: SeparatorSuppressionPolicy::default(),
            sequence_kind: SequenceKind::default(),
            initiator: None,
            initiator_prop: None,
            initiated_content: false,
            choice_length_kind: LengthKind::Implicit,
            choice_length: None,
            terminator: None,
            terminator_prop: None,
            occurs_count_kind: OccursCountKind::default(),
            occurs_count_expr: None,
            occurs_count_prop: None,
            discriminator: None,
            discriminator_test_kind: TestKind::default(),
            discriminator_message: None,
            nil_kind: NilKind::default(),
            nil_value: None,
            nil_value_prop: None,
            nil_value_delimiter_policy: NilValueDelimiterPolicy::default(),
            text_trim_kind: TextTrimKind::default(),
            text_pad_char: String::from(" "),
            fill_byte: 0,
            fill_byte_defined: true,
            fill_byte_raw: None,
            input_value_calc: None,
            output_value_calc: None,
            asserts: Vec::new(),
            hidden_group_ref: None,
            is_hidden: false,
            set_variables: Vec::new(),
            new_variable_instances: Vec::new(),
            text_number_check_policy: TextNumberCheckPolicy::Lax,
            text_number_pattern: None,
            text_standard_decimal_separator: String::from("."),
            text_standard_decimal_separator_prop: DfdlProp::constant(String::from(".")),
            text_standard_grouping_separator: String::from(","),
            text_standard_grouping_separator_prop: DfdlProp::constant(String::from(",")),
            truncate_specified_length_string: false,
            text_string_justification: TextJustification::Left,
            text_number_justification: TextJustification::Right,
            text_boolean_justification: TextJustification::Left,
            text_calendar_justification: TextJustification::Left,
            text_number_rounding_mode: TextNumberRoundingMode::RoundHalfEven,
            text_number_rounding_explicit: false,
            text_number_rounding_increment: None,
            encoding_error_policy_error: false,
            encoding_error_policy_defined: true,
            binary_decimal_virtual_point: 0,
            calendar_pattern: None,
            calendar_pattern_prop: None,
            calendar_language: None,
            calendar_language_prop: None,
            calendar_time_zone: None,
            calendar_time_zone_prop: None,
            binary_number_rep: BinaryNumberRep::default(),
            binary_packed_sign_codes: None,
            binary_calendar_rep: BinaryCalendarRep::default(),
            text_standard_base: 10,
            facets: SimpleTypeFacets::default(),
            choice_dispatch_key: None,
            choice_branch_key: None,
            choice_branch_key_ranges: None,
            empty_element_parse_policy: EmptyElementParsePolicy::default(),
            binary_calendar_epoch: None,
            parse_unparse_policy: ParseUnparsePolicy::default(),
            output_new_line: None,
            output_new_line_prop: None,
            text_standard_nan_rep: None,
            text_standard_infinity_rep: None,
            text_standard_zero_rep: None,
            text_number_pad_character: None,
            text_calendar_pad_character: None,
            text_standard_exponent_rep: None,
            text_standard_exponent_rep_prop: None,
            ignore_case: false,
            text_boolean_true_rep: None,
            text_boolean_true_rep_prop: None,
            text_boolean_false_rep: None,
            text_boolean_false_rep_prop: None,
            text_boolean_pad_character: None,
            binary_boolean_true_rep: BinaryBooleanRep::NotSpecified,
            binary_boolean_false_rep: BinaryBooleanRep::NotSpecified,
            document_final_terminator_can_be_missing: true,
            decimal_signed: true,
            text_number_rep: TextNumberRep::default(),
            text_zoned_sign_style: TextZonedSignStyle::default(),
            calendar_check_policy: CalendarCheckPolicy::default(),
            calendar_first_day_of_week: CalendarFirstDayOfWeek::default(),
            text_pad_kind: TextPadKind::default(),
            text_output_min_length: 0,
            escape_scheme: None,
            rep_type: None,
            rep_simple_type: None,
            layer: None,
            in_scope_namespaces: Vec::new(),
            string_as_xml: false,
            empty_value_delimiter_policy: EmptyValueDelimiterPolicy::default(),
        }
    }
}

/// Compiled schema type definition (Simple or Complex).
#[derive(Debug, Clone, PartialEq)]
pub enum CompiledType {
    /// Simple type with primitive DFDL scalar type.
    Simple(DfdlSimpleType),
    /// Complex container type referencing a child sequence or choice `NodeId`.
    Complex(NodeId),
}

/// Compiled Element Information Item declaration in IR graph.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledElement {
    /// Qualified name of the element.
    pub name: QName,
    /// Compiled type definition.
    pub type_ir: CompiledType,
    /// Minimum allowed occurrences (minOccurs).
    pub min_occurs: usize,
    /// Maximum allowed occurrences (maxOccurs, None = unbounded).
    pub max_occurs: Option<usize>,
    /// Whether element supports nillable state.
    pub is_nillable: bool,
    /// Optional element default value.
    pub default_value: Option<DfdlValue>,
}

/// Compiled sequence group representation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompiledSequence {
    /// Ordered list of member term `NodeId` references.
    pub members: Vec<NodeId>,
}

/// Compiled choice group representation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompiledChoice {
    /// List of alternative branch term `NodeId` references.
    pub branches: Vec<NodeId>,
}

/// Executable schema term kind.
#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    /// Element term.
    Element(CompiledElement),
    /// Sequence group term.
    Sequence(CompiledSequence),
    /// Choice group term.
    Choice(CompiledChoice),
    /// Group reference term.
    GroupRef(NodeId),
}

/// Executable schema term combining identifier, QName, term kind, and resolved properties.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTerm {
    /// Unique internal node identifier.
    pub id: NodeId,
    /// Qualified name of the term.
    pub name: QName,
    /// Specific term kind (Element, Sequence, Choice, GroupRef).
    pub kind: TermKind,
    /// Resolved DFDL property set.
    pub properties: ResolvedProperties,
}

/// Immutable, fully validated compiled DFDL schema intermediate representation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompiledSchema {
    /// Root entry element `NodeId`.
    pub root_element_id: NodeId,
    /// Immutable graph arena storing compiled terms by `NodeId`.
    pub terms: Vec<CompiledTerm>,
    /// Table of DFDL variables defined in schema (§7).
    pub variable_map: crate::expr::variables::VariableMap,
    /// Inverse of the `allowSignedIntegerLength1Bit` tunable. Defaults to `false`, i.e. 1-bit
    /// signed binary integers are allowed (they behave as unsigned); when `true` they are an error.
    pub disallow_signed_integer_length_1bit: bool,
    /// `maxOccursBounds` tunable: upper bound on array occurrences parsed; `None` means unlimited.
    pub max_occurs_bounds: Option<usize>,
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: crate::types::UnqualifiedPathStepPolicy,
    /// `maxHexBinaryLengthInBytes` tunable: maximum allowed byte length for xs:hexBinary values.
    pub max_hex_binary_length_in_bytes: Option<usize>,
}

impl CompiledSchema {
    /// Resolves a term by `NodeId` from the schema graph arena.
    #[must_use]
    pub fn get_term(&self, id: NodeId) -> Option<&CompiledTerm> {
        self.terms.iter().find(|t| t.id == id)
    }

    /// Returns the root `CompiledTerm` of the schema graph.
    #[must_use]
    pub fn root_term(&self) -> Option<&CompiledTerm> {
        self.get_term(self.root_element_id)
    }

    /// Finds a compiled element term NodeId by local name.
    #[must_use]
    pub fn find_term_by_name(&self, name: &str) -> Option<NodeId> {
        let clean = name.split(':').next_back().unwrap_or(name);
        self.terms
            .iter()
            .find(|t| t.name.local_name == clean)
            .map(|t| t.id)
    }

    /// Determines whether a term has physical representation in the data stream (§9.2, §11.3).
    ///
    /// An element has representation if:
    /// - It does NOT have `dfdl:inputValueCalc` specified, AND
    /// - Either it has framing (`dfdl:initiator` / `dfdl:terminator`), OR
    /// - It is a simple element, OR
    /// - It is a complex element containing at least one child term that has representation.
    #[must_use]
    pub fn term_has_representation(&self, term_id: NodeId) -> bool {
        let mut visited = alloc::vec::Vec::new();
        self.term_has_representation_impl(term_id, &mut visited)
    }

    fn term_has_representation_impl(
        &self,
        term_id: NodeId,
        visited: &mut alloc::vec::Vec<NodeId>,
    ) -> bool {
        if visited.contains(&term_id) {
            return false;
        }
        visited.push(term_id);

        let term = match self.get_term(term_id) {
            Some(t) => t,
            None => return false,
        };

        // If the element has inputValueCalc, it NEVER has representation in the data stream (§11.3)
        if term.properties.input_value_calc.is_some() {
            return false;
        }

        // If the term has a non-empty initiator or terminator, it has representation (§9.2)
        if term
            .properties
            .initiator
            .as_ref()
            .is_some_and(|s| !s.is_empty())
            || term
                .properties
                .terminator
                .as_ref()
                .is_some_and(|s| !s.is_empty())
        {
            return true;
        }

        match &term.kind {
            TermKind::Element(elem) => match &elem.type_ir {
                CompiledType::Simple(_) => true,
                CompiledType::Complex(child_id) => {
                    if matches!(
                        term.properties.length_kind,
                        LengthKind::Explicit | LengthKind::Prefixed | LengthKind::Pattern
                    ) {
                        true
                    } else {
                        self.term_has_representation_impl(*child_id, visited)
                    }
                }
            },
            TermKind::Sequence(seq) => {
                for &member_id in &seq.members {
                    if self.term_has_representation_impl(member_id, visited) {
                        return true;
                    }
                }
                false
            }
            TermKind::Choice(choice) => {
                for &branch_id in &choice.branches {
                    if self.term_has_representation_impl(branch_id, visited) {
                        return true;
                    }
                }
                false
            }
            TermKind::GroupRef(ref_id) => self.term_has_representation_impl(*ref_id, visited),
        }
    }

    /// Finds a compiled element term by traversing an `InfosetPath` from schema root.
    #[must_use]
    pub fn find_term_by_path(&self, path: &crate::types::InfosetPath) -> Option<&CompiledTerm> {
        let segs = path.segments();
        if segs.is_empty() {
            return self.root_term();
        }
        let root = self.root_term()?;
        let clean_root = root
            .name
            .local_name
            .split(':')
            .next_back()
            .unwrap_or(&root.name.local_name);

        let mut idx = 0;
        if let Some(first) = segs.first() {
            let clean_first = first.split(':').next_back().unwrap_or(first);
            if clean_first == clean_root {
                idx = 1;
            }
        }

        let mut curr_term = root;
        while idx < segs.len() {
            let seg = segs.get(idx)?;
            let clean_seg = seg.split(':').next_back().unwrap_or(seg);
            let target_name = clean_seg.split('[').next().unwrap_or(clean_seg);

            if target_name == "." || target_name.starts_with(".(") || target_name == ".." || target_name.starts_with("..(") {
                idx = idx.saturating_add(1);
                continue;
            }

            let found = self.find_child_element_term(curr_term.id, target_name)?;
            curr_term = found;
            idx = idx.saturating_add(1);
        }

        Some(curr_term)
    }

    fn find_child_element_term(
        &self,
        parent_id: NodeId,
        child_name: &str,
    ) -> Option<&CompiledTerm> {
        let parent = self.get_term(parent_id)?;
        match &parent.kind {
            TermKind::Element(el) => {
                if let CompiledType::Complex(seq_id) = el.type_ir {
                    self.find_child_element_term(seq_id, child_name)
                } else {
                    None
                }
            }
            TermKind::Sequence(seq) => {
                for &member_id in &seq.members {
                    if let Some(term) = self.get_term(member_id) {
                        if term.name.local_name == child_name {
                            return Some(term);
                        }
                        if let Some(deep) = self.find_child_element_term(member_id, child_name) {
                            return Some(deep);
                        }
                    }
                }
                None
            }
            TermKind::Choice(ch) => {
                for &branch_id in &ch.branches {
                    if let Some(term) = self.get_term(branch_id) {
                        if term.name.local_name == child_name {
                            return Some(term);
                        }
                        if let Some(deep) = self.find_child_element_term(branch_id, child_name) {
                            return Some(deep);
                        }
                    }
                }
                None
            }
            TermKind::GroupRef(target_id) => self.find_child_element_term(*target_id, child_name),
        }
    }

    /// Validates all path expressions in the compiled schema to ensure query-style paths
    /// (paths referencing array elements without index predicates) are rejected (§23.2).
    pub fn validate_query_style_paths(&self) -> DFDLResult<()> {
        for term in &self.terms {
            if !term.properties.in_scope_namespaces.is_empty() {
                let ns = &term.properties.in_scope_namespaces;
                if let Some(ref expr_str) = term.properties.input_value_calc {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.output_value_calc {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.length_expr {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.occurs_count_expr {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.discriminator {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.byte_order_expr {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                if let Some(ref expr_str) = term.properties.choice_dispatch_key {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                for assert in &term.properties.asserts {
                    crate::expr::validate_expression_namespaces(&assert.test_expr, ns)?;
                }
                for (_, expr_str) in &term.properties.set_variables {
                    crate::expr::validate_expression_namespaces(expr_str, ns)?;
                }
                for (_, opt_expr) in &term.properties.new_variable_instances {
                    if let Some(ref expr_str) = opt_expr {
                        crate::expr::validate_expression_namespaces(expr_str, ns)?;
                    }
                }
            }
            if let Some(ref expr_str) = term.properties.input_value_calc {
                self.validate_expr_query_paths(expr_str)?;
            }
            if let Some(ref expr_str) = term.properties.output_value_calc {
                self.validate_expr_query_paths(expr_str)?;
            }
            if let Some(ref expr_str) = term.properties.length_expr {
                self.validate_expr_query_paths(expr_str)?;
            }
            if let Some(ref expr_str) = term.properties.occurs_count_expr {
                self.validate_expr_query_paths(expr_str)?;
            }
            if let Some(ref expr_str) = term.properties.discriminator {
                self.validate_expr_query_paths(expr_str)?;
            }
            for assert in &term.properties.asserts {
                self.validate_expr_query_paths(&assert.test_expr)?;
            }
            for (_, expr_str) in &term.properties.set_variables {
                self.validate_expr_query_paths(expr_str)?;
            }
        }
        Ok(())
    }

    fn validate_expr_query_paths(&self, raw_expr: &str) -> DFDLResult<()> {
        let trimmed = raw_expr.trim();
        let expr = if trimmed.starts_with('{') && trimmed.ends_with('}') {
            trimmed[1..trimmed.len().saturating_sub(1)].trim()
        } else {
            trimmed
        };
        if let Ok(ast) = crate::expr::parse_expr(expr) {
            self.check_ast_query_paths(&ast)?;
        }
        Ok(())
    }

    fn check_ast_query_paths(&self, ast: &crate::expr::ExprAst) -> DFDLResult<()> {
        match ast {
            crate::expr::ExprAst::Path(ref path) => {
                self.check_path_for_query_style(path)?;
            }
            crate::expr::ExprAst::IfThenElse {
                cond,
                then_expr,
                else_expr,
            } => {
                self.check_ast_query_paths(cond)?;
                self.check_ast_query_paths(then_expr)?;
                self.check_ast_query_paths(else_expr)?;
            }
            crate::expr::ExprAst::Unary { expr, .. } => {
                self.check_ast_query_paths(expr)?;
            }
            crate::expr::ExprAst::Binary { left, right, .. } => {
                self.check_ast_query_paths(left)?;
                self.check_ast_query_paths(right)?;
            }
            crate::expr::ExprAst::FnCall { ref name, ref args } => {
                let local = name.local_name.as_str();
                if local == "count" || local == "exists" {
                    // fn:count and fn:exists take an array node sequence without index predicates per DFDL §23.5
                } else {
                    for arg in args {
                        self.check_ast_query_paths(arg)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn check_path_for_query_style(&self, path: &crate::types::InfosetPath) -> DFDLResult<()> {
        let segs = path.segments();
        if segs.is_empty() {
            return Ok(());
        }
        let root = match self.root_term() {
            Some(r) => r,
            None => return Ok(()),
        };
        let mut stack: Vec<&CompiledTerm> = Vec::new();
        stack.push(root);
        let mut curr_term = root;

        let clean_root = root
            .name
            .local_name
            .split(':')
            .next_back()
            .unwrap_or(&root.name.local_name);

        let mut idx = 0;
        if path.is_absolute() {
            if let Some(first) = segs.first() {
                let clean_first = first.split(':').next_back().unwrap_or(first);
                let target_first = clean_first.split('[').next().unwrap_or(clean_first);
                if target_first == clean_root {
                    idx = 1;
                }
            }
        }

        while idx < segs.len() {
            let seg = match segs.get(idx) {
                Some(s) => s,
                None => break,
            };
            let clean_seg = seg.split(':').next_back().unwrap_or(seg);
            let target_name = clean_seg.split('[').next().unwrap_or(clean_seg);

            if target_name == "." || target_name.starts_with(".(") {
                idx = idx.saturating_add(1);
                continue;
            }

            if target_name == ".." {
                if stack.len() > 1 {
                    let _ = stack.pop();
                }
                if let Some(parent) = stack.last() {
                    curr_term = *parent;
                }
                idx = idx.saturating_add(1);
                continue;
            }

            if let Some(found) = self.find_child_element_term(curr_term.id, target_name) {
                if let TermKind::Element(ref el) = found.kind {
                    let is_array = el.max_occurs.is_none() || el.max_occurs > Some(1);
                    let has_predicate = clean_seg.contains('[');
                    if is_array && !has_predicate {
                        let msg = alloc::format!(
                            "Schema Definition Error: Query-style paths not supported. Path step '{}' references array element without an index predicate",
                            target_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
                stack.push(found);
                curr_term = found;
            } else {
                break;
            }
            idx = idx.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    #[test]
    fn test_compiled_schema_term_lookup() {
        let qn = QName::local("root");
        let elem = CompiledElement {
            name: qn.clone(),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let term = CompiledTerm {
            id: NodeId(0),
            name: qn,
            kind: TermKind::Element(elem),
            properties: ResolvedProperties::default(),
        };

        let schema = CompiledSchema {
            root_element_id: NodeId(0),
            terms: Vec::from([term]),
            variable_map: Default::default(),
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
        };

        let root = schema.root_term().unwrap();
        assert_eq!(root.id, NodeId(0));
        assert_eq!(root.name.local_name, "root");
        assert_eq!(schema.find_term_by_name("root"), Some(NodeId(0)));
        assert!(schema.term_has_representation(NodeId(0)));

        let path = crate::types::InfosetPath::parse("/root");
        assert_eq!(schema.find_term_by_path(&path).map(|t| t.id), Some(NodeId(0)));
    }

    /// Verifies escape scheme unparsing for both character and block escape schemes.
    #[test]
    fn test_escape_scheme_unparsing_details() {
        let char_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeCharacter,
            escape_character: Some("/".into()),
            escape_escape_character: Some("\\".into()),
            escape_block_start: None,
            escape_block_end: None,
            extra_escaped_characters: alloc::vec![',', ';'],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };

        // Escapes extra chars and escape character itself
        let unp = char_scheme.escape_text("a,b/c", None, None, None, None, &[","]);
        assert_eq!(unp, "a/,b\\/c");

        // Block escape scheme with Always
        let block_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: Some("\"".into()),
            escape_block_start: Some("[".into()),
            escape_block_end: Some("]".into()),
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::Always,
        };
        let unp_block = block_scheme.escape_text("data]end", None, None, None, None, &[","]);
        assert_eq!(unp_block, "[data\"]end]");
    }

    /// Verifies numeric and hexBinary facet validations across types.
    #[test]
    fn test_facet_validations_across_types() {
        let facets = SimpleTypeFacets {
            min_inclusive: Some("10".into()),
            max_inclusive: Some("100".into()),
            min_exclusive: Some("5".into()),
            max_exclusive: Some("150".into()),
            ..Default::default()
        };
        assert!(facets.has_facets());

        // Validate Byte, Long, UnsignedLong, UnsignedShort, UnsignedByte
        assert!(facets.validate_value(&DfdlValue::Byte(20)));
        assert!(!facets.validate_value(&DfdlValue::Byte(5)));
        assert!(facets.validate_value_detailed(&DfdlValue::Long(50)).is_ok());
        assert!(facets.validate_value_detailed(&DfdlValue::UnsignedLong(50)).is_ok());
        assert!(facets.validate_value_detailed(&DfdlValue::UnsignedShort(50)).is_ok());
        assert!(facets.validate_value_detailed(&DfdlValue::UnsignedByte(50)).is_ok());

        // String patterns: valid match, mismatch, and invalid regex pattern syntax
        let pat_facets = SimpleTypeFacets {
            pattern: Some("[a-z]+".into()),
            ..Default::default()
        };
        assert!(pat_facets.validate_value_detailed(&DfdlValue::String("abc".into())).is_ok());
        assert!(pat_facets.validate_value_detailed(&DfdlValue::String("123".into())).is_err());

        let bad_pat = SimpleTypeFacets {
            pattern: Some("[(".into()),
            ..Default::default()
        };
        assert!(bad_pat.validate_value_detailed(&DfdlValue::String("abc".into())).is_err());

        // Enumerations: match and mismatch
        let enum_facets = SimpleTypeFacets {
            enumeration: alloc::vec!["RED".into(), "GREEN".into()],
            ..Default::default()
        };
        assert!(enum_facets.validate_value_detailed(&DfdlValue::String("RED".into())).is_ok());
        assert!(enum_facets.validate_value_detailed(&DfdlValue::String("BLUE".into())).is_err());

        // String length, minLength, maxLength
        let len_facets = SimpleTypeFacets {
            min_length: Some(2),
            max_length: Some(4),
            ..Default::default()
        };
        assert!(len_facets.validate_value_detailed(&DfdlValue::String("a".into())).is_err());
        assert!(len_facets.validate_value_detailed(&DfdlValue::String("abc".into())).is_ok());
        assert!(len_facets.validate_value_detailed(&DfdlValue::String("abcde".into())).is_err());

        // totalDigits and fractionDigits
        let digit_facets = SimpleTypeFacets {
            total_digits: Some(4),
            fraction_digits: Some(2),
            ..Default::default()
        };
        assert!(digit_facets.validate_value_detailed(&DfdlValue::String("12.34".into())).is_ok());
        assert!(digit_facets.validate_value_detailed(&DfdlValue::String("123.45".into())).is_err());
        assert!(digit_facets.validate_value_detailed(&DfdlValue::String("1.234".into())).is_err());
        assert!(digit_facets.validate_value_detailed(&DfdlValue::Int(12345)).is_err());

        // HexBinary length facet validation
        let hex_facets = SimpleTypeFacets {
            length: Some(2),
            ..Default::default()
        };
        let hex_ok = DfdlValue::HexBinary(alloc::vec![0xAA, 0xBB]);
        let hex_bad = DfdlValue::HexBinary(alloc::vec![0xAA]);
        assert!(hex_facets.validate_value_detailed(&hex_ok).is_ok());
        assert!(hex_facets.validate_value_detailed(&hex_bad).is_err());

        // Escape scheme edge cases: empty characters and escape_escape in blocks
        let empty_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeCharacter,
            escape_character: None,
            escape_escape_character: None,
            escape_block_start: None,
            escape_block_end: None,
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };
        assert_eq!(empty_scheme.escape_text("raw", None, None, None, None, &[]), "raw");

        let empty_block = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: None,
            escape_block_start: None,
            escape_block_end: None,
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };
        assert_eq!(empty_block.escape_text("raw", None, None, None, None, &[]), "raw");

        let block_esc_esc = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: Some("#".into()),
            escape_block_start: Some("[".into()),
            escape_block_end: Some("]".into()),
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::Always,
        };
        assert_eq!(block_esc_esc.escape_text("a#b]c", None, None, None, None, &[]), "[a##b#]c]");

        // Justification for boolean and other types
        let props = ResolvedProperties::default();
        assert_eq!(props.text_justification_for_value(&DfdlValue::Boolean(true)), TextJustification::Left);
        assert_eq!(props.text_justification_for_value(&DfdlValue::Long(42)), TextJustification::Right);
    }

    /// Verifies CompiledSchema nested path lookups, relative steps, and query-style path validation.
    #[test]
    fn test_compiled_schema_paths_and_validation() {
        use crate::types::InfosetPath;

        let root_qn = QName::local("root");
        let child_qn = QName::local("child");
        let grand_qn = QName::local("grand");

        let grand_elem = CompiledElement {
            name: grand_qn.clone(),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let grand_term = CompiledTerm {
            id: NodeId(3),
            name: grand_qn,
            kind: TermKind::Element(grand_elem),
            properties: ResolvedProperties::default(),
        };

        let seq_child = CompiledSequence {
            members: alloc::vec![NodeId(3)],
        };
        let seq_child_term = CompiledTerm {
            id: NodeId(2),
            name: QName::local("seqChild"),
            kind: TermKind::Sequence(seq_child),
            properties: ResolvedProperties::default(),
        };

        let child_elem = CompiledElement {
            name: child_qn.clone(),
            type_ir: CompiledType::Complex(NodeId(2)),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let child_term = CompiledTerm {
            id: NodeId(1),
            name: child_qn,
            kind: TermKind::Element(child_elem),
            properties: ResolvedProperties::default(),
        };

        let root_seq = CompiledSequence {
            members: alloc::vec![NodeId(1)],
        };
        let root_seq_term = CompiledTerm {
            id: NodeId(10),
            name: QName::local("rootSeq"),
            kind: TermKind::Sequence(root_seq),
            properties: ResolvedProperties::default(),
        };

        let mut root_props = ResolvedProperties::default();
        root_props.in_scope_namespaces.push(("ex".into(), "http://example.com".into()));
        root_props.in_scope_namespaces.push(("xs".into(), "http://www.w3.org/2001/XMLSchema".into()));
        root_props.in_scope_namespaces.push(("fn".into(), "http://www.w3.org/2005/xpath-functions".into()));
        root_props.input_value_calc = Some("{ xs:string(42) }".into());
        root_props.output_value_calc = Some("{ xs:string(42) }".into());
        root_props.length_expr = Some("{ 10 }".into());
        root_props.occurs_count_expr = Some("{ 1 }".into());
        root_props.discriminator = Some("{ fn:true() }".into());
        root_props.byte_order_expr = Some("{ 'bigEndian' }".into());
        root_props.choice_dispatch_key = Some("{ 'key1' }".into());
        root_props.asserts.push(CompiledAssert {
            test_kind: TestKind::Expression,
            test_expr: "{ fn:true() }".into(),
            message: None,
            failure_type: FailureType::ProcessingError,
        });

        let root_elem = CompiledElement {
            name: root_qn.clone(),
            type_ir: CompiledType::Complex(NodeId(10)),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_term = CompiledTerm {
            id: NodeId(0),
            name: root_qn,
            kind: TermKind::Element(root_elem),
            properties: root_props,
        };

        // GroupRef and Choice traversal
        let branch_elem = CompiledElement {
            name: QName::local("branchElem"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let branch_term = CompiledTerm {
            id: NodeId(6),
            name: QName::local("branchElem"),
            kind: TermKind::Element(branch_elem),
            properties: ResolvedProperties::default(),
        };
        let choice_term = CompiledTerm {
            id: NodeId(5),
            name: QName::local("choiceBlock"),
            kind: TermKind::Choice(CompiledChoice { branches: alloc::vec![NodeId(6)] }),
            properties: ResolvedProperties::default(),
        };
        let grp_term = CompiledTerm {
            id: NodeId(7),
            name: QName::local("grpRef"),
            kind: TermKind::GroupRef(NodeId(5)),
            properties: ResolvedProperties::default(),
        };

        let mut root_seq_term = root_seq_term;
        if let TermKind::Sequence(ref mut s) = root_seq_term.kind {
            s.members.push(NodeId(7));
        }

        let schema = CompiledSchema {
            root_element_id: NodeId(0),
            terms: alloc::vec![root_term, root_seq_term, child_term, seq_child_term, grand_term, choice_term, branch_term, grp_term],
            variable_map: Default::default(),
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
        };

        // Find path through nested complex elements
        let p1 = InfosetPath::parse("/root/child/grand");
        assert_eq!(schema.find_term_by_path(&p1).map(|t| t.id), Some(NodeId(3)));

        // Path with '.' and '..' steps
        let p2 = InfosetPath::parse("/root/child/./grand");
        assert_eq!(schema.find_term_by_path(&p2).map(|t| t.id), Some(NodeId(3)));

        // Path through GroupRef and Choice
        let p_choice = InfosetPath::parse("/root/branchElem");
        assert_eq!(schema.find_term_by_path(&p_choice).map(|t| t.id), Some(NodeId(6)));

        // Validate query style paths
        assert!(schema.validate_query_style_paths().is_ok());

        // Check query style path with array without predicate
        let array_elem = CompiledElement {
            name: QName::local("arr"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 0,
            max_occurs: None, // unbounded array
            is_nillable: false,
            default_value: None,
        };
        let array_term = CompiledTerm {
            id: NodeId(4),
            name: QName::local("arr"),
            kind: TermKind::Element(array_elem),
            properties: ResolvedProperties::default(),
        };
        let mut schema_with_arr = schema;
        schema_with_arr.terms.push(array_term);
        if let TermKind::Sequence(ref mut s) = schema_with_arr.terms[1].kind {
            s.members.push(NodeId(4));
        }
        let bad_path = InfosetPath::parse("/root/arr");
        assert!(schema_with_arr.check_path_for_query_style(&bad_path).is_err());
        let good_path = InfosetPath::parse("/root/arr[1]");
        assert!(schema_with_arr.check_path_for_query_style(&good_path).is_ok());

        // Check relative steps in query style check
        let dot_path = InfosetPath::parse("/root/child/../child/./arr[1]");
        assert!(schema_with_arr.check_path_for_query_style(&dot_path).is_ok());
    }
}
