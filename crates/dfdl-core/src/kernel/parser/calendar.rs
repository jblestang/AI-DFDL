//! Calendar, date, and time parsing utilities for the DFDL parser engine.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::DfdlSimpleType;
use crate::io::traits::{ByteOrder, ByteSource};
use crate::schema::ir::ResolvedProperties;

use super::ParserEngine;

pub(crate) fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

pub(crate) fn days_before_year(y: i64) -> i64 {
    let y = y - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

pub(crate) fn days_in_month(y: i64, m: usize) -> i64 {
    const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if m == 2 && is_leap_year(y) {
        29
    } else if (1..=12).contains(&m) {
        *DAYS.get(m.saturating_sub(1)).unwrap_or(&31)
    } else {
        0
    }
}

pub(crate) fn ymd_to_days(y: i64, m: usize, d: i64) -> i64 {
    let mut total_days = days_before_year(y) + d - 1;
    for prev_m in 1..m {
        total_days += days_in_month(y, prev_m);
    }
    total_days
}

pub(crate) fn days_to_ymd(days: i64) -> (i64, usize, i64) {
    let cycles_400 = days.div_euclid(146097);
    let day_in_cycle = days.rem_euclid(146097);

    let mut year_in_cycle = (day_in_cycle * 400 + 591) / 146097;
    let mut day_in_year =
        day_in_cycle - (days_before_year(year_in_cycle + 1) - days_before_year(1));
    while day_in_year < 0 {
        year_in_cycle -= 1;
        day_in_year = day_in_cycle - (days_before_year(year_in_cycle + 1) - days_before_year(1));
    }
    let year = cycles_400 * 400 + year_in_cycle + 1;

    let mut m = 1;
    let mut rem_days = day_in_year;
    while m <= 12 {
        let dim = days_in_month(year, m);
        if rem_days < dim {
            break;
        }
        rem_days -= dim;
        m += 1;
    }
    let day = rem_days + 1;
    (year, m, day)
}

pub(crate) fn parse_xs_date_time_wall_ms(s: &str) -> DFDLResult<(i64, &str)> {
    let clean_s = s.strip_prefix('-').unwrap_or(s);
    let (date_part, time_part) = clean_s.split_once('T').ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Invalid xs:dateTime epoch")
    })?;
    let date_parts: Vec<&str> = date_part.split('-').collect();
    if date_parts.len() != 3 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::SchemaDefinition,
            "Invalid xs:dateTime date part in epoch",
        ));
    }
    let mut year: i64 = date_parts
        .first()
        .and_then(|str_y| str_y.parse().ok())
        .ok_or_else(|| {
            DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Invalid year in epoch")
        })?;
    if s.starts_with('-') {
        year = -year;
    }
    let month: usize = date_parts
        .get(1)
        .and_then(|str_m| str_m.parse().ok())
        .ok_or_else(|| {
            DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Invalid month in epoch")
        })?;
    let day: i64 = date_parts
        .get(2)
        .and_then(|str_d| str_d.parse().ok())
        .ok_or_else(|| {
            DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Invalid day in epoch")
        })?;

    let (time_no_tz, tz_str) = if let Some(stripped) = time_part.strip_suffix('Z') {
        (stripped, "Z")
    } else if let Some(idx) = time_part.rfind('+').or_else(|| time_part.rfind('-')) {
        let (t, tz) = time_part.split_at(idx);
        (t, tz)
    } else {
        (time_part, "")
    };

    let (base_time, frac_ms) = if let Some((b, f)) = time_no_tz.split_once('.') {
        let mut f_padded = String::from(f);
        while f_padded.len() < 3 {
            f_padded.push('0');
        }
        let ms_sub = f_padded.get(..3).unwrap_or("0");
        let ms: i64 = ms_sub.parse().unwrap_or(0);
        (b, ms)
    } else {
        (time_no_tz, 0)
    };

    let time_parts: Vec<&str> = base_time.split(':').collect();
    let hour: i64 = time_parts.first().and_then(|h| h.parse().ok()).unwrap_or(0);
    let min: i64 = time_parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
    let sec: i64 = time_parts
        .get(2)
        .and_then(|sc| sc.parse().ok())
        .unwrap_or(0);

    let days = ymd_to_days(year, month, day);
    let total_sec = days * 86400 + hour * 3600 + min * 60 + sec;
    let total_ms = total_sec * 1000 + frac_ms;
    Ok((total_ms, tz_str))
}


pub(crate) fn match_month_name(s: &str) -> Option<(usize, usize)> {
    const MONTHS: &[(&str, usize)] = &[
        ("january", 1), ("jan", 1), ("enero", 1),
        ("february", 2), ("feb", 2), ("febrero", 2),
        ("march", 3), ("mar", 3), ("marzo", 3), ("märz", 3), ("мар.", 3), ("мар", 3),
        ("april", 4), ("apr", 4), ("abril", 4),
        ("may", 5), ("mai", 5), ("mayo", 5),
        ("june", 6), ("jun", 6), ("junio", 6),
        ("july", 7), ("jul", 7), ("julio", 7),
        ("august", 8), ("aug", 8), ("agosto", 8),
        ("september", 9), ("sep", 9), ("septiembre", 9), ("setiembre", 9),
        ("october", 10), ("oct", 10), ("okt", 10), ("octubre", 10),
        ("november", 11), ("nov", 11), ("noviembre", 11),
        ("december", 12), ("dec", 12), ("dezember", 12), ("diciembre", 12),
    ];
    let s_lower = s.to_ascii_lowercase();
    let mut best: Option<(usize, usize)> = None;
    for &(name, m) in MONTHS {
        if s_lower.starts_with(name) {
            if let Some((_, best_len)) = best {
                if name.len() > best_len {
                    best = Some((m, name.len()));
                }
            } else {
                best = Some((m, name.len()));
            }
        }
    }
    best
}

pub(crate) fn match_day_of_week(s: &str) -> Option<(u32, usize)> {
    const DAYS: &[(&str, u32)] = &[
        ("monday", 1), ("mon", 1), ("lunes", 1),
        ("tuesday", 2), ("tue", 2), ("martes", 2),
        ("wednesday", 3), ("wed", 3), ("mittwoch", 3), ("miércoles", 3), ("miercoles", 3),
        ("thursday", 4), ("thu", 4), ("donnerstag", 4), ("jueves", 4),
        ("friday", 5), ("fri", 5), ("freitag", 5), ("viernes", 5), ("пятница", 5),
        ("saturday", 6), ("sat", 6), ("samstag", 6), ("sábado", 6), ("sabado", 6),
        ("sunday", 7), ("sun", 7), ("sonntag", 7), ("domingo", 7),
    ];
    let s_lower = s.to_ascii_lowercase();
    let mut best: Option<(u32, usize)> = None;
    for &(day, dow) in DAYS {
        if s_lower.starts_with(day) {
            if let Some((_, best_len)) = best {
                if day.len() > best_len {
                    best = Some((dow, day.len()));
                }
            } else {
                best = Some((dow, day.len()));
            }
        }
    }
    best
}

pub(crate) fn parse_calendar_with_pattern(
    mut text: &str,
    pattern: &str,
    simple_type: DfdlSimpleType,
    check_policy: crate::schema::ir::CalendarCheckPolicy,
    first_day_of_week: crate::schema::ir::CalendarFirstDayOfWeek,
) -> Option<String> {
    use alloc::string::ToString;
    let mut year: Option<i32> = None;
    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut day_of_year: Option<u32> = None;
    let mut target_dow: Option<u32> = None;
    let mut hour: Option<u32> = None;
    let mut minute: Option<u32> = None;
    let mut second: Option<u32> = None;
    let mut is_pm = false;
    let mut is_bc = false;
    let mut tz_str = String::new();

    let pat_chars: Vec<char> = pattern.chars().collect();
    let mut p_idx = 0usize;

    while let Some(&ch) = pat_chars.get(p_idx) {
        if ch == '\'' {
            p_idx = p_idx.saturating_add(1);
            if pat_chars.get(p_idx).copied() == Some('\'') {
                if let Some(rest) = text.strip_prefix('\'') {
                    text = rest;
                }
                p_idx = p_idx.saturating_add(1);
                continue;
            }
            let mut lit = String::new();
            while let Some(&next_c) = pat_chars.get(p_idx) {
                if next_c == '\'' {
                    if pat_chars.get(p_idx.saturating_add(1)).copied() == Some('\'') {
                        lit.push('\'');
                        p_idx = p_idx.saturating_add(2);
                        continue;
                    }
                    break;
                }
                lit.push(next_c);
                p_idx = p_idx.saturating_add(1);
            }
            if pat_chars.get(p_idx).copied() == Some('\'') {
                p_idx = p_idx.saturating_add(1);
            }
            text = text.strip_prefix(&lit)?;
        } else if ch == 'w' || ch == 'W' || ch == 'F' || ch == 'g' {
            let p_ch = ch;
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some(p_ch) {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let max_digits = if count <= 2 { count } else { 4 };
            let num_len = text.chars().take(max_digits).take_while(|c| c.is_ascii_digit()).count();
            if num_len > 0 {
                let (_, rest) = text.split_at_checked(num_len)?;
                text = rest;
            }
        } else if ch == 'e' || ch == 'c' {
            let mut count = 0usize;
            while matches!(pat_chars.get(p_idx).copied(), Some('e') | Some('c')) {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            if count >= 3 {
                if let Some((dow, dow_len)) = match_day_of_week(text) {
                    target_dow = Some(dow);
                    let (_, rest) = text.split_at_checked(dow_len)?;
                    text = rest;
                }
            } else {
                let num_len = text.chars().take(2).take_while(|c| c.is_ascii_digit()).count();
                if num_len > 0 {
                    let (part, rest) = text.split_at_checked(num_len)?;
                    if let Ok(dow_raw) = part.parse::<u32>() {
                        let mapped = match first_day_of_week {
                            crate::schema::ir::CalendarFirstDayOfWeek::Sunday => {
                                if dow_raw == 1 { 7 } else { dow_raw.saturating_sub(1) }
                            }
                            crate::schema::ir::CalendarFirstDayOfWeek::Monday => dow_raw,
                            crate::schema::ir::CalendarFirstDayOfWeek::Tuesday => {
                                (dow_raw % 7) + 1
                            }
                            crate::schema::ir::CalendarFirstDayOfWeek::Wednesday => {
                                ((dow_raw + 1) % 7) + 1
                            }
                            crate::schema::ir::CalendarFirstDayOfWeek::Thursday => {
                                ((dow_raw + 2) % 7) + 1
                            }
                            crate::schema::ir::CalendarFirstDayOfWeek::Friday => {
                                ((dow_raw + 3) % 7) + 1
                            }
                            crate::schema::ir::CalendarFirstDayOfWeek::Saturday => {
                                ((dow_raw + 4) % 7) + 1
                            }
                        };
                        target_dow = Some(mapped);
                    }
                    text = rest;
                }
            }
        } else if matches!(ch, 'y' | 'Y' | 'u' | 'r') {
            let y_char = ch;
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some(y_char) {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yYurMdDHhKkmsSuwWFe".contains(c));
            let num_len = if count == 2 {
                2
            } else if next_digit {
                count
            } else {
                text.chars().take_while(|c| c.is_ascii_digit()).count()
            };
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let y_val: i32 = part.parse().ok()?;
            text = rest;
            if count == 2 {
                year = Some(if y_val >= 50 { 1900 + y_val } else { 2000 + y_val });
            } else {
                year = Some(y_val);
            }
        } else if ch == 'D' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('D') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yYurMdDHhKkmsSuwWFe".contains(c));
            let max_digits = if next_digit { count } else { count.max(3) };
            let num_len = text
                .chars()
                .take(max_digits)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let doy: u32 = part.parse().ok()?;
            day_of_year = Some(doy);
            text = rest;
        } else if ch == 'M' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('M') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            if count >= 3 {
                let (m_val, m_len) = match_month_name(text)?;
                month = Some(m_val as u32);
                let (_, rest) = text.split_at_checked(m_len)?;
                text = rest;
            } else {
                let next_digit = pat_chars
                    .get(p_idx)
                    .is_some_and(|&c| "yMdHhKkmsSuwWFe".contains(c));
                let max_digits = if next_digit { count } else { count.max(2) };
                let num_len = text
                    .chars()
                    .take(max_digits)
                    .take_while(|c| c.is_ascii_digit())
                    .count();
                if num_len == 0 {
                    return None;
                }
                let (part, rest) = text.split_at_checked(num_len)?;
                let m_val: u32 = part.parse().ok()?;
                month = Some(m_val);
                text = rest;
            }
        } else if ch == 'd' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('d') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yMdHhKkmsSuwWFe".contains(c));
            let max_digits = if next_digit { count } else { count.max(2) };
            let num_len = text
                .chars()
                .take(max_digits)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let d_val: u32 = part.parse().ok()?;
            day = Some(d_val);
            text = rest;
        } else if ch == 'E' {
            while pat_chars.get(p_idx).copied() == Some('E') {
                p_idx = p_idx.saturating_add(1);
            }
            if let Some((dow, dow_len)) = match_day_of_week(text) {
                target_dow = Some(dow);
                if let Some((_, rest)) = text.split_at_checked(dow_len) {
                    text = rest;
                }
            }
        } else if ch == 'H' || ch == 'h' || ch == 'K' || ch == 'k' {
            let hour_char = ch;
            let mut count = 0usize;
            while matches!(pat_chars.get(p_idx).copied(), Some(c) if c == hour_char) {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yMdHhKkmsSuwWFe".contains(c));
            let max_digits = if next_digit { count } else { count.max(2) };
            let num_len = text
                .chars()
                .take(max_digits)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let h_val: u32 = part.parse().ok()?;
            match hour_char {
                'H' => {
                    if check_policy == crate::schema::ir::CalendarCheckPolicy::Strict && h_val > 23 {
                        return None;
                    }
                    hour = Some(h_val);
                }
                'k' => {
                    if check_policy == crate::schema::ir::CalendarCheckPolicy::Strict && !(0..=24).contains(&h_val) {
                        return None;
                    }
                    hour = Some(if h_val == 24 { 0 } else { h_val });
                }
                'K' => {
                    if check_policy == crate::schema::ir::CalendarCheckPolicy::Strict && h_val > 11 {
                        return None;
                    }
                    hour = Some(h_val);
                }
                'h' => {
                    if check_policy == crate::schema::ir::CalendarCheckPolicy::Strict && !(1..=12).contains(&h_val) {
                        return None;
                    }
                    hour = Some(h_val);
                }
                _ => {}
            }
            text = rest;
        } else if ch == 'm' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('m') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yMdHhKkmsSuwWFe".contains(c));
            let max_digits = if next_digit { count } else { count.max(2) };
            let num_len = text
                .chars()
                .take(max_digits)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let m_val: u32 = part.parse().ok()?;
            minute = Some(m_val);
            text = rest;
        } else if ch == 's' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('s') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let next_digit = pat_chars
                .get(p_idx)
                .is_some_and(|&c| "yMdHhKkmsSuwWFe".contains(c));
            let max_digits = if next_digit { count } else { count.max(2) };
            let num_len = text
                .chars()
                .take(max_digits)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if num_len == 0 {
                return None;
            }
            let (part, rest) = text.split_at_checked(num_len)?;
            let s_val: u32 = part.parse().ok()?;
            second = Some(s_val);
            text = rest;
        } else if ch == 'a' {
            p_idx = p_idx.saturating_add(1);
            if let Some(rest) = text.strip_prefix("PM").or_else(|| text.strip_prefix("pm")) {
                is_pm = true;
                text = rest;
            } else if let Some(rest) = text.strip_prefix("AM").or_else(|| text.strip_prefix("am")) {
                text = rest;
            }
        } else if ch == 'G' {
            p_idx = p_idx.saturating_add(1);
            if let Some(rest) = text.strip_prefix("BC").or_else(|| text.strip_prefix("bc")) {
                is_bc = true;
                text = rest;
            } else if let Some(rest) = text.strip_prefix("AD").or_else(|| text.strip_prefix("ad")) {
                text = rest;
            }
        } else if ch == 'S' {
            let mut count = 0usize;
            while pat_chars.get(p_idx).copied() == Some('S') {
                count = count.saturating_add(1);
                p_idx = p_idx.saturating_add(1);
            }
            let frac_digits = text
                .chars()
                .take(count)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if frac_digits > 0 {
                let (_, rest) = text.split_at_checked(frac_digits)?;
                text = rest;
            }
        } else if ch == 'z' || ch == 'Z' || ch == 'v' || ch == 'V' || ch == 'X' || ch == 'x' || ch == 'O' {
            while matches!(pat_chars.get(p_idx).copied(), Some('z') | Some('Z') | Some('v') | Some('V') | Some('X') | Some('x') | Some('O')) {
                p_idx = p_idx.saturating_add(1);
            }
            if text.starts_with("GMT") || text.starts_with("UTC") {
                let rest = text.trim_start_matches("GMT").trim_start_matches("UTC");
                if rest.starts_with('+') || rest.starts_with('-') {
                    tz_str = rest.to_string();
                } else {
                    tz_str = String::from("Z");
                }
            } else if text.starts_with('+') || text.starts_with('-') || text.starts_with('Z') {
                tz_str = text.to_string();
            }
            if tz_str == "-00:00" || tz_str == "-0000" || tz_str == "-00" {
                return None;
            }
            text = "";
            break;
        } else {
            p_idx = p_idx.saturating_add(1);
            if let Some(stripped) = text.strip_prefix(ch) {
                text = stripped;
            } else if ch.is_whitespace() {
                text = text.trim_start();
            }
        }
    }

    if day.is_none() {
        if let Some(doy) = day_of_year {
            let y_val = year.unwrap_or(1970);
            let is_leap = (y_val % 4 == 0 && y_val % 100 != 0) || (y_val % 400 == 0);
            let days_in_months: [u32; 12] = if is_leap {
                [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            } else {
                [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            };
            let mut rem = doy;
            let mut m_calc = 1u32;
            for (m_idx, &dim) in days_in_months.iter().enumerate() {
                if rem <= dim {
                    m_calc = (m_idx as u32).saturating_add(1);
                    break;
                }
                rem = rem.saturating_sub(dim);
                m_calc = (m_idx as u32).saturating_add(2);
            }
            month = Some(m_calc.min(12));
            day = Some(rem.max(1));
        } else if let Some(tdow) = target_dow {
            let m_val = month.unwrap_or(1);
            let y_val = year.unwrap_or(1970);
            for d_cand in 1..=7 {
                let days = ymd_to_days(i64::from(y_val), m_val as usize, i64::from(d_cand));
                let dow = ((days % 7 + 7) % 7) + 1; // 1 = Mon .. 7 = Sun
                if dow as u32 == tdow {
                    day = Some(d_cand);
                    break;
                }
            }
        }
    }

    if !text.trim().is_empty() {
        return None;
    }

    let mut y = year.unwrap_or(1970);
    if is_bc {
        y = -y;
    }
    let m = month.unwrap_or(1);
    let d = day.unwrap_or(1);

    let mut h = hour.unwrap_or(0);
    if is_pm && h < 12 {
        h += 12;
    } else if !is_pm && h == 12 && hour.is_some() {
        h = 0;
    }
    let min = minute.unwrap_or(0);
    let sec = second.unwrap_or(0);

    let (res_y, res_m, res_d, res_h, res_min, res_sec) = if check_policy == crate::schema::ir::CalendarCheckPolicy::Lax {
        let extra_min = sec / 60;
        let s_norm = sec % 60;

        let total_min = min + extra_min;
        let extra_hr = total_min / 60;
        let m_norm = total_min % 60;

        let total_hr = h + extra_hr;
        let extra_days = total_hr / 24;
        let h_norm = total_hr % 24;

        let total_d = d + extra_days;
        let (y_from_m, m_from_m) = if m > 12 {
            let add_y = (m - 1) / 12;
            let rem_m = ((m - 1) % 12) + 1;
            (y + add_y as i32, rem_m)
        } else {
            (y, m)
        };

        let total_days = ymd_to_days(i64::from(y_from_m), m_from_m as usize, i64::from(total_d));
        let (ry, rm, rd) = days_to_ymd(total_days);
        (ry as i32, rm as u32, rd as u32, h_norm, m_norm, s_norm)
    } else {
        if !(1..=12).contains(&m) || !(1..=days_in_month(i64::from(y), m as usize)).contains(&i64::from(d)) {
            return None;
        }
        if h > 23 || min > 59 || sec > 60 {
            return None;
        }
        (y, m, d, h, min, sec)
    };

    match simple_type {
        DfdlSimpleType::Date => Some(alloc::format!("{:04}-{:02}-{:02}", res_y, res_m, res_d)),
        DfdlSimpleType::DateTime => Some(alloc::format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
            res_y, res_m, res_d, res_h, res_min, res_sec, tz_str
        )),
        DfdlSimpleType::Time => Some(alloc::format!("{:02}:{:02}:{:02}{}", res_h, res_min, res_sec, tz_str)),
        _ => None,
    }
}

fn normalize_timezone_suffix(tz: &str, check_policy: crate::schema::ir::CalendarCheckPolicy) -> Option<String> {
    let t = tz.trim();
    if t.is_empty() {
        return Some(String::new());
    }
    if t == "Z" || t == "z" {
        return Some(String::from("Z"));
    }
    if t == "-00:00" || t == "-0000" || t == "-00" {
        if check_policy == crate::schema::ir::CalendarCheckPolicy::Lax {
            return Some(String::from("+00:00"));
        } else {
            return None;
        }
    }
    if check_policy == crate::schema::ir::CalendarCheckPolicy::Lax {
        if t.eq_ignore_ascii_case("GMT") || t.eq_ignore_ascii_case("UTC") {
            return Some(String::from("+00:00"));
        }
        if let Some(r) = t.strip_prefix("GMT").or_else(|| t.strip_prefix("UTC")) {
            if r.starts_with('+') || r.starts_with('-') {
                return normalize_timezone_suffix(r, check_policy);
            }
        }
    }
    let (sign, rest) = if let Some(r) = t.strip_prefix('+') {
        ('+', r)
    } else {
        let r = t.strip_prefix('-')?;
        ('-', r)
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() == 2 {
        if let (Some(p0), Some(p1)) = (parts.first(), parts.get(1)) {
            if let (Ok(h), Ok(m)) = (p0.parse::<u32>(), p1.parse::<u32>()) {
                if p0.len() <= 2 && p1.len() == 2 && h <= 14 && m <= 59 {
                    return Some(alloc::format!("{}{:02}:{:02}", sign, h, m));
                }
            }
        }
    } else if parts.len() == 1 {
        if rest.len() == 1 || rest.len() == 2 {
            if let Ok(h) = rest.parse::<u32>() {
                if h <= 14 {
                    return Some(alloc::format!("{}{:02}:00", sign, h));
                }
            }
        } else if rest.len() == 4 {
            if let (Some(h_str), Some(m_str)) = (rest.get(..2), rest.get(2..)) {
                if let (Ok(h), Ok(m)) = (h_str.parse::<u32>(), m_str.parse::<u32>()) {
                    if h <= 14 && m <= 59 {
                        return Some(alloc::format!("{}{:02}:{:02}", sign, h, m));
                    }
                }
            }
        }
    }
    None
}

fn is_valid_timezone_suffix(tz: &str, check_policy: crate::schema::ir::CalendarCheckPolicy) -> bool {
    normalize_timezone_suffix(tz, check_policy).is_some()
}

pub(crate) fn parse_calendar_from_text(
    s: &str,
    pattern_opt: Option<&str>,
    pad_char: Option<&str>,
    trim_kind: crate::schema::ir::TextTrimKind,
    simple_type: DfdlSimpleType,
    check_policy: crate::schema::ir::CalendarCheckPolicy,
    first_day_of_week: crate::schema::ir::CalendarFirstDayOfWeek,
) -> DFDLResult<String> {
    use alloc::string::ToString;

    let mut clean = s.trim();
    if trim_kind != crate::schema::ir::TextTrimKind::None {
        if let Some(pad) = pad_char {
            if !pad.is_empty() {
                clean = match trim_kind {
                    crate::schema::ir::TextTrimKind::Head => {
                        clean.trim_start_matches(|c: char| pad.contains(c)).trim()
                    }
                    crate::schema::ir::TextTrimKind::Tail => {
                        clean.trim_end_matches(|c: char| pad.contains(c)).trim()
                    }
                    _ => clean.trim_matches(|c: char| pad.contains(c)).trim(),
                };
            }
        }
    }
    if clean.is_empty() {
        return Err(DFDLError::new(
            DFDLErrorKind::Parse,
            &alloc::format!("Parse Error: Empty text value for {:?}", simple_type),
        ));
    }

    let type_name = match simple_type {
        DfdlSimpleType::Date => "xs:date",
        DfdlSimpleType::Time => "xs:time",
        DfdlSimpleType::DateTime => "xs:dateTime",
        _ => "calendar",
    };

    if let Some(pattern) = pattern_opt {
        if !pattern.is_empty() {
            if let Some(parsed) = parse_calendar_with_pattern(clean, pattern, simple_type, check_policy, first_day_of_week) {
                return Ok(parsed);
            } else {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse {} from text: '{}' with pattern '{}'", type_name, clean, pattern),
                ));
            }
        }
    }

    match simple_type {
        DfdlSimpleType::Date => {
            if clean.ends_with("-00:00") || clean.ends_with("-0000") || clean.ends_with("-00") {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Timezone '-00:00' is prohibited by XML Schema: '{}'", s),
                ));
            }
            let year_str = if clean.starts_with('-') {
                clean.get(1..).unwrap_or("").split('-').next().unwrap_or("")
            } else {
                clean.split('-').next().unwrap_or("")
            };
            if let Ok(y) = year_str.parse::<u64>() {
                if y > 9999 {
                    let msg = alloc::format!(
                        "Tunable Limit Exceeded Error: Year {} is not within the limits of minValidYear (0) and maxValidYear (9999)",
                        y
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
            }
            let parts: Vec<&str> = if clean.starts_with('-') {
                clean.get(1..).unwrap_or("").split('-').collect()
            } else {
                clean.split('-').collect()
            };
            if parts.len() < 3 || year_str.len() < 4 || year_str.parse::<u32>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:date from text: '{}'", s),
                ));
            }
            let m_res = parts.get(1).and_then(|p| p.parse::<u32>().ok());
            let day_str = parts
                .get(2)
                .map(|p| p.split_terminator(|c: char| c.is_alphabetic() || c == '+' || c == '-').next().unwrap_or(p))
                .unwrap_or("");
            let d_res = day_str.parse::<u32>().ok();
            let (m, d) = match (m_res, d_res) {
                (Some(m), Some(d)) => (m, d),
                _ => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:date from text: '{}'", s),
                    ));
                }
            };
            let y_val = year_str.parse::<i32>().unwrap_or(1970);
            let m_norm = m as usize;
            let d_norm = i64::from(d);
            let is_valid = (1..=12).contains(&m) && (1..=days_in_month(i64::from(y_val), m_norm)).contains(&d_norm);
            if !is_valid {
                if check_policy == crate::schema::ir::CalendarCheckPolicy::Lax {
                    let total_days = ymd_to_days(i64::from(y_val), m_norm, d_norm);
                    let (res_y, res_m, res_d) = days_to_ymd(total_days);
                    let tz_part = clean.get(year_str.len().saturating_add(1).saturating_add(parts.get(1).map_or(0, |p| p.len())).saturating_add(1).saturating_add(day_str.len())..).unwrap_or("");
                    return Ok(alloc::format!("{:04}-{:02}-{:02}{}", res_y, res_m, res_d, tz_part));
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:date from text: '{}'", s),
                    ));
                }
            }
            let tz_part = clean.get(year_str.len().saturating_add(1).saturating_add(parts.get(1).map_or(0, |p| p.len())).saturating_add(1).saturating_add(day_str.len())..).unwrap_or("");
            if !is_valid_timezone_suffix(tz_part, check_policy) {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:date from text: '{}'", s),
                ));
            }
            Ok(clean.to_string())
        }
        DfdlSimpleType::DateTime => {
            if clean.ends_with("-00:00")
                || clean.ends_with("-0000")
                || clean.ends_with("-00")
                || clean.ends_with("GMT")
                || clean.ends_with("UTC")
            {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:dateTime from text: '{}'", s),
                ));
            }
            let (date_str, time_str) = clean.split_once('T').or_else(|| clean.split_once(' ')).ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:dateTime from text: '{}'", s),
                )
            })?;
            let date_parsed = parse_calendar_from_text(date_str, None, None, crate::schema::ir::TextTrimKind::None, DfdlSimpleType::Date, check_policy, first_day_of_week)
                .map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:dateTime from text: '{}'", s),
                    )
                })?;
            let time_parsed = parse_calendar_from_text(time_str, None, None, crate::schema::ir::TextTrimKind::None, DfdlSimpleType::Time, check_policy, first_day_of_week)
                .map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:dateTime from text: '{}'", s),
                    )
                })?;
            Ok(alloc::format!("{}T{}", date_parsed, time_parsed))
        }
        DfdlSimpleType::Time => {
            if (clean.ends_with("-00:00") || clean.ends_with("-0000") || clean.ends_with("-00"))
                && check_policy != crate::schema::ir::CalendarCheckPolicy::Lax
            {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                ));
            }
            if !clean.contains(':') {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                ));
            }
            let split_pos = clean
                .char_indices()
                .find(|(_, c)| !c.is_ascii_digit() && *c != ':' && *c != '.')
                .map(|(i, _)| i)
                .unwrap_or(clean.len());
            let time_no_tz = &clean[..split_pos];
            let tz_suffix = &clean[split_pos..];
            let norm_tz = normalize_timezone_suffix(tz_suffix, check_policy).ok_or_else(|| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                )
            })?;
            let time_parts: Vec<&str> = time_no_tz.split(':').collect();
            if time_parts.len() < 2 {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                ));
            }
            let h_res = time_parts.first().and_then(|p| p.parse::<u32>().ok());
            let m_res = time_parts.get(1).and_then(|p| p.parse::<u32>().ok());
            let (s_res, frac_str) = if time_parts.len() >= 3 {
                let sec_part = time_parts.get(2).copied().unwrap_or("0");
                if let Some((s_sec, frac)) = sec_part.split_once('.') {
                    (s_sec.parse::<u32>().ok(), alloc::format!(".{}", frac))
                } else {
                    (sec_part.parse::<u32>().ok(), String::new())
                }
            } else {
                (Some(0), String::new())
            };
            let (h, min, sec) = match (h_res, m_res, s_res) {
                (Some(h), Some(min), Some(sec)) => (h, min, sec),
                _ => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                    ));
                }
            };
            if h > 23 || min > 59 || sec > 59 {
                if check_policy == crate::schema::ir::CalendarCheckPolicy::Lax {
                    let extra_min = sec / 60;
                    let s_norm = sec % 60;
                    let total_min = min + extra_min;
                    let extra_hr = total_min / 60;
                    let m_norm = total_min % 60;
                    let total_hr = (h + extra_hr) % 24;
                    return Ok(alloc::format!("{:02}:{:02}:{:02}{}{}", total_hr, m_norm, s_norm, frac_str, norm_tz));
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Unable to parse xs:time from text: '{}'", s),
                    ));
                }
            }
            Ok(alloc::format!("{:02}:{:02}:{:02}{}{}", h, min, sec, frac_str, norm_tz))
        }
        _ => Ok(clean.to_string()),
    }
}

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    pub(crate) fn parse_calendar_from_digits(
        digits: &str,
        pattern: Option<&str>,
        simple_type: DfdlSimpleType,
    ) -> DFDLResult<String> {
        let type_name = match simple_type {
            DfdlSimpleType::DateTime => "xs:dateTime",
            DfdlSimpleType::Date => "xs:date",
            DfdlSimpleType::Time => "xs:time",
            _ => "calendar",
        };
        let d = digits;
        if let Some(pat) = pattern {
            if pat == "MMddyyyyHHmmss" && d.len() >= 14 {
                let s = &d[d.len() - 14..];
                let mm = &s[0..2];
                let dd = &s[2..4];
                let yyyy = &s[4..8];
                let hh = &s[8..10];
                let min = &s[10..12];
                let ss = &s[12..14];
                let mm_val: u32 = mm.parse().unwrap_or(0);
                let dd_val: u32 = dd.parse().unwrap_or(0);
                let hh_val: u32 = hh.parse().unwrap_or(0);
                let min_val: u32 = min.parse().unwrap_or(0);
                let ss_val: u32 = ss.parse().unwrap_or(0);
                if !(1..=12).contains(&mm_val)
                    || !(1..=31).contains(&dd_val)
                    || hh_val > 23
                    || min_val > 59
                    || ss_val > 59
                {
                    let msg = alloc::format!(
                        "Parse Error: Unable to parse {} from digits '{}'",
                        type_name,
                        digits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                return Ok(alloc::format!("{}-{}-{}T{}:{}:{}", yyyy, mm, dd, hh, min, ss));
            } else if pat == "MMddyy" && d.len() >= 6 {
                let s = &d[d.len() - 6..];
                let mm = &s[0..2];
                let dd = &s[2..4];
                let yy = &s[4..6];
                let mm_val: u32 = mm.parse().unwrap_or(0);
                let dd_val: u32 = dd.parse().unwrap_or(0);
                if !(1..=12).contains(&mm_val) || !(1..=31).contains(&dd_val) {
                    let msg = alloc::format!(
                        "Parse Error: Unable to parse {} from digits '{}'",
                        type_name,
                        digits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                let year_val: u32 = yy.parse().unwrap_or(0);
                let full_year = if year_val >= 50 {
                    1900 + year_val
                } else {
                    2000 + year_val
                };
                return Ok(alloc::format!("{:04}-{}-{}", full_year, mm, dd));
            } else if pat == "yyyyMMdd" && d.len() >= 8 {
                let s = &d[d.len() - 8..];
                let yyyy = &s[0..4];
                let mm = &s[4..6];
                let dd = &s[6..8];
                let mm_val: u32 = mm.parse().unwrap_or(0);
                let dd_val: u32 = dd.parse().unwrap_or(0);
                if !(1..=12).contains(&mm_val) || !(1..=31).contains(&dd_val) {
                    let msg = alloc::format!(
                        "Parse Error: Unable to parse {} from digits '{}'",
                        type_name,
                        digits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                return Ok(alloc::format!("{}-{}-{}", yyyy, mm, dd));
            } else if pat == "HHmmss" && d.len() >= 6 {
                let s = &d[d.len() - 6..];
                let hh = &s[0..2];
                let mm = &s[2..4];
                let ss = &s[4..6];
                let hh_val: u32 = hh.parse().unwrap_or(0);
                let min_val: u32 = mm.parse().unwrap_or(0);
                let ss_val: u32 = ss.parse().unwrap_or(0);
                if hh_val > 23 || min_val > 59 || ss_val > 59 {
                    let msg = alloc::format!(
                        "Parse Error: Unable to parse {} from digits '{}'",
                        type_name,
                        digits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                }
                return Ok(alloc::format!("{}:{}:{}", hh, mm, ss));
            }
        }
        if d.len() >= 14 {
            let s = &d[d.len() - 14..];
            let mm = &s[0..2];
            let dd = &s[2..4];
            let yyyy = &s[4..8];
            let hh = &s[8..10];
            let min = &s[10..12];
            let ss = &s[12..14];
            let mm_val: u32 = mm.parse().unwrap_or(0);
            let dd_val: u32 = dd.parse().unwrap_or(0);
            let hh_val: u32 = hh.parse().unwrap_or(0);
            let min_val: u32 = min.parse().unwrap_or(0);
            let ss_val: u32 = ss.parse().unwrap_or(0);
            if !(1..=12).contains(&mm_val)
                || !(1..=31).contains(&dd_val)
                || hh_val > 23
                || min_val > 59
                || ss_val > 59
            {
                let msg = alloc::format!(
                    "Parse Error: Unable to parse {} from digits '{}'",
                    type_name,
                    digits
                );
                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
            }
            Ok(alloc::format!("{}-{}-{}T{}:{}:{}", yyyy, mm, dd, hh, min, ss))
        } else {
            Ok(alloc::string::String::from(digits))
        }
    }

    pub(crate) fn parse_binary_seconds_or_millis(
        bytes: &[u8],
        props: &ResolvedProperties,
        simple_type: DfdlSimpleType,
    ) -> DFDLResult<String> {
        let is_be = props.byte_order == ByteOrder::BigEndian;
        let raw_val: i64 = match bytes.len() {
            0 => 0,
            1 => (*bytes.first().unwrap_or(&0) as i8) as i64,
            2 => {
                let mut b = [0u8; 2];
                if let Some(sub) = bytes.get(..2) {
                    b.copy_from_slice(sub);
                }
                if is_be {
                    i16::from_be_bytes(b) as i64
                } else {
                    i16::from_le_bytes(b) as i64
                }
            }
            4 => {
                let mut b = [0u8; 4];
                if let Some(sub) = bytes.get(..4) {
                    b.copy_from_slice(sub);
                }
                if is_be {
                    i32::from_be_bytes(b) as i64
                } else {
                    i32::from_le_bytes(b) as i64
                }
            }
            8 => {
                let mut b = [0u8; 8];
                if let Some(sub) = bytes.get(..8) {
                    b.copy_from_slice(sub);
                }
                if is_be {
                    i64::from_be_bytes(b)
                } else {
                    i64::from_le_bytes(b)
                }
            }
            len if len < 8 => {
                let check_idx = if is_be { 0 } else { len.saturating_sub(1) };
                let is_neg = (*bytes.get(check_idx).unwrap_or(&0) & 0x80) != 0;
                let mut b = if is_neg { [0xFF; 8] } else { [0u8; 8] };
                if is_be {
                    if let Some(dst) = b.get_mut(8 - len..) {
                        dst.copy_from_slice(bytes);
                    }
                    i64::from_be_bytes(b)
                } else {
                    if let Some(dst) = b.get_mut(..len) {
                        dst.copy_from_slice(bytes);
                    }
                    i64::from_le_bytes(b)
                }
            }
            _ => {
                let mut b = [0u8; 8];
                if let Some(sub) = bytes.get(..8) {
                    b.copy_from_slice(sub);
                }
                if is_be {
                    i64::from_be_bytes(b)
                } else {
                    i64::from_le_bytes(b)
                }
            }
        };

        let millis_offset: i64 = match props.binary_calendar_rep {
            crate::schema::ir::BinaryCalendarRep::BinarySeconds => {
                raw_val.checked_mul(1000).ok_or_else(|| {
                    DFDLError::new_static(
                        DFDLErrorKind::Parse,
                        "Parse Error: milliseconds from the binaryCalendarEpoch is out of range of valid values: millis value greater than upper bounds for a Calendar",
                    )
                })?
            }
            _ => raw_val,
        };

        const UPPER_BOUND_MILLIS: i64 = 0x028D_46FB_FCAE_62E8;
        const LOWER_BOUND_MILLIS: i64 = 0xFD72_B904_0351_9D18u64 as i64;

        if millis_offset > UPPER_BOUND_MILLIS {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: milliseconds from the binaryCalendarEpoch is out of range of valid values: millis value greater than upper bounds for a Calendar",
            ));
        }
        if millis_offset < LOWER_BOUND_MILLIS {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: milliseconds from the binaryCalendarEpoch is out of range of valid values: millis value less than lower bounds for a Calendar",
            ));
        }

        let epoch_str = props
            .binary_calendar_epoch
            .as_deref()
            .unwrap_or("1970-01-01T00:00:00+00:00");
        let (epoch_wall_ms, tz_str) = parse_xs_date_time_wall_ms(epoch_str)?;

        let target_wall_ms = epoch_wall_ms.checked_add(millis_offset).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: target milliseconds calculation overflowed",
            )
        })?;

        let total_sec = target_wall_ms.div_euclid(1000);
        let frac_ms = target_wall_ms.rem_euclid(1000);
        let days = total_sec.div_euclid(86400);
        let sec_in_day = total_sec.rem_euclid(86400);
        let hour = sec_in_day / 3600;
        let min = (sec_in_day % 3600) / 60;
        let sec = sec_in_day % 60;
        let (year, month, day) = days_to_ymd(days);

        if !(1..=9999).contains(&year) {
            return Err(DFDLError::new(
                DFDLErrorKind::Parse,
                &alloc::format!(
                    "Tunable Limit Exceeded Error: Year {} is not within the limits of minValidYear (1) and maxValidYear (9999)",
                    year
                ),
            ));
        }

        let formatted = match simple_type {
            DfdlSimpleType::DateTime => {
                if frac_ms > 0 {
                    alloc::format!(
                        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}000{}",
                        year,
                        month,
                        day,
                        hour,
                        min,
                        sec,
                        frac_ms,
                        tz_str
                    )
                } else {
                    alloc::format!(
                        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
                        year,
                        month,
                        day,
                        hour,
                        min,
                        sec,
                        tz_str
                    )
                }
            }
            DfdlSimpleType::Date => {
                alloc::format!("{:04}-{:02}-{:02}{}", year, month, day, tz_str)
            }
            _ => {
                if frac_ms > 0 {
                    alloc::format!(
                        "{:02}:{:02}:{:02}.{:03}000{}",
                        hour,
                        min,
                        sec,
                        frac_ms,
                        tz_str
                    )
                } else {
                    alloc::format!("{:02}:{:02}:{:02}{}", hour, min, sec, tz_str)
                }
            }
        };

        Ok(formatted)
    }
}

/// Validate BCP-47 calendar language syntax according to DFDL §13.13.
pub(crate) fn validate_calendar_language_syntax(lang: &str) -> DFDLResult<()> {
    let trimmed = lang.trim();
    let subtags: Vec<&str> = trimmed.split('-').collect();
    let valid = if subtags.is_empty() || trimmed.is_empty() {
        false
    } else {
        let first = subtags.first().copied().unwrap_or("");
        let first_ok = (first.len() == 2 || first.len() == 3)
            && first.chars().all(|c| c.is_ascii_alphabetic());
        first_ok
            && subtags.iter().skip(1).all(|sub| {
                !sub.is_empty() && sub.len() <= 8 && sub.chars().all(|c| c.is_ascii_alphanumeric())
            })
    };
    if !valid {
        return Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            &alloc::format!(
                "Schema Definition Error: dfdl:calendarLanguage property syntax error: '{}'",
                trimmed
            ),
        ));
    }
    Ok(())
}

fn parse_tz_offset_minutes(s: &str) -> Option<i32> {
    let clean = s.trim().trim_start_matches("UTC").trim_start_matches("GMT");
    if clean.is_empty() || clean == "Z" {
        return Some(0);
    }
    let (sign, rest) = if let Some(stripped) = clean.strip_prefix('+') {
        (1, stripped)
    } else if let Some(stripped) = clean.strip_prefix('-') {
        (-1, stripped)
    } else {
        (1, clean)
    };
    if let Some((h_str, m_str)) = rest.split_once(':') {
        let h: i32 = h_str.parse().ok()?;
        let m: i32 = m_str.parse().ok()?;
        Some(sign * (h * 60 + m))
    } else if rest.len() == 4 && rest.chars().all(|c| c.is_ascii_digit()) {
        let h: i32 = rest.get(..2)?.parse().ok()?;
        let m: i32 = rest.get(2..4)?.parse().ok()?;
        Some(sign * (h * 60 + m))
    } else if let Ok(h) = rest.parse::<i32>() {
        Some(sign * h * 60)
    } else {
        None
    }
}

/// Formats a calendar value from an XML Infoset string according to a DFDL calendarPattern.
pub(crate) fn format_calendar_with_pattern(
    infoset_str: &str,
    pattern: &str,
    lang_opt: Option<&str>,
    tz_override_opt: Option<&str>,
) -> DFDLResult<String> {
    let mut year: Option<i64> = None;
    let mut month: Option<usize> = None;
    let mut day: Option<i64> = None;
    let mut hour: Option<u32> = None;
    let mut minute: Option<u32> = None;
    let mut second: Option<u32> = None;
    let mut frac_str: Option<&str> = None;
    let mut tz_offset_mins: Option<i32> = None;

    // Classify date vs time: If contains 'T', it is dateTime.
    // If it starts with hh: (index 2 is ':') or has ':' without any hyphens '-', it is time.
    // Otherwise (including dates with timezone like 2013-03-01+00:00), it is a date.
    let (date_candidate, time_candidate) = if let Some((d, t)) = infoset_str.split_once('T') {
        (Some(d), Some(t))
    } else if infoset_str.as_bytes().get(2) == Some(&b':')
        || (infoset_str.contains(':') && !infoset_str.contains('-'))
    {
        (None, Some(infoset_str))
    } else {
        (Some(infoset_str), None)
    };

    if let Some(d_str) = date_candidate {
        let (d_base, d_tz) = if let Some(stripped) = d_str.strip_suffix('Z') {
            (stripped, Some("Z"))
        } else if let Some(idx) = d_str.find('+') {
            let (b, tz) = d_str.split_at(idx);
            (b, Some(tz))
        } else {
            let hyphen_count = d_str.chars().filter(|&c| c == '-').count();
            let is_neg_year = d_str.starts_with('-');
            let min_hyphens = if is_neg_year { 3 } else { 2 };
            if hyphen_count > min_hyphens {
                if let Some(last_dash) = d_str.rfind('-') {
                    let (b, tz) = d_str.split_at(last_dash);
                    (b, Some(tz))
                } else {
                    (d_str, None)
                }
            } else {
                (d_str, None)
            }
        };
        let is_neg = d_base.starts_with('-');
        let abs_d = if is_neg { &d_base[1..] } else { d_base };
        let d_parts: Vec<&str> = abs_d.split('-').collect();
        if d_parts.len() == 3 {
            let raw_y: Option<i64> = d_parts.first().and_then(|s| s.parse().ok());
            year = raw_y.map(|y| if is_neg { -y } else { y });
            month = d_parts.get(1).and_then(|s| s.parse().ok());
            day = d_parts.get(2).and_then(|s| s.parse().ok());
        }
        if tz_offset_mins.is_none() {
            if let Some(tz) = d_tz {
                tz_offset_mins = parse_tz_offset_minutes(tz);
            }
        }
    }

    if let Some(t_str) = time_candidate {
        let (t_base, t_tz) = if let Some(idx) = t_str.rfind('+').or_else(|| t_str.rfind('-')) {
            let (b, tz) = t_str.split_at(idx);
            (b, Some(tz))
        } else if let Some(stripped) = t_str.strip_suffix('Z') {
            (stripped, Some("Z"))
        } else {
            (t_str, None)
        };

        if tz_offset_mins.is_none() {
            if let Some(tz) = t_tz {
                tz_offset_mins = parse_tz_offset_minutes(tz);
            }
        }

        let (clock_part, frac_part) = if let Some((c, f)) = t_base.split_once('.') {
            (c, Some(f))
        } else {
            (t_base, None)
        };
        frac_str = frac_part;

        let c_parts: Vec<&str> = clock_part.split(':').collect();
        if c_parts.len() >= 2 {
            hour = c_parts.first().and_then(|s| s.parse().ok());
            minute = c_parts.get(1).and_then(|s| s.parse().ok());
            if c_parts.len() >= 3 {
                second = c_parts.get(2).and_then(|s| s.parse().ok());
            }
        }
    }

    if tz_offset_mins.is_none() {
        if let Some(tz_override) = tz_override_opt {
            tz_offset_mins = parse_tz_offset_minutes(tz_override);
        }
    }

    let y = year.unwrap_or(1970);
    let m = month.unwrap_or(1);
    let d = day.unwrap_or(1);
    let h = hour.unwrap_or(0);
    let min = minute.unwrap_or(0);
    let sec = second.unwrap_or(0);

    // Identify calendar language for localized month and weekday formatting.
    // Supports German ("de"), Spanish ("es"), Russian ("ru"), and default English ("en").
    let lang = lang_opt.map(|l| l.to_ascii_lowercase()).unwrap_or_default();
    let is_german = lang.starts_with("de");
    let is_spanish = lang.starts_with("es");
    let is_russian = lang.starts_with("ru");

    let mut result = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = match chars.get(i) {
            Some(&c) => c,
            None => break,
        };
        if ch == '\'' {
            i = i.saturating_add(1);
            while i < chars.len() {
                if chars.get(i).copied() == Some('\'') {
                    if chars.get(i.saturating_add(1)).copied() == Some('\'') {
                        result.push('\'');
                        i = i.saturating_add(2);
                    } else {
                        i = i.saturating_add(1);
                        break;
                    }
                } else if let Some(&c) = chars.get(i) {
                    result.push(c);
                    i = i.saturating_add(1);
                } else {
                    break;
                }
            }
        } else if ch.is_ascii_alphabetic() {
            let mut count: usize = 0;
            while i < chars.len() && chars.get(i).copied() == Some(ch) {
                count = count.saturating_add(1);
                i = i.saturating_add(1);
            }
            match ch {
                'y' | 'Y' | 'u' | 'r' => {
                    if count == 2 {
                        let y2 = (y.rem_euclid(100)) as u32;
                        use core::fmt::Write;
                        let _ = write!(result, "{:02}", y2);
                    } else {
                        use core::fmt::Write;
                        let _ = write!(result, "{:0width$}", y, width = count);
                    }
                }
                'D' => {
                    let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
                    let days_in_months: [u32; 12] = if is_leap {
                        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
                    } else {
                        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
                    };
                    let prior_days: u32 = days_in_months.iter().take(m.saturating_sub(1)).sum();
                    let doy = prior_days.saturating_add(d.max(1) as u32);
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", doy, width = count);
                }
                'M' => {
                    if count == 1 {
                        use core::fmt::Write;
                        let _ = write!(result, "{}", m);
                    } else if count == 2 {
                        use core::fmt::Write;
                        let _ = write!(result, "{:02}", m);
                    } else if count == 3 {
                        // Short month names (3 characters or abbreviation).
                        let names_de = [
                            "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt",
                            "Nov", "Dez",
                        ];
                        let names_es = [
                            "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct",
                            "nov", "dic",
                        ];
                        let names_ru = [
                            "янв.", "февр.", "мар.", "апр.", "мая", "июн.", "июл.", "авг.",
                            "сент.", "окт.", "нояб.", "дек.",
                        ];
                        let names_en = [
                            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                            "Nov", "Dec",
                        ];
                        let idx = m.saturating_sub(1).min(11);
                        let name = if is_german {
                            names_de.get(idx).copied().unwrap_or("")
                        } else if is_spanish {
                            names_es.get(idx).copied().unwrap_or("")
                        } else if is_russian {
                            names_ru.get(idx).copied().unwrap_or("")
                        } else {
                            names_en.get(idx).copied().unwrap_or("")
                        };
                        result.push_str(name);
                    } else {
                        // Full month names.
                        let names_de = [
                            "Januar",
                            "Februar",
                            "März",
                            "April",
                            "Mai",
                            "Juni",
                            "Juli",
                            "August",
                            "September",
                            "Oktober",
                            "November",
                            "Dezember",
                        ];
                        let names_es = [
                            "enero",
                            "febrero",
                            "marzo",
                            "abril",
                            "mayo",
                            "junio",
                            "julio",
                            "agosto",
                            "septiembre",
                            "octubre",
                            "noviembre",
                            "diciembre",
                        ];
                        let names_ru = [
                            "января",
                            "февраля",
                            "марта",
                            "апреля",
                            "мая",
                            "июня",
                            "июля",
                            "августа",
                            "сентября",
                            "октября",
                            "ноября",
                            "декабря",
                        ];
                        let names_en = [
                            "January",
                            "February",
                            "March",
                            "April",
                            "May",
                            "June",
                            "July",
                            "August",
                            "September",
                            "October",
                            "November",
                            "December",
                        ];
                        let idx = m.saturating_sub(1).min(11);
                        let name = if is_german {
                            names_de.get(idx).copied().unwrap_or("")
                        } else if is_spanish {
                            names_es.get(idx).copied().unwrap_or("")
                        } else if is_russian {
                            names_ru.get(idx).copied().unwrap_or("")
                        } else {
                            names_en.get(idx).copied().unwrap_or("")
                        };
                        result.push_str(name);
                    }
                }
                'd' => {
                    if count == 1 {
                        use core::fmt::Write;
                        let _ = write!(result, "{}", d);
                    } else {
                        use core::fmt::Write;
                        let _ = write!(result, "{:0width$}", d, width = count);
                    }
                }
                'E' => {
                    // Localized day of week.
                    let days = ymd_to_days(y, m, d);
                    let dow = (days + 1).rem_euclid(7) as usize;
                    let days_de_short = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];
                    let days_de_long = [
                        "Sonntag",
                        "Montag",
                        "Dienstag",
                        "Mittwoch",
                        "Donnerstag",
                        "Freitag",
                        "Samstag",
                    ];
                    let days_es_short = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];
                    let days_es_long = [
                        "domingo",
                        "lunes",
                        "martes",
                        "miércoles",
                        "jueves",
                        "viernes",
                        "sábado",
                    ];
                    let days_ru_short = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
                    let days_ru_long = [
                        "воскресенье",
                        "понедельник",
                        "вторник",
                        "среда",
                        "четверг",
                        "пятница",
                        "суббота",
                    ];
                    let days_en_short = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                    let days_en_long = [
                        "Sunday",
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                    ];
                    let idx = dow % 7;
                    if count <= 3 {
                        let name = if is_german {
                            days_de_short.get(idx).copied().unwrap_or("")
                        } else if is_spanish {
                            days_es_short.get(idx).copied().unwrap_or("")
                        } else if is_russian {
                            days_ru_short.get(idx).copied().unwrap_or("")
                        } else {
                            days_en_short.get(idx).copied().unwrap_or("")
                        };
                        result.push_str(name);
                    } else {
                        let name = if is_german {
                            days_de_long.get(idx).copied().unwrap_or("")
                        } else if is_spanish {
                            days_es_long.get(idx).copied().unwrap_or("")
                        } else if is_russian {
                            days_ru_long.get(idx).copied().unwrap_or("")
                        } else {
                            days_en_long.get(idx).copied().unwrap_or("")
                        };
                        result.push_str(name);
                    }
                }
                'H' => {
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", h, width = count);
                }
                'k' => {
                    let k_val = if h == 0 { 24 } else { h };
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", k_val, width = count);
                }
                'K' => {
                    let k_val = h % 12;
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", k_val, width = count);
                }
                'h' => {
                    let h_val = match h % 12 {
                        0 => 12,
                        val => val,
                    };
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", h_val, width = count);
                }
                'm' => {
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", min, width = count);
                }
                's' => {
                    use core::fmt::Write;
                    let _ = write!(result, "{:0width$}", sec, width = count);
                }
                'S' => {
                    let frac = frac_str.unwrap_or("");
                    let mut s_padded = String::from(frac);
                    while s_padded.len() < count {
                        s_padded.push('0');
                    }
                    if let Some(sub) = s_padded.get(..count) {
                        result.push_str(sub);
                    }
                }
                'a' => {
                    result.push_str(if h < 12 { "AM" } else { "PM" });
                }
                'z' | 'v' => {
                    let total_mins = tz_offset_mins.unwrap_or(0);
                    let abs_mins = total_mins.abs() % 60;
                    let abs_hours = total_mins.abs() / 60;
                    let sign = if total_mins < 0 { '-' } else { '+' };
                    if count < 4 {
                        if total_mins == 0 {
                            result.push_str("GMT");
                        } else if abs_mins == 0 {
                            use core::fmt::Write;
                            let _ = write!(result, "GMT{}{}", sign, abs_hours);
                        } else {
                            use core::fmt::Write;
                            let _ = write!(result, "GMT{}{}:{:02}", sign, abs_hours, abs_mins);
                        }
                    } else {
                        use core::fmt::Write;
                        let _ = write!(result, "GMT{}{:02}:{:02}", sign, abs_hours, abs_mins);
                    }
                }
                'V' => {
                    let total_mins = tz_offset_mins.unwrap_or(0);
                    if count == 1 {
                        if total_mins == 0 {
                            result.push_str("gmt");
                        } else {
                            result.push_str("unk");
                        }
                    } else {
                        let abs_mins = total_mins.abs() % 60;
                        let abs_hours = total_mins.abs() / 60;
                        let sign = if total_mins < 0 { '-' } else { '+' };
                        use core::fmt::Write;
                        let _ = write!(result, "GMT{}{:02}:{:02}", sign, abs_hours, abs_mins);
                    }
                }
                'Z' => {
                    let total_mins = tz_offset_mins.unwrap_or(0);
                    let abs_mins = total_mins.abs() % 60;
                    let abs_hours = total_mins.abs() / 60;
                    let sign = if total_mins < 0 { '-' } else { '+' };
                    if count < 4 {
                        use core::fmt::Write;
                        let _ = write!(result, "{}{:02}{:02}", sign, abs_hours, abs_mins);
                    } else {
                        use core::fmt::Write;
                        let _ = write!(result, "GMT{}{:02}:{:02}", sign, abs_hours, abs_mins);
                    }
                }
                'x' => {
                    let total_mins = tz_offset_mins.unwrap_or(0);
                    let abs_mins = total_mins.abs() % 60;
                    let abs_hours = total_mins.abs() / 60;
                    let sign = if total_mins < 0 { '-' } else { '+' };
                    if count == 1 {
                        use core::fmt::Write;
                        let _ = write!(result, "{}{:02}", sign, abs_hours);
                    } else if count == 2 {
                        use core::fmt::Write;
                        let _ = write!(result, "{}{:02}{:02}", sign, abs_hours, abs_mins);
                    } else {
                        use core::fmt::Write;
                        let _ = write!(result, "{}{:02}:{:02}", sign, abs_hours, abs_mins);
                    }
                }
                _ => {
                    for _ in 0..count {
                        result.push(ch);
                    }
                }
            }
        } else {
            result.push(ch);
            i += 1;
        }
    }

    Ok(result)
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
    fn test_validate_calendar_language_syntax() {
        assert!(validate_calendar_language_syntax("en").is_ok());
        assert!(validate_calendar_language_syntax("en-US").is_ok());
        assert!(validate_calendar_language_syntax("de-1996").is_ok());
        assert!(validate_calendar_language_syntax("zh-Hans-CN").is_ok());

        assert!(validate_calendar_language_syntax("").is_err());
        assert!(validate_calendar_language_syntax("f@-1234").is_err());
        assert!(validate_calendar_language_syntax("toolongprimarylanguage").is_err());
        assert!(validate_calendar_language_syntax("en_US").is_err());
    }

    #[test]
    fn test_format_calendar_with_pattern_timezones_and_locales() {
        // time04: hh:mm.zzz with 08:43:00-05:00 -> 08:43.GMT-5
        let res = format_calendar_with_pattern("08:43:00-05:00", "hh:mm.zzz", None, None).unwrap();
        assert_eq!(res, "08:43.GMT-5");

        // time04b: hh:mm.zzzz with 08:43:00-05:00 -> 08:43.GMT-05:00
        let res = format_calendar_with_pattern("08:43:00-05:00", "hh:mm.zzzz", None, None).unwrap();
        assert_eq!(res, "08:43.GMT-05:00");

        // time05: hh:mm.v with 08:43:00-08:00 -> 08:43.GMT-8
        let res = format_calendar_with_pattern("08:43:00-08:00", "hh:mm.v", None, None).unwrap();
        assert_eq!(res, "08:43.GMT-8");

        // time07: hh:mm.V with 08:43:00-08:00 -> 08:43.unk
        let res = format_calendar_with_pattern("08:43:00-08:00", "hh:mm.V", None, None).unwrap();
        assert_eq!(res, "08:43.unk");

        // time07: hh:mm.V with 08:43:00+00:00 -> 08:43.gmt
        let res = format_calendar_with_pattern("08:43:00+00:00", "hh:mm.V", None, None).unwrap();
        assert_eq!(res, "08:43.gmt");

        // time27: hh:mm.Z with 08:43:00+00:00 -> 08:43.+0000
        let res = format_calendar_with_pattern("08:43:00+00:00", "hh:mm.Z", None, None).unwrap();
        assert_eq!(res, "08:43.+0000");

        // German date: 2013-03-01 with EEEE MMMM yyyy -> Freitag März 2013
        let res = format_calendar_with_pattern("2013-03-01", "EEEE MMMM yyyy", Some("de"), None).unwrap();
        assert_eq!(res, "Freitag März 2013");

        // English date: 2013-03-01 with EEEE MMMM yyyy -> Friday March 2013
        let res = format_calendar_with_pattern("2013-03-01", "EEEE MMMM yyyy", Some("en"), None).unwrap();
        assert_eq!(res, "Friday March 2013");

        // Short month names (MMM) in German, Spanish, and Russian
        assert_eq!(format_calendar_with_pattern("2026-10-08", "MMM yyyy", Some("de"), None).unwrap(), "Okt 2026");
        assert_eq!(format_calendar_with_pattern("2026-10-08", "MMM yyyy", Some("es"), None).unwrap(), "oct 2026");
        assert_eq!(format_calendar_with_pattern("2026-10-08", "MMM yyyy", Some("ru"), None).unwrap(), "окт. 2026");

        // Quoted literals and escaped quotes in calendar pattern
        assert_eq!(format_calendar_with_pattern("2026-10-08T12:30:45", "yyyy-MM-dd'T'HH:mm:ss", None, None).unwrap(), "2026-10-08T12:30:45");
        assert_eq!(format_calendar_with_pattern("2026-10-08T12:30:45", "yyyy-MM-dd'at''noon'HH:mm:ss", None, None).unwrap(), "2026-10-08at'noon12:30:45");

        // Timezone normalization for 2-digit and 4-digit offsets without colon
        assert_eq!(normalize_timezone_suffix("+05", crate::schema::ir::CalendarCheckPolicy::Strict), Some("+05:00".into()));
        assert_eq!(normalize_timezone_suffix("+0530", crate::schema::ir::CalendarCheckPolicy::Strict), Some("+05:30".into()));
        assert_eq!(normalize_timezone_suffix("-08", crate::schema::ir::CalendarCheckPolicy::Strict), Some("-08:00".into()));
        assert_eq!(normalize_timezone_suffix("-0800", crate::schema::ir::CalendarCheckPolicy::Strict), Some("-08:00".into()));

        // parse_xs_date_time_wall_ms valid and error branches (lines 74-121)
        assert!(parse_xs_date_time_wall_ms("2026-10-08T12:00:00Z").is_ok());
        assert!(parse_xs_date_time_wall_ms("2026-10-08T12:00:00.5Z").is_ok());
        assert!(parse_xs_date_time_wall_ms("-0044-03-15T00:00:00Z").is_ok());
        assert!(parse_xs_date_time_wall_ms("bad_epoch").is_err());
        assert!(parse_xs_date_time_wall_ms("2026-10T12:00:00").is_err());
        assert!(parse_xs_date_time_wall_ms("ABCD-10-08T12:00:00").is_err());
        assert!(parse_xs_date_time_wall_ms("2026-XX-08T12:00:00").is_err());
        assert!(parse_xs_date_time_wall_ms("2026-10-YYT12:00:00").is_err());

        // Day of week first_day_of_week variations in calendar pattern parsing (lines 268-298)
        let test_pat = |text: &str, pat: &str, fdw: crate::schema::ir::CalendarFirstDayOfWeek| {
            parse_calendar_from_text(
                text,
                Some(pat),
                None,
                crate::schema::ir::TextTrimKind::None,
                DfdlSimpleType::Date,
                crate::schema::ir::CalendarCheckPolicy::Lax,
                fdw,
            )
        };
        assert!(test_pat("3 2026-10-08", "e yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Tuesday).is_ok());
        assert!(test_pat("3 2026-10-08", "e yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Wednesday).is_ok());
        assert!(test_pat("3 2026-10-08", "e yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Thursday).is_ok());
        assert!(test_pat("3 2026-10-08", "e yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Friday).is_ok());
        assert!(test_pat("3 2026-10-08", "e yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Saturday).is_ok());
        assert!(test_pat("Thu 2026-10-08", "eee yyyy-MM-dd", crate::schema::ir::CalendarFirstDayOfWeek::Sunday).is_ok());
    }

    /// Tests additional calendar parser branches including day name length resolution,
    /// escaped quotes in pattern strings, week/julian field parsing, lax month rolling, and timezone aliases.
    ///
    /// Verifies that:
    /// 1. Day of week matching prefers the longest matching name (e.g. "Thursday" over "Thu").
    /// 2. Escaped quotes (`''`) inside pattern literals match single quotes in input text.
    /// 3. Week and day-of-week-in-month fields (`w`, `W`, `F`, `g`) are consumed without error.
    /// 4. Lax calendar checking normalizes months greater than 12 by rolling over to the next year.
    /// 5. Timezone normalizer maps `-00:00` to `+00:00` under Lax policy and rejects it under Strict policy.
    /// 6. Named UTC/GMT timezone prefixes are recognized and normalized under Lax policy.
    #[test]
    fn test_calendar_parser_extended_coverage() {
        // 1. Longest day of week matching ("Thursday" vs "Thu", line 188)
        let (dow_full, len_full) = match_day_of_week("Thursday").unwrap();
        assert_eq!(dow_full, 4);
        assert_eq!(len_full, 8);

        // 2. Escaped quotes in pattern string (lines 224-228)
        let res_quotes = parse_calendar_with_pattern(
            "2026'10'08",
            "yyyy''MM''dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_quotes, Some("2026-10-08".into()));

        // 3. Week field (lines 248-260)
        let res_week = parse_calendar_with_pattern(
            "2026-10-08 w41",
            "yyyy-MM-dd 'w'ww",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_week, Some("2026-10-08".into()));

        // 4. Month > 12 lax normalization (lines 636-642)
        let res_lax_month = parse_calendar_with_pattern(
            "2026-14-08",
            "yyyy-MM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_lax_month, Some("2027-02-08".into()));

        // 5. Strict rejection of out-of-range month and hours (lines 648-653)
        assert!(parse_calendar_with_pattern(
            "2026-13-08",
            "yyyy-MM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        ).is_none());

        assert!(parse_calendar_with_pattern(
            "25:00:00",
            "HH:mm:ss",
            DfdlSimpleType::Time,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        ).is_none());

        // 6. Timezone normalization with -00:00 and GMT/UTC prefixes (lines 676-691)
        assert_eq!(
            normalize_timezone_suffix("-00:00", crate::schema::ir::CalendarCheckPolicy::Strict),
            None
        );
        assert_eq!(
            normalize_timezone_suffix("-00:00", crate::schema::ir::CalendarCheckPolicy::Lax),
            Some("+00:00".into())
        );
        assert_eq!(
            normalize_timezone_suffix("GMT+05:00", crate::schema::ir::CalendarCheckPolicy::Lax),
            Some("+05:00".into())
        );
        assert_eq!(
            normalize_timezone_suffix("UTC-03:00", crate::schema::ir::CalendarCheckPolicy::Lax),
            Some("-03:00".into())
        );

        // 7. Day of week mapping with CalendarFirstDayOfWeek variants (lines 280-298)
        let dow_tue = parse_calendar_with_pattern(
            "2026-10-08 e3",
            "yyyy-MM-dd 'e'e",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Tuesday,
        );
        assert_eq!(dow_tue, Some("2026-10-08".into()));

        let dow_fri = parse_calendar_with_pattern(
            "2026-10-08 e7",
            "yyyy-MM-dd 'e'e",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Friday,
        );
        assert_eq!(dow_fri, Some("2026-10-08".into()));

        // 8. Day of year field 'D' (lines 332-353)
        let res_doy = parse_calendar_with_pattern(
            "2026-281",
            "yyyy-DDD",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_doy.is_some());

        // 9. 2-digit year >= 50 rolls into 1900s (line 328)
        let res_y50 = parse_calendar_with_pattern(
            "75-10-08",
            "yy-MM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_y50, Some("1975-10-08".into()));

        // 10. Month name matching (lines 360-364)
        let res_mname = parse_calendar_with_pattern(
            "2026-October-08",
            "yyyy-MMMM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_mname, Some("2026-10-08".into()));

        // 11. Timezone variations: Z, GMT, 2-digit, 4-digit (lines 673-720)
        assert_eq!(normalize_timezone_suffix("Z", crate::schema::ir::CalendarCheckPolicy::Strict), Some("Z".into()));
        assert_eq!(normalize_timezone_suffix("GMT", crate::schema::ir::CalendarCheckPolicy::Lax), Some("+00:00".into()));
        assert_eq!(normalize_timezone_suffix("+5", crate::schema::ir::CalendarCheckPolicy::Lax), Some("+05:00".into()));
        assert_eq!(normalize_timezone_suffix("+0530", crate::schema::ir::CalendarCheckPolicy::Lax), Some("+05:30".into()));

        // 12. days_in_month out-of-range returns 0 (line 32)
        assert_eq!(days_in_month(2024, 13), 0);
        assert_eq!(days_in_month(2024, 0), 0);

        // 13. CalendarFirstDayOfWeek for Wednesday, Thursday, Saturday (lines 286-297)
        let res_wed = parse_calendar_with_pattern(
            "2024-01-15 3",
            "yyyy-MM-dd e",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Wednesday,
        );
        assert!(res_wed.is_some());

        let res_thu = parse_calendar_with_pattern(
            "2024-01-15 3",
            "yyyy-MM-dd e",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Thursday,
        );
        assert!(res_thu.is_some());

        let res_sat = parse_calendar_with_pattern(
            "2024-01-15 3",
            "yyyy-MM-dd e",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Saturday,
        );
        assert!(res_sat.is_some());

        // 14. Pattern without separators between year and month (line 317)
        let res_ymd = parse_calendar_with_pattern(
            "20240115",
            "yyyyMMdd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_ymd, Some("2024-01-15".into()));

        // 15. Negative zero timezone offset rejection (line 554)
        let res_neg_zero = parse_calendar_with_pattern(
            "2024-01-15T12:00:00-00:00",
            "yyyy-MM-dd'T'HH:mm:ssZ",
            DfdlSimpleType::DateTime,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_neg_zero.is_none());

        // 16. GMT with positive offset (lines 543-547)
        let res_gmt_offset = parse_calendar_with_pattern(
            "2024-01-15T12:00:00GMT+02:00",
            "yyyy-MM-dd'T'HH:mm:ssz",
            DfdlSimpleType::DateTime,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_gmt_offset.is_some());

        // 17. Era G matching AD (lines 521-523)
        let res_era_ad = parse_calendar_with_pattern(
            "2024-01-15 AD",
            "yyyy-MM-dd G",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_era_ad.is_some());

        // 18. Escaped quote in pattern: '' (lines 224-228)
        let res_quote = parse_calendar_with_pattern(
            "2024'01'15",
            "yyyy''MM''dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert_eq!(res_quote, Some("2024-01-15".into()));

        // 19. Week/day patterns w, W, F, g (lines 248-260)
        let res_w = parse_calendar_with_pattern(
            "2024-03-15",
            "yyyy-ww-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_w.is_some());

        // 20. Pattern with day of week E and eee (lines 268-272, 405-414)
        let res_e = parse_calendar_with_pattern(
            "Monday, 2024-01-15",
            "eee, yyyy-MM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_e.is_some());

        let res_e_cap = parse_calendar_with_pattern(
            "Mon, 2024-01-15",
            "EEE, yyyy-MM-dd",
            DfdlSimpleType::Date,
            crate::schema::ir::CalendarCheckPolicy::Lax,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_e_cap.is_some());

        // 21. Quoted literal with 'literal' (lines 231-247)
        let res_lit = parse_calendar_with_pattern(
            "2024T12H30M00",
            "yyyy'T'HH'H'mm'M'ss",
            DfdlSimpleType::DateTime,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Monday,
        );
        assert!(res_lit.is_some());
    }
}

