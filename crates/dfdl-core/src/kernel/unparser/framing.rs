//! Framing, delimiters, padding, and alignment logic for DFDL unparser.

extern crate alloc;
use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::io::bitstream::BitWriter;
use crate::io::traits::{BitOrder, ByteOrder, ByteSink};
use crate::schema::ir::ResolvedProperties;
use super::UnparserEngine;

/// `dfdl:fillByte` is needed (padding is actually emitted) but was never defined.
pub(crate) fn fill_byte_value(props: &crate::schema::ir::ResolvedProperties) -> DFDLResult<u64> {
    if props.fill_byte_defined {
        Ok(u64::from(props.fill_byte))
    } else {
        Err(DFDLError::new_static(
            DFDLErrorKind::SchemaDefinition,
            "Schema Definition Error: Property fillByte is not defined.",
        ))
    }
}

/// Writes `pad_bits` bits of padding using the in-scope `fillByte`.
///
/// Per DFDL 1.0 §13.3.1:
/// - When padding to an alignment boundary, full bytes of fillByte are written.
/// - If a fraction of a byte is required (sub-byte alignment), the most significant
///   bits of the fillByte are used when bitOrder is mostSignificantBitFirst, and the
///   least significant bits when bitOrder is leastSignificantBitFirst.
pub(crate) fn write_fill_padding<W: ByteSink>(
    writer: &mut BitWriter<W>,
    fill: u64,
    pad_bits: usize,
) -> DFDLResult<()> {
    if pad_bits == 0 {
        return Ok(());
    }
    let full_bytes = pad_bits.checked_div(8).unwrap_or(0);
    for _ in 0..full_bytes {
        writer.write_bits(fill & 0xFF, 8)?;
    }
    let rem_bits = pad_bits.checked_rem(8).unwrap_or(0);
    if rem_bits > 0 {
        let fill_byte = (fill & 0xFF) as u8;
        let bits_to_write = match writer.bit_order() {
            BitOrder::MostSignificantBitFirst => {
                let shift = 8usize.saturating_sub(rem_bits);
                u64::from(fill_byte >> shift)
            }
            BitOrder::LeastSignificantBitFirst => {
                let mask = (1u8.checked_shl(rem_bits as u32).unwrap_or(0)).saturating_sub(1);
                u64::from(fill_byte & mask)
            }
        };
        writer.write_bits(bits_to_write, rem_bits)?;
    }
    Ok(())
}

/// Decodes DFDL character entities in a delimiter string for unparsing.
pub fn decode_unparse_delimiter<'a>(val: &'a str, output_new_line: &str) -> Cow<'a, str> {
    if !val.contains('%') {
        return Cow::Borrowed(val);
    }
    let mut out = String::with_capacity(val.len());
    let bytes = val.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &val[i..];
        if rest.starts_with("%%") {
            out.push('%');
            i = i.saturating_add(2);
        } else if rest.starts_with("%,") {
            out.push(',');
            i = i.saturating_add(2);
        } else if rest.starts_with("%NL;") {
            out.push_str(output_new_line);
            i = i.saturating_add(4);
        } else if rest.starts_with("%CR;") {
            out.push('\r');
            i = i.saturating_add(4);
        } else if rest.starts_with("%LF;") {
            out.push('\n');
            i = i.saturating_add(4);
        } else if rest.starts_with("%NEL;") {
            out.push('\u{0085}');
            i = i.saturating_add(5);
        } else if rest.starts_with("%LS;") {
            out.push('\u{2028}');
            i = i.saturating_add(4);
        } else if rest.starts_with("%FF;") {
            out.push('\x0C');
            i = i.saturating_add(4);
        } else if rest.starts_with("%VT;") {
            out.push('\x0B');
            i = i.saturating_add(4);
        } else if rest.starts_with("%SP;") {
            out.push(' ');
            i = i.saturating_add(4);
        } else if rest.starts_with("%HT;") {
            out.push('\t');
            i = i.saturating_add(4);
        } else if rest.starts_with("%NUL;") {
            out.push('\0');
            i = i.saturating_add(5);
        } else if rest.starts_with("%ES;") {
            i = i.saturating_add(4);
        } else if rest.starts_with("%WSP*;") {
            i = i.saturating_add(6);
        } else if rest.starts_with("%WSP+;") {
            out.push(' ');
            i = i.saturating_add(6);
        } else if rest.starts_with("%WSP;") {
            out.push(' ');
            i = i.saturating_add(5);
        } else if rest.starts_with('%') {
            if let Some(semi_pos) = rest.find(';') {
                let entity_token = &rest[..=semi_pos];
                let decoded = crate::expr::properties::entities::decode_dfdl_character_entities(entity_token);
                if decoded != entity_token {
                    out.push_str(&decoded);
                    i = i.saturating_add(semi_pos.saturating_add(1));
                    continue;
                }
            }
            let ch = val[i..].chars().next().unwrap_or('%');
            out.push(ch);
            i = i.saturating_add(ch.len_utf8());
        } else {
            let ch = val[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i = i.saturating_add(ch.len_utf8());
        }
    }
    Cow::Owned(out)
}

impl<'a, S: ByteSink> UnparserEngine<'a, S> {
    pub(crate) fn resolve_delimiter_string<'d>(
        &mut self,
        raw_delim: &'d str,
        props: &'d ResolvedProperties,
    ) -> DFDLResult<Cow<'d, str>> {
        if raw_delim.is_empty() {
            return Ok(Cow::Borrowed(""));
        }
        let matching_prop = if props.initiator.as_deref() == Some(raw_delim) {
            props.initiator_prop.as_ref()
        } else if props.terminator.as_deref() == Some(raw_delim) {
            props.terminator_prop.as_ref()
        } else if props.separator.as_deref() == Some(raw_delim) {
            props.separator_prop.as_ref()
        } else if props.nil_value.as_deref() == Some(raw_delim) {
            props.nil_value_prop.as_ref()
        } else {
            None
        };

        let eval_str: Cow<'d, str> = match matching_prop {
            Some(crate::schema::ir::DfdlProp::Constant(c)) => Cow::Borrowed(c.as_str()),
            Some(crate::schema::ir::DfdlProp::Expression { ast, .. }) => {
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(ast, &mut ctx)?;
                Cow::Owned(alloc::format!("{}", val))
            }
            None => {
                let trimmed = raw_delim.trim();
                if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                    let ast = crate::expr::parse_expr(trimmed)?;
                    let mut ctx = self.make_expr_context();
                    let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                    Cow::Owned(alloc::format!("{}", val))
                } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                    Cow::Owned(alloc::format!("{{{stripped}"))
                } else {
                    Cow::Borrowed(raw_delim)
                }
            }
        };

        // If delimiter contains alternatives, choose the first alternative for unparsing (DFDL §12.3)
        let decoded = match eval_str {
            Cow::Borrowed(borrowed_str) => {
                let first_alt = crate::kernel::parser::delimiters::first_delimiter_alternative(borrowed_str);
                if first_alt.contains("%NL;") {
                    let nl_replacement = self.resolve_output_new_line(props);
                    Cow::Owned(decode_unparse_delimiter(first_alt, &nl_replacement).into_owned())
                } else if first_alt.contains('%') {
                    Cow::Owned(decode_unparse_delimiter(first_alt, "\n").into_owned())
                } else {
                    Cow::Borrowed(first_alt)
                }
            }
            Cow::Owned(ref owned_str) => {
                let first_alt = crate::kernel::parser::delimiters::first_delimiter_alternative(owned_str.as_str());
                if first_alt.contains("%NL;") {
                    let nl_replacement = self.resolve_output_new_line(props);
                    Cow::Owned(decode_unparse_delimiter(first_alt, &nl_replacement).into_owned())
                } else if first_alt.contains('%') {
                    Cow::Owned(decode_unparse_delimiter(first_alt, "\n").into_owned())
                } else {
                    Cow::Owned(alloc::string::ToString::to_string(first_alt))
                }
            }
        };

        Ok(decoded)
    }

    /// Resolves the target newline string from `dfdl:outputNewLine`, evaluating dynamic expressions if present.
    pub(crate) fn resolve_output_new_line(&mut self, props: &ResolvedProperties) -> String {
        let eval_str: Cow<'_, str> = match props.output_new_line_prop.as_ref() {
            Some(crate::schema::ir::DfdlProp::Constant(c)) => Cow::Borrowed(c.as_str()),
            Some(crate::schema::ir::DfdlProp::Expression { ast, raw }) => {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(ast, &mut ctx) {
                    Cow::Owned(alloc::format!("{}", val))
                } else {
                    Cow::Borrowed(raw.as_str())
                }
            }
            None => match props.output_new_line.as_deref() {
                Some(r) => {
                    let trimmed = r.trim();
                    if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                        if let Ok(ast) = crate::expr::parse_expr(trimmed) {
                            let mut ctx = self.make_expr_context();
                            if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                                Cow::Owned(alloc::format!("{}", val))
                            } else {
                                Cow::Borrowed(r)
                            }
                        } else {
                            Cow::Borrowed(r)
                        }
                    } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                        Cow::Owned(alloc::format!("{{{stripped}"))
                    } else {
                        Cow::Borrowed(r)
                    }
                }
                None => return String::from("\n"),
            },
        };

        match eval_str.as_ref() {
            "%CR;%LF;" | "\r\n" => String::from("\r\n"),
            "%LF;" | "\n" => String::from("\n"),
            "%CR;" | "\r" => String::from("\r"),
            "%NEL;" => String::from("\u{0085}"),
            "%LS;" => String::from("\u{2028}"),
            custom => crate::expr::properties::decode_dfdl_character_entities(custom),
        }
    }

    /// Evaluates a delimiter and writes its encoded bytes according to the target character set.
    pub(crate) fn write_evaluated_delimiter(
        &mut self,
        raw_delim: &str,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        if raw_delim.is_empty() {
            return Ok(());
        }
        let decoded = self.resolve_delimiter_string(raw_delim, props)?;
        if decoded.is_empty() {
            return Ok(());
        }

        if let Some(cb) = props.encoding_char_bits {
            for ch in decoded.chars() {
                let code = match crate::encoding::strict_sub_byte_code(ch, &props.encoding) {
                    Some(c) => c,
                    None if props.encoding_error_policy_error => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: UnmappableCharacterException: character '{}' (U+{:04X}) cannot be encoded in {}",
                                ch,
                                ch as u32,
                                props.encoding
                            ),
                        ));
                    }
                    None => crate::encoding::sub_byte_replacement_code(&props.encoding),
                };
                self.writer.write_bits(code, cb)?;
            }
            return Ok(());
        }

        // Fast-path: for standard UTF-8 and ASCII encodings, decoded.as_bytes() is already the encoded byte slice.
        if matches!(
            props.encoding,
            crate::encoding::EncodingKind::Utf8
                | crate::encoding::EncodingKind::Ascii
                | crate::encoding::EncodingKind::Iso8859_1
        ) {
            self.writer.write_bytes(decoded.as_bytes())?;
            return Ok(());
        }

        let encoded_bytes = crate::encoding::encode_text_string(&decoded, &props.encoding);
        self.writer.write_bytes(&encoded_bytes)?;
        Ok(())
    }

    pub(crate) fn extract_delimiter_tokens(
        &mut self,
        raw_delim: &str,
        props: &ResolvedProperties,
    ) -> Vec<String> {
        if raw_delim.is_empty() {
            return Vec::new();
        }
        let matching_prop = if props.initiator.as_deref() == Some(raw_delim) {
            props.initiator_prop.as_ref()
        } else if props.terminator.as_deref() == Some(raw_delim) {
            props.terminator_prop.as_ref()
        } else if props.separator.as_deref() == Some(raw_delim) {
            props.separator_prop.as_ref()
        } else if props.nil_value.as_deref() == Some(raw_delim) {
            props.nil_value_prop.as_ref()
        } else {
            None
        };

        let eval_str = match matching_prop {
            Some(crate::schema::ir::DfdlProp::Constant(c)) => c.clone(),
            Some(crate::schema::ir::DfdlProp::Expression { ast, raw }) => {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(ast, &mut ctx) {
                    alloc::format!("{}", val)
                } else {
                    raw.clone()
                }
            }
            None => {
                let trimmed = raw_delim.trim();
                if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                    if let Ok(ast) = crate::expr::parse_expr(trimmed) {
                        let mut ctx = self.make_expr_context();
                        if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                            alloc::format!("{}", val)
                        } else {
                            String::from(raw_delim)
                        }
                    } else {
                        String::from(raw_delim)
                    }
                } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                    alloc::format!("{{{stripped}")
                } else {
                    String::from(raw_delim)
                }
            }
        };

        let nl_replacement = if eval_str.contains("%NL;") {
            self.resolve_output_new_line(props)
        } else {
            String::from("\n")
        };

        let alts = crate::kernel::parser::delimiters::split_delimiter_alternatives(&eval_str);
        let mut tokens = Vec::with_capacity(alts.len());
        for alt in alts {
            let decoded = decode_unparse_delimiter(&alt, &nl_replacement);
            if !decoded.is_empty() {
                tokens.push(decoded.into_owned());
            }
        }
        tokens
    }

    pub(crate) fn apply_escape_scheme(
        &mut self,
        text: &str,
        scheme: &crate::schema::ir::CompiledEscapeScheme,
    ) -> String {
        let eval_ec = scheme
            .escape_character
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_eec = scheme
            .escape_escape_character
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_bs = scheme
            .escape_block_start
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_be = scheme
            .escape_block_end
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));

        let delims: Vec<&str> = self
            .active_delimiters
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect();

        scheme.escape_text(
            text,
            eval_ec.as_deref(),
            eval_eec.as_deref(),
            eval_bs.as_deref(),
            eval_be.as_deref(),
            &delims,
        )
    }

    /// Checks if a branch term or any of its nested members matches the element at cursor in parent.
    pub(crate) fn write_prefix_length(
        &mut self,
        val_len: usize,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let desc = props
            .prefix_length_type
            .as_ref()
            .cloned()
            .unwrap_or_else(|| crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc("xs:unsignedShort"));
        if desc.nested.is_some() {
            let msg = alloc::format!(
                "Unparse Error: Nested dfdl:lengthKind='prefixed' is not supported (prefixLengthType '{}')",
                desc.name
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }
        let is_text = desc.is_text();
        let prefix_bits = desc.prefix_bits();
        let prefix_units = match props.length_units {
            crate::schema::ir::LengthUnits::Bits => prefix_bits,
            crate::schema::ir::LengthUnits::Bytes
            | crate::schema::ir::LengthUnits::Characters => prefix_bits.div_ceil(8),
        };
        let raw_val = if props.prefix_includes_prefix_length {
            (val_len as u64).saturating_add(prefix_units as u64)
        } else {
            val_len as u64
        };
        if let Some(max) = desc.max_inclusive {
            if (raw_val as i64) > max {
                let name = self
                    .current_path
                    .last_step()
                    .map(|s| s.raw_target())
                    .unwrap_or("element");
                let msg = alloc::format!("Unparse Error: failed check {name} ({raw_val}) facet maxInclusive ({max})");
                return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
            }
        }
        if let Some(min) = desc.min_inclusive {
            if (raw_val as i64) < min {
                let name = self
                    .current_path
                    .last_step()
                    .map(|s| s.raw_target())
                    .unwrap_or("element");
                let msg = alloc::format!("Unparse Error: failed check {name} ({raw_val}) facet minInclusive ({min})");
                return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
            }
        }
        if is_text {
            let s = alloc::format!("{}", raw_val);
            let target_bytes = prefix_bits.div_ceil(8);
            let pad_c = desc.pad_char.unwrap_or(' ');
            let out = if s.len() < target_bytes {
                let pad_len = target_bytes.saturating_sub(s.len());
                let mut padded = String::with_capacity(target_bytes);
                for _ in 0..pad_len {
                    padded.push(pad_c);
                }
                padded.push_str(&s);
                padded
            } else {
                s
            };
            self.writer.write_bytes(out.as_bytes())?;
        } else {
            let prefix_val_u64 = match props.byte_order {
                ByteOrder::BigEndian => raw_val,
                ByteOrder::LittleEndian => match prefix_bits {
                    16 => (raw_val as u16).swap_bytes() as u64,
                    24 => {
                        let b0 = raw_val & 0xFF;
                        let b1 = (raw_val >> 8) & 0xFF;
                        let b2 = (raw_val >> 16) & 0xFF;
                        (b0 << 16) | (b1 << 8) | b2
                    }
                    32 => (raw_val as u32).swap_bytes() as u64,
                    64 => raw_val.swap_bytes(),
                    _ => raw_val,
                },
            };
            self.writer.write_bits(prefix_val_u64, prefix_bits)?;
        }
        Ok(())
    }
}
