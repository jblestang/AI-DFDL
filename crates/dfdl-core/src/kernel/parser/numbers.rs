//! Numeric parsing and conversion utilities for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::schema::ir::TextTrimKind;

pub(crate) fn sanitize_numeric_string(s: &str) -> String {
    use alloc::string::ToString;
    let mut clean = s.trim().to_string();
    if clean.contains(',') {
        clean = clean.replace(',', "");
    }
    if clean.starts_with('$')
        || clean.starts_with('#')
        || clean.starts_with('€')
        || clean.starts_with('£')
    {
        clean.remove(0);
        clean = clean.trim_start().to_string();
    }
    clean
}

pub(crate) fn parse_flexible_int_i64(s: &str) -> Option<i64> {
    let clean = sanitize_numeric_string(s);
    if clean.is_empty() {
        return None;
    }
    if clean.starts_with("0x") || clean.starts_with("0X") {
        return i64::from_str_radix(&clean[2..], 16).ok();
    }
    if clean.starts_with("-0x") || clean.starts_with("-0X") {
        return i64::from_str_radix(&clean[3..], 16).ok().map(|v| -v);
    }
    if let Some((int_part, dec_part)) = clean.split_once('.') {
        if dec_part.chars().all(|c| c == '0') {
            return int_part.parse::<i64>().ok();
        }
        return None;
    }
    clean.parse::<i64>().ok()
}

pub(crate) fn parse_flexible_uint_u64(s: &str) -> Option<u64> {
    let clean = sanitize_numeric_string(s);
    let s_ref = clean.strip_prefix('+').unwrap_or(&clean);
    if s_ref.is_empty() {
        return None;
    }
    if s_ref.starts_with("0x") || s_ref.starts_with("0X") {
        return u64::from_str_radix(&s_ref[2..], 16).ok();
    }
    if let Some((int_part, dec_part)) = s_ref.split_once('.') {
        if dec_part.chars().all(|c| c == '0') {
            return int_part.parse::<u64>().ok();
        }
        return None;
    }
    s_ref.parse::<u64>().ok()
}

pub(crate) fn parse_flexible_bool(s: &str) -> Option<bool> {
    let clean = s.trim();
    if clean.is_empty() {
        return None;
    }
    let lower = clean.to_lowercase();
    match lower.as_str() {
        "true" | "1" | "yes" | "y" | "t" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "f" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_flexible_f64(s: &str) -> Option<f64> {
    let clean = s.trim();
    match clean {
        "NaN" | "nan" | "NAN" => return Some(f64::NAN),
        "INF" | "inf" | "Infinity" | "infinity" | "+INF" | "+inf" | "+Infinity" | "+infinity" => {
            return Some(f64::INFINITY)
        }
        "-INF" | "-inf" | "-Infinity" | "-infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    parse_flexible_f64_with_props(s, ".", "", None, None, None, false)
}

pub(crate) fn parse_flexible_f64_with_props(
    s: &str,
    decimal_sep: &str,
    grouping_sep: &str,
    nan_rep: Option<&str>,
    inf_rep: Option<&str>,
    exp_rep: Option<&str>,
    ignore_case: bool,
) -> Option<f64> {
    use alloc::string::ToString;
    let clean = s.trim();
    if let Some(nr) = nan_rep {
        if !nr.is_empty() && (clean == nr || (ignore_case && clean.eq_ignore_ascii_case(nr))) {
            return Some(f64::NAN);
        }
    }
    if let Some(ir) = inf_rep {
        if !ir.is_empty() {
            if clean == ir
                || clean == alloc::format!("+{}", ir)
                || (ignore_case
                    && (clean.eq_ignore_ascii_case(ir)
                        || clean.eq_ignore_ascii_case(&alloc::format!("+{}", ir))))
            {
                return Some(f64::INFINITY);
            } else if clean == alloc::format!("-{}", ir)
                || (ignore_case && clean.eq_ignore_ascii_case(&alloc::format!("-{}", ir)))
            {
                return Some(f64::NEG_INFINITY);
            }
        }
    }
    let clean_str = sanitize_numeric_string(s);
    let s_ref = clean_str.strip_prefix('+').unwrap_or(&clean_str);
    if s_ref.is_empty() {
        return None;
    }
    let (is_neg, core) = if let Some(stripped) = s_ref.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s_ref)
    };
    let without_grp = if !grouping_sep.is_empty() && core.contains(grouping_sep) {
        core.replace(grouping_sep, "")
    } else {
        core.to_string()
    };
    if !decimal_sep.is_empty() && decimal_sep != "." && grouping_sep != "." && without_grp.contains('.') {
        return None;
    }
    let norm_dec = if !decimal_sep.is_empty() && decimal_sep != "." && without_grp.contains(decimal_sep) {
        without_grp.replace(decimal_sep, ".")
    } else {
        without_grp
    };
    let norm_exp = if let Some(exp) = exp_rep {
        if !exp.is_empty() && exp != "E" && exp != "e" {
            if ignore_case {
                let mut res = String::new();
                let lower_norm = norm_dec.to_ascii_lowercase();
                let lower_exp = exp.to_ascii_lowercase();
                let mut start = 0;
                while let Some(pos) = lower_norm[start..].find(&lower_exp) {
                    let actual_pos = start + pos;
                    res.push_str(&norm_dec[start..actual_pos]);
                    res.push('E');
                    start = actual_pos + lower_exp.len();
                }
                res.push_str(&norm_dec[start..]);
                res
            } else {
                norm_dec.replace(exp, "E")
            }
        } else if exp.is_empty() {
            // When textStandardExponentRep is empty string, the exponent is directly signaled by a '+' or '-'
            if let Some(pos) = norm_dec.char_indices().skip(1).find_map(|(idx, c)| {
                if (c == '+' || c == '-') && norm_dec[..idx].chars().last().is_some_and(|prev| prev.is_ascii_digit()) {
                    Some(idx)
                } else {
                    None
                }
            }) {
                let mut res = String::with_capacity(norm_dec.len() + 1);
                res.push_str(&norm_dec[..pos]);
                res.push('E');
                res.push_str(&norm_dec[pos..]);
                res
            } else {
                norm_dec
            }
        } else {
            norm_dec
        }
    } else {
        norm_dec
    };
    let val = norm_exp.parse::<f64>().ok()?;
    Some(if is_neg { -val } else { val })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternPadPos {
    BeforePrefix,
    AfterPrefix,
    BeforeSuffix,
    AfterSuffix,
}

/// Helper to find the index of the first unescaped '*' pad escape character.
fn find_unescaped_star(s: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut chars = s.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\'' {
            if chars.peek().map(|&(_, c)| c) == Some('\'') {
                chars.next();
            } else {
                in_quote = !in_quote;
            }
        } else if !in_quote && ch == '*' {
            return Some(idx);
        }
    }
    None
}

/// Unquotes an ICU pattern affix (prefix or suffix) per ICU DecimalFormat rules.
///
/// In ICU pattern syntax, single quotes (`'`) enclose literal characters.
/// Two consecutive single quotes (`''`) represent a single literal quote.
/// Any character inside `'...'` is treated as literal and unescaped.
pub(crate) fn unquote_icu_affix(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_quote = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                in_quote = !in_quote;
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Result of [`extract_pattern_affixes`]: `(prefix, suffix, (primary, secondary) grouping, pad)`.
pub(crate) type PatternAffixes<'a> =
    (&'a str, &'a str, Option<(usize, usize)>, Option<(char, PatternPadPos)>);

/// Extracts prefix, suffix, integer grouping size, and optional padding specification
/// from an ICU decimal subpattern according to DFDL §13.6 and ICU DecimalFormat syntax.
///
/// Literals inside quotes (`'...'`) and pad escapes (`*<pad_char>`) are properly skipped
/// when identifying number pattern digits (`#`, `0`, `@`).
pub(crate) fn extract_pattern_affixes(
    subpattern: &str,
) -> PatternAffixes<'_> {
    let mut first_digit_idx = None;
    let mut last_digit_idx = None;

    let mut in_quote = false;
    let mut chars = subpattern.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\'' {
            if chars.peek().map(|&(_, c)| c) == Some('\'') {
                chars.next();
            } else {
                in_quote = !in_quote;
            }
        } else if !in_quote {
            if ch == '*' {
                // Pad escape: skip the following pad character
                chars.next();
            } else if ch == '#' || ch == '0' || ch == '@' || ch == 'P' || ch == 'p' || ch == 'V' || ch == 'v' {
                if first_digit_idx.is_none() {
                    first_digit_idx = Some(idx);
                }
                last_digit_idx = Some(idx);
            }
        }
    }

    if let (Some(mut start), Some(mut end)) = (first_digit_idx, last_digit_idx) {
        if start > 0 && subpattern.get(..start).is_some_and(|s| s.ends_with('.') && !s.ends_with("'.'")) {
            start = start.saturating_sub(1);
        }
        if subpattern.get(end.saturating_add(1)..).is_some_and(|s| s.starts_with('.')) {
            end = end.saturating_add(1);
        }
        let mut prefix = subpattern.get(..start).unwrap_or("");
        let mut suffix = subpattern.get(end.saturating_add(1)..).unwrap_or("");
        let body = subpattern.get(start..=end).unwrap_or("");
        let grouping_size = match extract_dual_grouping_sizes(body) {
            (Some(p), Some(s)) => Some((p, s)),
            _ => None,
        };

        let mut pad = None;
        if let Some(star_pos) = find_unescaped_star(prefix) {
            if let Some(ch) = prefix[star_pos + 1..].chars().next() {
                let spec_len = 1 + ch.len_utf8();
                if star_pos == 0 {
                    pad = Some((ch, PatternPadPos::BeforePrefix));
                    prefix = &prefix[spec_len..];
                } else if star_pos + spec_len == prefix.len() {
                    pad = Some((ch, PatternPadPos::AfterPrefix));
                    prefix = &prefix[..star_pos];
                }
            }
        }
        if pad.is_none() {
            if let Some(star_pos) = find_unescaped_star(suffix) {
                if let Some(ch) = suffix[star_pos + 1..].chars().next() {
                    let spec_len = 1 + ch.len_utf8();
                    if star_pos == 0 {
                        pad = Some((ch, PatternPadPos::BeforeSuffix));
                        suffix = &suffix[spec_len..];
                    } else if star_pos + spec_len == suffix.len() {
                        pad = Some((ch, PatternPadPos::AfterSuffix));
                        suffix = &suffix[..star_pos];
                    }
                }
            }
        }

        (prefix, suffix, grouping_size, pad)
    } else {
        (subpattern, "", None, None)
    }
}

/// Strips pattern affixes and padding characters from a number string.
///
/// Order of stripping depends on [`PatternPadPos`]:
/// - BeforePrefix: pad characters stripped from head, then prefix stripped from head, then suffix from tail.
/// - AfterPrefix: prefix stripped from head, then pad characters from head, then suffix from tail.
/// - BeforeSuffix: suffix stripped from tail, then pad characters stripped from tail, then prefix from head.
/// - AfterSuffix: pad characters stripped from tail, then suffix from tail, then prefix from head.
pub(crate) fn strip_pattern_affixes_and_pad<'a>(
    input: &'a str,
    prefix: &str,
    suffix: &str,
    pad: Option<(char, PatternPadPos)>,
) -> Option<&'a str> {
    let pre_clean = unquote_icu_affix(prefix);
    let suf_clean = unquote_icu_affix(suffix);
    let mut s = input;

    match pad {
        Some((ch, PatternPadPos::BeforePrefix)) => {
            s = s.trim_start_matches(ch);
            if !pre_clean.is_empty() {
                s = s.strip_prefix(pre_clean.as_str())?;
            }
            if !suf_clean.is_empty() {
                s = s.strip_suffix(suf_clean.as_str())?;
            }
        }
        Some((ch, PatternPadPos::AfterPrefix)) => {
            if !pre_clean.is_empty() {
                s = s.strip_prefix(pre_clean.as_str())?;
            }
            s = s.trim_start_matches(ch);
            if !suf_clean.is_empty() {
                s = s.strip_suffix(suf_clean.as_str())?;
            }
        }
        Some((ch, PatternPadPos::BeforeSuffix)) => {
            if !suf_clean.is_empty() {
                s = s.strip_suffix(suf_clean.as_str())?;
            }
            s = s.trim_end_matches(ch);
            if !pre_clean.is_empty() {
                s = s.strip_prefix(pre_clean.as_str())?;
            }
        }
        Some((ch, PatternPadPos::AfterSuffix)) => {
            s = s.trim_end_matches(ch);
            if !suf_clean.is_empty() {
                s = s.strip_suffix(suf_clean.as_str())?;
            }
            if !pre_clean.is_empty() {
                s = s.strip_prefix(pre_clean.as_str())?;
            }
        }
        None => {
            if !pre_clean.is_empty() {
                s = s.strip_prefix(pre_clean.as_str())?;
            }
            if !suf_clean.is_empty() {
                s = s.strip_suffix(suf_clean.as_str())?;
            }
        }
    }
    Some(s)
}

pub(crate) fn normalize_text_number(
    s: &str,
    pattern: Option<&str>,
    decimal_sep: &str,
    grouping_sep: &str,
    pad_char: Option<&str>,
    trim_kind: TextTrimKind,
) -> Option<(bool, String)> {
    use alloc::string::ToString;
    let mut clean = s.trim();
    if clean.is_empty() {
        return None;
    }

    if trim_kind != TextTrimKind::None || pad_char.is_some() {
        if let Some(pad) = pad_char {
            if !pad.is_empty() {
                clean = match trim_kind {
                    TextTrimKind::Head => {
                        clean.trim_start_matches(|c: char| pad.contains(c)).trim()
                    }
                    TextTrimKind::Tail => {
                        clean.trim_end_matches(|c: char| pad.contains(c)).trim()
                    }
                    _ => clean.trim_matches(|c: char| pad.contains(c)).trim(),
                };
            }
        }
    }

    if clean.is_empty() {
        return None;
    }

    let (is_neg, body_str) = if let Some(pat) = pattern {
        let parts: Vec<&str> = pat.split(';').collect();
        let first_part = parts.first()?;
        let (pos_pre, pos_suf, _, pos_pad) = extract_pattern_affixes(first_part);

        if let Some(second_part) = parts.get(1) {
            let (neg_pre, neg_suf, _, neg_pad) = extract_pattern_affixes(second_part);
            // DFDL §13.6: A negative subpattern specifies only prefix and suffix; pad char is taken from positive subpattern.
            let effective_neg_pad = pos_pad.map(|(pos_ch, pos_pos)| {
                if let Some((_, neg_pos)) = neg_pad {
                    (pos_ch, neg_pos)
                } else {
                    (pos_ch, pos_pos)
                }
            });
            let neg_pre_clean = unquote_icu_affix(neg_pre);
            let neg_suf_clean = unquote_icu_affix(neg_suf);

            if let Some(inner) = strip_pattern_affixes_and_pad(clean, neg_pre, neg_suf, effective_neg_pad) {
                (true, inner)
            } else if clean.starts_with('-')
                && (neg_pre_clean.contains('(') || neg_suf_clean.contains(')'))
            {
                return None;
            } else if let Some(inner) = strip_pattern_affixes_and_pad(clean, pos_pre, pos_suf, pos_pad) {
                (false, inner)
            } else if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped)
            } else {
                (false, clean)
            }
        } else {
            let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped)
            } else if let Some(stripped) = clean.strip_prefix('+') {
                (false, stripped)
            } else if let Some(stripped) = clean.strip_suffix('-') {
                (true, stripped)
            } else if let Some(stripped) = clean.strip_suffix('+') {
                (false, stripped)
            } else {
                (false, clean)
            };
            let inner = strip_pattern_affixes_and_pad(rest, pos_pre, pos_suf, pos_pad).unwrap_or(rest);
            (is_negative, inner)
        }
    } else {
        let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
            (true, stripped)
        } else if let Some(stripped) = clean.strip_prefix('+') {
            (false, stripped)
        } else if let Some(stripped) = clean.strip_suffix('-') {
            (true, stripped)
        } else if let Some(stripped) = clean.strip_suffix('+') {
            (false, stripped)
        } else {
            (false, clean)
        };
        (is_negative, rest)
    };

    let mut body = body_str.trim();
    if let Some(pad) = pad_char {
        if !pad.is_empty() {
            body = body.trim_matches(|c: char| pad.contains(c)).trim();
        }
    }
    if body.is_empty() {
        return None;
    }

    if let Some(pat) = pattern {
        let has_vp = pat.contains(['V', 'v', 'P', 'p']);
        if has_vp {
            // DFDL §13.7.1: It is a parse error if the data contains an explicit decimal point
            // character when 'V' or 'P' is specified in textNumberPattern.
            if body.contains('.') || (!decimal_sep.is_empty() && body.contains(decimal_sep)) {
                return None;
            }
        }
    }

    let sanitized_grp = if !grouping_sep.is_empty() && body.contains(grouping_sep) {
        body.replace(grouping_sep, "")
    } else {
        body.to_string()
    };

    let sanitized_spaces = if let Some(pad) = pad_char {
        if !pad.is_empty() && trim_kind == TextTrimKind::None {
            sanitized_grp
                .chars()
                .filter(|c| !pad.contains(*c))
                .collect::<String>()
        } else {
            sanitized_grp
        }
    } else {
        sanitized_grp
    };

    if !decimal_sep.is_empty() && decimal_sep != "." && grouping_sep != "." && sanitized_spaces.contains('.') {
        return None;
    }
    let normalized_dec = if !decimal_sep.is_empty() && decimal_sep != "." && sanitized_spaces.contains(decimal_sep) {
        sanitized_spaces.replace(decimal_sep, ".")
    } else {
        sanitized_spaces
    };

    let scaled_dec = if let Some(pat) = pattern {
        if pat.contains(['V', 'v', 'P', 'p']) && !normalized_dec.contains('.') {
            apply_virtual_decimal_and_scaling(&normalized_dec, pat)
        } else {
            normalized_dec
        }
    } else {
        normalized_dec
    };

    Some((is_neg, scaled_dec))
}

/// Applies DFDL §13.7.1 virtual decimal point ('V' / 'v') and scaling factor ('P' / 'p')
/// to an unsigned digits string.
///
/// - 'V' or 'v': Virtual decimal point. The number of digit positions following 'V' defines
///   the scale factor (number of fraction digits).
/// - 'P' or 'p' at the start (left): Implied leading zeros between decimal point and digits ("0." + "0"*p + digits).
/// - 'P' or 'p' at the end (right): Implied trailing zeros between digits and decimal point (digits + "0"*p).
pub(crate) fn apply_virtual_decimal_and_scaling(digits: &str, pattern: &str) -> String {
    let first_part = pattern.split(';').next().unwrap_or(pattern);
    let (pos_pre, pos_suf, _, _) = extract_pattern_affixes(first_part);
    let body = first_part
        .strip_prefix(pos_pre)
        .unwrap_or(first_part)
        .strip_suffix(pos_suf)
        .unwrap_or(first_part);

    if let Some(v_idx) = body.find(['V', 'v']) {
        let after_v = body.get(v_idx.saturating_add(1)..).unwrap_or("");
        let v_scale = after_v.chars().take_while(|c| *c == '0' || *c == '#').count();
        if v_scale > 0 {
            if digits.len() <= v_scale {
                let mut s = alloc::string::String::from("0.");
                for _ in 0..(v_scale.saturating_sub(digits.len())) {
                    s.push('0');
                }
                s.push_str(digits);
                s
            } else {
                let split_pos = digits.len().saturating_sub(v_scale);
                let mut s = alloc::string::String::with_capacity(digits.len().saturating_add(1));
                s.push_str(digits.get(..split_pos).unwrap_or(""));
                s.push('.');
                s.push_str(digits.get(split_pos..).unwrap_or(""));
                s
            }
        } else {
            alloc::string::ToString::to_string(digits)
        }
    } else {
        let p_left = body.chars().take_while(|c| *c == 'P' || *c == 'p').count();
        let p_right = body.chars().rev().take_while(|c| *c == 'P' || *c == 'p').count();
        if p_left > 0 {
            let mut s = alloc::string::String::from("0.");
            for _ in 0..p_left {
                s.push('0');
            }
            s.push_str(digits);
            s
        } else if p_right > 0 {
            let mut s = alloc::string::String::with_capacity(digits.len().saturating_add(p_right));
            s.push_str(digits);
            for _ in 0..p_right {
                s.push('0');
            }
            s
        } else {
            alloc::string::ToString::to_string(digits)
        }
    }
}

/// Extracts (primary, secondary) grouping sizes from a pattern body.
/// Per DFDL §13.7.1, the secondary is the interval between the two rightmost commas.
/// If there is only one comma, secondary == primary.
pub(crate) fn extract_dual_grouping_sizes(body: &str) -> (Option<usize>, Option<usize>) {
    let int_body = if let Some(dot) = body.find('.') { &body[..dot] } else { body };
    let last_comma = match int_body.rfind(',') {
        Some(p) => p,
        None => return (None, None),
    };
    let primary = int_body.len().saturating_sub(last_comma).saturating_sub(1);
    if primary == 0 {
        return (None, None);
    }
    // Look for second-to-last comma in int_body[..last_comma]
    let secondary = if let Some(prev_comma) = int_body[..last_comma].rfind(',') {
        let sec = last_comma.saturating_sub(prev_comma).saturating_sub(1);
        if sec == 0 { Some(primary) } else { Some(sec) }
    } else {
        // Only one comma: secondary == primary
        Some(primary)
    };
    (Some(primary), secondary)
}

/// Validates and cleans an integer string given optional `(primary, secondary)` grouping sizes.
pub(crate) fn validate_and_clean_integer_grouping(
    int_part: &str,
    grouping_sep: &str,
    grouping_size: Option<(usize, usize)>,
) -> Option<String> {
    validate_and_clean_integer_grouping_dual(
        int_part,
        grouping_sep,
        grouping_size.map(|g| g.0),
        grouping_size.map(|g| g.1),
    )
}

/// Validates and cleans an integer string according to primary+secondary grouping sizes.
/// - `primary_size`: size of the rightmost (last) group
/// - `secondary_size`: size of all intermediate groups (if None, uses primary_size)
pub(crate) fn validate_and_clean_integer_grouping_dual(
    int_part: &str,
    grouping_sep: &str,
    primary_size: Option<usize>,
    secondary_size: Option<usize>,
) -> Option<String> {
    use alloc::string::ToString;
    let prim = primary_size.unwrap_or(3);
    if !grouping_sep.is_empty() && int_part.contains(grouping_sep) {
        let groups: Vec<&str> = int_part.split(grouping_sep).collect();
        if groups.len() < 2 {
            return None;
        }
        // Determine effective secondary (default = primary)
        let sec = secondary_size.unwrap_or(prim);
        // Last group must equal primary size
        let last_group = *groups.last()?;
        if last_group.len() != prim || !last_group.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // First group may be 1..=secondary size
        let first_grp = *groups.first()?;
        if first_grp.is_empty()
            || first_grp.len() > sec
            || !first_grp.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        // All intermediate groups must equal secondary size
        let n = groups.len();
        for g in groups.iter().take(n.saturating_sub(1)).skip(1) {
            if g.len() != sec || !g.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
        }
        Some(groups.join(""))
    } else {
        if !int_part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(int_part.to_string())
    }
}

/// True when a pattern's literal prefix or suffix contains whitespace, in which case outer
/// whitespace in the data is part of the pattern (e.g. `"    0000    "`) rather than stray padding.
fn affix_has_whitespace(prefix: &str, suffix: &str) -> bool {
    unquote_icu_affix(prefix)
        .chars()
        .chain(unquote_icu_affix(suffix).chars())
        .any(char::is_whitespace)
}

pub(crate) fn parse_strict_int_i64(
    s: &str,
    pattern: Option<&str>,
    decimal_sep: &str,
    grouping_sep: &str,
    pad_char: Option<&str>,
    trim_kind: TextTrimKind,
) -> Option<i64> {
    let had_outer_ws = s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace);
    let mut clean = s;
    if trim_kind != TextTrimKind::None {
        if let Some(pad) = pad_char {
            if !pad.is_empty() {
                clean = match trim_kind {
                    TextTrimKind::Head => clean.trim_start_matches(|c: char| pad.contains(c)),
                    TextTrimKind::Tail => clean.trim_end_matches(|c: char| pad.contains(c)),
                    _ => clean.trim_matches(|c: char| pad.contains(c)),
                };
            }
        }
    }
    if clean.is_empty() {
        return None;
    }
    if clean.starts_with("0x")
        || clean.starts_with("0X")
        || clean.starts_with("-0x")
        || clean.starts_with("-0X")
    {
        return None;
    }

    let (is_neg, body_str, grouping_size) = if let Some(pat) = pattern {
        let parts: Vec<&str> = pat.split(';').collect();
        let first_part = parts.first()?;
        let (pos_pre, pos_suf, pos_grp, pos_pad) = extract_pattern_affixes(first_part);
        if had_outer_ws
            && pos_pad.is_none()
            && !affix_has_whitespace(pos_pre, pos_suf)
            && (trim_kind == TextTrimKind::None || pad_char.is_none_or(|p| !p.contains(' ')))
        {
            return None;
        }
        if let Some(second_part) = parts.get(1) {
            let (neg_pre, neg_suf, _neg_grp, neg_pad) = extract_pattern_affixes(second_part);
            // DFDL §13.6: A negative subpattern specifies only prefix and suffix; pad char is taken from positive subpattern.
            let effective_neg_pad = pos_pad.map(|(pos_ch, pos_pos)| {
                if let Some((_, neg_pos)) = neg_pad {
                    (pos_ch, neg_pos)
                } else {
                    (pos_ch, pos_pos)
                }
            });
            let neg_pre_clean = unquote_icu_affix(neg_pre);
            let neg_suf_clean = unquote_icu_affix(neg_suf);

            if let Some(inner) = strip_pattern_affixes_and_pad(clean, neg_pre, neg_suf, effective_neg_pad) {
                (true, inner, pos_grp)
            } else if clean.starts_with('-')
                && (neg_pre_clean.contains('(') || neg_suf_clean.contains(')'))
            {
                return None;
            } else if let Some(inner) = strip_pattern_affixes_and_pad(clean, pos_pre, pos_suf, pos_pad) {
                (false, inner, pos_grp)
            } else if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped, pos_grp)
            } else {
                (false, clean, pos_grp)
            }
        } else {
            let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped)
            } else if let Some(stripped) = clean.strip_prefix('+') {
                (false, stripped)
            } else {
                (false, clean)
            };
            let inner = strip_pattern_affixes_and_pad(rest, pos_pre, pos_suf, pos_pad).unwrap_or(rest);
            (is_negative, inner, pos_grp)
        }
    } else {
        if had_outer_ws
            && (trim_kind == TextTrimKind::None || pad_char.is_none_or(|p| !p.contains(' ')))
        {
            return None;
        }
        let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
            (true, stripped)
        } else if let Some(stripped) = clean.strip_prefix('+') {
            (false, stripped)
        } else {
            (false, clean)
        };
        (is_negative, rest, None)
    };

    if body_str.starts_with(char::is_whitespace) || body_str.ends_with(char::is_whitespace) {
        return None;
    }

    let (int_part, dec_part) = if let Some(idx) = body_str.find(decimal_sep) {
        (
            &body_str[..idx],
            Some(&body_str[idx + decimal_sep.len()..]),
        )
    } else {
        (body_str, None)
    };

    let pat_has_decimal = pattern.is_some_and(|p| p.contains(decimal_sep) || p.contains('.'));
    if let Some(dec) = dec_part {
        if !pat_has_decimal || dec.chars().any(|c| c != '0') {
            return None;
        }
    }

    let clean_int = validate_and_clean_integer_grouping(int_part, grouping_sep, grouping_size)?;

    let abs_val = clean_int.parse::<u64>().ok()?;
    if is_neg {
        if abs_val > (i64::MAX as u64) + 1 {
            None
        } else if abs_val == (i64::MAX as u64) + 1 {
            Some(i64::MIN)
        } else {
            Some(-(abs_val as i64))
        }
    } else {
        if abs_val > i64::MAX as u64 {
            None
        } else {
            Some(abs_val as i64)
        }
    }
}

pub(crate) fn parse_strict_uint_u64(
    s: &str,
    pattern: Option<&str>,
    decimal_sep: &str,
    grouping_sep: &str,
    pad_char: Option<&str>,
    trim_kind: TextTrimKind,
) -> Option<u64> {
    let had_outer_ws = s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace);
    let mut clean = s;
    if trim_kind != TextTrimKind::None {
        if let Some(pad) = pad_char {
            if !pad.is_empty() {
                clean = match trim_kind {
                    TextTrimKind::Head => clean.trim_start_matches(|c: char| pad.contains(c)),
                    TextTrimKind::Tail => clean.trim_end_matches(|c: char| pad.contains(c)),
                    _ => clean.trim_matches(|c: char| pad.contains(c)),
                };
            }
        }
    }
    if had_outer_ws
        && pattern.is_none()
        && (trim_kind == TextTrimKind::None || pad_char.is_none_or(|p| !p.contains(' ')))
    {
        return None;
    }
    if clean.starts_with('-') {
        return None;
    }
    let val = parse_strict_int_i64(clean, pattern, decimal_sep, grouping_sep, pad_char, trim_kind)?;
    if val >= 0 {
        Some(val as u64)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_strict_f64(
    s: &str,
    pattern: Option<&str>,
    decimal_sep: &str,
    grouping_sep: &str,
    nan_rep: Option<&str>,
    inf_rep: Option<&str>,
    exp_rep: Option<&str>,
    ignore_case: bool,
    pad_char: Option<&str>,
    trim_kind: TextTrimKind,
) -> Option<f64> {
    use alloc::string::ToString;
    let had_outer_ws = s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace);
    let mut clean = s;
    if trim_kind != TextTrimKind::None {
        if let Some(pad) = pad_char {
            if !pad.is_empty() {
                clean = match trim_kind {
                    TextTrimKind::Head => clean.trim_start_matches(|c: char| pad.contains(c)),
                    TextTrimKind::Tail => clean.trim_end_matches(|c: char| pad.contains(c)),
                    _ => clean.trim_matches(|c: char| pad.contains(c)),
                };
            }
        }
    }
    if clean.is_empty() {
        return None;
    }
    let eff_nan = nan_rep.unwrap_or("NaN");
    if !eff_nan.is_empty() && (clean == eff_nan || (ignore_case && clean.eq_ignore_ascii_case(eff_nan))) {
        return Some(f64::NAN);
    }
    let eff_inf = inf_rep.unwrap_or("Infinity");
    if !eff_inf.is_empty() {
        if clean == eff_inf
            || clean == "INF"
            || clean == alloc::format!("+{}", eff_inf)
            || clean == "+INF"
            || (ignore_case
                && (clean.eq_ignore_ascii_case(eff_inf)
                    || clean.eq_ignore_ascii_case("INF")
                    || clean.eq_ignore_ascii_case(&alloc::format!("+{}", eff_inf))
                    || clean.eq_ignore_ascii_case("+INF")))
        {
            return Some(f64::INFINITY);
        } else if clean == alloc::format!("-{}", eff_inf)
            || clean == "-INF"
            || (ignore_case
                && (clean.eq_ignore_ascii_case(&alloc::format!("-{}", eff_inf))
                    || clean.eq_ignore_ascii_case("-INF")))
        {
            return Some(f64::NEG_INFINITY);
        }
    }

    let (is_neg, body_str, grouping_size) = if let Some(pat) = pattern {
        let parts: Vec<&str> = pat.split(';').collect();
        let first_part = parts.first()?;
        let (pos_pre, pos_suf, pos_grp, pos_pad) = extract_pattern_affixes(first_part);
        if had_outer_ws
            && pos_pad.is_none()
            && !affix_has_whitespace(pos_pre, pos_suf)
            && (trim_kind == TextTrimKind::None || pad_char.is_none_or(|p| !p.contains(' ')))
        {
            return None;
        }
        if let Some(second_part) = parts.get(1) {
            let (neg_pre, neg_suf, _neg_grp, neg_pad) = extract_pattern_affixes(second_part);
            // DFDL §13.6: A negative subpattern specifies only prefix and suffix; pad char is taken from positive subpattern.
            let effective_neg_pad = pos_pad.map(|(pos_ch, pos_pos)| {
                if let Some((_, neg_pos)) = neg_pad {
                    (pos_ch, neg_pos)
                } else {
                    (pos_ch, pos_pos)
                }
            });
            let neg_pre_clean = unquote_icu_affix(neg_pre);
            let neg_suf_clean = unquote_icu_affix(neg_suf);

            if let Some(inner) = strip_pattern_affixes_and_pad(clean, neg_pre, neg_suf, effective_neg_pad) {
                (true, inner, pos_grp)
            } else if clean.starts_with('-')
                && (neg_pre_clean.contains('(') || neg_suf_clean.contains(')'))
            {
                return None;
            } else if let Some(inner) = strip_pattern_affixes_and_pad(clean, pos_pre, pos_suf, pos_pad) {
                (false, inner, pos_grp)
            } else if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped, pos_grp)
            } else {
                (false, clean, pos_grp)
            }
        } else {
            let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
                (true, stripped)
            } else if let Some(stripped) = clean.strip_prefix('+') {
                (false, stripped)
            } else {
                (false, clean)
            };
            let inner = strip_pattern_affixes_and_pad(rest, pos_pre, pos_suf, pos_pad).unwrap_or(rest);
            (is_negative, inner, pos_grp)
        }
    } else {
        if had_outer_ws
            && (trim_kind == TextTrimKind::None || pad_char.is_none_or(|p| !p.contains(' ')))
        {
            return None;
        }
        let (is_negative, rest) = if let Some(stripped) = clean.strip_prefix('-') {
            (true, stripped)
        } else if let Some(stripped) = clean.strip_prefix('+') {
            (false, stripped)
        } else {
            (false, clean)
        };
        (is_negative, rest, None)
    };

    if body_str.starts_with(char::is_whitespace) || body_str.ends_with(char::is_whitespace) {
        return None;
    }

    let (int_part, frac_and_exp) = if let Some(idx) = body_str.find(decimal_sep) {
        (&body_str[..idx], Some(&body_str[idx + decimal_sep.len()..]))
    } else {
        (body_str, None)
    };

    let clean_int = validate_and_clean_integer_grouping(int_part, grouping_sep, grouping_size)?;

    let mut full_norm = clean_int;
    if let Some(frac_exp) = frac_and_exp {
        full_norm.push('.');
        let norm_exp = if let Some(exp) = exp_rep {
            if !exp.is_empty() && exp != "E" && exp != "e" {
                if ignore_case {
                    let mut res = String::new();
                    let lower_norm = frac_exp.to_ascii_lowercase();
                    let lower_exp = exp.to_ascii_lowercase();
                    let mut start = 0;
                    while let Some(pos) = lower_norm[start..].find(&lower_exp) {
                        let actual_pos = start + pos;
                        res.push_str(&frac_exp[start..actual_pos]);
                        res.push('E');
                        start = actual_pos + lower_exp.len();
                    }
                    res.push_str(&frac_exp[start..]);
                    res
                } else {
                    frac_exp.replace(exp, "E")
                }
            } else if exp.is_empty() {
                // When textStandardExponentRep is empty string, the exponent is directly signaled by a '+' or '-'
                if let Some(pos) = frac_exp.char_indices().find_map(|(idx, c)| {
                    if (c == '+' || c == '-') && frac_exp[..idx].chars().last().is_some_and(|prev| prev.is_ascii_digit()) {
                        Some(idx)
                    } else {
                        None
                    }
                }) {
                    let mut res = String::with_capacity(frac_exp.len() + 1);
                    res.push_str(&frac_exp[..pos]);
                    res.push('E');
                    res.push_str(&frac_exp[pos..]);
                    res
                } else {
                    frac_exp.to_string()
                }
            } else {
                frac_exp.to_string()
            }
        } else {
            frac_exp.to_string()
        };
        full_norm.push_str(&norm_exp);
    }

    let v = full_norm.parse::<f64>().ok()?;
    Some(if is_neg { -v } else { v })
}

pub(crate) fn convert_big_radix_to_dec(digits: &str, base: u32) -> DFDLResult<String> {
    let mut dec_digits: Vec<u8> = alloc::vec![0];
    for ch in digits.chars() {
        let d = ch.to_digit(base).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!("Invalid digit '{}' for base {}", ch, base),
            )
        })?;
        let mut carry = d;
        for place in &mut dec_digits {
            let prod = (*place as u32) * base + carry;
            *place = (prod % 10) as u8;
            carry = prod / 10;
        }
        while carry > 0 {
            dec_digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    dec_digits.reverse();
    let s: String = dec_digits.into_iter().map(|d| (b'0' + d) as char).collect();
    Ok(s)
}

/// Decodes an overpunched digit character according to the configured zoned sign style (§13.7.3).
///
/// Returns `Ok((digit, sign))` where `digit` is the decoded ASCII digit character `'0'..='9'`,
/// and `sign` is `Some(true)` if negative, `Some(false)` if positive, or `None` if unsigned.
fn decode_ebcdic_alternate_negative_digit(ch: char) -> Option<char> {
    match ch {
        '^' => Some('0'),
        '£' => Some('1'),
        '¥' => Some('2'),
        '·' => Some('3'),
        '©' => Some('4'),
        '§' => Some('5'),
        '¶' => Some('6'),
        '¼' => Some('7'),
        '½' => Some('8'),
        '¾' => Some('9'),
        _ => None,
    }
}

pub(crate) fn decode_zoned_overpunch_digit(
    ch: char,
    style: crate::schema::ir::TextZonedSignStyle,
    is_ebcdic: bool,
) -> DFDLResult<(char, Option<bool>)> {
    use crate::schema::ir::TextZonedSignStyle;
    match style {
        TextZonedSignStyle::AsciiStandard => {
            if is_ebcdic {
                if let Some(d) = decode_ebcdic_alternate_negative_digit(ch) {
                    return Ok((d, Some(true)));
                }
            }
            match ch {
                '0'..='9' => Ok((ch, Some(false))),
                '{' => Ok(('0', Some(false))),
                'A'..='I' => {
                    let offset = (ch as u32).saturating_sub('A' as u32);
                    let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                    Ok((d, Some(false)))
                }
                'p'..='y' if is_ebcdic => Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
                )),
                'p' => Ok(('0', Some(true))),
                'q'..='y' => {
                    let offset = (ch as u32).saturating_sub('q' as u32);
                    let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                    Ok((d, Some(true)))
                }
                '}' => Ok(('0', Some(true))),
                'J'..='R' => {
                    let offset = (ch as u32).saturating_sub('J' as u32);
                    let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                    Ok((d, Some(true)))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
                )),
            }
        }
        TextZonedSignStyle::AsciiTranslatedEbcdic => {
            if is_ebcdic {
                if let Some(d) = decode_ebcdic_alternate_negative_digit(ch) {
                    return Ok((d, Some(true)));
                }
                match ch {
                    '0'..='9' => Ok((ch, Some(false))),
                    '{' => Ok(('0', Some(false))),
                    'A'..='I' => {
                        let offset = (ch as u32).saturating_sub('A' as u32);
                        let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                        Ok((d, Some(false)))
                    }
                    '}' => Ok(('0', Some(true))),
                    'J'..='R' => {
                        let offset = (ch as u32).saturating_sub('J' as u32);
                        let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                        Ok((d, Some(true)))
                    }
                    _ => Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
                    )),
                }
            } else {
                match ch {
                    '0'..='9' => Ok((ch, Some(false))),
                    '{' => Ok(('0', Some(false))),
                    'A'..='I' => {
                        let offset = (ch as u32).saturating_sub('A' as u32);
                        let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                        Ok((d, Some(false)))
                    }
                    '}' => Ok(('0', Some(true))),
                    'J'..='R' => {
                        let offset = (ch as u32).saturating_sub('J' as u32);
                        let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                        Ok((d, Some(true)))
                    }
                    _ => Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
                    )),
                }
            }
        }
        TextZonedSignStyle::AsciiCaRealiaModified => match ch {
            '0'..='9' => Ok((ch, Some(false))),
            ' ' => Ok(('0', Some(true))),
            '!'..=')' => {
                let offset = (ch as u32).saturating_sub('!' as u32);
                let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                Ok((d, Some(true)))
            }
            _ => Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
            )),
        },
        TextZonedSignStyle::AsciiTandemModified => match ch {
            '0'..='9' => Ok((ch, Some(false))),
            '\u{80}' => Ok(('0', Some(true))),
            '\u{81}'..='\u{89}' => {
                let offset = (ch as u32).saturating_sub(0x81);
                let d = core::char::from_u32(('1' as u32).saturating_add(offset)).unwrap_or('1');
                Ok((d, Some(true)))
            }
            _ => Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
            )),
        },
    }
}

/// Decodes a zoned numeric text representation according to DFDL §13.7.2 and §13.7.3.
///
/// Supports leading overpunch (+pattern), trailing overpunch (pattern+), unsigned zoned numbers,
/// and virtual decimal point positioning via 'V' or 'v' in `textNumberPattern`.
pub(crate) fn parse_zoned_number(
    input: &str,
    pattern_opt: Option<&str>,
    style: crate::schema::ir::TextZonedSignStyle,
    is_ebcdic: bool,
    decimal_signed: bool,
) -> DFDLResult<String> {
    if let Some(pat) = pattern_opt {
        if pat.contains(';') {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!(
                    "Parse Error: textNumberPattern for zoned number cannot contain multiple subpatterns: '{}'",
                    pat
                ),
            ));
        }
    }

    let pat = pattern_opt.unwrap_or("+0");
    let (pat_before_v, pat_after_v) = if let Some(idx) = pat.find(['V', 'v']) {
        (&pat[..idx], Some(&pat[idx.saturating_add(1)..]))
    } else {
        (pat, None)
    };

    if let Some(after_v) = pat_after_v {
        let after_clean = after_v.trim_end_matches('+');
        if after_clean.contains('#') {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!(
                    "Parse Error: textNumberPattern for zoned format cannot contain '#' after 'V': '{}'",
                    pat
                ),
            ));
        }
    }

    let is_leading = pat_before_v.starts_with('+');
    let is_trailing = if let Some(after_v) = pat_after_v {
        after_v.ends_with('+')
    } else {
        pat_before_v.ends_with('+')
    };


    let clean = input.trim();
    if clean.is_empty() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: Unable to parse empty string for zoned decimal",
        ));
    }

    let char_count = clean.chars().count();
    let overpunch_idx = if is_leading {
        Some(0usize)
    } else if is_trailing {
        Some(char_count.saturating_sub(1))
    } else {
        None
    };

    let mut digits = String::with_capacity(clean.len().saturating_add(2));
    let mut is_negative = false;

    for (idx, ch) in clean.chars().enumerate() {
        if Some(idx) == overpunch_idx {
            let (d, sign) = decode_zoned_overpunch_digit(ch, style, is_ebcdic)?;
            digits.push(d);
            if let Some(neg) = sign {
                if neg {
                    is_negative = true;
                }
            }
        } else {
            if !ch.is_ascii_digit() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Invalid zoned digit: {}", ch),
                ));
            }
            digits.push(ch);
        }
    }

    if is_negative && !decimal_signed {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: negative zoned decimal not allowed when dfdl:decimalSigned is 'no'",
        ));
    }

    let num_str = if pat.contains(['V', 'v', 'P', 'p']) {
        apply_virtual_decimal_and_scaling(&digits, pat)
    } else {
        digits
    };

    if is_negative {
        let all_zeros = num_str.chars().all(|c| c == '0' || c == '.');
        if all_zeros {
            Ok(num_str)
        } else {
            Ok(alloc::format!("-{}", num_str))
        }
    } else {
        Ok(num_str)
    }
}

/// Returns true if a numeric DfdlValue represents 0.
pub(crate) fn is_numeric_zero(val: &crate::infoset::DfdlValue) -> bool {
    use crate::infoset::DfdlValue;
    match val {
        DfdlValue::Float(f) => *f == 0.0,
        DfdlValue::Double(d) => *d == 0.0,
        DfdlValue::Decimal(s) | DfdlValue::String(s) => {
            s.trim().parse::<f64>().map(|f| f == 0.0).unwrap_or(false)
        }
        DfdlValue::Byte(b) => *b == 0,
        DfdlValue::Short(s) => *s == 0,
        DfdlValue::Int(i) => *i == 0,
        DfdlValue::Long(l) => *l == 0,
        DfdlValue::UnsignedByte(b) => *b == 0,
        DfdlValue::UnsignedShort(s) => *s == 0,
        DfdlValue::UnsignedInt(i) => *i == 0,
        DfdlValue::UnsignedLong(l) => *l == 0,
        _ => false,
    }
}

/// Formats a numeric `DfdlValue` according to DFDL §13.7 text number formatting rules,
/// ICU DecimalFormat pattern, custom decimal and grouping separators, and exponent representations.
pub(crate) fn format_text_number(
    val: &crate::infoset::DfdlValue,
    pattern: Option<&str>,
    decimal_sep: &str,
    grouping_sep: &str,
    exponent_rep: Option<&str>,
    rounding: super::rounding::NumberRounding<'_>,
) -> String {
    use super::rounding::{expand_exponent, pattern_increment, round_fraction, round_to_increment};
    use alloc::string::ToString;
    let pat = match pattern {
        Some(p) if !p.is_empty() => p,
        _ => {
            let raw = alloc::format!("{}", val);
            if !decimal_sep.is_empty() && decimal_sep != "." && raw.contains('.') {
                return raw.replace('.', decimal_sep);
            }
            return raw;
        }
    };

    let raw_val_str = expand_exponent(&alloc::format!("{}", val));
    let (is_negative, unsigned_str) = if let Some(stripped) = raw_val_str.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, raw_val_str.as_str())
    };

    let (mut int_part, mut frac_part) = match unsigned_str.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (unsigned_str.to_string(), String::new()),
    };

    // Subpattern selection
    let subpatterns: Vec<&str> = pat.split(';').collect();
    let (use_subpattern, is_explicit_neg) = if is_negative && subpatterns.len() > 1 {
        (subpatterns.get(1).copied().unwrap_or(pat), true)
    } else {
        (subpatterns.first().copied().unwrap_or(pat), false)
    };

    let (raw_pre, raw_suf, _grp, _pad) = extract_pattern_affixes(use_subpattern);
    let prefix = unquote_icu_affix(raw_pre);
    let suffix = unquote_icu_affix(raw_suf);
    let body = if use_subpattern.len() >= raw_pre.len().saturating_add(raw_suf.len()) {
        &use_subpattern[raw_pre.len()..use_subpattern.len().saturating_sub(raw_suf.len())]
    } else {
        use_subpattern
    };

    let formatted_body = if let Some(e_idx) = body.find('E') {
        let mantissa_pat = &body[..e_idx];
        let exp_pat = &body[e_idx.saturating_add(1)..];
        let exp_rep_str = exponent_rep.unwrap_or("E");
        let min_exp_digits = exp_pat.chars().filter(|c| *c == '0').count().max(1);

        let (mantissa_int_pat, mantissa_frac_pat) = match mantissa_pat.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (mantissa_pat, None),
        };
        let min_int = if mantissa_int_pat.starts_with('#') {
            1
        } else {
            mantissa_int_pat.chars().filter(|c| *c == '0').count().max(1)
        };
        let max_int = mantissa_int_pat.chars().filter(|c| *c == '0' || *c == '#').count().max(min_int);
        let min_frac = mantissa_frac_pat.map(|f| f.chars().filter(|c| *c == '0').count()).unwrap_or(0);
        let max_frac = mantissa_frac_pat.map(|f| f.chars().filter(|c| *c == '0' || *c == '#').count()).unwrap_or(0);

        // Check if value is 0
        if int_part.chars().all(|c| c == '0') && frac_part.chars().all(|c| c == '0') {
            let frac_str = if min_frac > 0 {
                let mut f = String::with_capacity(min_frac.saturating_add(decimal_sep.len()));
                f.push_str(decimal_sep);
                for _ in 0..min_frac {
                    f.push('0');
                }
                f
            } else {
                String::new()
            };
            let mut exp_str = String::with_capacity(min_exp_digits.saturating_add(exp_rep_str.len()));
            exp_str.push_str(exp_rep_str);
            for _ in 0..min_exp_digits {
                exp_str.push('0');
            }
            alloc::format!("0{}{}", frac_str, exp_str)
        } else {
            let all_digits = alloc::format!("{}{}", int_part, frac_part);
            let lead = all_digits.bytes().take_while(|b| *b == b'0').count();
            let sig = all_digits.get(lead..).unwrap_or("");

            // Decimal exponent of the first significant digit (negative for values < 1).
            let raw_exp = int_part.len() as i64 - lead as i64 - 1;
            let step = max_int as i64;
            let rem = raw_exp.rem_euclid(step);
            let mut effective_exp = raw_exp - rem;
            let num_int_digits = (1 + rem) as usize;

            let mut m_int: String = sig.chars().take(num_int_digits).collect();
            while m_int.len() < num_int_digits {
                m_int.push('0');
            }
            let mut m_frac: String = sig.chars().skip(num_int_digits).collect();

            if let Some((i, f)) = rounding
                .increment
                .and_then(|inc| round_to_increment(&m_int, &m_frac, inc, rounding.mode, is_negative))
            {
                m_int = i;
                m_frac = f;
            }
            round_fraction(&mut m_int, &mut m_frac, max_frac, rounding.mode, is_negative);
            // Rounding carried past the mantissa width (e.g. 9.6 -> 10): renormalise.
            if m_int.len() > max_int {
                effective_exp += step;
                m_int = String::from("1");
                m_frac.clear();
            }

            let mut rounded_frac = m_frac;
            while rounded_frac.len() > min_frac && rounded_frac.ends_with('0') {
                rounded_frac.pop();
            }
            while rounded_frac.len() < min_frac {
                rounded_frac.push('0');
            }

            let final_frac_str = if !rounded_frac.is_empty() {
                alloc::format!("{}{}", decimal_sep, rounded_frac)
            } else {
                String::new()
            };

            let exp_sign = if effective_exp < 0 { "-" } else { "" };
            let exp_val_abs = effective_exp.abs();
            let exp_str = alloc::format!("{}{}{:0>width$}", exp_rep_str, exp_sign, exp_val_abs, width = min_exp_digits);

            alloc::format!("{}{}{}", m_int, final_frac_str, exp_str)
        }
    } else {
        let (int_pat, frac_pat) = match body.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (body, None),
        };
        let min_int = int_pat.chars().filter(|c| *c == '0').count().max(1);
        // Digits 1-9 in the fraction pattern are mandatory digits too (ICU), e.g. `0.020`.
        let is_frac_digit = |c: &char| c.is_ascii_digit();
        let min_frac = frac_pat.map(|f| f.chars().filter(is_frac_digit).count()).unwrap_or(0);
        let max_frac = frac_pat
            .map(|f| f.chars().filter(|c| *c == '#' || c.is_ascii_digit()).count())
            .unwrap_or(0);

        let increment = rounding
            .increment
            .map(String::from)
            .or_else(|| pattern_increment(int_pat, frac_pat.unwrap_or("")));
        if let Some((i, f)) = increment
            .as_deref()
            .and_then(|inc| round_to_increment(&int_part, &frac_part, inc, rounding.mode, is_negative))
        {
            int_part = i;
            frac_part = f;
        }
        round_fraction(&mut int_part, &mut frac_part, max_frac, rounding.mode, is_negative);
        if max_frac > 0 {
            while frac_part.len() > min_frac && frac_part.ends_with('0') {
                frac_part.pop();
            }
            while frac_part.len() < min_frac {
                frac_part.push('0');
            }
        } else {
            frac_part.clear();
        }

        while int_part.len() < min_int {
            int_part.insert(0, '0');
        }

        let grouped_int = if int_pat.contains(',') && !grouping_sep.is_empty() {
            let (prim_opt, sec_opt) = extract_dual_grouping_sizes(int_pat);
            let prim = prim_opt.unwrap_or(3);
            let sec = sec_opt.unwrap_or(prim);

            let chars: Vec<char> = int_part.chars().collect();
            let len = chars.len();
            if len > prim {
                let split_idx = len.saturating_sub(prim);
                let left = chars.get(..split_idx).unwrap_or(&[]);
                let right = chars.get(split_idx..).unwrap_or(&[]);

                let left_len = left.len();
                let mut left_chunks = Vec::new();
                let mut curr_end = left_len;
                while curr_end > 0 {
                    let start = curr_end.saturating_sub(sec);
                    let chunk: String = left.get(start..curr_end).unwrap_or(&[]).iter().collect();
                    left_chunks.push(chunk);
                    curr_end = start;
                }
                left_chunks.reverse();
                let mut out = left_chunks.join(grouping_sep);
                out.push_str(grouping_sep);
                let right_str: String = right.iter().collect();
                out.push_str(&right_str);
                out
            } else {
                int_part
            }
        } else {
            int_part
        };

        if !frac_part.is_empty() {
            alloc::format!("{}{}{}", grouped_int, decimal_sep, frac_part)
        } else if frac_pat == Some("") {
            alloc::format!("{}{}", grouped_int, decimal_sep)
        } else {
            grouped_int
        }
    };

    let neg_sign = if is_negative && !is_explicit_neg { "-" } else { "" };
    alloc::format!("{}{}{}{}", neg_sign, prefix, formatted_body, suffix)
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    /// Pattern `#,##,###,####` yields primary 4 and secondary 3.
    #[test]
    fn dual_sizes_extracted() {
        assert_eq!(extract_dual_grouping_sizes("#,##,###,####"), (Some(4), Some(3)));
        assert_eq!(extract_dual_grouping_sizes("#,###.00"), (Some(3), Some(3)));
        assert_eq!(extract_dual_grouping_sizes("####"), (None, None));
    }

    /// Valid and invalid inputs for primary=4, secondary=3.
    #[test]
    fn dual_grouping_validation() {
        let g = Some((4, 3));
        assert_eq!(
            validate_and_clean_integer_grouping("123,123,1234", ",", g).as_deref(),
            Some("1231231234")
        );
        assert!(validate_and_clean_integer_grouping("123,1234,1234", ",", g).is_none());
        assert!(validate_and_clean_integer_grouping("1234,123,1234", ",", g).is_none());
    }

    /// Whitespace literals in the pattern's affixes consume the data's outer whitespace.
    #[test]
    fn strict_parse_accepts_whitespace_affixes() {
        let pat = Some("    0000    ");
        let t = TextTrimKind::None;
        assert_eq!(parse_strict_int_i64("    0052    ", pat, ".", ",", None, t), Some(52));
        assert_eq!(parse_strict_int_i64("    52    ", pat, ".", ",", None, t), Some(52));
        assert_eq!(parse_strict_uint_u64("    0052    ", pat, ".", ",", None, t), Some(52));
        let f = parse_strict_f64("    0052    ", pat, ".", ",", None, None, None, false, None, t);
        assert_eq!(f, Some(52.0));
        // Without whitespace in the pattern affixes, stray outer whitespace is still rejected.
        assert_eq!(parse_strict_int_i64(" 52 ", Some("0000"), ".", ",", None, t), None);
    }

    /// Verifies that a pattern ending with a decimal point outputs the custom decimal separator.
    #[test]
    fn test_format_text_number_decimal_separator_always_shown() {
        use crate::infoset::DfdlValue;
        let val = DfdlValue::Int(5);
        let res = format_text_number(&val, Some("'$'#0."), "^", ",", None, Default::default());
        assert_eq!(res, "$5^");

        let val_dec = DfdlValue::Decimal(alloc::string::String::from("5"));
        let res_dec = format_text_number(&val_dec, Some("'$'#0."), "^", ",", None, Default::default());
        assert_eq!(res_dec, "$5^");
    }

    fn fmt(
        v: &str,
        pat: &str,
        mode: crate::schema::ir::TextNumberRoundingMode,
        inc: Option<&str>,
    ) -> alloc::string::String {
        let val = crate::infoset::DfdlValue::Decimal(alloc::string::String::from(v));
        let r = crate::kernel::parser::rounding::NumberRounding { mode, increment: inc };
        format_text_number(&val, Some(pat), ".", ",", None, r)
    }

    /// Default ICU behaviour is half-even rounding, never truncation (DFDL §13.7.1.4).
    #[test]
    fn test_format_text_number_rounds_instead_of_truncating() {
        use crate::schema::ir::TextNumberRoundingMode::*;
        assert_eq!(fmt("0.129", "0.00", RoundHalfEven, None), "0.13");
        assert_eq!(fmt("0.125", "0.00", RoundHalfEven, None), "0.12");
        assert_eq!(fmt("10.9", "####", RoundHalfEven, None), "11");
        assert_eq!(fmt("0.129", "0.00", RoundDown, None), "0.12");
        assert_eq!(fmt("9.999", "0.00", RoundHalfEven, None), "10.00");
    }

    /// Explicit and pattern-implied rounding increments (e.g. `0.020` => step 0.02).
    #[test]
    fn test_format_text_number_rounding_increment() {
        use crate::schema::ir::TextNumberRoundingMode::*;
        assert_eq!(fmt("0.128", "#,##0.020", RoundHalfEven, Some("0.02")), "0.120");
        assert_eq!(fmt("0.128", "#,##0.020", RoundHalfEven, None), "0.120");
        assert_eq!(fmt("1235.5", "##00", RoundHalfEven, Some("1")), "1236");
    }

    /// Scientific patterns accept E-notation input, small magnitudes and round the mantissa.
    #[test]
    fn test_format_text_number_scientific_rounding() {
        use crate::schema::ir::TextNumberRoundingMode::*;
        assert_eq!(fmt("8.6E-200", "0.0#E+000", RoundHalfEven, Some("1")), "9.0E-200");
        assert_eq!(fmt("0.00123", "0.00E0", RoundHalfEven, None), "1.23E-3");
        assert_eq!(fmt("9.996", "0.00E0", RoundHalfEven, None), "1.00E1");
    }

    /// Verifies that numbers with unauthorized '.' are rejected when textStandardDecimalSeparator is not '.'.
    #[test]
    fn test_reject_unauthorized_dot_when_decimal_sep_is_non_dot() {
        let t = TextTrimKind::None;
        // Colon is decimal separator; '5.00' contains unauthorized dot and must be rejected.
        assert!(normalize_text_number("5.00", Some("0.00"), ":", ",", None, t).is_none());
        assert!(parse_flexible_f64_with_props("5.00", ":", ",", None, None, None, false).is_none());

        // '5:00' contains valid decimal separator and must succeed.
        assert_eq!(
            normalize_text_number("5:00", Some("0.00"), ":", ",", None, t),
            Some((false, alloc::string::String::from("5.00")))
        );
        assert_eq!(
            parse_flexible_f64_with_props("5:00", ":", ",", None, None, None, false),
            Some(5.0)
        );
    }

    #[test]
    fn test_apply_virtual_decimal_and_scaling() {
        // V virtual decimal
        assert_eq!(apply_virtual_decimal_and_scaling("123", "##0V00;-##0V00"), "1.23");
        assert_eq!(apply_virtual_decimal_and_scaling("5", "##0V00"), "0.05");
        assert_eq!(apply_virtual_decimal_and_scaling("999999999", "######0V00"), "9999999.99");

        // P on left: implied zeros after decimal point
        assert_eq!(apply_virtual_decimal_and_scaling("123", "PP000;-PP000"), "0.00123");

        // P on right: implied trailing zeros
        assert_eq!(apply_virtual_decimal_and_scaling("123", "##0PP+;##0PP-"), "12300");

        // Affixes with P on right
        let (pos_pre, pos_suf, _, _) = extract_pattern_affixes("##0PP+");
        assert_eq!(pos_pre, "");
        assert_eq!(pos_suf, "+");

        let (neg_pre, neg_suf, _, _) = extract_pattern_affixes("##0PP-");
        assert_eq!(neg_pre, "");
        assert_eq!(neg_suf, "-");

        // Normalization of 123- with ##0PP+;##0PP-
        let t = TextTrimKind::None;
        assert_eq!(
            normalize_text_number("123-", Some("##0PP+;##0PP-"), ".", ",", None, t),
            Some((true, alloc::string::String::from("12300")))
        );

        // Explicit decimal point with V/P must be rejected
        assert!(normalize_text_number("1.23", Some("##0V00"), ".", ",", None, t).is_none());
        assert!(normalize_text_number("1.23", Some("PP000"), ".", ",", None, t).is_none());

        // Flexible i64 and u64 parsing with hex and floats (lines 38, 56, 59-62)
        assert_eq!(parse_flexible_int_i64("-0x10"), Some(-16));
        assert_eq!(parse_flexible_uint_u64("0x20"), Some(32));
        assert_eq!(parse_flexible_uint_u64("42.000"), Some(42));
        assert_eq!(parse_flexible_uint_u64("42.001"), None);

        // Flexible f64 with +INF, -INF, and positive sign (lines 114-115, 121, 131)
        assert_eq!(parse_flexible_f64_with_props("+INF", ".", ",", Some("INF"), None, None, false), Some(f64::INFINITY));
        assert_eq!(parse_flexible_f64_with_props("-INF", ".", ",", Some("INF"), None, None, false), Some(f64::NEG_INFINITY));
        assert_eq!(parse_flexible_f64_with_props("+42.5", ".", ",", None, None, None, false), Some(42.5));

        // Grouping separator removal and exponential notation normalization (lines 136, 164, 168-179)
        assert_eq!(parse_flexible_f64_with_props("1,234.56", ".", ",", None, None, None, false), Some(1234.56));
        assert_eq!(parse_flexible_f64_with_props("1.23+04", ".", ",", None, None, Some(""), false), Some(12300.0));
        assert_eq!(parse_flexible_f64_with_props("1.23e02", ".", ",", None, None, Some("e"), false), Some(123.0));
    }

    /// Tests additional number parsing branches including pattern pad positions,
    /// head and tail trimming, empty strings, and affix sign variations.
    ///
    /// Verifies that:
    /// 1. Pattern pad escapes in prefix and suffix positions (`AfterPrefix`, `BeforeSuffix`, `AfterSuffix`) are extracted and stripped.
    /// 2. `normalize_text_number` correctly applies `TextTrimKind::Head` and `TextTrimKind::Tail`.
    /// 3. Completely trimmed strings or empty inputs yield `None`.
    /// 4. Negative subpatterns with parentheses reject plain leading minus signs.
    /// 5. Trailing signs (`+` and `-`) are stripped correctly with and without positive patterns.
    #[test]
    fn test_numbers_parser_extended_coverage() {
        // 1. Pattern pad positions (lines 356-382)
        let (_, _, _, pad_after_pre) = extract_pattern_affixes("ABC*_##0");
        assert_eq!(pad_after_pre, Some(('_', PatternPadPos::AfterPrefix)));
        assert_eq!(
            normalize_text_number("ABC___42", Some("ABC*_##0"), ".", ",", None, TextTrimKind::None),
            Some((false, alloc::string::String::from("42")))
        );

        let (_, _, _, pad_before_suf) = extract_pattern_affixes("##0*_DEF");
        assert_eq!(pad_before_suf, Some(('_', PatternPadPos::BeforeSuffix)));
        assert_eq!(
            normalize_text_number("42___DEF", Some("##0*_DEF"), ".", ",", None, TextTrimKind::None),
            Some((false, alloc::string::String::from("42")))
        );

        let (_, _, _, pad_after_suf) = extract_pattern_affixes("##0DEF*_");
        assert_eq!(pad_after_suf, Some(('_', PatternPadPos::AfterSuffix)));
        assert_eq!(
            normalize_text_number("42DEF___", Some("##0DEF*_"), ".", ",", None, TextTrimKind::None),
            Some((false, alloc::string::String::from("42")))
        );

        // 2. Text trimming head and tail (lines 414, 417, 426)
        assert_eq!(
            normalize_text_number("**123", None, ".", ",", Some("*"), TextTrimKind::Head),
            Some((false, alloc::string::String::from("123")))
        );
        assert_eq!(
            normalize_text_number("123**", None, ".", ",", Some("*"), TextTrimKind::Tail),
            Some((false, alloc::string::String::from("123")))
        );
        assert_eq!(
            normalize_text_number("****", None, ".", ",", Some("*"), TextTrimKind::Head),
            None
        );

        // 3. Negative pattern with parentheses rejecting leading minus (line 452)
        assert_eq!(
            normalize_text_number("-123", Some("#,##0;(#,##0)"), ".", ",", None, TextTrimKind::None),
            None
        );

        // 4. Trailing sign variations (lines 465-468, 480-485)
        assert_eq!(
            normalize_text_number("123-", Some("#,##0"), ".", ",", None, TextTrimKind::None),
            Some((true, alloc::string::String::from("123")))
        );
        assert_eq!(
            normalize_text_number("123+", Some("#,##0"), ".", ",", None, TextTrimKind::None),
            Some((false, alloc::string::String::from("123")))
        );
        assert_eq!(
            normalize_text_number("456-", None, ".", ",", None, TextTrimKind::None),
            Some((true, alloc::string::String::from("456")))
        );
        assert_eq!(
            normalize_text_number("456+", None, ".", ",", None, TextTrimKind::None),
            Some((false, alloc::string::String::from("456")))
        );

        // 5. Empty inputs to flexible integer parsers (lines 53, 70)
        assert_eq!(parse_flexible_int_i64(""), None);
        assert_eq!(parse_flexible_uint_u64(""), None);

        // 6. parse_flexible_f64_with_props infinity representation, grouping separator, and exponent reps (lines 114-181)
        assert_eq!(parse_flexible_f64_with_props("+Infinity", ".", ",", Some("NaN"), Some("Infinity"), None, true), Some(f64::INFINITY));
        assert_eq!(parse_flexible_f64_with_props("-Infinity", ".", ",", Some("NaN"), Some("Infinity"), None, true), Some(f64::NEG_INFINITY));
        assert_eq!(parse_flexible_f64_with_props("1,234.5", ".", ",", None, None, None, false), Some(1234.5));
        assert_eq!(parse_flexible_f64_with_props("1.2x3", ".", ",", None, None, Some("x"), false), Some(1200.0));
        // empty exponent representation directly signaled by '+'
        assert_eq!(parse_flexible_f64_with_props("1.2+3", ".", ",", None, None, Some(""), false), Some(1200.0));
        assert_eq!(parse_flexible_f64_with_props("123", ".", ",", None, None, Some(""), false), Some(123.0));

        // 7. parse_strict_int_i64 head/tail trim and grouping rejection (lines 698-734)
        assert_eq!(parse_strict_int_i64("##42", Some("##0"), ".", ",", Some("#"), TextTrimKind::Head), Some(42));
        assert_eq!(parse_strict_int_i64("42##", Some("##0"), ".", ",", Some("#"), TextTrimKind::Tail), Some(42));
        // Pattern without grouping rejects input containing grouping separator with invalid group lengths
        assert_eq!(parse_strict_int_i64("1,234,56", Some("0000"), ".", ",", None, TextTrimKind::None), None);

        // 8. parse_flexible_bool empty input returns None (line 70)
        assert_eq!(parse_flexible_bool(""), None);
        assert_eq!(parse_flexible_bool("  "), None);

        // 9. All 4 PatternPadPos variants in extract_pattern_affixes & strip_pattern_affixes_and_pad (lines 300-382)
        // BeforePrefix: *#$#,##0
        let (p1, s1, _, pad1) = extract_pattern_affixes("*#$#,##0");
        assert_eq!(pad1, Some(('#', PatternPadPos::BeforePrefix)));
        assert_eq!(strip_pattern_affixes_and_pad("###$1234", p1, s1, pad1), Some("1234"));

        // AfterPrefix: $*##,##0
        let (p2, s2, _, pad2) = extract_pattern_affixes("$*##,##0");
        assert_eq!(pad2, Some(('#', PatternPadPos::AfterPrefix)));
        assert_eq!(strip_pattern_affixes_and_pad("$###1234", p2, s2, pad2), Some("1234"));

        // BeforeSuffix: #,##0*#%
        let (p3, s3, _, pad3) = extract_pattern_affixes("#,##0*#%");
        assert_eq!(pad3, Some(('#', PatternPadPos::BeforeSuffix)));
        assert_eq!(strip_pattern_affixes_and_pad("1234###%", p3, s3, pad3), Some("1234"));

        // AfterSuffix: #,##0%*#
        let (p4, s4, _, pad4) = extract_pattern_affixes("#,##0%*#");
        assert_eq!(pad4, Some(('#', PatternPadPos::AfterSuffix)));
        assert_eq!(strip_pattern_affixes_and_pad("1234%###", p4, s4, pad4), Some("1234"));

        // 10. parse_flexible_f64_with_props with non-standard decimal separator rejecting '.' in input (line 140)
        assert_eq!(parse_flexible_f64_with_props("12.34", ",", " ", None, None, None, false), None);

        // 11. extract_dual_grouping_sizes with trailing comma or primary == 0 (lines 623-625)
        assert_eq!(extract_dual_grouping_sizes("###,"), (None, None));
        assert_eq!(extract_dual_grouping_sizes("###"), (None, None));

        // 12. parse_strict_int_i64 empty clean string and 0x prefix rejection (lines 738-746)
        assert_eq!(parse_strict_int_i64("", None, ".", ",", None, TextTrimKind::None), None);
        assert_eq!(parse_strict_int_i64("0x123", None, ".", ",", None, TextTrimKind::None), None);
        assert_eq!(parse_strict_int_i64("-0x123", None, ".", ",", None, TextTrimKind::None), None);

        // 13. parse_strict_int_i64 without pattern accepts valid grouping and rejects non-digits (lines 660-692)
        assert_eq!(parse_strict_int_i64("1,234", None, ".", ",", None, TextTrimKind::None), Some(1234));
        assert_eq!(parse_strict_int_i64("12a34", None, ".", ",", None, TextTrimKind::None), None);

        // 14. parse_strict_int_i64 negative subpattern matching with prefix '-' (lines 781-785)
        assert_eq!(parse_strict_int_i64("-123", Some("+#,##0;[#,##0]"), ".", ",", None, TextTrimKind::None), Some(-123));
        assert_eq!(parse_strict_int_i64("+123", Some("+#,##0;[#,##0]"), ".", ",", None, TextTrimKind::None), Some(123));
        assert_eq!(parse_strict_int_i64("[123]", Some("+#,##0;[#,##0]"), ".", ",", None, TextTrimKind::None), Some(-123));
    }
}

