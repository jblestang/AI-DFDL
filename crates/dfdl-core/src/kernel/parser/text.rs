//! Text parsing and character decoding logic for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::{DfdlSimpleType, DfdlValue, InfosetBuilder};
use crate::io::bitstream::BitReader;
use crate::io::traits::{BitOrder, ByteSource};
use crate::schema::ir::{ResolvedProperties, TextNumberCheckPolicy};
use crate::util::try_push;

use super::calendar::parse_calendar_from_text;
use super::numbers::{
    convert_big_radix_to_dec, normalize_text_number, parse_flexible_bool,
    parse_flexible_f64_with_props, parse_flexible_int_i64, parse_flexible_uint_u64,
    parse_strict_f64, parse_strict_int_i64, parse_strict_uint_u64,
};
use super::ParserEngine;

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    pub(crate) fn read_utf8_char_bytes(&mut self, bytes: &mut Vec<u8>) -> DFDLResult<()> {
        if self.reader.is_eof() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Insufficient text data for explicit length scalar",
            ));
        }
        let lead = self.reader.read_bits(8)? as u8;
        try_push(bytes, lead)?;
        let extra = if lead & 0x80 == 0 {
            0
        } else if lead & 0xE0 == 0xC0 {
            1
        } else if lead & 0xF0 == 0xE0 {
            2
        } else if lead & 0xF8 == 0xF0 {
            3
        } else {
            0
        };
        for _ in 0..extra {
            if self.reader.is_eof() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::Parse,
                    "Insufficient text data for explicit length scalar",
                ));
            }
            let cp = self.reader.checkpoint();
            let b = self.reader.read_bits(8)? as u8;
            if b & 0xC0 != 0x80 {
                // Malformed sequence: this byte starts the next character.
                let _ = self.reader.rollback(cp);
                break;
            }
            try_push(bytes, b)?;
        }
        Ok(())
    }

    pub(crate) fn check_mandatory_alignment(&self, encoding: &str) -> DFDLResult<()> {
        let enc = encoding.to_ascii_uppercase();
        let align = if enc.starts_with("X-DFDL-")
            || enc.contains("-BIT")
            || enc.contains("BIT-PACKED")
        {
            1
        } else {
            8
        };

        if align > 1 {
            let bit_pos = self.reader.position().0;
            if !bit_pos.is_multiple_of(align) {
                let bit_in_byte_1based = bit_pos.saturating_add(1);
                let msg = alloc::format!(
                    "Parse Error: charset not byte aligned: current bit position is {}",
                    bit_in_byte_1based
                );
                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
            }
        }
        Ok(())
    }

    pub(crate) fn parse_text_value(
        &mut self,
        simple_type: DfdlSimpleType,
        props: &ResolvedProperties,
        dynamic_len: Option<usize>,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
    ) -> DFDLResult<DfdlValue> {
        self.check_mandatory_alignment(&props.encoding)?;
        let mut bytes = Vec::new();

        let raw_len = dynamic_len;
        let byte_len = raw_len.map(|l| match props.length_units {
            crate::schema::ir::LengthUnits::Bits => l.saturating_add(7) / 8,
            crate::schema::ir::LengthUnits::Characters => {
                let unit_bits = crate::encoding::encoding_unit_bits(&props.encoding);
                l.saturating_mul(unit_bits).saturating_add(7) / 8
            }
            crate::schema::ir::LengthUnits::Bytes => l,
        });

        let eval_sep = if let Some(ref raw) = props.separator {
            if raw.is_empty() {
                None
            } else {
                Some(self.evaluate_property_str_at(raw, builder, elem_name)?)
            }
        } else {
            None
        };
        let sep_str = eval_sep.as_deref();

        let eval_term = if let Some(ref raw) = props.terminator {
            if raw.is_empty() {
                None
            } else {
                Some(self.evaluate_property_str_at(raw, builder, elem_name)?)
            }
        } else {
            None
        };
        let term_str = eval_term.as_deref();

        let sub_byte_bits = crate::encoding::encoding_char_bits(&props.encoding);
        let raw_val_string = if let Some(cb) = sub_byte_bits {
            let mut s = String::new();
            if let Some(char_or_byte_len) = raw_len {
                let num_chars = match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => char_or_byte_len / cb,
                    crate::schema::ir::LengthUnits::Characters => char_or_byte_len,
                    crate::schema::ir::LengthUnits::Bytes => (char_or_byte_len.saturating_mul(8)) / cb,
                };
                for _ in 0..num_chars {
                    if self.reader.is_eof() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Insufficient text data for explicit length scalar",
                        ));
                    }
                    let code = self.reader.read_bits(cb)?;
                    let ch = crate::encoding::decode_sub_byte_char(code, &props.encoding);
                    s.push(ch);
                }
            } else {
                while !self.reader.is_eof() {
                    if let Some(sep) = sep_str {
                        if !sep.is_empty() && self.peek_literal_delimiter(sep) {
                            break;
                        }
                    }
                    if let Some(term) = term_str {
                        if !term.is_empty() && self.peek_literal_delimiter(term) {
                            break;
                        }
                    }
                    let mut matched_in_scope = false;
                    for i in (0..self.in_scope_delimiters.len()).rev() {
                        if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                            let delim_clone = delim.clone();
                            if !delim_clone.is_empty() && self.peek_literal_delimiter(&delim_clone) {
                                matched_in_scope = true;
                                break;
                            }
                        }
                    }
                    if matched_in_scope {
                        break;
                    }

                    if let Ok(code) = self.reader.read_bits(cb) {
                        let ch = crate::encoding::decode_sub_byte_char(code, &props.encoding);
                        s.push(ch);
                    } else {
                        break;
                    }
                }
            }
            s
        } else {
            if let Some(char_or_byte_len) = raw_len {
                if props.length_units == crate::schema::ir::LengthUnits::Characters
                    && props.encoding.to_ascii_uppercase().contains("UTF-8")
                {
                    for _ in 0..char_or_byte_len {
                        self.read_utf8_char_bytes(&mut bytes)?;
                    }
                } else if props.length_units == crate::schema::ir::LengthUnits::Characters
                    && (props.encoding.to_ascii_uppercase().contains("UTF-16")
                        || props.encoding.to_ascii_uppercase().contains("UCS-2"))
                {
                    for _ in 0..char_or_byte_len {
                        for _ in 0..2 {
                            if self.reader.is_eof() {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Insufficient text data for explicit length scalar",
                                ));
                            }
                            let b = self.reader.read_bits(8)? as u8;
                            try_push(&mut bytes, b)?;
                        }
                    }
                } else if props.length_units == crate::schema::ir::LengthUnits::Characters
                    && (props.encoding.to_ascii_uppercase().contains("UTF-32")
                        || props.encoding.to_ascii_uppercase().contains("UCS-4"))
                {
                    for _ in 0..char_or_byte_len {
                        for _ in 0..4 {
                            if self.reader.is_eof() {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Parse,
                                    "Insufficient text data for explicit length scalar",
                                ));
                            }
                            let b = self.reader.read_bits(8)? as u8;
                            try_push(&mut bytes, b)?;
                        }
                    }
                } else if props.length_units == crate::schema::ir::LengthUnits::Bits {
                    let mut bits_left = char_or_byte_len;
                    while bits_left > 0 {
                        let chunk = bits_left.min(8);
                        if self.reader.is_eof() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Insufficient text data for explicit length scalar",
                            ));
                        }
                        let val = self.reader.read_bits(chunk)? as u8;
                        let byte_val = if chunk < 8
                            && self.reader.bit_order() == BitOrder::MostSignificantBitFirst
                        {
                            val << (8 - chunk)
                        } else {
                            val
                        };
                        try_push(&mut bytes, byte_val)?;
                        bits_left = bits_left.saturating_sub(chunk);
                    }
                } else if let Some(len) = byte_len {
                    for _ in 0..len {
                        if self.reader.is_eof() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Parse,
                                "Insufficient text data for explicit length scalar",
                            ));
                        }
                        let b = self.reader.read_bits(8)? as u8;
                        try_push(&mut bytes, b)?;
                    }
                }
            } else if let Some(ref scheme) = props.escape_scheme {
                let peek_bytes = |reader: &mut BitReader<S>, target: &[u8]| -> bool {
                    if target.is_empty() {
                        return false;
                    }
                    let cp = reader.checkpoint();
                    for &expected in target {
                        match reader.read_bits(8) {
                            Ok(b) if (b as u8) == expected => {}
                            _ => {
                                let _ = reader.rollback(cp);
                                return false;
                            }
                        }
                    }
                    let _ = reader.rollback(cp);
                    true
                };

                let consume_bytes = |reader: &mut BitReader<S>, count: usize| -> DFDLResult<()> {
                    for _ in 0..count {
                        let _ = reader.read_bits(8)?;
                    }
                    Ok(())
                };

                let mut decode_scheme_prop = |raw_opt: Option<&String>| -> DFDLResult<Option<Vec<u8>>> {
                    if let Some(raw) = raw_opt {
                        let evaluated = if raw.starts_with('{') && raw.ends_with('}') {
                            self.evaluate_property_str_at(raw, builder, elem_name)?
                        } else {
                            raw.clone()
                        };
                        let decoded = crate::expr::properties::decode_dfdl_character_entities(&evaluated);
                        Ok(Some(crate::encoding::encode_text_string(&decoded, &props.encoding)))
                    } else {
                        Ok(None)
                    }
                };

                let ec_bytes_opt = decode_scheme_prop(scheme.escape_character.as_ref())?;
                let eec_bytes_opt = decode_scheme_prop(scheme.escape_escape_character.as_ref())?;
                let bs_bytes_opt = decode_scheme_prop(scheme.escape_block_start.as_ref())?;
                let be_bytes_opt = decode_scheme_prop(scheme.escape_block_end.as_ref())?;

                let extra_escaped_bytes: Vec<Vec<u8>> = scheme
                    .extra_escaped_characters
                    .iter()
                    .map(|&ch| {
                        let mut buf = [0u8; 4];
                        let ch_str = ch.encode_utf8(&mut buf);
                        crate::encoding::encode_text_string(ch_str, &props.encoding)
                    })
                    .collect();

                let mut all_delimiters: Vec<String> = Vec::new();
                if let Some(t) = term_str {
                    if !t.is_empty() {
                        let _ = try_push(&mut all_delimiters, t.to_string());
                    }
                }
                if let Some(s) = sep_str {
                    if !s.is_empty() {
                        let _ = try_push(&mut all_delimiters, s.to_string());
                    }
                }
                for d in &self.in_scope_delimiters {
                    if !d.is_empty() && !all_delimiters.iter().any(|existing| existing == d) {
                        let _ = try_push(&mut all_delimiters, d.clone());
                    }
                }

                match scheme.escape_kind {
                    crate::schema::ir::EscapeKind::EscapeBlock => {
                        let bs_bytes = bs_bytes_opt.as_deref().unwrap_or(&[]);
                        let be_bytes = be_bytes_opt.as_deref().unwrap_or(&[]);
                        let is_block = !bs_bytes.is_empty() && peek_bytes(self.reader, bs_bytes);

                        if is_block {
                            consume_bytes(self.reader, bs_bytes.len())?;
                            loop {
                                if self.reader.is_eof() {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::Parse,
                                        "Parse Error: Unclosed escape block: end of data reached before escapeBlockEnd",
                                    ));
                                }

                                if let Some(ref eec_bytes) = eec_bytes_opt {
                                    if !eec_bytes.is_empty() && peek_bytes(self.reader, eec_bytes) {
                                        let cp = self.reader.checkpoint();
                                        consume_bytes(self.reader, eec_bytes.len())?;
                                        if !be_bytes.is_empty() && peek_bytes(self.reader, be_bytes) {
                                            consume_bytes(self.reader, be_bytes.len())?;
                                            for b in be_bytes {
                                                try_push(&mut bytes, *b)?;
                                            }
                                            continue;
                                        } else if peek_bytes(self.reader, eec_bytes) {
                                            consume_bytes(self.reader, eec_bytes.len())?;
                                            for b in eec_bytes {
                                                try_push(&mut bytes, *b)?;
                                            }
                                            continue;
                                        } else {
                                            let _ = self.reader.rollback(cp);
                                            consume_bytes(self.reader, eec_bytes.len())?;
                                            for b in eec_bytes {
                                                try_push(&mut bytes, *b)?;
                                            }
                                            continue;
                                        }
                                    }
                                }

                                if !be_bytes.is_empty() && peek_bytes(self.reader, be_bytes) {
                                    consume_bytes(self.reader, be_bytes.len())?;
                                    break;
                                }

                                let b = self.reader.read_bits(8)? as u8;
                                try_push(&mut bytes, b)?;
                            }
                        }

                        while !self.reader.is_eof() {
                            if let Some(sep) = sep_str {
                                if !sep.is_empty() && self.peek_literal_delimiter(sep) {
                                    break;
                                }
                            }
                            if let Some(term) = term_str {
                                if !term.is_empty() && self.peek_literal_delimiter(term) {
                                    break;
                                }
                            }
                            let mut matched_in_scope = false;
                            for i in (0..self.in_scope_delimiters.len()).rev() {
                                if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                                    let delim_clone = delim.clone();
                                    if !delim_clone.is_empty() && self.peek_literal_delimiter(&delim_clone) {
                                        matched_in_scope = true;
                                        break;
                                    }
                                }
                            }
                            if matched_in_scope {
                                break;
                            }

                            let b = self.reader.read_bits(8)? as u8;
                            try_push(&mut bytes, b)?;
                        }
                    }
                    crate::schema::ir::EscapeKind::EscapeCharacter => {
                        let ec_bytes = ec_bytes_opt.as_deref().unwrap_or(&[]);
                        let eec_bytes_opt = eec_bytes_opt.as_deref();

                        while !self.reader.is_eof() {
                            // 1. Check if escapeEscapeCharacter matches
                            if let Some(eec_bytes) = eec_bytes_opt {
                                if !eec_bytes.is_empty() && peek_bytes(self.reader, eec_bytes) {
                                    let cp = self.reader.checkpoint();
                                    consume_bytes(self.reader, eec_bytes.len())?;
                                    if !ec_bytes.is_empty() && peek_bytes(self.reader, ec_bytes) {
                                        consume_bytes(self.reader, ec_bytes.len())?;
                                        for b in ec_bytes {
                                            try_push(&mut bytes, *b)?;
                                        }
                                        continue;
                                    } else if peek_bytes(self.reader, eec_bytes) {
                                        consume_bytes(self.reader, eec_bytes.len())?;
                                        for b in eec_bytes {
                                            try_push(&mut bytes, *b)?;
                                        }
                                        continue;
                                    } else if eec_bytes == ec_bytes {
                                        // When eec == ec and neither ec nor eec follows, this character is escapeCharacter,
                                        // NOT an unescaped eec data character. Roll back so step 2 handles it as escapeCharacter.
                                        let _ = self.reader.rollback(cp);
                                    } else {
                                        let _ = self.reader.rollback(cp);
                                        consume_bytes(self.reader, eec_bytes.len())?;
                                        for b in eec_bytes {
                                            try_push(&mut bytes, *b)?;
                                        }
                                        continue;
                                    }
                                }
                            }

                            // 2. Check if escapeCharacter matches
                            if !ec_bytes.is_empty() && peek_bytes(self.reader, ec_bytes) {
                                consume_bytes(self.reader, ec_bytes.len())?;

                                // a) Escaped delimiter?
                                let mut matched_escaped_delim = None;
                                for d in &all_delimiters {
                                    if !d.is_empty() && self.peek_literal_delimiter(d) {
                                        matched_escaped_delim = Some(d.clone());
                                        break;
                                    }
                                }
                                if let Some(d) = matched_escaped_delim {
                                    let cp_d = self.reader.checkpoint();
                                    let _ = self.match_literal_delimiter(&d);
                                    let num_bits = self.reader.position().0.saturating_sub(cp_d.bit_position.0);
                                    let _ = self.reader.rollback(cp_d);
                                    for _ in 0..(num_bits / 8) {
                                        let b = self.reader.read_bits(8)? as u8;
                                        try_push(&mut bytes, b)?;
                                    }
                                    continue;
                                }

                                // b) Escaped extra character?
                                let mut matched_extra = None;
                                for extra_b in &extra_escaped_bytes {
                                    if !extra_b.is_empty() && peek_bytes(self.reader, extra_b) {
                                        matched_extra = Some(extra_b.clone());
                                        break;
                                    }
                                }
                                if let Some(extra_b) = matched_extra {
                                    consume_bytes(self.reader, extra_b.len())?;
                                    for b in &extra_b {
                                        try_push(&mut bytes, *b)?;
                                    }
                                    continue;
                                }

                                // c) If escapeEscapeCharacter is not defined and ec is followed by ec:
                                if eec_bytes_opt.is_none() && peek_bytes(self.reader, ec_bytes) {
                                    consume_bytes(self.reader, ec_bytes.len())?;
                                    for b in ec_bytes {
                                        try_push(&mut bytes, *b)?;
                                    }
                                    continue;
                                }

                                // d) ec was NOT escaping a delimiter or extra character:
                                // In DFDL §13.2.1: ec is removed from data, and following char is data.
                                if !self.reader.is_eof() {
                                    let b = self.reader.read_bits(8)? as u8;
                                    try_push(&mut bytes, b)?;
                                    continue;
                                } else {
                                    break;
                                }
                            }

                            // 3. Unescaped in-scope delimiter check
                            if let Some(sep) = sep_str {
                                if !sep.is_empty() && self.peek_literal_delimiter(sep) {
                                    break;
                                }
                            }
                            if let Some(term) = term_str {
                                if !term.is_empty() && self.peek_literal_delimiter(term) {
                                    break;
                                }
                            }
                            let mut matched_in_scope = false;
                            for i in (0..self.in_scope_delimiters.len()).rev() {
                                if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                                    let delim_clone = delim.clone();
                                    if !delim_clone.is_empty() && self.peek_literal_delimiter(&delim_clone) {
                                        matched_in_scope = true;
                                        break;
                                    }
                                }
                            }
                            if matched_in_scope {
                                break;
                            }

                            // 4. Regular data byte
                            let b = self.reader.read_bits(8)? as u8;
                            try_push(&mut bytes, b)?;
                        }
                    }
                }
            } else {
                while !self.reader.is_eof() {
                    if let Some(sep) = sep_str {
                        if !sep.is_empty() && self.peek_literal_delimiter(sep) {
                            break;
                        }
                    }
                    if let Some(term) = term_str {
                        if !term.is_empty() && self.peek_literal_delimiter(term) {
                            break;
                        }
                    }
                    let mut matched_in_scope = false;
                    for i in (0..self.in_scope_delimiters.len()).rev() {
                        if let Ok(delim) = crate::util::get_checked(&self.in_scope_delimiters, i) {
                            let delim_clone = delim.clone();
                            if !delim_clone.is_empty() && self.peek_literal_delimiter(&delim_clone) {
                                matched_in_scope = true;
                                break;
                            }
                        }
                    }
                    if matched_in_scope {
                        break;
                    }

                    let b = self.reader.read_bits(8)? as u8;
                    try_push(&mut bytes, b)?;
                }
            }

            if props.encoding_error_policy_error
                && crate::encoding::has_malformed_input(&bytes, &props.encoding)
            {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!(
                        "Parse Error: Malformed input for encoding {} with encodingErrorPolicy 'error'",
                        props.encoding
                    ),
                ));
            }
            crate::encoding::decode_text_bytes(&bytes, &props.encoding)?
        };
        let raw_val_str = raw_val_string.as_str();

        let pad_char_str = if simple_type.is_numeric() {
            props
                .text_number_pad_character
                .as_deref()
                .unwrap_or(&props.text_pad_char)
        } else if matches!(
            simple_type,
            DfdlSimpleType::Date | DfdlSimpleType::Time | DfdlSimpleType::DateTime
        ) {
            props
                .text_calendar_pad_character
                .as_deref()
                .unwrap_or(&props.text_pad_char)
        } else if simple_type == DfdlSimpleType::Boolean {
            props
                .text_boolean_pad_character
                .as_deref()
                .unwrap_or(&props.text_pad_char)
        } else {
            &props.text_pad_char
        };
        let val_str = match props.text_trim_kind {
            crate::schema::ir::TextTrimKind::None => raw_val_str,
            crate::schema::ir::TextTrimKind::Head => {
                raw_val_str.trim_start_matches(|c: char| pad_char_str.contains(c))
            }
            crate::schema::ir::TextTrimKind::Tail => {
                raw_val_str.trim_end_matches(|c: char| pad_char_str.contains(c))
            }
            crate::schema::ir::TextTrimKind::Both => {
                raw_val_str.trim_matches(|c: char| pad_char_str.contains(c))
            }
        };

        let check_policy = props.text_number_check_policy;
        let pattern_opt = props.text_number_pattern.as_deref();

        let eval_dec_sep = if props.text_standard_decimal_separator.starts_with('{')
            && props.text_standard_decimal_separator.ends_with('}')
        {
            let s = self.evaluate_property_str_at(&props.text_standard_decimal_separator, builder, elem_name)?;
            let decoded = crate::expr::properties::decode_dfdl_character_entities(&s);
            if decoded.chars().count() != 1 {
                let msg = alloc::format!(
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardDecimalSeparator, got '{}'",
                    s
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            Some(decoded)
        } else {
            None
        };
        let dec_sep = if let Some(ref d) = eval_dec_sep {
            d.as_str()
        } else if props.text_standard_decimal_separator.is_empty() {
            "."
        } else {
            &props.text_standard_decimal_separator
        };

        let eval_grp_sep = if props.text_standard_grouping_separator.starts_with('{')
            && props.text_standard_grouping_separator.ends_with('}')
        {
            let s =
                self.evaluate_property_str_at(&props.text_standard_grouping_separator, builder, elem_name)?;
            let decoded = crate::expr::properties::decode_dfdl_character_entities(&s);
            if decoded.chars().count() != 1 {
                let msg = alloc::format!(
                    "Schema Definition Error: Length of string must be exactly 1 character for textStandardGroupingSeparator, got '{}'",
                    s
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            Some(decoded)
        } else {
            None
        };
        let grp_sep = if let Some(ref g) = eval_grp_sep {
            g.as_str()
        } else if props.text_standard_grouping_separator.is_empty() {
            ","
        } else {
            &props.text_standard_grouping_separator
        };

        let base = props.text_standard_base;
        let nan_rep = props.text_standard_nan_rep.as_deref();
        let inf_rep = props.text_standard_infinity_rep.as_deref();
        let exp_rep_prop = props.text_standard_exponent_rep.as_deref();
        let eval_exp_rep = if let Some(e) = exp_rep_prop {
            if e.starts_with('{') && e.ends_with('}') && !e.starts_with("{{") {
                Some(self.evaluate_property_str_at(e, builder, elem_name)?)
            } else {
                None
            }
        } else {
            None
        };
        let exp_rep = eval_exp_rep.as_deref().or(exp_rep_prop);
        let ignore_case = props.ignore_case;
        let num_pad_char_str = props
            .text_number_pad_character
            .as_deref()
            .unwrap_or(&props.text_pad_char);

        let parse_int = |s: &str| -> Option<i64> {
            if base != 10 {
                let clean = s.trim();
                if clean.starts_with('+') || clean.starts_with('-') {
                    return None;
                }
                i64::from_str_radix(clean, base).ok()
            } else if check_policy == TextNumberCheckPolicy::Strict {
                parse_strict_int_i64(s, pattern_opt, dec_sep, grp_sep, Some(num_pad_char_str), props.text_trim_kind)
            } else {
                if let Some((is_neg, norm_str)) = normalize_text_number(
                    s,
                    pattern_opt,
                    dec_sep,
                    grp_sep,
                    Some(num_pad_char_str),
                    props.text_trim_kind,
                ) {
                    if let Ok(v) = norm_str.parse::<i64>() {
                        return Some(if is_neg { -v } else { v });
                    }
                    if let Some((int_part, dec_part)) = norm_str.split_once('.') {
                        if dec_part.chars().all(|c| c == '0') {
                            if let Ok(v) = int_part.parse::<i64>() {
                                return Some(if is_neg { -v } else { v });
                            }
                        }
                    }
                }
                parse_flexible_int_i64(s)
            }
        };

        let parse_uint = |s: &str| -> Option<u64> {
            if base != 10 {
                let clean = s.trim();
                if clean.starts_with('+') || clean.starts_with('-') {
                    return None;
                }
                u64::from_str_radix(clean, base).ok()
            } else if check_policy == TextNumberCheckPolicy::Strict {
                parse_strict_uint_u64(s, pattern_opt, dec_sep, grp_sep, Some(num_pad_char_str), props.text_trim_kind)
            } else {
                if let Some((is_neg, norm_str)) = normalize_text_number(
                    s,
                    pattern_opt,
                    dec_sep,
                    grp_sep,
                    Some(num_pad_char_str),
                    props.text_trim_kind,
                ) {
                    if !is_neg {
                        if let Ok(v) = norm_str.parse::<u64>() {
                            return Some(v);
                        }
                        if let Some((int_part, dec_part)) = norm_str.split_once('.') {
                            if dec_part.chars().all(|c| c == '0') {
                                if let Ok(v) = int_part.parse::<u64>() {
                                    return Some(v);
                                }
                            }
                        }
                    }
                }
                parse_flexible_uint_u64(s)
            }
        };
        let parse_float = |s: &str| -> Option<f64> {
            if check_policy == TextNumberCheckPolicy::Strict {
                parse_strict_f64(
                    s,
                    pattern_opt,
                    dec_sep,
                    grp_sep,
                    nan_rep,
                    inf_rep,
                    exp_rep,
                    ignore_case,
                    Some(num_pad_char_str),
                    props.text_trim_kind,
                )
            } else {
                if let Some((is_neg, norm_str)) = normalize_text_number(
                    s,
                    pattern_opt,
                    dec_sep,
                    grp_sep,
                    Some(num_pad_char_str),
                    props.text_trim_kind,
                ) {
                    if let Some(mut v) = parse_flexible_f64_with_props(
                        &norm_str,
                        ".",
                        "",
                        nan_rep,
                        inf_rep,
                        exp_rep,
                        ignore_case,
                    ) {
                        if is_neg {
                            v = -v;
                        }
                        return Some(v);
                    }
                }
                parse_flexible_f64_with_props(
                    s,
                    dec_sep,
                    grp_sep,
                    nan_rep,
                    inf_rep,
                    exp_rep,
                    ignore_case,
                )
            }
        };

        let normalized_storage = if simple_type.is_numeric() && base == 10 {
            normalize_text_number(
                val_str,
                pattern_opt,
                dec_sep,
                grp_sep,
                Some(num_pad_char_str),
                props.text_trim_kind,
            )
            .map(|(is_neg, body)| {
                if is_neg && !body.starts_with('-') {
                    alloc::format!("-{}", body)
                } else {
                    body
                }
            })
        } else {
            None
        };

        let clean_num_str = normalized_storage
            .as_deref()
            .unwrap_or_else(|| val_str.trim());

        if simple_type.is_numeric()
            && base != 10
            && (clean_num_str.starts_with('+') || clean_num_str.starts_with('-'))
        {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!(
                    "Parse Error: Non-base 10 representation cannot contain leading sign: '{}'",
                    clean_num_str
                ),
            ));
        }

        if simple_type.is_numeric() {
            if let Some(ref zero_reps) = props.text_standard_zero_rep {
                let zero_candidate = match props.text_trim_kind {
                    crate::schema::ir::TextTrimKind::Head => {
                        val_str.trim_start_matches(|c: char| num_pad_char_str.contains(c))
                    }
                    crate::schema::ir::TextTrimKind::Tail => {
                        val_str.trim_end_matches(|c: char| num_pad_char_str.contains(c))
                    }
                    crate::schema::ir::TextTrimKind::Both => {
                        val_str.trim_matches(|c: char| num_pad_char_str.contains(c))
                    }
                    crate::schema::ir::TextTrimKind::None => val_str,
                };

                let matches_target = |s: &str, target: &str| -> bool {
                    if props.ignore_case {
                        s.eq_ignore_ascii_case(target)
                    } else {
                        s == target
                    }
                };

                let check_zrep_match = |zrep: &str| -> bool {
                    match zrep {
                        "%WSP;" => zero_candidate == " " || zero_candidate == "\t",
                        "%WSP+;" => {
                            !zero_candidate.is_empty()
                                && zero_candidate.chars().all(|c| c == ' ' || c == '\t')
                        }
                        "%WSP*;" => zero_candidate.chars().all(|c| c == ' ' || c == '\t'),
                        "%ES;" | "" => zero_candidate.is_empty(),
                        _ => {
                            let unescaped =
                                crate::expr::properties::decode_dfdl_character_entities(zrep);
                            matches_target(zero_candidate, zrep)
                                || matches_target(zero_candidate, &unescaped)
                                || (unescaped.trim().is_empty() && zero_candidate.is_empty())
                        }
                    }
                };

                // An empty zero-rep list defines no zero representation (DFDL §13.6).
                let is_zero = if zero_reps.trim().is_empty() {
                    false
                } else {
                    zero_reps.split_whitespace().any(check_zrep_match)
                };

                if is_zero {
                    let zero_val = match simple_type {
                        DfdlSimpleType::Int => Some(DfdlValue::Int(0)),
                        DfdlSimpleType::Long => Some(DfdlValue::Long(0)),
                        DfdlSimpleType::Short => Some(DfdlValue::Short(0)),
                        DfdlSimpleType::Byte => Some(DfdlValue::Byte(0)),
                        DfdlSimpleType::UnsignedInt => Some(DfdlValue::UnsignedInt(0)),
                        DfdlSimpleType::UnsignedLong => Some(DfdlValue::UnsignedLong(0)),
                        DfdlSimpleType::UnsignedShort => Some(DfdlValue::UnsignedShort(0)),
                        DfdlSimpleType::UnsignedByte => Some(DfdlValue::UnsignedByte(0)),
                        DfdlSimpleType::Float => Some(DfdlValue::Float(0.0)),
                        DfdlSimpleType::Double => Some(DfdlValue::Double(0.0)),
                        DfdlSimpleType::Decimal => {
                            Some(DfdlValue::Decimal(alloc::string::ToString::to_string("0")))
                        }
                        _ => None,
                    };
                    if let Some(val) = zero_val {
                        return Ok(val);
                    }
                }
            }
        }

        if props.text_number_rep == crate::schema::ir::TextNumberRep::Zoned {
            if matches!(simple_type, DfdlSimpleType::Float | DfdlSimpleType::Double) {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: textNumberRep=\"zoned\" is not allowed for {:?}", simple_type),
                ));
            }
            let enc_upper = props.encoding.to_ascii_uppercase();
            let is_ebcdic = enc_upper.starts_with("EBCDIC") || enc_upper == "IBM037" || enc_upper == "CP037";
            let zoned_str = crate::kernel::parser::numbers::parse_zoned_number(
                val_str,
                pattern_opt,
                props.text_zoned_sign_style,
                is_ebcdic,
                props.decimal_signed,
            )
            .map_err(|e| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!(
                        "Parse Error: Unable to parse zoned xs:decimal from text '{}': {}",
                        val_str, e
                    ),
                )
            })?;
            return match simple_type {
                DfdlSimpleType::Decimal => Ok(DfdlValue::Decimal(zoned_str)),
                DfdlSimpleType::Int => {
                    let v = zoned_str.parse::<i32>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:int", zoned_str))
                    })?;
                    Ok(DfdlValue::Int(v))
                }
                DfdlSimpleType::Long => {
                    let v = zoned_str.parse::<i64>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:long", zoned_str))
                    })?;
                    Ok(DfdlValue::Long(v))
                }
                DfdlSimpleType::Short => {
                    let v = zoned_str.parse::<i16>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:short", zoned_str))
                    })?;
                    Ok(DfdlValue::Short(v))
                }
                DfdlSimpleType::Byte => {
                    let v = zoned_str.parse::<i8>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:byte", zoned_str))
                    })?;
                    Ok(DfdlValue::Byte(v))
                }
                DfdlSimpleType::UnsignedInt => {
                    let v = zoned_str.parse::<u32>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:unsignedInt", zoned_str))
                    })?;
                    Ok(DfdlValue::UnsignedInt(v))
                }
                DfdlSimpleType::UnsignedLong => {
                    let v = zoned_str.parse::<u64>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:unsignedLong", zoned_str))
                    })?;
                    Ok(DfdlValue::UnsignedLong(v))
                }
                DfdlSimpleType::UnsignedShort => {
                    let v = zoned_str.parse::<u16>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:unsignedShort", zoned_str))
                    })?;
                    Ok(DfdlValue::UnsignedShort(v))
                }
                DfdlSimpleType::UnsignedByte => {
                    let v = zoned_str.parse::<u8>().map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parsed value {} out of range for xs:unsignedByte", zoned_str))
                    })?;
                    Ok(DfdlValue::UnsignedByte(v))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: textNumberRep=\"zoned\" is not supported for {:?}", simple_type),
                )),
            };
        }

        match simple_type {
            DfdlSimpleType::Int => {
                let v = parse_int(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:int from text: {}", val_str.trim()),
                    )
                })?;
                let val_i32 = i32::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:int", val_str),
                    )
                })?;
                Ok(DfdlValue::Int(val_i32))
            }
            DfdlSimpleType::Long => {
                let v = parse_int(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:long from text: {}", val_str.trim()),
                    )
                })?;
                Ok(DfdlValue::Long(v))
            }
            DfdlSimpleType::Short => {
                let v = parse_int(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:short from text: {}", val_str.trim()),
                    )
                })?;
                let val_i16 = i16::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:short", val_str),
                    )
                })?;
                Ok(DfdlValue::Short(val_i16))
            }
            DfdlSimpleType::Byte => {
                let v = parse_int(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:byte from text: {}", val_str.trim()),
                    )
                })?;
                let val_i8 = i8::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parsed value {} out of range for xs:byte", val_str),
                    )
                })?;
                Ok(DfdlValue::Byte(val_i8))
            }
            DfdlSimpleType::UnsignedInt => {
                if val_str.trim().starts_with('-') || clean_num_str.starts_with('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedInt: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    ));
                }
                let v = parse_uint(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parse Error: Unable to parse xs:unsignedInt from text: {}",
                            val_str.trim()
                        ),
                    )
                })?;
                let val_u32 = u32::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedInt (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedInt(val_u32))
            }
            DfdlSimpleType::UnsignedLong => {
                if val_str.trim().starts_with('-') || clean_num_str.starts_with('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedLong: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    ));
                }
                let v = parse_uint(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parse Error: Unable to parse xs:unsignedLong from text: {}",
                            val_str.trim()
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedLong(v))
            }
            DfdlSimpleType::UnsignedShort => {
                if val_str.trim().starts_with('-') || clean_num_str.starts_with('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedShort: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    ));
                }
                let v = parse_uint(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parse Error: Unable to parse xs:unsignedShort from text: {}",
                            val_str.trim()
                        ),
                    )
                })?;
                let val_u16 = u16::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedShort (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedShort(val_u16))
            }
            DfdlSimpleType::UnsignedByte => {
                if val_str.trim().starts_with('-') || clean_num_str.starts_with('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedByte: must match in signedness (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    ));
                }
                let v = parse_uint(val_str).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parse Error: Unable to parse xs:unsignedByte from text: {}",
                            val_str.trim()
                        ),
                    )
                })?;
                let val_u8 = u8::try_from(v).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!(
                            "Parsed value {} out of range for xs:unsignedByte (xs:unsignedLong or xs:long or a subtype of those)",
                            val_str.trim()
                        ),
                    )
                })?;
                Ok(DfdlValue::UnsignedByte(val_u8))
            }
            DfdlSimpleType::Boolean => {
                let bool_pad = props
                    .text_boolean_pad_character
                    .as_deref()
                    .unwrap_or(&props.text_pad_char);
                let trimmed = match props.text_trim_kind {
                    crate::schema::ir::TextTrimKind::None => val_str.trim(),
                    crate::schema::ir::TextTrimKind::Head => {
                        val_str.trim_start_matches(|c: char| bool_pad.contains(c)).trim()
                    }
                    crate::schema::ir::TextTrimKind::Tail => {
                        val_str.trim_end_matches(|c: char| bool_pad.contains(c)).trim()
                    }
                    crate::schema::ir::TextTrimKind::Both => {
                        val_str.trim_matches(|c: char| bool_pad.contains(c)).trim()
                    }
                };
                let matches_rep = |candidate: &str, rep_list: &str| -> bool {
                    for rep in rep_list.split_whitespace() {
                        let decoded = crate::expr::properties::decode_dfdl_character_entities(rep);
                        if props.ignore_case {
                            if candidate.eq_ignore_ascii_case(rep)
                                || candidate.eq_ignore_ascii_case(&decoded)
                            {
                                return true;
                            }
                        } else if candidate == rep || candidate == decoded {
                            return true;
                        }
                    }
                    false
                };

                let is_true = if let Some(ref true_rep) = props.text_boolean_true_rep {
                    if matches_rep(trimmed, true_rep) {
                        Some(true)
                    } else if let Some(ref false_rep) = props.text_boolean_false_rep {
                        if matches_rep(trimmed, false_rep) {
                            Some(false)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if let Some(ref false_rep) = props.text_boolean_false_rep {
                    if matches_rep(trimmed, false_rep) {
                        Some(false)
                    } else {
                        None
                    }
                } else {
                    None
                };

                is_true
                    .or_else(|| parse_flexible_bool(trimmed))
                    .map(DfdlValue::Boolean)
                    .ok_or_else(|| {
                        DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Failed to parse text boolean scalar",
                        )
                    })
            }
            DfdlSimpleType::Float => parse_float(val_str)
                .map(|v| DfdlValue::Float(v as f32))
                .ok_or_else(|| {
                    DFDLError::new_static(DFDLErrorKind::Parse, "Failed to parse text float scalar")
                }),
            DfdlSimpleType::Double => parse_float(val_str)
                .map(DfdlValue::Double)
                .ok_or_else(|| {
                    DFDLError::new_static(
                        DFDLErrorKind::Parse,
                        "Failed to parse text double scalar",
                    )
                }),
            DfdlSimpleType::String => Ok(DfdlValue::String(alloc::string::ToString::to_string(
                val_str,
            ))),
            DfdlSimpleType::HexBinary => {
                if let Ok(s) = crate::encoding::decode_text_bytes(&bytes, &props.encoding) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty()
                        && trimmed.len() % 2 == 0
                        && trimmed.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        let mut parsed_bytes = Vec::with_capacity(trimmed.len() / 2);
                        let b_arr = trimmed.as_bytes();
                        for i in (0..b_arr.len()).step_by(2) {
                            if let (Some(&b1), Some(&b2)) = (b_arr.get(i), b_arr.get(i + 1)) {
                                if let (Some(h1), Some(h2)) =
                                    ((b1 as char).to_digit(16), (b2 as char).to_digit(16))
                                {
                                    parsed_bytes.push(((h1 << 4) | h2) as u8);
                                }
                            }
                        }
                        Ok(DfdlValue::HexBinary(parsed_bytes))
                    } else {
                        Ok(DfdlValue::HexBinary(bytes))
                    }
                } else {
                    Ok(DfdlValue::HexBinary(bytes))
                }
            }
            DfdlSimpleType::DateTime | DfdlSimpleType::Date | DfdlSimpleType::Time => {
                if let Some(ref raw_lang) = props.calendar_language {
                    let trimmed = raw_lang.trim();
                    let eval_lang = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                        let ast = crate::expr::parse_expr(trimmed)?;
                        let current_path = builder.current_path();
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
                        let v = crate::expr::eval_expr(&ast, &mut ctx)?;
                        alloc::format!("{}", v)
                    } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                        alloc::format!("{{{stripped}")
                    } else {
                        alloc::string::ToString::to_string(raw_lang)
                    };
                    crate::kernel::parser::calendar::validate_calendar_language_syntax(&eval_lang)?;
                }
                let cal_pad_str = props
                    .text_calendar_pad_character
                    .as_deref()
                    .unwrap_or(&props.text_pad_char);
                let pat = props.calendar_pattern.as_deref();
                let parsed = parse_calendar_from_text(
                    val_str,
                    pat,
                    Some(cal_pad_str),
                    props.text_trim_kind,
                    simple_type,
                    props.calendar_check_policy,
                    props.calendar_first_day_of_week,
                )?;
                match simple_type {
                    DfdlSimpleType::DateTime => Ok(DfdlValue::DateTime(parsed)),
                    DfdlSimpleType::Date => Ok(DfdlValue::Date(parsed)),
                    _ => Ok(DfdlValue::Time(parsed)),
                }
            }
            DfdlSimpleType::Decimal => {
                let raw = val_str.trim();
                let has_v = pattern_opt.is_some_and(|p| p.contains('V') || p.contains('v'));
                if has_v && (raw.contains('.') || (!dec_sep.is_empty() && raw.contains(dec_sep))) {
                    let msg = alloc::format!(
                        "Parse Error: Unable to parse xs:decimal from text: '{}': explicit decimal separator not permitted with virtual decimal point pattern",
                        raw
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                let (norm, is_neg) = if base == 10 {
                    let num_pad_str = props
                        .text_number_pad_character
                        .as_deref()
                        .unwrap_or(&props.text_pad_char);
                    if let Some((neg, norm_str)) = normalize_text_number(
                        raw,
                        pattern_opt,
                        dec_sep,
                        grp_sep,
                        Some(num_pad_str),
                        props.text_trim_kind,
                    ) {
                        (norm_str, neg)
                    } else {
                        let msg = alloc::format!("Parse Error: xs:decimal: Unable to parse '{}'", raw);
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                } else {
                    let clean = if !grp_sep.is_empty() && raw.contains(grp_sep) {
                        raw.replace(grp_sep, "")
                    } else {
                        alloc::string::ToString::to_string(raw)
                    };
                    let norm = if !dec_sep.is_empty() && dec_sep != "." && clean.contains(dec_sep) {
                        clean.replace(dec_sep, ".")
                    } else {
                        clean
                    };
                    let is_neg = norm.starts_with('-');
                    (norm, is_neg)
                };

                let norm = if has_v && !norm.contains('.') {
                    let pat = pattern_opt.unwrap_or("");
                    let v_idx = pat.find(['V', 'v']).unwrap_or(0);
                    let after_v = &pat[v_idx.saturating_add(1)..];
                    let v_scale = after_v.chars().take_while(|c| *c == '0' || *c == '#').count();
                    if v_scale > 0 {
                        let digits = norm.trim_start_matches('-');
                        let is_negative = norm.starts_with('-');
                        let scaled = if digits.len() <= v_scale {
                            let mut s = alloc::string::String::from("0.");
                            for _ in 0..(v_scale.saturating_sub(digits.len())) {
                                s.push('0');
                            }
                            s.push_str(digits);
                            s
                        } else {
                            let split_pos = digits.len().saturating_sub(v_scale);
                            let mut s = alloc::string::String::with_capacity(digits.len().saturating_add(1));
                            s.push_str(&digits[..split_pos]);
                            s.push('.');
                            s.push_str(&digits[split_pos..]);
                            s
                        };
                        if is_negative {
                            alloc::format!("-{}", scaled)
                        } else {
                            scaled
                        }
                    } else {
                        norm
                    }
                } else {
                    norm
                };

                if check_policy == TextNumberCheckPolicy::Strict {
                    if let Some(pat) = pattern_opt {
                        let parts: Vec<&str> = pat.split(';').collect();
                        if let Some(first_part) = parts.first() {
                            let (_, _, pos_grp, _) = crate::kernel::parser::numbers::extract_pattern_affixes(first_part);
                            if let Some(grp_size) = pos_grp {
                                let (int_part, _) = if let Some(idx) = norm.find('.') {
                                    (&norm[..idx], Some(&norm[idx + 1..]))
                                } else {
                                    (norm.as_str(), None)
                                };
                                let int_clean = int_part.trim_start_matches('+').trim_start_matches('-');
                                if crate::kernel::parser::numbers::validate_and_clean_integer_grouping(int_clean, grp_sep, Some(grp_size)).is_none() {
                                    let msg = alloc::format!("Parse Error: xs:decimal: Invalid grouping in strict mode for '{}'", raw);
                                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                                }
                            } else if !grp_sep.is_empty() && raw.contains(grp_sep) {
                                let msg = alloc::format!("Parse Error: xs:decimal: Grouping separator '{}' present but grouping not defined in pattern '{}'", grp_sep, first_part);
                                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                            }
                        }
                    }
                }

                if !props.decimal_signed && is_neg {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::Parse,
                        "Parse Error: xs:decimal: negative decimal not allowed when dfdl:decimalSigned is 'no'",
                    ));
                }

                let stripped = norm
                    .strip_prefix('+')
                    .or_else(|| norm.strip_prefix('-'))
                    .unwrap_or(&norm);

                if stripped.is_empty() {
                    let msg = alloc::format!("Parse Error: xs:decimal: Unable to parse '{}'", raw);
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }

                if base != 10 {
                    let digits = stripped;
                    let dec_str = if let Ok(v) = u128::from_str_radix(digits, base) {
                        alloc::format!("{}", v)
                    } else {
                        convert_big_radix_to_dec(digits, base)?
                    };
                    if is_neg {
                        Ok(DfdlValue::Decimal(alloc::format!("-{}", dec_str)))
                    } else {
                        Ok(DfdlValue::Decimal(dec_str))
                    }
                } else {
                    let mut parts = stripped.split('.');
                    let int_part = parts.next().unwrap_or("");
                    let frac_part = parts.next();
                    let has_more_dots = parts.next().is_some();
                    let valid_digits = !has_more_dots
                        && (int_part.is_empty() || int_part.chars().all(|c| c.is_ascii_digit()))
                        && frac_part
                            .is_none_or(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
                        && (!int_part.is_empty() || frac_part.is_some_and(|f| !f.is_empty()));

                    if !valid_digits {
                        let msg = alloc::format!(
                            "Parse Error: Unable to parse value '{}' for xs:decimal or xs:integer",
                            raw
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }

                    // Integer types: an all-zero fraction (e.g. "5.000") is the same integer.
                    let zero_fraction = frac_part.is_none_or(|f| f.chars().all(|c| c == '0'));
                    if props.facets.fraction_digits == Some(0)
                        && (frac_part.is_some() || norm.contains('.'))
                        && !zero_fraction
                    {
                        let msg =
                            alloc::format!("Parse Error: xs:integer: Unable to parse '{}'", raw);
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }
                    let norm = if props.facets.fraction_digits == Some(0) && frac_part.is_some() {
                        let int_only = norm.split('.').next().unwrap_or("");
                        alloc::string::ToString::to_string(int_only)
                    } else {
                        norm
                    };

                    if props.facets.min_inclusive.as_deref() == Some("0") && (is_neg || norm.starts_with('-')) {
                        let msg = alloc::format!(
                            "Parse Error: nonNegativeInteger: Out of Range '{}'",
                            raw
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    }

                    let final_dec = if is_neg && !norm.starts_with('-') {
                        alloc::format!("-{}", norm)
                    } else {
                        norm
                    };
                    Ok(DfdlValue::Decimal(final_dec))
                }
            }
        }
    }
}
