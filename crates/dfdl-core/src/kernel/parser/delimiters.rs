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
        } else if let Some(&b) = bytes.get(i) {
            current_lit.push(b);
            i += 1;
        } else {
            break;
        }
    }
    if !current_lit.is_empty() {
        tokens.push(DelimToken::Literal(current_lit));
    }
    Ok(tokens)
}

/// Turns a leading `{{` into a literal `{` (DFDL §6.3.1); later braces are literal as written.
#[allow(dead_code)]
pub(crate) fn unescape_leading_brace(s: &str) -> String {
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

    pub(crate) fn evaluate_delimiter_prop(
        &mut self,
        prop: &crate::schema::ir::DfdlProp<String>,
        builder: &InfosetBuilder,
    ) -> DFDLResult<String> {
        self.evaluate_prop_at(prop, builder, None, None)
    }

    /// Evaluates a pre-compiled [`DfdlProp<String>`] without runtime regex or string scanning.
    pub(crate) fn evaluate_prop_at(
        &mut self,
        prop: &crate::schema::ir::DfdlProp<String>,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
        namespaces: Option<&[(alloc::string::String, alloc::string::String)]>,
    ) -> DFDLResult<String> {
        match prop {
            crate::schema::ir::DfdlProp::Constant(s) => Ok(s.clone()),
            crate::schema::ir::DfdlProp::Expression { ast, .. } => {
                self.evaluate_ast_at(ast, builder, elem_name, namespaces)
            }
        }
    }

    /// Directly evaluates a pre-compiled [`ExprAst`] in the current parser execution context.
    pub(crate) fn evaluate_ast_at(
        &mut self,
        ast: &crate::expr::ast::ExprAst,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
        namespaces: Option<&[(alloc::string::String, alloc::string::String)]>,
    ) -> DFDLResult<String> {
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
        let val = crate::expr::eval_expr(ast, &mut ctx)?;
        let res = match val {
            DfdlValue::String(s) => s,
            other => alloc::format!("{}", other),
        };
        Ok(res)
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
        let prop = crate::schema::ir::DfdlProp::parse_str(delim_str)?;
        self.evaluate_prop_at(&prop, builder, elem_name, namespaces)
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
                        core::str::from_utf8(expected_bytes)
                            .map(|s| crate::encoding::encode_text_string(s, &self.delim_encoding))
                            .unwrap_or_else(|_| expected_bytes.clone())
                    } else {
                        expected_bytes.clone()
                    };
                    for &b in &encoded_bytes {
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
                        let cp = self.reader.checkpoint();
                        let b1 = self.reader.read_bits(8).map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            )
                        })? as u8;
                        if b1 == b'\r' {
                            let cp2 = self.reader.checkpoint();
                            if !matches!(self.reader.read_bits(8), Ok(b2) if b2 as u8 == b'\n') {
                                let _ = self.reader.rollback(cp2);
                            }
                        } else if b1 == b'\n' || b1 == 0x85 {
                            // matched LF or single-byte NEL (e.g. ISO-8859-1 / EBCDIC)
                        } else if b1 == 0xC2 {
                            // UTF-8 NEL: 0xC2 0x85
                            let Ok(b2) = self.reader.read_bits(8) else {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch: end of data reached",
                                ));
                            };
                            if b2 as u8 != 0x85 {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch in bitstream",
                                ));
                            }
                        } else if b1 == 0xE2 {
                            // UTF-8 LS: 0xE2 0x80 0xA8
                            let (Ok(b2), Ok(b3)) = (self.reader.read_bits(8), self.reader.read_bits(8)) else {
                                let _ = self.reader.rollback(cp);
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Delimiter mismatch: end of data reached",
                                ));
                            };
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
                                match self.reader.read_bits(8) {
                                    Ok(actual) if actual as u8 == b => {}
                                    _ => {
                                        cand_matches = false;
                                        break;
                                    }
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
                        let Ok(b2) = self.reader.read_bits(8) else {
                            let _ = self.reader.rollback(cp);
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Delimiter mismatch: end of data reached",
                            ));
                        };
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
                        let cp2 = self.reader.checkpoint();
                        if !matches!(self.reader.read_bits(8), Ok(b2) if b2 as u8 == b'\n') {
                            let _ = self.reader.rollback(cp2);
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
                    loop {
                        let cp = self.reader.checkpoint();
                        let Ok(b) = self.reader.read_bits(8) else {
                            break;
                        };
                        let byte = b as u8;
                        if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == 0x85 {
                            // consumed
                        } else if byte == b'\r' {
                            let cp2 = self.reader.checkpoint();
                            if !matches!(self.reader.read_bits(8), Ok(b2) if b2 as u8 == b'\n') {
                                let _ = self.reader.rollback(cp2);
                            }
                        } else {
                            let _ = self.reader.rollback(cp);
                            break;
                        }
                    }
                }
                DelimToken::WSPPlus => {
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
                        let cp2 = self.reader.checkpoint();
                        if !matches!(self.reader.read_bits(8), Ok(b2) if b2 as u8 == b'\n') {
                            let _ = self.reader.rollback(cp2);
                        }
                    } else {
                        let _ = self.reader.rollback(cp);
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Delimiter mismatch in bitstream",
                        ));
                    }
                    loop {
                        let cp = self.reader.checkpoint();
                        let Ok(b) = self.reader.read_bits(8) else {
                            break;
                        };
                        let byte = b as u8;
                        if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == 0x85 {
                            // consumed
                        } else if byte == b'\r' {
                            let cp2 = self.reader.checkpoint();
                            if !matches!(self.reader.read_bits(8), Ok(b2) if b2 as u8 == b'\n') {
                                let _ = self.reader.rollback(cp2);
                            }
                        } else {
                            let _ = self.reader.rollback(cp);
                            break;
                        }
                    }
                }
                DelimToken::CharRef(val) => {
                    if *val <= 0xFF {
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
        let mut best: Option<(usize, &String)> = None; // (consumed bits, alternative)
        for alt in &alternatives {
            let cp = self.reader.checkpoint();
            let tokens = parse_single_delim_tokens(alt)?;
            if self.match_single_delim_tokens_with_case(&tokens, self.delim_ignore_case).is_ok() {
                let consumed = self
                    .reader
                    .position()
                    .0
                    .saturating_sub(start.bit_position.0);
                if best.is_none_or(|(len, _)| consumed > len) {
                    best = Some((consumed, alt));
                }
            }
            let _ = self.reader.rollback(cp);
        }
        if let Some((_, alt)) = best {
            let tokens = parse_single_delim_tokens(alt)?;
            return self.match_single_delim_tokens_with_case(&tokens, self.delim_ignore_case);
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
        let delims = self.in_scope_delimiters.clone();
        for delim in delims.iter().rev() {
            let saved_case = self.delim_ignore_case;
            let saved_enc = core::mem::replace(&mut self.delim_encoding, delim.encoding.clone());
            self.delim_ignore_case = delim.ignore_case;
            let d_len = self.peek_delimiter_match_length(&delim.text);
            self.delim_ignore_case = saved_case;
            self.delim_encoding = saved_enc;
            if let Some(d_len) = d_len {
                if d_len > sep_match_len {
                    return true;
                }
            }
        }
        let terms = self.in_scope_terminators.clone();
        for term in terms.iter().rev() {
            let saved_case = self.delim_ignore_case;
            let saved_enc = core::mem::replace(&mut self.delim_encoding, term.encoding.clone());
            self.delim_ignore_case = term.ignore_case;
            let t_len = self.peek_delimiter_match_length(&term.text);
            self.delim_ignore_case = saved_case;
            self.delim_encoding = saved_enc;
            if let Some(t_len) = t_len {
                if t_len > sep_match_len {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn peek_any_in_scope_delimiter(&mut self) -> bool {
        let delims = self.in_scope_delimiters.clone();
        for delim in delims.iter().rev() {
            if self.peek_in_scope_delimiter(delim) {
                return true;
            }
        }
        let terms = self.in_scope_terminators.clone();
        for term in terms.iter().rev() {
            if self.peek_in_scope_delimiter(term) {
                return true;
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
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod brace_tests {
    use super::*;
    use crate::io::bitstream::BitReader;
    use crate::io::source::SliceByteSource;
    use crate::io::traits::{BitOrder, ByteOrder};
    use crate::limits::WorkBudget;
    use crate::schema::builder::SchemaBuilder;
    use crate::schema::ir::{ResolvedProperties, TermKind};

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

    /// Verifies that delimiter alternative splitting correctly separates tokens on whitespace,
    /// preserves DFDL entities, respects escaped spaces and backslashes, and ignores empty entries.
    #[test]
    fn test_split_delimiter_alternatives_comprehensive() {
        // Standard multi-character delimiter alternatives separated by spaces
        let alts1 = split_delimiter_alternatives(":: || : $");
        assert_eq!(alts1, alloc::vec!["--::", "--||", "--:", "--$"]
            .iter().map(|s| &s[2..]).collect::<alloc::vec::Vec<&str>>());

        // Multi-whitespace padding between tokens and leading/trailing whitespace
        let alts2 = split_delimiter_alternatives("   first    second   third   ");
        assert_eq!(alts2, alloc::vec!["first", "second", "third"]);

        // Preserves DFDL character entities with internal semicolons
        let alts3 = split_delimiter_alternatives("foo %SP; bar %#x2C; baz");
        assert_eq!(alts3, alloc::vec!["foo", "%SP;", "bar", "%#x2C;", "baz"]);

        // Escaped whitespace and escaped backslashes
        let alts4 = split_delimiter_alternatives(r"escaped\ space normal\\item");
        assert_eq!(alts4, alloc::vec![r"escaped\ space", r"normal\\item"]);

        // Empty input returns an empty alternatives vector
        let alts5 = split_delimiter_alternatives("");
        assert!(alts5.is_empty());
    }

    /// Verifies tokenization of all standard DFDL delimiter entities, escaped percent/comma,
    /// character references (hex/decimal), and surrounding literal character spans.
    #[test]
    fn test_parse_single_delim_tokens_all_entities() {
        // All primary character class entities and controls
        let line = "%NL;%CR;%LF;%NEL;%LS;%FF;%VT;%SP;%HT;%NUL;%ES;%WSP;%WSP+;%WSP*;";
        let tokens = parse_single_delim_tokens(line).unwrap();
        assert_eq!(
            tokens,
            alloc::vec![
                DelimToken::NL,
                DelimToken::CR,
                DelimToken::LF,
                DelimToken::NEL,
                DelimToken::LS,
                DelimToken::FF,
                DelimToken::VT,
                DelimToken::SP,
                DelimToken::HT,
                DelimToken::NUL,
                DelimToken::ES,
                DelimToken::WSP,
                DelimToken::WSPPlus,
                DelimToken::WSPStar,
            ]
        );

        // Escaped percent and escaped comma
        let esc_tokens = parse_single_delim_tokens("%%%,text").unwrap();
        assert_eq!(
            esc_tokens,
            alloc::vec![DelimToken::Literal(b"%,text".to_vec())]
        );

        // Character references: hex (%#x...), raw hex (%#r...), decimal (%#d...), pure digits (%#...)
        let cr_tokens = parse_single_delim_tokens("%#x20;%#r0D;%#d65;%#66;").unwrap();
        assert_eq!(
            cr_tokens,
            alloc::vec![
                DelimToken::CharRef(0x20),
                DelimToken::CharRef(0x0D),
                DelimToken::CharRef(65),
                DelimToken::CharRef(66),
            ]
        );

        // Mixed literals and entities
        let mixed = parse_single_delim_tokens("PREFIX_%SP;_SUFFIX").unwrap();
        assert_eq!(
            mixed,
            alloc::vec![
                DelimToken::Literal(b"PREFIX_".to_vec()),
                DelimToken::SP,
                DelimToken::Literal(b"_SUFFIX".to_vec()),
            ]
        );
    }

    /// Verifies that invalid DFDL entities or malformed character references produce SchemaDefinition errors.
    #[test]
    fn test_parse_single_delim_tokens_error_paths() {
        // Unterminated entity (no semicolon)
        assert!(parse_single_delim_tokens("%NL").is_err());

        // Unknown entity name
        assert!(parse_single_delim_tokens("%UNKNOWN_ENTITY;").is_err());

        // Malformed hex in character reference
        assert!(parse_single_delim_tokens("%#xZZ;").is_err());

        // Malformed decimal in character reference
        assert!(parse_single_delim_tokens("%#d-1;").is_err());

        // Naked unescaped percent sign
        assert!(parse_single_delim_tokens("abc%def").is_err());
    }

    /// Verifies string matching across all DelimToken variants, case sensitivity, and multi-char sequences.
    #[test]
    fn test_match_delim_tokens_against_str_variants() {
        // Literal token matching with and without case sensitivity
        let lit_token = [DelimToken::Literal(b"Hello".to_vec())];
        assert!(match_delim_tokens_against_str(&lit_token, "Hello", false));
        assert!(!match_delim_tokens_against_str(&lit_token, "hello", false));
        assert!(match_delim_tokens_against_str(&lit_token, "hello", true));
        assert!(!match_delim_tokens_against_str(&lit_token, "Hell", false));

        // NL matches \r\n, \n, \r, and Unicode NEL \u{0085}
        let nl_token = [DelimToken::NL];
        assert!(match_delim_tokens_against_str(&nl_token, "\r\n", false));
        assert!(match_delim_tokens_against_str(&nl_token, "\n", false));
        assert!(match_delim_tokens_against_str(&nl_token, "\r", false));
        assert!(match_delim_tokens_against_str(&nl_token, "\u{0085}", false));
        assert!(!match_delim_tokens_against_str(&nl_token, "x", false));

        // Individual control entities
        assert!(match_delim_tokens_against_str(&[DelimToken::CR], "\r", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::CR], "\n", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::LF], "\n", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::LF], "\r", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NEL], "\u{0085}", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::LS], "\u{2028}", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::FF], "\u{000C}", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::VT], "\u{000B}", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::SP], " ", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::HT], "\t", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NUL], "\0", false));

        // Empty String entity (%ES;) followed by a literal
        let es_tokens = [DelimToken::ES, DelimToken::Literal(b"abc".to_vec())];
        assert!(match_delim_tokens_against_str(&es_tokens, "abc", false));

        // Character reference matching
        let cr_tok = [DelimToken::CharRef(0x41)]; // 'A'
        assert!(match_delim_tokens_against_str(&cr_tok, "A", false));
        assert!(match_delim_tokens_against_str(&cr_tok, "a", true));
        assert!(!match_delim_tokens_against_str(&cr_tok, "B", false));

        // WSP matches a single DFDL whitespace character
        let wsp_tok = [DelimToken::WSP];
        assert!(match_delim_tokens_against_str(&wsp_tok, " ", false));
        assert!(match_delim_tokens_against_str(&wsp_tok, "\t", false));
        assert!(match_delim_tokens_against_str(&wsp_tok, "\n", false));
        assert!(match_delim_tokens_against_str(&wsp_tok, "\r", false));
        assert!(match_delim_tokens_against_str(&wsp_tok, "\u{0085}", false));
        assert!(match_delim_tokens_against_str(&wsp_tok, "\u{00A0}", false));
        assert!(!match_delim_tokens_against_str(&wsp_tok, "abc", false));

        // WSPPlus matches 1 or more whitespace characters
        let wsp_plus_tok = [DelimToken::WSPPlus, DelimToken::Literal(b"end".to_vec())];
        assert!(match_delim_tokens_against_str(&wsp_plus_tok, "   end", false));
        assert!(match_delim_tokens_against_str(&wsp_plus_tok, " \t\n end", false));
        assert!(!match_delim_tokens_against_str(&wsp_plus_tok, "end", false));

        // WSPStar matches 0 or more whitespace characters
        let wsp_star_tok = [DelimToken::WSPStar, DelimToken::Literal(b"end".to_vec())];
        assert!(match_delim_tokens_against_str(&wsp_star_tok, "end", false));
        assert!(match_delim_tokens_against_str(&wsp_star_tok, "   end", false));
    }

    /// Verifies character classification according to DFDL §6.3.1.2 whitespace rules.
    #[test]
    fn test_is_dfdl_whitespace_all() {
        // Valid whitespace per DFDL §6.3.1.2
        assert!(is_dfdl_whitespace(' '));
        assert!(is_dfdl_whitespace('\t'));
        assert!(is_dfdl_whitespace('\n'));
        assert!(is_dfdl_whitespace('\r'));
        assert!(is_dfdl_whitespace('\u{0085}')); // NEL
        assert!(is_dfdl_whitespace('\u{00A0}')); // Non-breaking space

        // Non-whitespace characters must return false
        assert!(!is_dfdl_whitespace('a'));
        assert!(!is_dfdl_whitespace('0'));
        assert!(!is_dfdl_whitespace('\0'));
        assert!(!is_dfdl_whitespace('\u{2000}'));
    }

    /// Verifies property evaluation logic for literal strings, escaped braces, and dynamic XPath expressions.
    #[test]
    fn test_evaluate_property_str_at_expressions_and_literals() {
        let mut builder = SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let source = SliceByteSource::new(b"test");
        let mut reader = BitReader::new(source, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let info_builder = InfosetBuilder::new();

        // Plain string literal
        let res1 = engine.evaluate_delimiter_str(";", &info_builder).unwrap();
        assert_eq!(res1, ";");

        // Escaped leading double brace
        let res2 = engine.evaluate_delimiter_str("{{bracket", &info_builder).unwrap();
        assert_eq!(res2, "{bracket");

        // Dynamic expression returning string
        let res3 = engine.evaluate_delimiter_str("{ 'delim_val' }", &info_builder).unwrap();
        assert_eq!(res3, "delim_val");

        // Dynamic expression returning integer
        let res4 = engine.evaluate_delimiter_str("{ 40 + 2 }", &info_builder).unwrap();
        assert_eq!(res4, "42");

        // Helper to test delimiter matching on parser engine
        let test_delim = |bytes: &[u8], tokens: &[DelimToken]| -> DFDLResult<()> {
            let src = SliceByteSource::new(bytes);
            let mut r = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut r, &mut b);
            eng.match_single_delim_tokens_with_case(tokens, false)
        };

        // UTF-8 NEL (0xC2 0x85) valid and error branches
        assert!(test_delim(b"\xC2\x85", &[DelimToken::NL]).is_ok());
        assert!(test_delim(b"\xC2\x00", &[DelimToken::NL]).is_err());
        assert!(test_delim(b"\xC2", &[DelimToken::NL]).is_err());

        // UTF-8 LS (0xE2 0x80 0xA8) valid and error branches
        assert!(test_delim(b"\xE2\x80\xA8", &[DelimToken::NL]).is_ok());
        assert!(test_delim(b"\xE2\x80\x00", &[DelimToken::NL]).is_err());
        assert!(test_delim(b"\xE2\x80", &[DelimToken::NL]).is_err());

        // WSPPlus on bare CR followed by non-LF (e.g. \rX)
        assert!(test_delim(b"\rX", &[DelimToken::WSPPlus]).is_ok());

        // WSPStar on bare CR followed by non-LF (e.g. \rX)
        assert!(test_delim(b"\rX", &[DelimToken::WSPStar]).is_ok());

        // Parse delimiter expression with literals before special entities (lines 95, 101, 107, 113, 119, 131, 137, 143, 155, 161)
        let complex_delims = parse_single_delim_tokens(
            "X%LF;X%NEL;X%LS;X%FF;X%VT;X%SP;X%HT;X%NUL;X%ES;X%WSP*;X%WSP+;X%WSP;"
        ).unwrap();
        assert!(complex_delims.len() >= 12);

        // DelimToken CR, LF, NEL mismatch and EOF branches (lines 504-550)
        assert!(test_delim(b"X", &[DelimToken::CR]).is_err());
        assert!(test_delim(b"", &[DelimToken::CR]).is_err());
        assert!(test_delim(b"X", &[DelimToken::LF]).is_err());
        assert!(test_delim(b"", &[DelimToken::LF]).is_err());
        assert!(test_delim(b"X", &[DelimToken::NEL]).is_err());
        assert!(test_delim(b"", &[DelimToken::NEL]).is_err());

        // Sub-byte delimiter matching (lines 318-337)
        let test_sub_delim = |bytes: &[u8], tokens: &[DelimToken], ignore_case: bool| -> DFDLResult<()> {
            let src = SliceByteSource::new(bytes);
            let mut r = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut r, &mut b);
            eng.delim_encoding = "X-DFDL-BITS".into();
            eng.match_single_delim_tokens_with_case(tokens, ignore_case)
        };
        assert!(test_sub_delim(&[0b00000000], &[DelimToken::Literal(alloc::vec![b'0'])], true).is_ok());
        assert!(test_sub_delim(&[0b10000000], &[DelimToken::Literal(alloc::vec![b'0'])], false).is_err());
        assert!(test_sub_delim(&[], &[DelimToken::Literal(alloc::vec![b'0'])], false).is_err());

        // CharRef with preceding literal and invalid numeric entity (lines 184, 188)
        let parsed_char_ref = parse_single_delim_tokens("prefix%#65;").unwrap();
        assert_eq!(parsed_char_ref.len(), 2);
        assert!(matches!(parsed_char_ref[0], DelimToken::Literal(_)));
        assert!(matches!(parsed_char_ref[1], DelimToken::CharRef(65)));

        assert!(parse_single_delim_tokens("%#notnum;").is_err());

        // DelimToken::NL on CR followed by non-LF (e.g. \rX) rolling back X (lines 401-409)
        assert!(test_delim(b"\rX", &[DelimToken::NL]).is_ok());

        // DelimToken::NEL on UTF-8 0xC2 followed by non-0x85 or EOF (lines 563-578)
        assert!(test_delim(b"\xC2X", &[DelimToken::NEL]).is_err());
        assert!(test_delim(b"\xC2", &[DelimToken::NEL]).is_err());

        // DelimToken::FF, VT, SP, HT matches and mismatches
        assert!(test_delim(&[0x0C], &[DelimToken::FF]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::FF]).is_err());
        assert!(test_delim(&[0x0B], &[DelimToken::VT]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::VT]).is_err());
        assert!(test_delim(b" ", &[DelimToken::SP]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::SP]).is_err());
        assert!(test_delim(b"\t", &[DelimToken::HT]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::HT]).is_err());

        // Literal delimiter with ISO-8859-1 encoding (lines 346-350)
        let test_iso_delim = |bytes: &[u8], tokens: &[DelimToken]| -> DFDLResult<()> {
            let src = SliceByteSource::new(bytes);
            let mut r = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut r, &mut b);
            eng.delim_encoding = "ISO-8859-1".into();
            eng.match_single_delim_tokens_with_case(tokens, false)
        };
        assert!(test_iso_delim(&[0xE9], &[DelimToken::Literal(alloc::vec![0xC3, 0xA9])]).is_ok());

        // DelimToken::NUL matches 0x00 and rejects non-zero (lines 648-659)
        assert!(test_delim(&[0x00], &[DelimToken::NUL]).is_ok());
        assert!(test_delim(&[0x01], &[DelimToken::NUL]).is_err());
        assert!(test_delim(&[], &[DelimToken::NUL]).is_err());

        // DelimToken::WSP matches space, tab, LF, single-byte NEL, and CRLF (lines 662-687)
        assert!(test_delim(b" ", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"\t", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"\n", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(&[0x85], &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"\r\n", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"\rX", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::WSP]).is_err());

        // DelimToken::WSPStar and WSPPlus (lines 689-760)
        assert!(test_delim(b" \t\n\r\nX", &[DelimToken::WSPStar]).is_ok());
        assert!(test_delim(b"\rX", &[DelimToken::WSPStar]).is_ok());
        assert!(test_delim(b" \rX", &[DelimToken::WSPPlus]).is_ok());
        assert!(test_delim(b"\r\nX", &[DelimToken::WSPPlus]).is_ok());
        assert!(test_delim(b"X", &[DelimToken::WSPPlus]).is_err());

        // DelimToken::CharRef multi-byte UTF-8 character (e.g. Euro sign 0x20AC) (lines 780-796)
        assert!(test_delim(&[0xE2, 0x82, 0xAC], &[DelimToken::CharRef(0x20AC)]).is_ok());
        assert!(test_delim(&[0xE2, 0x82], &[DelimToken::CharRef(0x20AC)]).is_err());
        assert!(test_delim(&[0xE2, 0x82, 0x00], &[DelimToken::CharRef(0x20AC)]).is_err());

        // NL with UTF-8 LS (0xE2 0x80 0xA8) and NEL (0xC2 0x85) (lines 400-444)
        assert!(test_delim(&[0xE2, 0x80, 0xA8], &[DelimToken::NL]).is_ok());
        assert!(test_delim(&[0xE2, 0x80, 0x00], &[DelimToken::NL]).is_err());
        assert!(test_delim(&[0xE2, 0x80], &[DelimToken::NL]).is_err());
        assert!(test_delim(&[0xC2, 0x85], &[DelimToken::NL]).is_ok());
        assert!(test_delim(&[0xC2, 0x00], &[DelimToken::NL]).is_err());
        assert!(test_delim(&[0xC2], &[DelimToken::NL]).is_err());

        // match_literal_delimiter empty and whitespace (lines 807-813)
        let src_empty = SliceByteSource::new(b"");
        let mut r_empty = BitReader::new(src_empty, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_empty = WorkBudget::new(100);
        let mut eng_empty = ParserEngine::new(&schema, &mut r_empty, &mut b_empty);
        assert!(eng_empty.match_literal_delimiter("").is_ok());
        assert!(eng_empty.match_literal_delimiter("   ").is_ok());

        // NEL with 0xC2 and invalid byte / end of data (lines 529-552)
        assert!(test_delim(&[0xC2, 0x85], &[DelimToken::NEL]).is_ok());
        assert!(test_delim(&[0xC2, 0x00], &[DelimToken::NEL]).is_err());
        assert!(test_delim(&[0xC2], &[DelimToken::NEL]).is_err());
        assert!(test_delim(&[0x85], &[DelimToken::NEL]).is_ok());

        // HT with end of data (lines 634-639)
        assert!(test_delim(&[], &[DelimToken::HT]).is_err());

        // peek_literal_delimiter and peek_delimiter_match_length with empty (lines 843-858)
        assert!(!eng_empty.peek_literal_delimiter(""));
        assert_eq!(eng_empty.peek_delimiter_match_length(""), None);

        // is_in_scope_terminator_longer (lines 882-915)
        let src_long = SliceByteSource::new(b"LONGTERMINATOR");
        let mut r_long = BitReader::new(src_long, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_long = WorkBudget::new(100);
        let mut eng_long = ParserEngine::new(&schema, &mut r_long, &mut b_long);
        eng_long.in_scope_delimiters.push(crate::schema::ir::InScopeDelimiter {
            text: "LONG".into(),
            ignore_case: false,
            encoding: "UTF-8".into(),
        });
        eng_long.in_scope_terminators.push(crate::schema::ir::InScopeDelimiter {
            text: "LONGTERMINATOR".into(),
            ignore_case: false,
            encoding: "UTF-8".into(),
        });
        assert!(eng_long.is_in_scope_terminator_longer(16));

        // peek_only_separators_to_end (lines 907-925)
        let src_seps = SliceByteSource::new(b",,,");
        let mut r_seps = BitReader::new(src_seps, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_seps = WorkBudget::new(100);
        let mut eng_seps = ParserEngine::new(&schema, &mut r_seps, &mut b_seps);
        assert!(!eng_seps.peek_only_separators_to_end(""));
        assert!(eng_seps.peek_only_separators_to_end(","));
        let src_bad = SliceByteSource::new(b",,X");
        let mut r_bad = BitReader::new(src_bad, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bad = ParserEngine::new(&schema, &mut r_bad, &mut b_seps);
        assert!(!eng_bad.peek_only_separators_to_end(","));

        // peek_any_in_scope_delimiter (lines 887-905)
        assert!(eng_long.peek_any_in_scope_delimiter());

        // CharRef with unicode value > 0xFF (lines 750-768)
        assert!(test_delim(&[0xE1, 0x88, 0xB4], &[DelimToken::CharRef(0x1234)]).is_ok());
        assert!(test_delim(&[0xE1, 0x00], &[DelimToken::CharRef(0x1234)]).is_err());
        assert!(test_delim(&[], &[DelimToken::CharRef(0x1234)]).is_err());

        // CR followed by non-LF in WSP and WSPStar (lines 672-704)
        assert!(test_delim(b"\rX", &[DelimToken::WSP]).is_ok());
        assert!(test_delim(b"\rX", &[DelimToken::WSPStar]).is_ok());
        assert!(test_delim(b"\rX", &[DelimToken::WSPPlus]).is_ok());

        // Additional tests for match_dfdl_string_literal and tokens (lines 927-1052)
        assert!(match_dfdl_string_literal("%INVALID_ENTITY;", "%INVALID_ENTITY;", false));
        assert!(match_dfdl_string_literal("%INVALID_ENTITY;", "%invalid_entity;", true));
        assert!(!match_dfdl_string_literal("%INVALID_ENTITY;", "other", false));
        assert!(!match_dfdl_string_literal("literal", "lit", false));

        // DelimToken variants against str
        assert!(!match_delim_tokens_against_str(&[DelimToken::Literal(alloc::vec![0xFF, 0xFF])], "test", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::Literal(alloc::vec![]), DelimToken::SP], " ", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::CharRef(0xD800)], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::CharRef(0x61)], "", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::NEL], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::LS], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::FF], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::VT], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::CR], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::HT], "a", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::NUL], "a", false));

        // In-scope terminator not longer test (line 882)
        assert!(!eng_long.is_in_scope_terminator_longer(200));

        // Additional edge case coverage for delimiters
        // 1. Invalid entity reference (line 194)
        assert!(parse_single_delim_tokens("%unknown_entity;").is_err());
        assert!(parse_single_delim_tokens("%").is_err());

        // 2. evaluate_property_str_at_with_namespaces (line 259)
        let builder = InfosetBuilder::new();
        assert_eq!(
            eng_long.evaluate_property_str_at_with_namespaces("literal", &builder, None, None).unwrap(),
            "literal"
        );
        assert_eq!(
            eng_long.evaluate_property_str_at_with_namespaces("{ 'dynamic_delim' }", &builder, None, None).unwrap(),
            "dynamic_delim"
        );

        // 3. String matching with NL, WSP, WSPStar, WSPPlus against str (lines 1062-1100)
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::SP], "\r\n ", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::SP], "\n ", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::WSP, DelimToken::SP], "  ", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::WSP], "", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::WSPStar, DelimToken::SP], " ", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::WSPStar, DelimToken::SP], "   ", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::WSPPlus, DelimToken::SP], "\t ", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::WSPPlus], "", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::WSPPlus, DelimToken::SP], "  X", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::WSPStar, DelimToken::SP], "  X", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::SP], "\r\nX", false));

        // 4. peek_delimiter_match_length and match_literal_delimiter with whitespace-only or invalid
        assert_eq!(eng_long.peek_delimiter_match_length("   "), None);
        assert!(eng_long.match_literal_delimiter("   ").is_ok());
        assert_eq!(eng_long.peek_delimiter_match_length("%invalid_alt;"), None);

        // 5. WSP, WSPStar, and WSPPlus matching CRLF and CR with rollback
        let crlf_bytes = b"\r\nREST";
        let src_crlf = SliceByteSource::new(crlf_bytes);
        let mut r_crlf = BitReader::new(src_crlf, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_crlf = ParserEngine::new(&schema, &mut r_crlf, &mut budget);
        assert!(eng_crlf.match_single_delim_tokens_with_case(&[DelimToken::WSP], false).is_ok());

        let cr_only_bytes = b"\rX";
        let src_cr = SliceByteSource::new(cr_only_bytes);
        let mut r_cr = BitReader::new(src_cr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr = ParserEngine::new(&schema, &mut r_cr, &mut budget);
        assert!(eng_cr.match_single_delim_tokens_with_case(&[DelimToken::WSPStar], false).is_ok());
        assert_eq!(eng_cr.reader.read_bits(8).unwrap() as u8, b'X');

        let cr_plus_bytes = b"\rX";
        let src_cr_plus = SliceByteSource::new(cr_plus_bytes);
        let mut r_cr_plus = BitReader::new(src_cr_plus, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr_plus = ParserEngine::new(&schema, &mut r_cr_plus, &mut budget);
        assert!(eng_cr_plus.match_single_delim_tokens_with_case(&[DelimToken::WSPPlus], false).is_ok());
        assert_eq!(eng_cr_plus.reader.read_bits(8).unwrap() as u8, b'X');

        // Test NL matching isolated CR when followed by non-LF character
        let cr_nl_bytes = b"\rX";
        let src_cr_nl = SliceByteSource::new(cr_nl_bytes);
        let mut r_cr_nl = BitReader::new(src_cr_nl, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr_nl = ParserEngine::new(&schema, &mut r_cr_nl, &mut budget);
        assert!(eng_cr_nl.match_single_delim_tokens_with_case(&[DelimToken::NL], false).is_ok());
        assert_eq!(eng_cr_nl.reader.read_bits(8).unwrap() as u8, b'X');

        // Test WSP matching isolated CR when followed by non-LF character
        let cr_wsp_bytes = b"\rX";
        let src_cr_wsp = SliceByteSource::new(cr_wsp_bytes);
        let mut r_cr_wsp = BitReader::new(src_cr_wsp, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr_wsp = ParserEngine::new(&schema, &mut r_cr_wsp, &mut budget);
        assert!(eng_cr_wsp.match_single_delim_tokens_with_case(&[DelimToken::WSP], false).is_ok());
        assert_eq!(eng_cr_wsp.reader.read_bits(8).unwrap() as u8, b'X');

        // Test CharRef with multibyte UTF-8 codepoints (> 0xFF, e.g., Euro sign U+20AC)
        let euro_bytes = "\u{20AC}".as_bytes(); // 0xE2, 0x82, 0xAC
        let src_euro = SliceByteSource::new(euro_bytes);
        let mut r_euro = BitReader::new(src_euro, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_euro = ParserEngine::new(&schema, &mut r_euro, &mut budget);
        assert!(eng_euro.match_single_delim_tokens_with_case(&[DelimToken::CharRef(0x20AC)], false).is_ok());

        // Test CharRef mismatch and EOF branches for multibyte UTF-8 codepoint
        let euro_bad_bytes = b"\xE2\x82\x00";
        let src_euro_bad = SliceByteSource::new(euro_bad_bytes);
        let mut r_euro_bad = BitReader::new(src_euro_bad, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_euro_bad = ParserEngine::new(&schema, &mut r_euro_bad, &mut budget);
        assert!(eng_euro_bad.match_single_delim_tokens_with_case(&[DelimToken::CharRef(0x20AC)], false).is_err());

        // Test CharRef with surrogate scalar value (char::from_u32 is None)
        let empty_bytes = b"";
        let src_empty = SliceByteSource::new(empty_bytes);
        let mut r_empty = BitReader::new(src_empty, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_empty = ParserEngine::new(&schema, &mut r_empty, &mut budget);
        assert!(eng_empty.match_single_delim_tokens_with_case(&[DelimToken::CharRef(0xD800)], false).is_ok());

        // Test delimiter expression parser error branches for invalid entities and formatting
        assert!(parse_single_delim_tokens("%#invalid;").is_err());
        assert!(parse_single_delim_tokens("%#99999999999999999999999999999999999999999999;").is_err());
        assert!(parse_single_delim_tokens("%#xZZ;").is_err());
        assert!(parse_single_delim_tokens("%not_a_dfdl_entity;").is_err());
        assert!(parse_single_delim_tokens("%#").is_err());
        assert!(parse_single_delim_tokens("%").is_err());

        // Test unescape_leading_brace helper with double braces and single brace
        assert_eq!(unescape_leading_brace("{{escaped"), "{escaped");
        assert_eq!(unescape_leading_brace("{unescaped"), "{unescaped");
        assert_eq!(unescape_leading_brace("literal"), "literal");
    }
}
