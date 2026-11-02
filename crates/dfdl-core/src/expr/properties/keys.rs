//! Strongly typed DFDL Property Keys, PropertyKeyName, and PropertyBinding.
//!
//! Provides zero-allocation matching for normative DFDL property keys (DFDL 1.0 §6.3, §7, §8.1)
//! while accommodating custom and dynamic vendor properties (`dfdlx:`, `daf:`).

extern crate alloc;
use alloc::string::String;

/// Strongly typed enumeration of standard DFDL property keys.
///
/// Implements RUST-SCH-04 to avoid dynamic heap allocations when storing and matching normative DFDL property keys.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PropertyKey {
    // Alignment and framing
    Alignment,
    AlignmentUnits,
    LeadingSkip,
    TrailingSkip,
    // Byte order and encoding
    ByteOrder,
    Encoding,
    Utf16Width,
    EncodingErrorPolicy,
    BitOrder,
    OutputNewLine,
    // Length and Occurs
    Length,
    LengthKind,
    LengthUnits,
    OccursCount,
    OccursCountKind,
    MinOccurs,
    MaxOccurs,
    PrefixLengthType,
    PrefixIncludesPrefixLength,
    // Delimiters
    Initiator,
    Terminator,
    Separator,
    SeparatorPosition,
    SeparatorSuppressionPolicy,
    // Sequence / Choice
    SequenceKind,
    InitiatedContent,
    HiddenGroupRef,
    ChoiceDispatchKey,
    ChoiceBranchKey,
    ChoiceLengthKind,
    ChoiceLength,
    // Representations
    Representation,
    BinaryNumberRep,
    BinaryFloatRep,
    BinaryDecimalVirtualPoint,
    BinaryPackedSignCodes,
    BinaryCalendarRep,
    BinaryBooleanTrueRep,
    BinaryBooleanFalseRep,
    // Text representation
    TextNumberRep,
    TextNumberPattern,
    TextStandardDecimalSeparator,
    TextStandardGroupingSeparator,
    TextStandardExponentRep,
    TextNumberCheckPolicy,
    TextNumberRounding,
    TextNumberRoundingMode,
    TextNumberRoundingIncrement,
    TextNumberBase,
    TextNumberJustification,
    TextNumberPadCharacter,
    TextStandardZeroRep,
    TextStandardNaNRep,
    TextStandardInfinityRep,
    TextStringJustification,
    TextStringPadCharacter,
    TextTrimKind,
    TextPadKind,
    TextBooleanTrueRep,
    TextBooleanFalseRep,
    TextBooleanJustification,
    TextBooleanPadCharacter,
    TextOutputMinLength,
    // Calendar
    CalendarPattern,
    CalendarPatternKind,
    CalendarCheckPolicy,
    CalendarTimeZone,
    CalendarFirstDayOfWeek,
    CalendarDaysInFirstWeek,
    CalendarCenturyStart,
    CalendarLanguage,
    CalendarObserveDST,
    // Escaping
    EscapeSchemeRef,
    EscapeKind,
    EscapeCharacter,
    EscapeEscapeCharacter,
    EscapeBlockStart,
    EscapeBlockEnd,
    ExtraEscapedCharacters,
    GenerateEscapeBlock,
    // Nillable
    NilKind,
    NilValue,
    NilValueDelimiterPolicy,
    UseNilForDefault,
    // Value Calculation & Defaults
    InputValueCalc,
    OutputValueCalc,
    // Fill and Truncation
    FillByte,
    TruncateSpecifiedLengthString,
    CheckConstraints,
    IgnoreCase,
    // Extension properties (dfdlx, daf)
    RepType,
    RepValues,
    RepValueRanges,
    Layer,
    LayerTransform,
    LayerLength,
    LayerLengthUnits,
    LayerBoundaryMark,
    SuppressSchemaDefinitionWarnings,
    // Facet
    Enumeration,
}

impl PropertyKey {
    /// Parses a string into a normative [`PropertyKey`], stripping optional `dfdl:`, `dfdlx:`, or `daf:` prefixes.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let clean = s
            .trim_start_matches("dfdl:")
            .trim_start_matches("dfdlx:")
            .trim_start_matches("daf:");
        match clean {
            "alignment" => Some(Self::Alignment),
            "alignmentUnits" => Some(Self::AlignmentUnits),
            "leadingSkip" => Some(Self::LeadingSkip),
            "trailingSkip" => Some(Self::TrailingSkip),
            "byteOrder" => Some(Self::ByteOrder),
            "encoding" => Some(Self::Encoding),
            "utf16Width" => Some(Self::Utf16Width),
            "encodingErrorPolicy" => Some(Self::EncodingErrorPolicy),
            "bitOrder" => Some(Self::BitOrder),
            "outputNewLine" => Some(Self::OutputNewLine),
            "length" => Some(Self::Length),
            "lengthKind" => Some(Self::LengthKind),
            "lengthUnits" => Some(Self::LengthUnits),
            "occursCount" => Some(Self::OccursCount),
            "occursCountKind" => Some(Self::OccursCountKind),
            "minOccurs" => Some(Self::MinOccurs),
            "maxOccurs" => Some(Self::MaxOccurs),
            "prefixLengthType" => Some(Self::PrefixLengthType),
            "prefixIncludesPrefixLength" => Some(Self::PrefixIncludesPrefixLength),
            "initiator" => Some(Self::Initiator),
            "terminator" => Some(Self::Terminator),
            "separator" => Some(Self::Separator),
            "separatorPosition" => Some(Self::SeparatorPosition),
            "separatorSuppressionPolicy" => Some(Self::SeparatorSuppressionPolicy),
            "sequenceKind" => Some(Self::SequenceKind),
            "initiatedContent" => Some(Self::InitiatedContent),
            "hiddenGroupRef" => Some(Self::HiddenGroupRef),
            "choiceDispatchKey" => Some(Self::ChoiceDispatchKey),
            "choiceBranchKey" => Some(Self::ChoiceBranchKey),
            "choiceLengthKind" => Some(Self::ChoiceLengthKind),
            "choiceLength" => Some(Self::ChoiceLength),
            "representation" => Some(Self::Representation),
            "binaryNumberRep" => Some(Self::BinaryNumberRep),
            "binaryFloatRep" => Some(Self::BinaryFloatRep),
            "binaryDecimalVirtualPoint" => Some(Self::BinaryDecimalVirtualPoint),
            "binaryPackedSignCodes" => Some(Self::BinaryPackedSignCodes),
            "binaryCalendarRep" => Some(Self::BinaryCalendarRep),
            "binaryBooleanTrueRep" => Some(Self::BinaryBooleanTrueRep),
            "binaryBooleanFalseRep" => Some(Self::BinaryBooleanFalseRep),
            "textNumberRep" => Some(Self::TextNumberRep),
            "textNumberPattern" => Some(Self::TextNumberPattern),
            "textStandardDecimalSeparator" => Some(Self::TextStandardDecimalSeparator),
            "textStandardGroupingSeparator" => Some(Self::TextStandardGroupingSeparator),
            "textStandardExponentRep" => Some(Self::TextStandardExponentRep),
            "textNumberCheckPolicy" => Some(Self::TextNumberCheckPolicy),
            "textNumberRounding" => Some(Self::TextNumberRounding),
            "textNumberRoundingMode" => Some(Self::TextNumberRoundingMode),
            "textNumberRoundingIncrement" => Some(Self::TextNumberRoundingIncrement),
            "textNumberBase" => Some(Self::TextNumberBase),
            "textNumberJustification" => Some(Self::TextNumberJustification),
            "textNumberPadCharacter" => Some(Self::TextNumberPadCharacter),
            "textStandardZeroRep" => Some(Self::TextStandardZeroRep),
            "textStandardNaNRep" => Some(Self::TextStandardNaNRep),
            "textStandardInfinityRep" => Some(Self::TextStandardInfinityRep),
            "textStringJustification" => Some(Self::TextStringJustification),
            "textStringPadCharacter" => Some(Self::TextStringPadCharacter),
            "textTrimKind" => Some(Self::TextTrimKind),
            "textPadKind" => Some(Self::TextPadKind),
            "textBooleanTrueRep" => Some(Self::TextBooleanTrueRep),
            "textBooleanFalseRep" => Some(Self::TextBooleanFalseRep),
            "textBooleanJustification" => Some(Self::TextBooleanJustification),
            "textBooleanPadCharacter" => Some(Self::TextBooleanPadCharacter),
            "textOutputMinLength" => Some(Self::TextOutputMinLength),
            "calendarPattern" => Some(Self::CalendarPattern),
            "calendarPatternKind" => Some(Self::CalendarPatternKind),
            "calendarCheckPolicy" => Some(Self::CalendarCheckPolicy),
            "calendarTimeZone" => Some(Self::CalendarTimeZone),
            "calendarFirstDayOfWeek" => Some(Self::CalendarFirstDayOfWeek),
            "calendarDaysInFirstWeek" => Some(Self::CalendarDaysInFirstWeek),
            "calendarCenturyStart" => Some(Self::CalendarCenturyStart),
            "calendarLanguage" => Some(Self::CalendarLanguage),
            "calendarObserveDST" => Some(Self::CalendarObserveDST),
            "escapeSchemeRef" => Some(Self::EscapeSchemeRef),
            "escapeKind" => Some(Self::EscapeKind),
            "escapeCharacter" => Some(Self::EscapeCharacter),
            "escapeEscapeCharacter" => Some(Self::EscapeEscapeCharacter),
            "escapeBlockStart" => Some(Self::EscapeBlockStart),
            "escapeBlockEnd" => Some(Self::EscapeBlockEnd),
            "extraEscapedCharacters" => Some(Self::ExtraEscapedCharacters),
            "generateEscapeBlock" => Some(Self::GenerateEscapeBlock),
            "nilKind" => Some(Self::NilKind),
            "nilValue" => Some(Self::NilValue),
            "nilValueDelimiterPolicy" => Some(Self::NilValueDelimiterPolicy),
            "useNilForDefault" => Some(Self::UseNilForDefault),
            "inputValueCalc" => Some(Self::InputValueCalc),
            "outputValueCalc" => Some(Self::OutputValueCalc),
            "fillByte" => Some(Self::FillByte),
            "truncateSpecifiedLengthString" => Some(Self::TruncateSpecifiedLengthString),
            "checkConstraints" => Some(Self::CheckConstraints),
            "ignoreCase" => Some(Self::IgnoreCase),
            "repType" => Some(Self::RepType),
            "repValues" => Some(Self::RepValues),
            "repValueRanges" => Some(Self::RepValueRanges),
            "layer" => Some(Self::Layer),
            "layerTransform" => Some(Self::LayerTransform),
            "layerLength" => Some(Self::LayerLength),
            "layerLengthUnits" => Some(Self::LayerLengthUnits),
            "layerBoundaryMark" => Some(Self::LayerBoundaryMark),
            "suppressSchemaDefinitionWarnings" => Some(Self::SuppressSchemaDefinitionWarnings),
            "enumeration" => Some(Self::Enumeration),
            _ => None,
        }
    }

    /// Returns the canonical normative string representation of the property key.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Alignment => "alignment",
            Self::AlignmentUnits => "alignmentUnits",
            Self::LeadingSkip => "leadingSkip",
            Self::TrailingSkip => "trailingSkip",
            Self::ByteOrder => "byteOrder",
            Self::Encoding => "encoding",
            Self::Utf16Width => "utf16Width",
            Self::EncodingErrorPolicy => "encodingErrorPolicy",
            Self::BitOrder => "bitOrder",
            Self::OutputNewLine => "outputNewLine",
            Self::Length => "length",
            Self::LengthKind => "lengthKind",
            Self::LengthUnits => "lengthUnits",
            Self::OccursCount => "occursCount",
            Self::OccursCountKind => "occursCountKind",
            Self::MinOccurs => "minOccurs",
            Self::MaxOccurs => "maxOccurs",
            Self::PrefixLengthType => "prefixLengthType",
            Self::PrefixIncludesPrefixLength => "prefixIncludesPrefixLength",
            Self::Initiator => "initiator",
            Self::Terminator => "terminator",
            Self::Separator => "separator",
            Self::SeparatorPosition => "separatorPosition",
            Self::SeparatorSuppressionPolicy => "separatorSuppressionPolicy",
            Self::SequenceKind => "sequenceKind",
            Self::InitiatedContent => "initiatedContent",
            Self::HiddenGroupRef => "hiddenGroupRef",
            Self::ChoiceDispatchKey => "choiceDispatchKey",
            Self::ChoiceBranchKey => "choiceBranchKey",
            Self::ChoiceLengthKind => "choiceLengthKind",
            Self::ChoiceLength => "choiceLength",
            Self::Representation => "representation",
            Self::BinaryNumberRep => "binaryNumberRep",
            Self::BinaryFloatRep => "binaryFloatRep",
            Self::BinaryDecimalVirtualPoint => "binaryDecimalVirtualPoint",
            Self::BinaryPackedSignCodes => "binaryPackedSignCodes",
            Self::BinaryCalendarRep => "binaryCalendarRep",
            Self::BinaryBooleanTrueRep => "binaryBooleanTrueRep",
            Self::BinaryBooleanFalseRep => "binaryBooleanFalseRep",
            Self::TextNumberRep => "textNumberRep",
            Self::TextNumberPattern => "textNumberPattern",
            Self::TextStandardDecimalSeparator => "textStandardDecimalSeparator",
            Self::TextStandardGroupingSeparator => "textStandardGroupingSeparator",
            Self::TextStandardExponentRep => "textStandardExponentRep",
            Self::TextNumberCheckPolicy => "textNumberCheckPolicy",
            Self::TextNumberRounding => "textNumberRounding",
            Self::TextNumberRoundingMode => "textNumberRoundingMode",
            Self::TextNumberRoundingIncrement => "textNumberRoundingIncrement",
            Self::TextNumberBase => "textNumberBase",
            Self::TextNumberJustification => "textNumberJustification",
            Self::TextNumberPadCharacter => "textNumberPadCharacter",
            Self::TextStandardZeroRep => "textStandardZeroRep",
            Self::TextStandardNaNRep => "textStandardNaNRep",
            Self::TextStandardInfinityRep => "textStandardInfinityRep",
            Self::TextStringJustification => "textStringJustification",
            Self::TextStringPadCharacter => "textStringPadCharacter",
            Self::TextTrimKind => "textTrimKind",
            Self::TextPadKind => "textPadKind",
            Self::TextBooleanTrueRep => "textBooleanTrueRep",
            Self::TextBooleanFalseRep => "textBooleanFalseRep",
            Self::TextBooleanJustification => "textBooleanJustification",
            Self::TextBooleanPadCharacter => "textBooleanPadCharacter",
            Self::TextOutputMinLength => "textOutputMinLength",
            Self::CalendarPattern => "calendarPattern",
            Self::CalendarPatternKind => "calendarPatternKind",
            Self::CalendarCheckPolicy => "calendarCheckPolicy",
            Self::CalendarTimeZone => "calendarTimeZone",
            Self::CalendarFirstDayOfWeek => "calendarFirstDayOfWeek",
            Self::CalendarDaysInFirstWeek => "calendarDaysInFirstWeek",
            Self::CalendarCenturyStart => "calendarCenturyStart",
            Self::CalendarLanguage => "calendarLanguage",
            Self::CalendarObserveDST => "calendarObserveDST",
            Self::EscapeSchemeRef => "escapeSchemeRef",
            Self::EscapeKind => "escapeKind",
            Self::EscapeCharacter => "escapeCharacter",
            Self::EscapeEscapeCharacter => "escapeEscapeCharacter",
            Self::EscapeBlockStart => "escapeBlockStart",
            Self::EscapeBlockEnd => "escapeBlockEnd",
            Self::ExtraEscapedCharacters => "extraEscapedCharacters",
            Self::GenerateEscapeBlock => "generateEscapeBlock",
            Self::NilKind => "nilKind",
            Self::NilValue => "nilValue",
            Self::NilValueDelimiterPolicy => "nilValueDelimiterPolicy",
            Self::UseNilForDefault => "useNilForDefault",
            Self::InputValueCalc => "inputValueCalc",
            Self::OutputValueCalc => "outputValueCalc",
            Self::FillByte => "fillByte",
            Self::TruncateSpecifiedLengthString => "truncateSpecifiedLengthString",
            Self::CheckConstraints => "checkConstraints",
            Self::IgnoreCase => "ignoreCase",
            Self::RepType => "repType",
            Self::RepValues => "repValues",
            Self::RepValueRanges => "repValueRanges",
            Self::Layer => "layer",
            Self::LayerTransform => "layerTransform",
            Self::LayerLength => "layerLength",
            Self::LayerLengthUnits => "layerLengthUnits",
            Self::LayerBoundaryMark => "layerBoundaryMark",
            Self::SuppressSchemaDefinitionWarnings => "suppressSchemaDefinitionWarnings",
            Self::Enumeration => "enumeration",
        }
    }

    /// Returns `true` if this property is inheritable from parent to child components per DFDL §13.
    #[must_use]
    pub const fn is_inheritable(&self) -> bool {
        !matches!(
            self,
            Self::InputValueCalc
                | Self::OutputValueCalc
                | Self::Initiator
                | Self::Terminator
                | Self::Separator
                | Self::SeparatorPosition
                | Self::SeparatorSuppressionPolicy
                | Self::SequenceKind
                | Self::InitiatedContent
                | Self::HiddenGroupRef
                | Self::ChoiceDispatchKey
                | Self::ChoiceBranchKey
                | Self::ChoiceLengthKind
                | Self::ChoiceLength
                | Self::LayerTransform
                | Self::LayerLength
                | Self::LayerLengthUnits
                | Self::LayerBoundaryMark
                | Self::RepType
                | Self::RepValues
                | Self::RepValueRanges
        )
    }
}

/// Representation of a property key: either a normative [`PropertyKey`] or a custom dynamic string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropertyKeyName {
    /// Standard normative DFDL property.
    Known(PropertyKey),
    /// Custom, non-standard, or dynamic property name (e.g. `repValue:xxx`, `__dfdl_element_ref`).
    Custom(String),
}

impl PropertyKeyName {
    /// Creates a property key from a string slice without allocating if it matches a known key.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let clean = s
            .trim_start_matches("dfdl:")
            .trim_start_matches("dfdlx:")
            .trim_start_matches("daf:");
        if let Some(known) = PropertyKey::parse(clean) {
            Self::Known(known)
        } else {
            Self::Custom(alloc::string::ToString::to_string(clean))
        }
    }

    /// Returns the string slice of this property key name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(k) => k.as_str(),
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl core::ops::Deref for PropertyKeyName {
    type Target = str;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for PropertyKeyName {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for PropertyKeyName {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        let clean = other
            .trim_start_matches("dfdl:")
            .trim_start_matches("dfdlx:")
            .trim_start_matches("daf:");
        self.as_str() == clean
    }
}

impl PartialEq<&str> for PropertyKeyName {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl PartialEq<PropertyKey> for PropertyKeyName {
    #[inline]
    fn eq(&self, other: &PropertyKey) -> bool {
        match self {
            Self::Known(k) => k == other,
            Self::Custom(_) => false,
        }
    }
}

impl PartialEq<PropertyKeyName> for PropertyKey {
    #[inline]
    fn eq(&self, other: &PropertyKeyName) -> bool {
        other == self
    }
}

impl core::fmt::Display for PropertyKeyName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Individual key-value property binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyBinding {
    /// Property name representation (normative enum or dynamic custom string).
    pub key: PropertyKeyName,
    /// Raw string value assigned to property.
    pub value: String,
}
