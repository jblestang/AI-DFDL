//! DFDL Property Resolution Engine (DFDL 1.0 §6.3, §7, §8.1, §12-§17).
//!
//! Resolves effective DFDL properties for elements, sequences, choices, and formats
//! into strongly typed `ResolvedProperties`.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::traits::{BitOrder, ByteOrder};
use crate::schema::ir::{
    CompiledAssert, DfdlProp, LengthKind, NilKind, OccursCountKind, ParseUnparsePolicy,
    Representation, ResolvedProperties, SeparatorPosition, SeparatorSuppressionPolicy, SequenceKind,
};
use crate::util::try_push;

use super::entities::{decode_dfdl_character_entities, is_valid_dfdl_regex};
use super::store::PropertyStore;

/// Parses a `dfdl:byteOrder` value, whether literal or the result of a runtime expression.
///
/// Unknown values raise a Schema Definition Error (a runtime SDE when the value came from an
/// expression), so literal and computed values are validated identically.
pub fn parse_byte_order(value: &str) -> DFDLResult<ByteOrder> {
    match value.trim() {
        "bigEndian" | "BE" => Ok(ByteOrder::BigEndian),
        "littleEndian" | "LE" => Ok(ByteOrder::LittleEndian),
        other => Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            &alloc::format!("Schema Definition Error: Unknown value for byteOrder property: {}", other),
        )),
    }
}

impl PropertyStore {
    /// Compiles property store into [`ResolvedProperties`] using default [`crate::limits::ResourceLimits`].
    pub fn to_resolved_properties(
        &self,
        parent: Option<&PropertyStore>,
    ) -> DFDLResult<ResolvedProperties> {
        self.to_resolved_properties_with_limits(parent, &crate::limits::ResourceLimits::default())
    }

    /// Compiles property store into [`ResolvedProperties`] for a schema term with explicit [`crate::limits::ResourceLimits`].
    pub fn to_resolved_properties_with_limits(
        &self,
        parent: Option<&PropertyStore>,
        limits: &crate::limits::ResourceLimits,
    ) -> DFDLResult<ResolvedProperties> {
        let rep_str = self.resolve_property(parent, "representation", "text");
        let rep = match rep_str {
            "binary" => Representation::Binary,
            "text" => Representation::Text,
            _ => {
                let msg = alloc::format!("Invalid representation property: {}", rep_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let bo_str = self.resolve_property(parent, "byteOrder", "bigEndian");
        let (byte_order, byte_order_expr) = if bo_str.starts_with('{') {
            (ByteOrder::BigEndian, Some(String::from(bo_str)))
        } else {
            (parse_byte_order(bo_str)?, None)
        };

        let bit_str = self.resolve_property(parent, "bitOrder", "mostSignificantBitFirst");
        let bit_order = match bit_str {
            "mostSignificantBitFirst" | "MSBF" => BitOrder::MostSignificantBitFirst,
            "leastSignificantBitFirst" | "LSBF" => BitOrder::LeastSignificantBitFirst,
            _ => {
                let msg = alloc::format!("Invalid bitOrder property: {}", bit_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let text_bidi = self.resolve_property(parent, "textBidi", "no");
        if text_bidi == "yes" {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: textBidi='yes' is not supported",
            ));
        }

        let floating = self.resolve_property(parent, "floating", "no");
        if floating == "yes" {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: floating='yes' is not supported",
            ));
        }

        let default_lk = if self.get_property("length").is_some() {
            "explicit"
        } else {
            "implicit"
        };
        let lk_str = self.resolve_property(parent, "lengthKind", default_lk);
        let length_kind = match lk_str {
            "explicit" => LengthKind::Explicit,
            "implicit" => LengthKind::Implicit,
            "prefixed" => LengthKind::Prefixed,
            "expression" => LengthKind::Expression,
            "delimited" => LengthKind::Delimited,
            "pattern" => LengthKind::Pattern,
            "endOfParent" => LengthKind::EndOfParent,
            _ => {
                let msg = alloc::format!("Invalid lengthKind property: {}", lk_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        if length_kind == LengthKind::Pattern {
            let lp_str = self
                .get_property("lengthPattern")
                .or_else(|| parent.and_then(|p| p.get_property("lengthPattern")))
                .unwrap_or("");
            if lp_str.is_empty() {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "dfdl:lengthPattern property missing when lengthKind='pattern'",
                ));
            }
            if !is_valid_dfdl_regex(lp_str) {
                let msg = alloc::format!(
                    "Schema Definition Error: InvalidRegex: '{}' is not a valid regular expression",
                    lp_str
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let len_str = self
            .get_property("length")
            .or_else(|| parent.and_then(|p| p.get_property("length")));
        let (length, length_expr) = if let Some(s) = len_str {
            if s.starts_with('{') && s.ends_with('}') {
                (None, Some(s.to_string()))
            } else if let Ok(val) = s.parse::<usize>() {
                (Some(val), None)
            } else {
                (None, Some(s.to_string()))
            }
        } else {
            (None, None)
        };

        let lu_str = self.resolve_property(parent, "lengthUnits", "bytes");
        let length_units = match lu_str {
            "bits" => crate::schema::ir::LengthUnits::Bits,
            "characters" => crate::schema::ir::LengthUnits::Characters,
            _ => crate::schema::ir::LengthUnits::Bytes,
        };

        let au_str = self.resolve_property(parent, "alignmentUnits", "bytes");
        let alignment_units = match au_str {
            "bytes" => crate::schema::ir::AlignmentUnits::Bytes,
            "bits" => crate::schema::ir::AlignmentUnits::Bits,
            _ => {
                let msg = alloc::format!("Invalid alignmentUnits property: {}", au_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let align_str = self.resolve_property(parent, "alignment", "1");
        let alignment = if align_str == "implicit" {
            match alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => 1,
                crate::schema::ir::AlignmentUnits::Bits => 8,
            }
        } else {
            align_str.parse::<usize>().unwrap_or(1)
        };

        let ak_str = self.resolve_property(parent, "alignmentKind", "automatic");
        let alignment_kind = match ak_str {
            "manual" => crate::schema::ir::AlignmentKind::Manual,
            _ => crate::schema::ir::AlignmentKind::Automatic,
        };

        let encoding_raw = self.resolve_property(parent, "encoding", "UTF-8");
        let encoding = crate::encoding::EncodingKind::parse_name(encoding_raw);

        let separator = self.get_property("separator").map(String::from);

        let sep_pos_str = self.resolve_property(parent, "separatorPosition", "infix");
        let separator_position = match sep_pos_str {
            "infix" => SeparatorPosition::Infix,
            "prefix" => SeparatorPosition::Prefix,
            "postfix" => SeparatorPosition::Postfix,
            _ => {
                let msg = alloc::format!("Invalid separatorPosition property: {}", sep_pos_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let sep_supp_str = self.resolve_property(parent, "separatorSuppressionPolicy", "never");
        let separator_suppression_policy = match sep_supp_str {
            "never" => SeparatorSuppressionPolicy::Never,
            "trailingEmpty" => SeparatorSuppressionPolicy::TrailingEmpty,
            "trailingEmptyStrict" => SeparatorSuppressionPolicy::TrailingEmptyStrict,
            "anyEmpty" => SeparatorSuppressionPolicy::AnyEmpty,
            _ => {
                let msg = alloc::format!(
                    "Invalid separatorSuppressionPolicy property: {}",
                    sep_supp_str
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let seq_kind_str = self.resolve_property(parent, "sequenceKind", "ordered");
        let sequence_kind = match seq_kind_str {
            "ordered" => SequenceKind::Ordered,
            "unordered" => SequenceKind::Unordered,
            _ => {
                let msg = alloc::format!("Invalid sequenceKind property: {}", seq_kind_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let initiator = self.get_property("initiator").map(String::from);
        let terminator = self.get_property("terminator").map(String::from);

        let ock_str = self.resolve_property(parent, "occursCountKind", "implicit");
        let occurs_count_kind = match ock_str {
            "fixed" => OccursCountKind::Fixed,
            "implicit" => OccursCountKind::Implicit,
            "parsed" => OccursCountKind::Parsed,
            "expression" => OccursCountKind::Expression,
            "stopValue" => OccursCountKind::StopValue,
            _ => OccursCountKind::Implicit,
        };

        let occurs_count_expr = self.get_property("occursCount").map(String::from);

        let discriminator = self.get_property("discriminator").map(String::from);
        let discriminator_test_kind = match self.get_property("discriminatorTestKind") {
            Some("pattern") => crate::schema::ir::TestKind::Pattern,
            _ => crate::schema::ir::TestKind::Expression,
        };
        let discriminator_message = self.get_property("discriminatorMessage").map(String::from);

        let nk_str = self.resolve_property(parent, "nilKind", "literalValue");
        let nil_kind = match nk_str {
            "literalValue" => NilKind::LiteralValue,
            "literalCharacter" => NilKind::LiteralCharacter,
            "logicalValue" => NilKind::LogicalValue,
            _ => NilKind::LiteralValue,
        };

        let nil_value = self
            .get_property("nilValue")
            .or_else(|| parent.and_then(|p| p.get_property("nilValue")))
            .map(String::from);

        let is_nillable = self.get_property("nillable") == Some("true");

        if is_nillable
            && (nil_kind == NilKind::LiteralValue || nil_kind == NilKind::LiteralCharacter)
            && nil_value.as_deref() == Some("")
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: nilValue cannot be empty string (DFDL-06-036R). When empty string is desired, %ES; must be used.",
            ));
        }

        let nvdp_str = self.resolve_property(parent, "nilValueDelimiterPolicy", "both");
        let nil_value_delimiter_policy = match nvdp_str {
            "both" => crate::schema::ir::NilValueDelimiterPolicy::Both,
            "initiator" => crate::schema::ir::NilValueDelimiterPolicy::Initiator,
            "terminator" => crate::schema::ir::NilValueDelimiterPolicy::Terminator,
            "none" => crate::schema::ir::NilValueDelimiterPolicy::None,
            _ => crate::schema::ir::NilValueDelimiterPolicy::Both,
        };

        let leading_skip_str = self.get_property("leadingSkip").unwrap_or("0");
        let leading_skip = if leading_skip_str.starts_with('-') {
            let msg = alloc::format!("Invalid leadingSkip property: {}", leading_skip_str);
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        } else {
            leading_skip_str.parse::<usize>().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Invalid leadingSkip property: {}", leading_skip_str),
                )
            })?
        };
        let max_skip_limit = limits.max_skip_length_in_bytes;
        if !limits.check_skip_length(leading_skip) {
            let msg = alloc::format!(
                "Tunable Limit Exceeded Error: Property leadingSkip ({}) is larger than limit ({})",
                leading_skip,
                max_skip_limit
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        let trailing_skip_str = self.get_property("trailingSkip").unwrap_or("0");
        let trailing_skip = if trailing_skip_str.starts_with('-') {
            let msg = alloc::format!("Invalid trailingSkip property: {}", trailing_skip_str);
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        } else {
            trailing_skip_str.parse::<usize>().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Invalid trailingSkip property: {}", trailing_skip_str),
                )
            })?
        };
        if !limits.check_skip_length(trailing_skip) {
            let msg = alloc::format!(
                "Tunable Limit Exceeded Error: Property trailingSkip ({}) is larger than limit ({})",
                trailing_skip,
                max_skip_limit
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        let base_str = self.resolve_property(parent, "textStandardBase", "10");
        let text_standard_base = match base_str {
            "2" => 2,
            "8" => 8,
            "10" => 10,
            "16" => 16,
            _ => {
                let msg = alloc::format!("Invalid textStandardBase property: {}", base_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let ttk_str = self.resolve_property(parent, "textTrimKind", "none");
        let text_trim_kind = match ttk_str {
            "none" => crate::schema::ir::TextTrimKind::None,
            "head" => crate::schema::ir::TextTrimKind::Head,
            "tail" => crate::schema::ir::TextTrimKind::Tail,
            "both" | "padChar" => crate::schema::ir::TextTrimKind::Both,
            _ => {
                let msg = alloc::format!("Invalid textTrimKind property: {}", ttk_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let parse_justification = |prop_name: &str, val_str: &str| match val_str {
            "left" => Ok(crate::schema::ir::TextJustification::Left),
            "right" => Ok(crate::schema::ir::TextJustification::Right),
            "center" => Ok(crate::schema::ir::TextJustification::Center),
            "none" => Ok(crate::schema::ir::TextJustification::None),
            _ => {
                let msg = alloc::format!("Invalid {} property: {}", prop_name, val_str);
                Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
            }
        };

        let tsj_str = self.resolve_property(parent, "textStringJustification", "left");
        let text_string_justification = parse_justification("textStringJustification", tsj_str)?;

        let tnj_str = self.resolve_property(parent, "textNumberJustification", "right");
        let text_number_justification = parse_justification("textNumberJustification", tnj_str)?;

        let tbj_str = self.resolve_property(parent, "textBooleanJustification", "left");
        let text_boolean_justification = parse_justification("textBooleanJustification", tbj_str)?;

        let tcj_str = self.resolve_property(parent, "textCalendarJustification", "left");
        let text_calendar_justification = parse_justification("textCalendarJustification", tcj_str)?;

        let tnrm_str = self.resolve_property(parent, "textNumberRoundingMode", "roundHalfEven");
        let text_number_rounding_mode = match tnrm_str {
            "roundCeiling" => crate::schema::ir::TextNumberRoundingMode::RoundCeiling,
            "roundFloor" => crate::schema::ir::TextNumberRoundingMode::RoundFloor,
            "roundDown" => crate::schema::ir::TextNumberRoundingMode::RoundDown,
            "roundUp" => crate::schema::ir::TextNumberRoundingMode::RoundUp,
            "roundHalfEven" => crate::schema::ir::TextNumberRoundingMode::RoundHalfEven,
            "roundHalfDown" => crate::schema::ir::TextNumberRoundingMode::RoundHalfDown,
            "roundHalfUp" => crate::schema::ir::TextNumberRoundingMode::RoundHalfUp,
            "roundUnnecessary" => crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary,
            _ => {
                let msg = alloc::format!("Invalid textNumberRoundingMode property: {}", tnrm_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };
        let text_number_rounding_explicit =
            self.resolve_property(parent, "textNumberRounding", "pattern") == "explicit";
        let text_number_rounding_increment = if text_number_rounding_explicit {
            let inc = self.resolve_property(parent, "textNumberRoundingIncrement", "").trim();
            if inc.is_empty() {
                None
            } else {
                Some(inc.to_string())
            }
        } else {
            None
        };

        let text_pad_char_raw = self
            .get_property("textStringPadCharacter")
            .or_else(|| self.get_property("textPadChar"))
            .or_else(|| parent.and_then(|p| p.get_property("textStringPadCharacter")))
            .or_else(|| parent.and_then(|p| p.get_property("textPadChar")))
            .unwrap_or(" ");
        let text_pad_char = decode_dfdl_character_entities(text_pad_char_raw);

        let fill_byte_raw = self.get_property("fillByte");
        let fill_byte_defined = fill_byte_raw.is_some();
        let fill_byte = if let Some(fb) = fill_byte_raw {
            if fb.is_empty() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: fillByte property cannot be empty",
                ));
            }
            if fb.starts_with('%') && fb.ends_with(';') {
                let inner = &fb[1..fb.len().saturating_sub(1)];
                if let Some(hex_val) = inner
                    .strip_prefix("#r")
                    .or_else(|| inner.strip_prefix("#x"))
                {
                    u8::from_str_radix(hex_val, 16).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Invalid byte entity in fillByte: must be 1 character or byte",
                        )
                    })?
                } else if let Some(dec_val) = inner.strip_prefix("#d") {
                    dec_val.parse::<u8>().map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Invalid decimal entity in fillByte: must be 1 character or byte",
                        )
                    })?
                } else {
                    match fb {
                        "%NUL;" => 0,
                        "%SP;" => b' ',
                        "%HT;" => b'\t',
                        "%LF;" => b'\n',
                        "%CR;" => b'\r',
                        _ => {
                            return Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                &alloc::format!(
                                    "Schema Definition Error: Unsupported entity '{}' for fillByte: must be 1 character or byte",
                                    fb
                                ),
                            ));
                        }
                    }
                }
            } else if fb.chars().count() == 1 {
                let Some(ch) = fb.chars().next() else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte '{}' must be 1 character or byte",
                            fb
                        ),
                    ));
                };
                let enc = self.get_property("encoding").unwrap_or("UTF-8");
                let enc_upper = enc.to_ascii_uppercase();
                if enc_upper.contains("6-BIT")
                    || enc_upper.contains("5-BIT")
                    || enc_upper.contains("7-BIT")
                    || enc_upper.contains("BIT-PACKED")
                {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte property requires a single-byte character for encoding '{}', but '{}' is not a byte-sized encoding",
                            enc, enc
                        ),
                    ));
                }
                if enc.eq_ignore_ascii_case("UTF-8") && ch.len_utf8() > 1 {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: fillByte must be a single-byte character for encoding '{}', but '{}' takes {} bytes",
                            enc, ch, ch.len_utf8()
                        ),
                    ));
                }
                ch as u8
            } else {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: fillByte '{}' must be 1 character or byte",
                        fb
                    ),
                ));
            }
        } else {
            0
        };

        let input_value_calc = self.get_property("inputValueCalc").map(String::from);
        let output_value_calc = self.get_property("outputValueCalc").map(String::from);

        let hidden_group_ref = self
            .get_property("hiddenGroupRef")
            .or_else(|| parent.and_then(|p| p.get_property("hiddenGroupRef")))
            .map(String::from);

        let parent_hidden = parent.is_some_and(|p| {
            p.get_property("hiddenGroupRef").is_some()
                || p.get_property("hidden") == Some("true")
                || p.get_property("is_hidden_group") == Some("true")
                || p.get_property("is_hidden") == Some("true")
        });
        let is_hidden = hidden_group_ref.is_some()
            || parent_hidden
            || self.get_property("hidden") == Some("true")
            || self.get_property("is_hidden_group") == Some("true")
            || self.get_property("is_hidden") == Some("true");

        let mut asserts = self.asserts.clone();
        if let Some(test_expr) = self.get_property("assert") {
            let test_kind = if self.get_property("testKind") == Some("pattern") {
                crate::schema::ir::TestKind::Pattern
            } else {
                crate::schema::ir::TestKind::Expression
            };
            let test_ast = if test_kind == crate::schema::ir::TestKind::Expression {
                Some(crate::expr::parse_expr(test_expr)?)
            } else {
                None
            };
            let compiled_regex = if test_kind == crate::schema::ir::TestKind::Pattern {
                let anchored = if let Some(stripped) = test_expr.strip_prefix('^') {
                    alloc::format!("^(?:{})", stripped)
                } else {
                    alloc::format!("^(?:{})", test_expr)
                };
                let re = crate::pattern::DfdlRegex::new(&anchored)
                    .or_else(|_| crate::pattern::DfdlRegex::new(test_expr))
                    .map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: Invalid regex pattern in dfdl:assert: '{}': {}", test_expr, e),
                        )
                    })?;
                Some(re)
            } else {
                None
            };
            let _ = try_push(
                &mut asserts,
                CompiledAssert {
                    test_kind,
                    test_expr: test_expr.to_string(),
                    test_ast,
                    compiled_regex,
                    message: None,
                    failure_type: crate::schema::ir::FailureType::ProcessingError,
                },
            );
        }

        if self.get_property("discriminator").is_some() && !asserts.is_empty() {
            let msg = "Schema Definition Error: A component cannot have both a discriminator and an assert statement.";
            return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
        }

        for assert_item in &mut asserts {
            if assert_item.test_kind == crate::schema::ir::TestKind::Pattern {
                if !is_valid_dfdl_regex(&assert_item.test_expr) {
                    let msg = alloc::format!(
                        "Schema Definition Error: Invalid regex pattern in dfdl:assert testPattern: '{}'",
                        assert_item.test_expr
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            } else if assert_item.test_kind == crate::schema::ir::TestKind::Expression
                && assert_item.test_ast.is_none()
            {
                assert_item.test_ast = Some(crate::expr::parse_expr(&assert_item.test_expr)?);
            }
        }

        let prefix_length_type = self
            .get_property("prefixLengthType")
            .or_else(|| parent.and_then(|p| p.get_property("prefixLengthType")))
            .map(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc);

        let prefix_includes_prefix_length = self
            .get_property("prefixIncludesPrefixLength")
            .or_else(|| parent.and_then(|p| p.get_property("prefixIncludesPrefixLength")))
            .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"));

        let tncp_str = self.resolve_property(parent, "textNumberCheckPolicy", "lax");
        let text_number_check_policy = match tncp_str {
            "strict" => crate::schema::ir::TextNumberCheckPolicy::Strict,
            _ => crate::schema::ir::TextNumberCheckPolicy::Lax,
        };

        let text_number_pattern = self
            .get_property("textNumberPattern")
            .or_else(|| parent.and_then(|p| p.get_property("textNumberPattern")))
            .map(String::from);

        let text_decimal_sep_opt = self
            .get_property("textStandardDecimalSeparator")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardDecimalSeparator")));
        let text_standard_decimal_separator = match text_decimal_sep_opt {
            Some(sep) if !sep.is_empty() => {
                if sep.starts_with('{') && sep.ends_with('}') {
                    String::from(sep)
                } else {
                    let decoded = decode_dfdl_character_entities(sep);
                    if decoded.chars().count() != 1 {
                        let msg = alloc::format!(
                            "Schema Definition Error: Length of string must be exactly 1 character (cannot have more than one character) for textStandardDecimalSeparator, got '{}'",
                            sep
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    decoded
                }
            }
            Some(_) => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardDecimalSeparator",
                ));
            }
            None => {
                // If not explicitly set, default for resolution while permitting empty checks
                String::from(".")
            }
        };

        if let Some(ref pat) = text_number_pattern {
            if pat.starts_with(';') {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Invalid textNumberPattern: A positive part is required in the pattern",
                ));
            }
            if (pat.contains('E') || pat.contains('e')) && pat.contains(',') {
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid textNumberPattern: Cannot have grouping separator in scientific notation '{}'",
                    pat
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if pat.ends_with('*') {
                let msg = alloc::format!(
                    "Schema Definition Error: Invalid textNumberPattern: Malformed pattern: pad character missing after '*' in \"{}\"",
                    pat
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if pat.contains('V') {
                for subpat in pat.split(';') {
                    if let Some(idx) = subpat.find('V') {
                        let after_v = &subpat[idx.saturating_add(1)..];
                        if after_v.contains('#') {
                            let msg = alloc::format!(
                                "Schema Definition Error: Invalid textNumberPattern '{}': In textNumberPattern with 'V', pattern must consist of '#', then digits 0-9 then 'V' then digits 0-9 (the portion after 'V' must contain only digits 0-9)",
                                pat
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
            if pat.contains(';') && pat.contains("text") {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Property textNumberPattern must contain only digits 0-9",
                ));
            }
        }

        let text_number_rep = self.resolve_property(parent, "textNumberRep", "standard");
        if text_number_rep == "zoned" {
            if let Some(ref pat) = text_number_pattern {
                if pat.contains('@')
                    || pat.contains('E')
                    || pat.contains('e')
                    || pat.contains(';')
                    || pat.chars().filter(|&c| c == '+').count() > 1
                {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: textNumberPattern for zoned number rep cannot contain '@', 'E', 'e', ';', or multiple '+'",
                    ));
                }
            }
        }

        let text_grp_sep_opt = self
            .get_property("textStandardGroupingSeparator")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardGroupingSeparator")));
        if let Some(ref pat) = text_number_pattern {
            if pat.contains(',') && text_grp_sep_opt.is_none() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Property textStandardGroupingSeparator is not defined",
                ));
            }
        }
        let text_standard_grouping_separator = match text_grp_sep_opt {
            Some(sep) if !sep.is_empty() => {
                if sep.starts_with('{') && sep.ends_with('}') {
                    String::from(sep)
                } else {
                    let decoded = decode_dfdl_character_entities(sep);
                    if decoded.chars().count() != 1 {
                        let msg = alloc::format!(
                            "Schema Definition Error: Length of string must be exactly 1 character (cannot have more than one character) for textStandardGroupingSeparator, got '{}'",
                            sep
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    decoded
                }
            }
            Some(_) => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardGroupingSeparator",
                ));
            }
            None => String::from(","),
        };

        let text_exp_sep_opt = self
            .get_property("textStandardExponentRep")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")));
        let text_standard_exponent_rep = match text_exp_sep_opt {
            Some(exp) => decode_dfdl_character_entities(exp),
            None => String::from("E"),
        };

        if text_number_rep == "standard" {
            let has_custom_num_props = text_decimal_sep_opt.is_some()
                || text_grp_sep_opt.is_some()
                || text_exp_sep_opt.is_some();
            if has_custom_num_props {
                let nan_rep = self
                    .get_property("textStandardNaNRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardNaNRep")))
                    .unwrap_or("");
                let inf_rep = self
                    .get_property("textStandardInfinityRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardInfinityRep")))
                    .unwrap_or("");
                let zero_rep = self
                    .get_property("textStandardZeroRep")
                    .or_else(|| parent.and_then(|p| p.get_property("textStandardZeroRep")))
                    .unwrap_or("");

                let mut vals = alloc::vec::Vec::new();
                if !text_standard_decimal_separator.is_empty() {
                    vals.push((
                        "textStandardDecimalSeparator",
                        text_standard_decimal_separator.as_str(),
                    ));
                }
                if !text_standard_grouping_separator.is_empty() {
                    vals.push((
                        "textStandardGroupingSeparator",
                        text_standard_grouping_separator.as_str(),
                    ));
                }
                if !text_standard_exponent_rep.is_empty() {
                    vals.push((
                        "textStandardExponentRep",
                        text_standard_exponent_rep.as_str(),
                    ));
                }
                if !nan_rep.is_empty() {
                    vals.push(("textStandardNaNRep", nan_rep));
                }
                if !inf_rep.is_empty() {
                    vals.push(("textStandardInfinityRep", inf_rep));
                }
                if !zero_rep.is_empty() {
                    vals.push(("textStandardZeroRep", zero_rep));
                }

                for (idx1, &(name1, val1)) in vals.iter().enumerate() {
                    for &(name2, val2) in vals.iter().skip(idx1.saturating_add(1)) {
                        if val1 == val2 {
                            let msg = alloc::format!(
                                "Schema Definition Error: Non-distinct property values for textStandardDecimalSeparator, textStandardGroupingSeparator, textStandardExponentRep, textStandardInfinityRep, textStandardNaNRep, textStandardZeroRep ('{}' and '{}' both have value '{}')",
                                name1, name2, val1
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
        }

        let facets = crate::schema::ir::SimpleTypeFacets {
            min_inclusive: self.get_property("minInclusive").map(String::from),
            max_inclusive: self.get_property("maxInclusive").map(String::from),
            min_exclusive: self.get_property("minExclusive").map(String::from),
            max_exclusive: self.get_property("maxExclusive").map(String::from),
            pattern: self.get_property("pattern").map(String::from),
            enumeration: self
                .bindings
                .iter()
                .filter(|b| b.key == "enumeration")
                .map(|b| b.value.clone())
                .collect(),
            min_length: self.get_property("minLength").and_then(|s| s.parse().ok()),
            max_length: self.get_property("maxLength").and_then(|s| s.parse().ok()),
            length: self
                .get_property("xsdLength")
                .or_else(|| self.get_property("facetLength"))
                .and_then(|s| s.parse().ok()),
            total_digits: self
                .get_property("totalDigits")
                .and_then(|s| s.parse().ok()),
            fraction_digits: self
                .get_property("fractionDigits")
                .and_then(|s| s.parse().ok()),
            rep_values: self
                .bindings
                .iter()
                .filter_map(|b| {
                    b.key.strip_prefix("repValue:").map(|k| (String::from(k), b.value.clone()))
                })
                .collect(),
        };

        let length_pattern = self
            .get_property("lengthPattern")
            .or_else(|| parent.and_then(|p| p.get_property("lengthPattern")))
            .map(String::from);

        let binary_number_rep = match self.resolve_property(parent, "binaryNumberRep", "binary") {
            "packed" => crate::schema::ir::BinaryNumberRep::Packed,
            "bcd" => crate::schema::ir::BinaryNumberRep::Bcd,
            "ibm4690Packed" => crate::schema::ir::BinaryNumberRep::Ibm4690Packed,
            _ => crate::schema::ir::BinaryNumberRep::Binary,
        };

        let binary_calendar_rep = match self.resolve_property(parent, "binaryCalendarRep", "bcd") {
            "binarySeconds" => crate::schema::ir::BinaryCalendarRep::BinarySeconds,
            "binaryMilliseconds" => crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds,
            "packed" => crate::schema::ir::BinaryCalendarRep::Packed,
            "ibm4690Packed" => crate::schema::ir::BinaryCalendarRep::Ibm4690Packed,
            _ => crate::schema::ir::BinaryCalendarRep::Bcd,
        };



        if let Some(l_str) = self.get_property("length") {
            if let Ok(val) = l_str.parse::<i64>() {
                if val < 0 {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: dfdl:length cannot be negative: {}",
                        val
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let initiated_content_str = self.resolve_property(parent, "initiatedContent", "no");
        let initiated_content = matches!(initiated_content_str, "yes");

        let truncate_str = self.resolve_property(parent, "truncateSpecifiedLengthString", "no");
        let truncate_specified_length_string = matches!(truncate_str, "yes");

        let encoding_error_policy_error =
            self.resolve_property(parent, "encodingErrorPolicy", "replace") == "error";
        let encoding_error_policy_defined = self.get_property("encodingErrorPolicy").is_some()
            || parent.is_some_and(|p| p.get_property("encodingErrorPolicy").is_some());

        let binary_decimal_virtual_point = self
            .resolve_property(parent, "binaryDecimalVirtualPoint", "0")
            .parse::<i32>()
            .unwrap_or(0);

        let cal_pat_kind = self.resolve_property(parent, "calendarPatternKind", "implicit");
        let calendar_pattern = if cal_pat_kind == "explicit" {
            self.get_property("calendarPattern")
                .or_else(|| parent.and_then(|p| p.get_property("calendarPattern")))
                .map(String::from)
        } else {
            None
        };
        let calendar_language = self
            .get_property("calendarLanguage")
            .or_else(|| parent.and_then(|p| p.get_property("calendarLanguage")))
            .map(String::from);
        if let Some(ref lang) = calendar_language {
            let trimmed = lang.trim();
            if !trimmed.starts_with('{') {
                crate::kernel::parser::calendar::validate_calendar_language_syntax(trimmed)?;
            }
        }
        let calendar_time_zone = self
            .get_property("calendarTimeZone")
            .or_else(|| parent.and_then(|p| p.get_property("calendarTimeZone")))
            .map(String::from);
        let empty_element_parse_policy =
            match self.resolve_property(parent, "emptyElementParsePolicy", "treatAsEmpty") {
                "treatAsAbsent" => crate::schema::ir::EmptyElementParsePolicy::TreatAsAbsent,
                _ => crate::schema::ir::EmptyElementParsePolicy::TreatAsEmpty,
            };

        let choice_length_kind = match self.resolve_property(parent, "choiceLengthKind", "implicit") {
            "explicit" => LengthKind::Explicit,
            _ => LengthKind::Implicit,
        };
        let choice_length = self
            .get_property("choiceLength")
            .or_else(|| parent.and_then(|p| p.get_property("choiceLength")))
            .and_then(|s| s.parse::<usize>().ok());

        let choice_dispatch_key = self.get_property("choiceDispatchKey").map(String::from);
        let choice_branch_key = self.get_property("choiceBranchKey").map(String::from);
        let choice_branch_key_ranges = self.get_property("choiceBranchKeyRanges").map(String::from);

        let binary_calendar_epoch = self
            .get_property("binaryCalendarEpoch")
            .or_else(|| parent.and_then(|p| p.get_property("binaryCalendarEpoch")))
            .map(String::from);

        if let Some(ref epoch_str) = binary_calendar_epoch {
            if !is_valid_xs_date_time(epoch_str) {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Invalid binaryCalendarEpoch '{}'; must be a valid xs:dateTime",
                        epoch_str
                    ),
                ));
            }
        }



        let pup_str = self
            .get_property("parseUnparsePolicy")
            .or_else(|| self.get_property("dfdlx:parseUnparsePolicy"))
            .or_else(|| self.get_property("daf:parseUnparsePolicy"))
            .or_else(|| {
                parent.and_then(|p| {
                    p.get_property("parseUnparsePolicy")
                        .or_else(|| p.get_property("dfdlx:parseUnparsePolicy"))
                        .or_else(|| p.get_property("daf:parseUnparsePolicy"))
                })
            })
            .unwrap_or("both");
        let parse_unparse_policy = match pup_str {
            "both" => ParseUnparsePolicy::Both,
            "parseOnly" => ParseUnparsePolicy::ParseOnly,
            "unparseOnly" => ParseUnparsePolicy::UnparseOnly,
            _ => {
                let msg = alloc::format!("Schema Definition Error: Invalid parseUnparsePolicy '{}'", pup_str);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        };

        let output_new_line = self
            .get_property("outputNewLine")
            .or_else(|| parent.and_then(|p| p.get_property("outputNewLine")))
            .map(String::from);

        if let Some(ref onl) = output_new_line {
            if onl.is_empty() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: dfdl:outputNewLine cannot be empty string",
                ));
            }
        }

        if let Some(p) = parent {
            let parent_pup = p
                .get_property("parseUnparsePolicy")
                .or_else(|| p.get_property("dfdlx:parseUnparsePolicy"))
                .or_else(|| p.get_property("daf:parseUnparsePolicy"));
            if let Some(p_policy) = parent_pup {
                if (p_policy == "both" && (pup_str == "parseOnly" || pup_str == "unparseOnly"))
                    || (p_policy == "parseOnly" && pup_str == "unparseOnly")
                    || (p_policy == "unparseOnly" && pup_str == "parseOnly")
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: Incompatible parseUnparsePolicy: parent is '{}' and child is '{}'",
                        p_policy, pup_str
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let effective_ns: &[(String, String)] = if self.in_scope_namespaces.is_empty() {
            parent.map(|p| p.in_scope_namespaces.as_slice()).unwrap_or(&[])
        } else {
            &self.in_scope_namespaces
        };
        if !effective_ns.is_empty() {
            if let Some(ref expr) = input_value_calc {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = output_value_calc {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = discriminator {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = length_expr {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = occurs_count_expr {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = byte_order_expr {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            if let Some(ref expr) = choice_dispatch_key {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            for a in &asserts {
                crate::expr::validate_expression_namespaces(&a.test_expr, effective_ns)?;
            }
            for (_, expr) in &self.set_variables {
                crate::expr::validate_expression_namespaces(expr, effective_ns)?;
            }
            for (_, opt_expr) in &self.new_variable_instances {
                if let Some(expr) = opt_expr {
                    crate::expr::validate_expression_namespaces(expr, effective_ns)?;
                }
            }
        }

        let byte_order_prop = if let Some(ref bo_expr) = byte_order_expr {
            DfdlProp::parse_with(bo_expr, parse_byte_order)?
        } else {
            DfdlProp::constant(byte_order)
        };
        let encoding_prop = DfdlProp::parse_with(encoding_raw, |raw| Ok(crate::encoding::EncodingKind::parse_name(raw)))?;
        let separator_prop = separator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let initiator_prop = initiator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let terminator_prop = terminator.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let length_prop = if let Some(s) = len_str {
            if let Ok(p) = DfdlProp::parse_with(s, |raw| {
                raw.trim().parse::<usize>().map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Invalid length constant '{}': {}", raw, e),
                    )
                })
            }) {
                Some(p)
            } else if length_kind == LengthKind::Explicit {
                Some(DfdlProp::parse_with(s, |raw| {
                    raw.trim().parse::<usize>().map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Invalid length constant '{}': {}", raw, e),
                        )
                    })
                })?)
            } else {
                None
            }
        } else {
            None
        };
        let occurs_count_prop = if let Some(ref oce) = occurs_count_expr {
            DfdlProp::parse_with(oce, |raw| {
                raw.trim().parse::<usize>().map_err(|e| {
                    DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Invalid occursCount constant '{}': {}", raw, e),
                    )
                })
            }).ok()
        } else {
            None
        };
        let nil_value_prop = nil_value.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let text_standard_decimal_separator_prop = DfdlProp::parse_str(&text_standard_decimal_separator)?;
        let text_standard_grouping_separator_prop = DfdlProp::parse_str(&text_standard_grouping_separator)?;
        let calendar_pattern_prop = calendar_pattern.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let calendar_language_prop = calendar_language.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let calendar_time_zone_prop = calendar_time_zone.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let output_new_line_prop = output_new_line.as_deref().and_then(|s| if s.is_empty() { None } else { DfdlProp::parse_str(s).ok() });
        let text_standard_exponent_rep_prop = self
            .get_property("textStandardExponentRep")
            .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_with(s, |raw| Ok(decode_dfdl_character_entities(raw))).ok()
                }
            });
        let text_boolean_true_rep_prop = self
            .get_property("textBooleanTrueRep")
            .or_else(|| parent.and_then(|p| p.get_property("textBooleanTrueRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_str(s).ok()
                }
            });
        let text_boolean_false_rep_prop = self
            .get_property("textBooleanFalseRep")
            .or_else(|| parent.and_then(|p| p.get_property("textBooleanFalseRep")))
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    DfdlProp::parse_str(s).ok()
                }
            });
        let input_value_calc_prop = if let Some(s) = input_value_calc.as_deref() {
            if s.is_empty() {
                None
            } else {
                let trimmed = s.trim();
                let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                    trimmed[1..trimmed.len().saturating_sub(1)].trim()
                } else {
                    trimmed
                };
                let ast = crate::expr::parse_expr(inner).map_err(|e| {
                    if e.kind == DFDLErrorKind::SchemaDefinition {
                        e
                    } else {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: Invalid expression '{}': Unable to parse expression: {}", inner, e),
                        )
                    }
                })?;
                Some(DfdlProp::expression(s, ast))
            }
        } else {
            None
        };
        let output_value_calc_prop = if let Some(s) = output_value_calc.as_deref() {
            if s.is_empty() {
                None
            } else {
                let trimmed = s.trim();
                let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                    trimmed[1..trimmed.len().saturating_sub(1)].trim()
                } else {
                    trimmed
                };
                let ast = crate::expr::parse_expr(inner).map_err(|e| {
                    if e.kind == DFDLErrorKind::SchemaDefinition {
                        e
                    } else {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: Invalid expression '{}': Unable to parse expression: {}", inner, e),
                        )
                    }
                })?;
                Some(DfdlProp::expression(s, ast))
            }
        } else {
            None
        };

        let discriminator_ast = if let Some(ref expr) = discriminator {
            if discriminator_test_kind == crate::schema::ir::TestKind::Expression {
                let trimmed = expr.trim();
                if !trimmed.is_empty() {
                    Some(crate::expr::parse_expr(trimmed)?)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let discriminator_regex = if let Some(ref expr) = discriminator {
            if discriminator_test_kind == crate::schema::ir::TestKind::Pattern {
                let trimmed = expr.trim();
                let anchored = if let Some(stripped) = trimmed.strip_prefix('^') {
                    alloc::format!("^(?:{})", stripped)
                } else {
                    alloc::format!("^(?:{})", trimmed)
                };
                let re = crate::pattern::DfdlRegex::new(&anchored)
                    .or_else(|_| crate::pattern::DfdlRegex::new(trimmed))
                    .map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: Invalid regex pattern in discriminator: '{}': {}", trimmed, e),
                        )
                    })?;
                Some(re)
            } else {
                None
            }
        } else {
            None
        };
        let choice_dispatch_key_ast = if let Some(ref expr) = choice_dispatch_key {
            let trimmed = expr.trim();
            if !trimmed.is_empty() {
                Some(crate::expr::parse_expr(trimmed)?)
            } else {
                None
            }
        } else {
            None
        };
        let set_variables_compiled: Vec<(crate::types::QName, Option<crate::expr::ExprAst>, String)> = self
            .set_variables
            .iter()
            .map(|(q, val_expr)| {
                let trimmed = val_expr.trim();
                let ast = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                    let expr_body = trimmed
                        .get(1..trimmed.len().saturating_sub(1))
                        .unwrap_or("")
                        .trim();
                    crate::expr::parse_expr(expr_body).ok()
                } else {
                    None
                };
                (q.clone(), ast, val_expr.clone())
            })
            .collect();

        let new_variable_instances_compiled: Vec<(crate::types::QName, Option<crate::expr::ExprAst>, Option<String>)> = self
            .new_variable_instances
            .iter()
            .map(|(q, opt_expr)| {
                let ast = opt_expr.as_ref().and_then(|val_expr| {
                    let trimmed = val_expr.trim();
                    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                        let expr_body = trimmed
                            .get(1..trimmed.len().saturating_sub(1))
                            .unwrap_or("")
                            .trim();
                        crate::expr::parse_expr(expr_body).ok()
                    } else {
                        None
                    }
                });
                (q.clone(), ast, opt_expr.clone())
            })
            .collect();

        Ok(ResolvedProperties {
            representation: rep,
            byte_order,
            byte_order_expr,
            byte_order_prop,
            bit_order,
            length_kind,
            length,
            length_expr,
            length_prop,
            prefix_length_type,
            prefix_includes_prefix_length,
            length_pattern,
            length_units,
            alignment,
            alignment_units,
            alignment_kind,
            leading_skip,
            trailing_skip,
            encoding_char_bits: encoding.char_bits(),
            encoding_unit_bits: encoding.unit_bits(),
            encoding,
            encoding_prop,
            separator,
            separator_prop,
            separator_position,
            separator_suppression_policy,
            sequence_kind,
            initiator,
            initiator_prop,
            initiated_content,
            choice_length_kind,
            choice_length,
            terminator,
            terminator_prop,
            occurs_count_kind,
            occurs_count_expr,
            occurs_count_prop,
            discriminator,
            discriminator_ast,
            discriminator_regex,
            discriminator_test_kind,
            discriminator_message,
            nil_kind,
            nil_value,
            nil_value_prop,
            nil_value_delimiter_policy,
            text_trim_kind,
            text_pad_char,
            fill_byte,
            fill_byte_defined,
            fill_byte_raw: fill_byte_raw.map(String::from),
            input_value_calc,
            input_value_calc_prop,
            output_value_calc,
            output_value_calc_prop,
            asserts,
            hidden_group_ref,
            is_hidden,
            set_variables: self.set_variables.clone(),
            set_variables_compiled,
            new_variable_instances: self.new_variable_instances.clone(),
            new_variable_instances_compiled,
            text_number_check_policy,
            text_number_pattern,
            text_standard_decimal_separator,
            text_standard_decimal_separator_prop,
            text_standard_grouping_separator,
            text_standard_grouping_separator_prop,
            truncate_specified_length_string,
            text_string_justification,
            text_number_justification,
            text_boolean_justification,
            text_calendar_justification,
            text_number_rounding_mode,
            text_number_rounding_explicit,
            text_number_rounding_increment,
            encoding_error_policy_error,
            encoding_error_policy_defined,
            binary_decimal_virtual_point,
            calendar_pattern,
            calendar_pattern_prop,
            calendar_language,
            calendar_language_prop,
            calendar_time_zone,
            calendar_time_zone_prop,
            binary_number_rep,
            binary_packed_sign_codes: self
                .get_property("binaryPackedSignCodes")
                .or_else(|| parent.and_then(|p| p.get_property("binaryPackedSignCodes")))
                .map(String::from),
            binary_calendar_rep,
            text_standard_base,
            facets,
            choice_dispatch_key,
            choice_dispatch_key_ast,
            choice_branch_key,
            choice_branch_key_ranges,
            empty_element_parse_policy,
            binary_calendar_epoch,
            parse_unparse_policy,
            output_new_line,
            output_new_line_prop,
            text_standard_nan_rep: self
                .get_property("textStandardNaNRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardNaNRep")))
                .map(decode_dfdl_character_entities),
            text_standard_infinity_rep: self
                .get_property("textStandardInfinityRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardInfinityRep")))
                .map(decode_dfdl_character_entities),
            text_standard_zero_rep: self
                .get_property("textStandardZeroRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardZeroRep")))
                .map(String::from),
            text_number_pad_character: self
                .get_property("textNumberPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textNumberPadCharacter")))
                .map(decode_dfdl_character_entities),
            text_calendar_pad_character: self
                .get_property("textCalendarPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textCalendarPadCharacter")))
                .map(decode_dfdl_character_entities),
            text_standard_exponent_rep: self
                .get_property("textStandardExponentRep")
                .or_else(|| parent.and_then(|p| p.get_property("textStandardExponentRep")))
                .map(decode_dfdl_character_entities),
            text_standard_exponent_rep_prop,
            ignore_case: self
                .get_property("ignoreCase")
                .or_else(|| parent.and_then(|p| p.get_property("ignoreCase")))
                .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")),
            text_boolean_true_rep: self
                .get_property("textBooleanTrueRep")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanTrueRep")))
                .map(String::from),
            text_boolean_true_rep_prop,
            text_boolean_false_rep: self
                .get_property("textBooleanFalseRep")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanFalseRep")))
                .map(String::from),
            text_boolean_false_rep_prop,
            text_boolean_pad_character: self
                .get_property("textBooleanPadCharacter")
                .or_else(|| parent.and_then(|p| p.get_property("textBooleanPadCharacter")))
                .map(decode_dfdl_character_entities),
            binary_boolean_true_rep: self
                .get_property("binaryBooleanTrueRep")
                .or_else(|| parent.and_then(|p| p.get_property("binaryBooleanTrueRep")))
                .map_or(crate::schema::ir::BinaryBooleanRep::NotSpecified, |s| {
                    if s.is_empty() {
                        crate::schema::ir::BinaryBooleanRep::Empty
                    } else if let Ok(v) = s.parse::<i64>() {
                        crate::schema::ir::BinaryBooleanRep::Value(v)
                    } else {
                        crate::schema::ir::BinaryBooleanRep::NotSpecified
                    }
                }),
            binary_boolean_false_rep: self
                .get_property("binaryBooleanFalseRep")
                .or_else(|| parent.and_then(|p| p.get_property("binaryBooleanFalseRep")))
                .map_or(crate::schema::ir::BinaryBooleanRep::NotSpecified, |s| {
                    if s.is_empty() {
                        crate::schema::ir::BinaryBooleanRep::Empty
                    } else if let Ok(v) = s.parse::<i64>() {
                        crate::schema::ir::BinaryBooleanRep::Value(v)
                    } else {
                        crate::schema::ir::BinaryBooleanRep::NotSpecified
                    }
                }),
            document_final_terminator_can_be_missing: !self
                .get_property("documentFinalTerminatorCanBeMissing")
                .or_else(|| parent.and_then(|p| p.get_property("documentFinalTerminatorCanBeMissing")))
                .is_some_and(|v| v.eq_ignore_ascii_case("no") || v.eq_ignore_ascii_case("false")),
            decimal_signed: !self
                .get_property("decimalSigned")
                .or_else(|| parent.and_then(|p| p.get_property("decimalSigned")))
                .is_some_and(|v| v.eq_ignore_ascii_case("no") || v.eq_ignore_ascii_case("false")),
            text_number_rep: match self.resolve_property(parent, "textNumberRep", "standard") {
                "zoned" => crate::schema::ir::TextNumberRep::Zoned,
                _ => crate::schema::ir::TextNumberRep::Standard,
            },
            text_zoned_sign_style: match self.resolve_property(parent, "textZonedSignStyle", "asciiStandard") {
                "asciiTranslatedEBCDIC" => crate::schema::ir::TextZonedSignStyle::AsciiTranslatedEbcdic,
                "asciiCARealiaModified" => crate::schema::ir::TextZonedSignStyle::AsciiCaRealiaModified,
                "asciiTandemModified" => crate::schema::ir::TextZonedSignStyle::AsciiTandemModified,
                _ => crate::schema::ir::TextZonedSignStyle::AsciiStandard,
            },
            calendar_check_policy: match self.resolve_property(parent, "calendarCheckPolicy", "lax") {
                "strict" => crate::schema::ir::CalendarCheckPolicy::Strict,
                _ => crate::schema::ir::CalendarCheckPolicy::Lax,
            },
            calendar_first_day_of_week: match self.resolve_property(parent, "calendarFirstDayOfWeek", "Sunday") {
                "Monday" => crate::schema::ir::CalendarFirstDayOfWeek::Monday,
                "Tuesday" => crate::schema::ir::CalendarFirstDayOfWeek::Tuesday,
                "Wednesday" => crate::schema::ir::CalendarFirstDayOfWeek::Wednesday,
                "Thursday" => crate::schema::ir::CalendarFirstDayOfWeek::Thursday,
                "Friday" => crate::schema::ir::CalendarFirstDayOfWeek::Friday,
                "Saturday" => crate::schema::ir::CalendarFirstDayOfWeek::Saturday,
                _ => crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
            },
            text_pad_kind: {
                let s = self
                    .get_property("textPadKind")
                    .or_else(|| parent.and_then(|p| p.get_property("textPadKind")))
                    .unwrap_or("none");
                match s {
                    "none" => crate::schema::ir::TextPadKind::None,
                    "padChar" => crate::schema::ir::TextPadKind::PadChar,
                    _ => {
                        let msg = alloc::format!("Invalid textPadKind property: {}", s);
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            },
            text_output_min_length: {
                let toml = self
                    .get_property("textOutputMinLength")
                    .or_else(|| parent.and_then(|p| p.get_property("textOutputMinLength")))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let facet_min = self
                    .get_property("minLength")
                    .or_else(|| self.get_property("length"))
                    .or_else(|| parent.and_then(|p| p.get_property("minLength").or_else(|| p.get_property("length"))))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                toml.max(facet_min)
            },
            escape_scheme: {
                let es_ref = self
                    .get_property("escapeSchemeRef")
                    .or_else(|| parent.and_then(|p| p.get_property("escapeSchemeRef")));
                if es_ref == Some("") {
                    None
                } else {
                    let escape_kind_str = self
                        .get_property("escapeKind")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeKind")));
                    let escape_char = self
                        .get_property("escapeCharacter")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeCharacter")));
                    let escape_block_start = self
                        .get_property("escapeBlockStart")
                        .or_else(|| parent.and_then(|p| p.get_property("escapeBlockStart")));

                    if escape_kind_str.is_some() || escape_char.is_some() || escape_block_start.is_some() {
                        let escape_kind = match escape_kind_str.unwrap_or("escapeCharacter") {
                            "escapeBlock" => crate::schema::ir::EscapeKind::EscapeBlock,
                            _ => crate::schema::ir::EscapeKind::EscapeCharacter,
                        };
                        let escape_character = escape_char.map(decode_dfdl_character_entities);
                        let escape_escape_char_raw = self
                            .get_property("escapeEscapeCharacter")
                            .or_else(|| parent.and_then(|p| p.get_property("escapeEscapeCharacter")));
                        let escape_escape_character = escape_escape_char_raw
                            .map(decode_dfdl_character_entities)
                            .filter(|s| !s.is_empty());
                        let escape_block_start = escape_block_start.map(decode_dfdl_character_entities);
                        let escape_block_end = self
                            .get_property("escapeBlockEnd")
                            .or_else(|| parent.and_then(|p| p.get_property("escapeBlockEnd")))
                            .map(decode_dfdl_character_entities);
                        let extra_escaped_characters = self
                            .get_property("extraEscapedCharacters")
                            .or_else(|| parent.and_then(|p| p.get_property("extraEscapedCharacters")))
                            .map(|s| {
                                s.split_whitespace()
                                    .flat_map(|item| decode_dfdl_character_entities(item).chars().collect::<Vec<_>>())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let gen_block_str = self
                            .get_property("generateEscapeBlock")
                            .or_else(|| parent.and_then(|p| p.get_property("generateEscapeBlock")))
                            .unwrap_or("whenNeeded");
                        let generate_escape_block = match gen_block_str {
                            "always" => crate::schema::ir::GenerateEscapeBlock::Always,
                            _ => crate::schema::ir::GenerateEscapeBlock::WhenNeeded,
                        };
                        Some(crate::schema::ir::CompiledEscapeScheme {
                            escape_kind,
                            escape_character,
                            escape_escape_character,
                            escape_block_start,
                            escape_block_end,
                            extra_escaped_characters,
                            generate_escape_block,
                        })
                    } else {
                        None
                    }
                }
            },
            rep_type: self
                .get_property("repType")
                .or_else(|| self.get_property("dfdlx:repType"))
                .or_else(|| parent.and_then(|p| p.get_property("repType").or_else(|| p.get_property("dfdlx:repType"))))
                .map(String::from),
            rep_simple_type: None,
            layer: self
                .get_property("layer")
                .or_else(|| self.get_property("dfdlx:layer"))
                .or_else(|| self.get_property("layerTransform"))
                .or_else(|| self.get_property("dfdlx:layerTransform"))
                .or_else(|| self.get_property("daf:layerTransform"))
                .map(String::from),
            in_scope_namespaces: effective_ns.to_vec(),
            string_as_xml: self
                .get_property("runtimeProperties")
                .or_else(|| self.get_property("dfdlx:runtimeProperties"))
                .or_else(|| {
                    parent.and_then(|p| {
                        p.get_property("runtimeProperties")
                            .or_else(|| p.get_property("dfdlx:runtimeProperties"))
                    })
                })
                .map(|p| p.contains("stringAsXml=true"))
                .unwrap_or(false),
            empty_value_delimiter_policy: self
                .get_property("emptyValueDelimiterPolicy")
                .or_else(|| parent.and_then(|p| p.get_property("emptyValueDelimiterPolicy")))
                .map(|s| match s.trim().to_ascii_lowercase().as_str() {
                    "none" => crate::schema::ir::EmptyValueDelimiterPolicy::None,
                    "initiator" => crate::schema::ir::EmptyValueDelimiterPolicy::Initiator,
                    "terminator" => crate::schema::ir::EmptyValueDelimiterPolicy::Terminator,
                    _ => crate::schema::ir::EmptyValueDelimiterPolicy::Both,
                })
                .unwrap_or(crate::schema::ir::EmptyValueDelimiterPolicy::Both),
        })
    }
}

pub(crate) fn is_valid_xs_date_time(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (date_part, time_part) = match s.split_once('T') {
        Some(parts) => parts,
        None => return false,
    };
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return false;
    }
    let (Some(year_str), Some(month_str), Some(day_str)) =
        (date_parts.first(), date_parts.get(1), date_parts.get(2))
    else {
        return false;
    };
    if year_str.len() < 4 || !year_str.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if month_str.len() != 2
        || !month_str.chars().all(|c| c.is_ascii_digit())
        || month_str.parse::<u32>().map_or(true, |m| m == 0 || m > 12)
    {
        return false;
    }
    if day_str.len() != 2
        || !day_str.chars().all(|c| c.is_ascii_digit())
        || day_str.parse::<u32>().map_or(true, |d| d == 0 || d > 31)
    {
        return false;
    }

    let (time_no_tz, _tz) = if let Some(stripped) = time_part.strip_suffix('Z') {
        (stripped, Some("Z"))
    } else if let Some(idx) = time_part.rfind('+').or_else(|| time_part.rfind('-')) {
        let (t, tz) = time_part.split_at(idx);
        let tz_digits = tz.get(1..).unwrap_or("");
        let b2 = tz_digits.as_bytes().get(2).copied();
        let sub_0_2 = tz_digits.get(..2).unwrap_or("");
        let sub_3 = tz_digits.get(3..).unwrap_or("");
        if tz_digits.len() != 5
            || b2 != Some(b':')
            || !sub_0_2.chars().all(|c| c.is_ascii_digit())
            || !sub_3.chars().all(|c| c.is_ascii_digit())
        {
            return false;
        }
        (t, Some(tz))
    } else {
        (time_part, None)
    };

    let (base_time, frac) = time_no_tz
        .split_once('.')
        .map_or((time_no_tz, None), |(b, f)| (b, Some(f)));
    if let Some(f) = frac {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }

    let time_parts: Vec<&str> = base_time.split(':').collect();
    if time_parts.len() != 3 {
        return false;
    }
    for p in &time_parts {
        if p.len() != 2 || !p.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let (Some(h_str), Some(m_str), Some(s_str)) =
        (time_parts.first(), time_parts.get(1), time_parts.get(2))
    else {
        return false;
    };
    let h = match h_str.parse::<u32>() {
        Ok(v) if v <= 24 => v,
        _ => return false,
    };
    let m = match m_str.parse::<u32>() {
        Ok(v) if v <= 59 => v,
        _ => return false,
    };
    let s = match s_str.parse::<u32>() {
        Ok(v) if v <= 60 => v,
        _ => return false,
    };
    if h == 24 && (m != 0 || s != 0) {
        return false;
    }

    true
}
