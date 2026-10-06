//! Delimiter tokenization and matching logic for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::{DfdlValue, InfosetBuilder};
use crate::io::traits::ByteSource;

use super::ParserEngine;

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum DelimToken {
    Literal(Vec<u8>),
    NL,
    CR,
    LF,
    NEL,
    LS,
    FF,
    VT,
    SP,
    HT,
    NUL,
    ES,
    WSP,
    WSPStar,
    WSPPlus,
    CharRef(u32),
}

pub(crate) fn split_delimiter_alternatives(s: &str) -> Vec<String> {
    let mut alts = Vec::new();
    let mut current = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &s[i..];
        if rest.starts_with('%') {
            if let Some(semi) = rest.find(';') {
                let end = i + semi + 1;
                current.push_str(&s[i..end]);
                i = end;
                continue;
            }
        }
        if rest.starts_with('\\') && i + 1 < bytes.len() {
            current.push_str(&s[i..i + 2]);
            i += 2;
            continue;
        }
        let ch = s[i..].chars().next().unwrap_or(' ');
        let ch_len = ch.len_utf8();
        if ch.is_whitespace() {
            if !current.is_empty() {
                alts.push(core::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
        i += ch_len;
    }
    if !current.is_empty() {
        alts.push(current);
    }
    alts
}

pub(crate) fn parse_single_delim_tokens(s: &str) -> DFDLResult<Vec<DelimToken>> {
    let mut tokens = Vec::new();
    let mut current_lit = Vec::new();

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &s[i..];
        if rest.starts_with("%NL;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::NL);
            i += 4;
        } else if rest.starts_with("%CR;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::CR);
            i += 4;
        } else if rest.starts_with("%LF;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::LF);
            i += 4;
        } else if rest.starts_with("%NEL;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::NEL);
            i += 5;
        } else if rest.starts_with("%LS;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::LS);
            i += 4;
        } else if rest.starts_with("%FF;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::FF);
            i += 4;
        } else if rest.starts_with("%VT;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::VT);
            i += 4;
        } else if rest.starts_with("%SP;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::SP);
            i += 4;
        } else if rest.starts_with("%HT;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::HT);
            i += 4;
        } else if rest.starts_with("%NUL;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::NUL);
            i += 5;
        } else if rest.starts_with("%ES;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::ES);
            i += 4;
        } else if rest.starts_with("%WSP*;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::WSPStar);
            i += 6;
        } else if rest.starts_with("%WSP+;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::WSPPlus);
            i += 6;
        } else if rest.starts_with("%WSP;") {
            if !current_lit.is_empty() {
                tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
            }
            tokens.push(DelimToken::WSP);
            i += 5;
        } else if rest.starts_with("%%") {
            current_lit.push(b'%');
            i += 2;
        } else if rest.starts_with("%,") {
            current_lit.push(b',');
            i += 2;
        } else if rest.starts_with("%#") {
            if let Some(semi_pos) = rest.find(';') {
                let entity = &rest[2..semi_pos];
                let val_opt = if let Some(hex_part) = entity
                    .strip_prefix('x')
                    .or_else(|| entity.strip_prefix('r'))
                {
                    u32::from_str_radix(hex_part, 16).ok()
                } else if let Some(dec_part) = entity.strip_prefix('d') {
                    dec_part.parse::<u32>().ok()
                } else if entity.chars().all(|c| c.is_ascii_digit()) {
                    entity.parse::<u32>().ok()
                } else {
                    None
                };
                if let Some(val) = val_opt {
                    if !current_lit.is_empty() {
                        tokens.push(DelimToken::Literal(core::mem::take(&mut current_lit)));
                    }
                    tokens.push(DelimToken::CharRef(val));
                    i += semi_pos + 1;
                    continue;
                }
            }
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Invalid DFDL entity reference or unescaped '%' character",
            ));
        } else if rest.starts_with('%') {
            // Check for standard DFDL character entity references (%<NAME>;) per DFDL §6.3.1.
            // If the entity name is recognized, decode it into its UTF-8 byte representation
            // and append it to the current delimiter literal bytes.
            if let Some(semi_pos) = rest.find(';') {
                let ent_span = &rest[..=semi_pos];
                let decoded = crate::expr::properties::decode_dfdl_character_entities(ent_span);
                if decoded != ent_span {
                    current_lit.extend_from_slice(decoded.as_bytes());
                    i += semi_pos + 1;
                    continue;
                }
            }
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Invalid DFDL entity reference or unescaped '%' character",
            ));
        } else {
            if let Some(&b) = bytes.get(i) {
                current_lit.push(b);
            }
            i += 1;
        }
    }
    if !current_lit.is_empty() {
        tokens.push(DelimToken::Literal(current_lit));
    }
    Ok(tokens)
}

/// Turns a leading `{{` into a literal `{` (DFDL §6.3.1); later braces are literal as written.
fn unescape_leading_brace(s: &str) -> String {
    match s.strip_prefix("{{") {
        Some(rest) => alloc::format!("{{{rest}"),
        None => String::from(s),
    }
}

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    pub(crate) fn evaluate_delimiter_str(
        &mut self,
        delim_str: &str,
        builder: &InfosetBuilder,
    ) -> DFDLResult<String> {
        self.evaluate_property_str_at(delim_str, builder, None)
    }

    /// Evaluates a property string that may be a `{ expr }`, with relative paths resolved from
    /// the element `elem_name` (not yet pushed on the builder) when given.
    pub(crate) fn evaluate_property_str_at(
        &mut self,
        delim_str: &str,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
    ) -> DFDLResult<String> {
        self.evaluate_property_str_at_with_namespaces(delim_str, builder, elem_name, None)
    }

    /// Evaluates a property string that may be a `{ expr }`, with relative paths resolved from
    /// the element `elem_name` (not yet pushed on the builder) and namespaces when given.
    pub(crate) fn evaluate_property_str_at_with_namespaces(
        &mut self,
        delim_str: &str,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
        namespaces: Option<&[(alloc::string::String, alloc::string::String)]>,
    ) -> DFDLResult<String> {
        let is_expr =
            delim_str.starts_with('{') && !delim_str.starts_with("{{") && delim_str.ends_with('}');
        if is_expr {
            let expr_body = delim_str
                .get(1..delim_str.len().saturating_sub(1))
                .unwrap_or("");
            let ast = crate::expr::parse_expr(expr_body)?;
            let mut current_path = builder.current_path();
            if let Some(name) = elem_name {
                let clean_name = name.split(':').next_back().unwrap_or(name);
                let last_seg = current_path.segments().last().map(|s| s.as_str());
                if last_seg != Some(clean_name) {
                    let _ = current_path.try_push(clean_name);
                }
            }
            let active_doc = builder.active_doc();
            let mut ctx = crate::expr::ExprContext::with_variable_map(
                Some(&active_doc),
                &current_path,
                &[],
                Some(&self.variable_map),
                self.budget,
            )
            .with_occurs_index(self.current_occurs_index)
            .with_schema(self.schema)
            .with_enclosing_lengths(&self.enclosing_complex_elements);
            if let Some(ns) = namespaces {
                ctx = ctx.with_namespaces(ns);
            }
            let val = crate::expr::eval_expr(&ast, &mut ctx)?;
            let res = match val {
                DfdlValue::String(s) => s,
                other => alloc::format!("{}", other),
            };
            Ok(res)
        } else {
            Ok(unescape_leading_brace(delim_str))
        }
    }


    pub(crate) fn match_single_delim_tokens_with_case(
        &mut self,
        tokens: &[DelimToken],
        ignore_case: bool,
    ) -> DFDLResult<()> {
        for token in tokens {
            match token {
                DelimToken::Literal(expected_bytes) => {
                    if let Some(cb) = crate::encoding::encoding_char_bits(&self.delim_encoding) {
                        // Sub-byte encoding: compare decoded characters, `cb` bits at a time.
                        for &b in expected_bytes {
                            let code = self.reader.read_bits(cb).map_err(|_| {
                                DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch: end of data reached",
                                )
                            })?;
                            let actual =
                                crate::encoding::decode_sub_byte_char(code, &self.delim_encoding);
                            let expected = b as char;
                            let same = if ignore_case {
                                actual.eq_ignore_ascii_case(&expected)
                            } else {
                                actual == expected
                            };
                            if !same {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch in bitstream",
                                ));
                            }
                        }
                        continue;
                    }
                    let encoded_bytes = if !self.delim_encoding.is_empty()
                        && !self.delim_encoding.eq_ignore_ascii_case("UTF-8")
                        && !self.delim_encoding.eq_ignore_ascii_case("US-ASCII")
                        && !self.delim_encoding.eq_ignore_ascii_case("ASCII")
                    {
                        if let Ok(s) = core::str::from_utf8(expected_bytes) {
                            crate::encoding::encode_text_string(s, &self.delim_encoding)
                        } else {
                            expected_bytes.clone()
                        }
                    } else {
                        expected_bytes.clone()
                    };
                    for &b in &encoded_bytes {
                        if self.reader.is_eof() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            ));
                        }
                        let actual = self.reader.read_bits(8).map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            )
                        })? as u8;
                        let matches = if ignore_case {
                            actual.eq_ignore_ascii_case(&b)
                        } else {
                            actual == b
                        };
                        if !matches {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch in bitstream",
                            ));
                        }
                    }
                }
                DelimToken::NL => {
                    let is_standard_single_byte = self.delim_encoding.is_empty()
                        || self.delim_encoding.eq_ignore_ascii_case("UTF-8")
                        || self.delim_encoding.eq_ignore_ascii_case("US-ASCII")
                        || self.delim_encoding.eq_ignore_ascii_case("ASCII")
                        || self.delim_encoding.eq_ignore_ascii_case("ISO-8859-1");

                    if is_standard_single_byte {
                        if self.reader.is_eof() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            ));
                        }
                        let cp = self.reader.checkpoint();
                        let b1 = self.reader.read_bits(8).map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            )
                        })? as u8;
                        if b1 == b'\r' {
                            if !self.reader.is_eof() {
                                let cp2 = self.reader.checkpoint();
                                if let Ok(b2) = self.reader.read_bits(8) {
                                    if b2 as u8 != b'\n' {
                                        let _ = self.reader.rollback(cp2);
                                    }
                                }
                            }
                        } else if b1 == b'\n' || b1 == 0x85 {
                            // matched LF or single-byte NEL (e.g. ISO-8859-1 / EBCDIC)
                        } else if b1 == 0xC2 {
                            // UTF-8 NEL: 0xC2 0x85
                            if !self.reader.is_eof() {
                                if let Ok(b2) = self.reader.read_bits(8) {
                                    if b2 as u8 != 0x85 {
                                        let _ = self.reader.rollback(cp);
                                        return Err(DFDLError::new_static(
                                            DFDLErrorKind::Parse,
                                            "Delimiter mismatch in bitstream",
                                        ));
                                    }
                                } else {
                                    let _ = self.reader.rollback(cp);
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch: end of data reached",
                                    ));
                                }
                            } else {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch: end of data reached",
                                ));
                            }
                        } else if b1 == 0xE2 {
                            // UTF-8 LS: 0xE2 0x80 0xA8
                            let b2_res = self.reader.read_bits(8);
                            let b3_res = self.reader.read_bits(8);
                            if let (Ok(b2), Ok(b3)) = (b2_res, b3_res) {
                                if b2 as u8 != 0x80 || b3 as u8 != 0xA8 {
                                    let _ = self.reader.rollback(cp);
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch in bitstream",
                                    ));
                                }
                            } else {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch: end of data reached",
                                ));
                            }
                        } else {
                            let _ = self.reader.rollback(cp);
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch in bitstream",
                            ));
                        }
                    } else {
                        // Multi-byte encoding (UTF-16BE/LE, UTF-32BE/LE, etc.): match candidates in order
                        let candidates = ["\r\n", "\n", "\r", "\u{0085}", "\u{2028}"];
                        let mut matched = false;
                        let cp = self.reader.checkpoint();
                        for cand in &candidates {
                            let encoded = crate::encoding::encode_text_string(cand, &self.delim_encoding);
                            let trial_cp = self.reader.checkpoint();
                            let mut cand_matches = true;
                            for &b in &encoded {
                                if self.reader.is_eof() {
                                    cand_matches = false;
                                    break;
                                }
                                if let Ok(actual) = self.reader.read_bits(8) {
                                    if actual as u8 != b {
                                        cand_matches = false;
                                        break;
                                    }
                                } else {
                                    cand_matches = false;
                                    break;
                                }
                            }
                            if cand_matches {
                                matched = true;
                                break;
                            } else {
                                let _ = self.reader.rollback(trial_cp);
                            }
                        }
                        if !matched {
                            let _ = self.reader.rollback(cp);
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch in bitstream",
                            ));
                        }
                    }
                }
                DelimToken::CR => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != b'\r' {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::LF => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != b'\n' {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::NEL => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let cp = self.reader.checkpoint();
                    let b1 = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b1 == 0x85 {
                        // ok
                    } else if b1 == 0xC2 {
                        if !self.reader.is_eof() {
                            if let Ok(b2) = self.reader.read_bits(8) {
                                if b2 as u8 != 0x85 {
                                    let _ = self.reader.rollback(cp);
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch in bitstream",
                                    ));
                                }
                            } else {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch in bitstream",
                                ));
                            }
                        } else {
                            let _ = self.reader.rollback(cp);
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            ));
                        }
                    } else {
                        let _ = self.reader.rollback(cp);
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::LS => {
                    let cp = self.reader.checkpoint();
                    let b1 = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    let b2 = self.reader.read_bits(8).map_err(|_| {
                        let _ = self.reader.rollback(cp);
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    let b3 = self.reader.read_bits(8).map_err(|_| {
                        let _ = self.reader.rollback(cp);
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b1 != 0xE2 || b2 != 0x80 || b3 != 0xA8 {
                        let _ = self.reader.rollback(cp);
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::FF => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != 0x0C {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::VT => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != 0x0B {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::SP => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != b' ' {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::HT => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != b'\t' {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::NUL => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b != 0 {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::ES => {}
                DelimToken::WSP => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let cp = self.reader.checkpoint();
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b == b' ' || b == b'\t' || b == b'\n' || b == 0x85 {
                        // matched single byte whitespace
                    } else if b == b'\r' {
                        if !self.reader.is_eof() {
                            let cp2 = self.reader.checkpoint();
                            if let Ok(b2) = self.reader.read_bits(8) {
                                if b2 as u8 != b'\n' {
                                    let _ = self.reader.rollback(cp2);
                                }
                            }
                        }
                    } else {
                        let _ = self.reader.rollback(cp);
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                }
                DelimToken::WSPStar => {
                    while !self.reader.is_eof() {
                        let cp = self.reader.checkpoint();
                        if let Ok(b) = self.reader.read_bits(8) {
                            let byte = b as u8;
                            if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == 0x85 {
                                // consumed
                            } else if byte == b'\r' {
                                if !self.reader.is_eof() {
                                    let cp2 = self.reader.checkpoint();
                                    if let Ok(b2) = self.reader.read_bits(8) {
                                        if b2 as u8 != b'\n' {
                                            let _ = self.reader.rollback(cp2);
                                        }
                                    }
                                }
                            } else {
                                let _ = self.reader.rollback(cp);
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                DelimToken::WSPPlus => {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        ));
                    }
                    let cp = self.reader.checkpoint();
                    let b = self.reader.read_bits(8).map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch: end of data reached",
                        )
                    })? as u8;
                    if b == b' ' || b == b'\t' || b == b'\n' || b == 0x85 {
                        // matched
                    } else if b == b'\r' {
                        if !self.reader.is_eof() {
                            let cp2 = self.reader.checkpoint();
                            if let Ok(b2) = self.reader.read_bits(8) {
                                if b2 as u8 != b'\n' {
                                    let _ = self.reader.rollback(cp2);
                                }
                            }
                        }
                    } else {
                        let _ = self.reader.rollback(cp);
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                    while !self.reader.is_eof() {
                        let cp = self.reader.checkpoint();
                        if let Ok(b) = self.reader.read_bits(8) {
                            let byte = b as u8;
                            if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == 0x85 {
                                // consumed
                            } else if byte == b'\r' {
                                if !self.reader.is_eof() {
                                    let cp2 = self.reader.checkpoint();
                                    if let Ok(b2) = self.reader.read_bits(8) {
                                        if b2 as u8 != b'\n' {
                                            let _ = self.reader.rollback(cp2);
                                        }
                                    }
                                }
                            } else {
                                let _ = self.reader.rollback(cp);
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
                DelimToken::CharRef(val) => {
                    if *val <= 0xFF {
                        if self.reader.is_eof() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            ));
                        }
                        let b = self.reader.read_bits(8).map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            )
                        })? as u8;
                        if b != *val as u8 {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch in bitstream",
                            ));
                        }
                    } else {
                        let mut buf = [0u8; 4];
                        if let Some(ch) = char::from_u32(*val) {
                            let str_bytes = ch.encode_utf8(&mut buf).as_bytes();
                            for &b in str_bytes {
                                if self.reader.is_eof() {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch: end of data reached",
                                    ));
                                }
                                let actual = self.reader.read_bits(8).map_err(|_| {
                                    DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch: end of data reached",
                                    )
                                })? as u8;
                                if actual != b {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::Parse,
                                        "Delimiter mismatch in bitstream",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Matches one of the whitespace-separated delimiter alternatives, preferring the longest
    /// match when several alternatives apply (DFDL §12.3.2).
    pub(crate) fn match_literal_delimiter(&mut self, delimiter: &str) -> DFDLResult<()> {
        if delimiter.is_empty() {
            return Ok(());
        }
        let alternatives = split_delimiter_alternatives(delimiter);
        if alternatives.is_empty() {
            return Ok(());
        }
        let start = self.reader.checkpoint();
        let mut best: Option<(usize, usize)> = None; // (consumed bits, alternative index)
        for (idx, alt) in alternatives.iter().enumerate() {
            let cp = self.reader.checkpoint();
            let tokens = parse_single_delim_tokens(alt)?;
            if self.match_single_delim_tokens_with_case(&tokens, self.delim_ignore_case).is_ok() {
                let consumed = self
                    .reader
                    .position()
                    .0
                    .saturating_sub(start.bit_position.0);
                if best.is_none_or(|(len, _)| consumed > len) {
                    best = Some((consumed, idx));
                }
            }
            let _ = self.reader.rollback(cp);
        }
        if let Some((_, idx)) = best {
            if let Some(alt) = alternatives.get(idx) {
                let tokens = parse_single_delim_tokens(alt)?;
                return self.match_single_delim_tokens_with_case(&tokens, self.delim_ignore_case);
            }
        }
        let err_msg = alloc::format!("Delimiter mismatch in bitstream: delimiter='{}'", delimiter);
        Err(DFDLError::new(DFDLErrorKind::Parse, &err_msg))

    }

    pub(crate) fn peek_literal_delimiter(&mut self, delimiter: &str) -> bool {
        if delimiter.is_empty() {
            return false;
        }
        let cp = self.reader.checkpoint();
        let res = self.match_literal_delimiter(delimiter);
        let _ = self.reader.rollback(cp);
        res.is_ok()
    }

    pub(crate) fn peek_delimiter_match_length(&mut self, delimiter: &str) -> Option<usize> {
        if delimiter.is_empty() {
            return None;
        }
        let alternatives = split_delimiter_alternatives(delimiter);
        if alternatives.is_empty() {
            return None;
        }
        let cp = self.reader.checkpoint();
        let mut best_len: Option<usize> = None;
        for alt in &alternatives {
            let trial_cp = self.reader.checkpoint();
            if let Ok(tokens) = parse_single_delim_tokens(alt) {
                if self.match_single_delim_tokens_with_case(&tokens, self.delim_ignore_case).is_ok() {
                    let consumed = self
                        .reader
                        .position()
                        .0
                        .saturating_sub(trial_cp.bit_position.0);
                    if best_len.is_none() || Some(consumed) > best_len {
                        best_len = Some(consumed);
                    }
                }
            }
            let _ = self.reader.rollback(trial_cp);
        }
        let _ = self.reader.rollback(cp);
        best_len
    }

    pub(crate) fn is_in_scope_terminator_longer(&mut self, sep_match_len: usize) -> bool {
        for i in (0..self.in_scope_delimiters.len()).rev() {
            if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                let delim_clone = delim.clone();
                let saved_case = self.delim_ignore_case;
                let saved_enc = core::mem::replace(&mut self.delim_encoding, delim_clone.encoding.clone());
                self.delim_ignore_case = delim_clone.ignore_case;
                let d_len = self.peek_delimiter_match_length(&delim_clone.text);
                self.delim_ignore_case = saved_case;
                self.delim_encoding = saved_enc;
                if let Some(d_len) = d_len {
                    if d_len > sep_match_len {
                        return true;
                    }
                }
            }
        }
        for i in (0..self.in_scope_terminators.len()).rev() {
            if let Ok(term) = crate::util::get_checked(&self.in_scope_terminators, i) {
                let term_clone = term.clone();
                let saved_case = self.delim_ignore_case;
                let saved_enc = core::mem::replace(&mut self.delim_encoding, term_clone.encoding.clone());
                self.delim_ignore_case = term_clone.ignore_case;
                let t_len = self.peek_delimiter_match_length(&term_clone.text);
                self.delim_ignore_case = saved_case;
                self.delim_encoding = saved_enc;
                if let Some(t_len) = t_len {
                    if t_len > sep_match_len {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub(crate) fn peek_any_in_scope_delimiter(&mut self) -> bool {
        for i in (0..self.in_scope_delimiters.len()).rev() {
            if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                let delim_clone = delim.clone();
                if self.peek_in_scope_delimiter(&delim_clone) {
                    return true;
                }
            }
        }
        for i in (0..self.in_scope_terminators.len()).rev() {
            if let Ok(term) = crate::util::get_checked(&self.in_scope_terminators, i) {
                let term_clone = term.clone();
                if self.peek_in_scope_delimiter(&term_clone) {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn peek_only_separators_to_end(&mut self, delimiter: &str) -> bool {
        if delimiter.is_empty() {
            return false;
        }
        let cp = self.reader.checkpoint();
        let mut count: usize = 0;
        while !self.reader.is_eof() {
            if self.match_literal_delimiter(delimiter).is_err() {
                let _ = self.reader.rollback(cp);
                return false;
            }
            count = count.saturating_add(1);
        }
        let _ = self.reader.rollback(cp);
        count > 0
    }
}

/// Matches an input string slice against a DFDL string literal pattern.
///
/// Per DFDL §6.3.1 and §14.2.1, the pattern can contain DFDL character entities
/// (%SP;, %HT;, %LF;, %CR;, %NL;, %NEL;, %LS;, etc.) and character class entities
/// (%WSP;, %WSP+;, %WSP*;).
pub(crate) fn match_dfdl_string_literal(pattern: &str, s: &str, ignore_case: bool) -> bool {
    let tokens = match parse_single_delim_tokens(pattern) {
        Ok(t) => t,
        Err(_) => {
            let unescaped = crate::expr::properties::decode_dfdl_character_entities(pattern);
            if ignore_case {
                return s.eq_ignore_ascii_case(pattern) || s.eq_ignore_ascii_case(&unescaped);
            } else {
                return s == pattern || s == unescaped;
            }
        }
    };
    match_delim_tokens_against_str(&tokens, s, ignore_case)
}

/// Matches a whitespace-separated list of DFDL string literals against an input string.
///
/// Per DFDL §14.2.1 and §13.7.1, properties such as `dfdl:textStandardZeroRep` and
/// `dfdl:nilValue` are lists of string literals that can each contain character class entities.
pub(crate) fn match_dfdl_string_literal_list(pattern_list: &str, s: &str, ignore_case: bool) -> bool {
    let alts = split_delimiter_alternatives(pattern_list);
    for alt in &alts {
        if match_dfdl_string_literal(alt, s, ignore_case) {
            return true;
        }
    }
    false
}

/// Matches a sequence of [`DelimToken`] against an in-memory string slice.
pub(crate) fn match_delim_tokens_against_str(tokens: &[DelimToken], s: &str, ignore_case: bool) -> bool {
    let Some(first) = tokens.first() else {
        return s.is_empty();
    };
    let rest_tokens = tokens.get(1..).unwrap_or(&[]);

    match first {
        DelimToken::Literal(bytes) => {
            let Ok(lit_str) = core::str::from_utf8(bytes) else {
                return false;
            };
            if lit_str.is_empty() {
                return match_delim_tokens_against_str(rest_tokens, s, ignore_case);
            }
            if s.len() < lit_str.len() {
                return false;
            }
            let (head, tail) = s.split_at(lit_str.len());
            let matches = if ignore_case {
                head.eq_ignore_ascii_case(lit_str)
            } else {
                head == lit_str
            };
            matches && match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
        }
        DelimToken::CharRef(cp) => {
            let Some(ch) = char::from_u32(*cp) else {
                return false;
            };
            let mut chars = s.chars();
            let Some(sc) = chars.next() else {
                return false;
            };
            let matches = if ignore_case {
                sc.eq_ignore_ascii_case(&ch)
            } else {
                sc == ch
            };
            matches && match_delim_tokens_against_str(rest_tokens, chars.as_str(), ignore_case)
        }
        DelimToken::ES => match_delim_tokens_against_str(rest_tokens, s, ignore_case),
        DelimToken::SP => {
            if let Some(tail) = s.strip_prefix(' ') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::HT => {
            if let Some(tail) = s.strip_prefix('\t') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::LF => {
            if let Some(tail) = s.strip_prefix('\n') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::CR => {
            if let Some(tail) = s.strip_prefix('\r') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::NEL => {
            if let Some(tail) = s.strip_prefix('\u{0085}') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::LS => {
            if let Some(tail) = s.strip_prefix('\u{2028}') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::FF => {
            if let Some(tail) = s.strip_prefix('\x0C') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::VT => {
            if let Some(tail) = s.strip_prefix('\x0B') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::NUL => {
            if let Some(tail) = s.strip_prefix('\0') {
                match_delim_tokens_against_str(rest_tokens, tail, ignore_case)
            } else {
                false
            }
        }
        DelimToken::NL => {
            if let Some(tail) = s.strip_prefix("\r\n") {
                if match_delim_tokens_against_str(rest_tokens, tail, ignore_case) {
                    return true;
                }
            }
            if let Some(tail) = s
                .strip_prefix('\n')
                .or_else(|| s.strip_prefix('\r'))
                .or_else(|| s.strip_prefix('\u{0085}'))
                .or_else(|| s.strip_prefix('\u{2028}'))
            {
                if match_delim_tokens_against_str(rest_tokens, tail, ignore_case) {
                    return true;
                }
            }
            false
        }
        DelimToken::WSP => {
            let mut chars = s.chars();
            let Some(c) = chars.next() else {
                return false;
            };
            if is_dfdl_whitespace(c) {
                match_delim_tokens_against_str(rest_tokens, chars.as_str(), ignore_case)
            } else {
                false
            }
        }
        DelimToken::WSPPlus => {
            let wsp_count = s.chars().take_while(|&c| is_dfdl_whitespace(c)).count();
            if wsp_count == 0 {
                return false;
            }
            for k in (1..=wsp_count).rev() {
                let split_idx: usize = s.chars().take(k).map(|c| c.len_utf8()).sum();
                let tail = s.get(split_idx..).unwrap_or("");
                if match_delim_tokens_against_str(rest_tokens, tail, ignore_case) {
                    return true;
                }
            }
            false
        }
        DelimToken::WSPStar => {
            let wsp_count = s.chars().take_while(|&c| is_dfdl_whitespace(c)).count();
            for k in (0..=wsp_count).rev() {
                let split_idx: usize = s.chars().take(k).map(|c| c.len_utf8()).sum();
                let tail = s.get(split_idx..).unwrap_or("");
                if match_delim_tokens_against_str(rest_tokens, tail, ignore_case) {
                    return true;
                }
            }
            false
        }
    }
}

/// Returns true if a Unicode character is recognized as DFDL whitespace per DFDL §6.3.1.2.
#[inline]
pub(crate) fn is_dfdl_whitespace(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\u{0085}' || c == '\u{00A0}'
}

#[cfg(test)]
mod brace_tests {
    use super::*;

    /// Only the first `{{` is an escape, so `{{ {{ [` keeps the second `{{` as a two-char delimiter.
    #[test]
    fn test_unescape_leading_brace_only() {
        assert_eq!(unescape_leading_brace("{{ {{ ["), "{ {{ [");
        assert_eq!(unescape_leading_brace("a{{b"), "a{{b");
        assert_eq!(unescape_leading_brace("{{"), "{");
        assert_eq!(unescape_leading_brace("plain"), "plain");
    }

    #[test]
    fn test_match_dfdl_string_literal_wsp_star() {
        assert!(match_dfdl_string_literal("Z%WSP*;Z%WSP*;Z", "Z Z Z", false));
        assert!(match_dfdl_string_literal("Z%WSP*;Z%WSP*;Z", "ZZZ", false));
        assert!(match_dfdl_string_literal("Z%WSP*;Z%WSP*;Z", "Z   Z\tZ", false));
        assert!(!match_dfdl_string_literal("Z%WSP*;Z%WSP*;Z", "Z 0 Z", false));
    }

    #[test]
    fn test_match_dfdl_string_literal_list() {
        let pattern = "zero Z%WSP*;Z%WSP*;Z";
        assert!(match_dfdl_string_literal_list(pattern, "zero", false));
        assert!(match_dfdl_string_literal_list(pattern, "Z Z Z", false));
        assert!(match_dfdl_string_literal_list(pattern, "ZZZ", false));
        assert!(!match_dfdl_string_literal_list(pattern, "one", false));
    }
}
