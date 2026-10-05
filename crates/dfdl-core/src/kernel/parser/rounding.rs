//! Decimal-string rounding for text-number formatting (DFDL §13.7.1.4).
//!
//! DFDL delegates `textNumberPattern` formatting to ICU `DecimalFormat`, whose default
//! is `ROUND_HALF_EVEN` (never truncation) and which supports a rounding increment
//! (explicit via `dfdl:textNumberRoundingIncrement`, or implied by non-zero digits
//! `1`-`9` in the pattern, e.g. `0.02#`). Values are handled as exact decimal digit
//! strings so no binary floating-point error is introduced.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::schema::ir::{ResolvedProperties, TextNumberRoundingMode as Mode};

/// Largest decimal exponent magnitude expanded to plain digits (memory guard).
const MAX_EXPANDED_EXPONENT: i64 = 1000;
/// Largest digit count handled by the exact increment arithmetic (`u128` holds 38 digits).
const MAX_INCREMENT_DIGITS: usize = 36;

/// Rounding configuration for one formatting operation.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NumberRounding<'a> {
    /// Rounding mode applied to discarded digits.
    pub mode: Mode,
    /// Explicit rounding increment (decimal string), if any.
    pub increment: Option<&'a str>,
}

impl<'a> NumberRounding<'a> {
    /// Derives the configuration from resolved properties: mode and increment apply only
    /// when `textNumberRounding="explicit"`, otherwise ICU's half-even default is used.
    pub(crate) fn from_props(p: &'a ResolvedProperties) -> Self {
        if p.text_number_rounding_explicit {
            Self {
                mode: p.text_number_rounding_mode,
                increment: p.text_number_rounding_increment.as_deref(),
            }
        } else {
            Self::default()
        }
    }
}

/// Expands an `E`-notation decimal string (`"8.6E-200"`) to plain digits; other input
/// (including NaN/INF and absurd exponents) is returned unchanged.
pub(crate) fn expand_exponent(raw: &str) -> String {
    let Some(e_idx) = raw.find(['E', 'e']) else {
        return raw.to_string();
    };
    let (mantissa, exp_str) = raw.split_at(e_idx);
    let Ok(exp) = exp_str.get(1..).unwrap_or("").parse::<i64>() else {
        return raw.to_string();
    };
    let (m_int, m_frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits_ok = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if exp.abs() > MAX_EXPANDED_EXPONENT || !digits_ok(m_int) || !digits_ok(m_frac) {
        return raw.to_string();
    }
    let all = format!("{}{}", m_int, m_frac);
    let point = m_int.len() as i64 + exp;
    if point <= 0 {
        format!("0.{}{}", "0".repeat(point.unsigned_abs() as usize), all)
    } else if point as usize >= all.len() {
        format!("{}{}", all, "0".repeat(point as usize - all.len()))
    } else {
        let (i, f) = all.split_at(point as usize);
        format!("{}.{}", i, f)
    }
}

/// Decides whether a truncated magnitude must be incremented by one unit in the last
/// kept place. `half` compares the discarded part against exactly one half.
fn should_round_up(mode: Mode, negative: bool, half: Ordering, last_odd: bool) -> bool {
    match mode {
        Mode::RoundDown => false,
        Mode::RoundUp => true,
        Mode::RoundCeiling => !negative,
        Mode::RoundFloor => negative,
        Mode::RoundHalfUp => half != Ordering::Less,
        Mode::RoundHalfDown => half == Ordering::Greater,
        // `RoundUnnecessary` is policed by the caller; fall back to the ICU default.
        Mode::RoundHalfEven | Mode::RoundUnnecessary => match half {
            Ordering::Greater => true,
            Ordering::Equal => last_odd,
            Ordering::Less => false,
        },
    }
}

/// Adds one unit in the last place of `int.frac`, propagating the carry.
fn add_ulp(int: &mut String, frac: &mut String) {
    let int_len = int.len();
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).collect();
    let mut carry = true;
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d = d.saturating_add(1);
            carry = false;
            break;
        }
    }
    let (i_digits, f_digits) = digits.split_at(int_len.min(digits.len()));
    let mut new_int = String::new();
    if carry {
        new_int.push('1');
    }
    new_int.extend(i_digits.iter().map(|b| *b as char));
    *int = new_int;
    *frac = f_digits.iter().map(|b| *b as char).collect();
}

/// Rounds `int.frac` (an unsigned magnitude) to `keep` fractional digits using `mode`.
pub(crate) fn round_fraction(
    int: &mut String,
    frac: &mut String,
    keep: usize,
    mode: Mode,
    negative: bool,
) {
    if frac.len() <= keep {
        return;
    }
    let discarded = frac.split_off(keep);
    let first = discarded.bytes().next().unwrap_or(b'0');
    let rest_nonzero = discarded.bytes().skip(1).any(|b| b != b'0');
    if first == b'0' && !rest_nonzero {
        return; // discarded digits were all zero: exact
    }
    let half = if first > b'5' || (first == b'5' && rest_nonzero) {
        Ordering::Greater
    } else if first == b'5' {
        Ordering::Equal
    } else {
        Ordering::Less
    };
    let last_odd = frac
        .bytes()
        .last()
        .or_else(|| int.bytes().last())
        .is_some_and(|b| b.wrapping_sub(b'0') % 2 == 1);
    if should_round_up(mode, negative, half, last_odd) {
        add_ulp(int, frac);
    }
}

/// Rounds `int.frac` to the nearest multiple of `inc` (e.g. `"0.02"`) using `mode`.
/// Returns `None` when the increment is invalid, zero, or exceeds exact arithmetic range;
/// callers then fall back to plain digit rounding.
pub(crate) fn round_to_increment(
    int: &str,
    frac: &str,
    inc: &str,
    mode: Mode,
    negative: bool,
) -> Option<(String, String)> {
    let (inc_int, inc_frac) = inc.split_once('.').unwrap_or((inc, ""));
    let is_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !is_digits(inc_int) || !is_digits(inc_frac) || inc_int.is_empty() && inc_frac.is_empty() {
        return None;
    }
    let scale = frac.len().max(inc_frac.len());
    if int.len().max(inc_int.len()) + scale > MAX_INCREMENT_DIGITS {
        return None;
    }
    let scaled = |i: &str, f: &str| -> Option<u128> {
        let mut s = format!("{}{}", i, f);
        s.push_str(&"0".repeat(scale - f.len()));
        s.parse::<u128>().ok()
    };
    let value = scaled(int, frac)?;
    let step = scaled(inc_int, inc_frac)?;
    if step == 0 {
        return None;
    }
    let (mut q, r) = (value / step, value % step);
    if r != 0 {
        let half = r.checked_mul(2)?.cmp(&step);
        if should_round_up(mode, negative, half, q % 2 == 1) {
            q = q.checked_add(1)?;
        }
    }
    let digits = format!("{:0>width$}", q.checked_mul(step)?, width = scale + 1);
    let (i, f) = digits.split_at(digits.len() - scale);
    let i = i.trim_start_matches('0');
    Some((
        if i.is_empty() { "0".to_string() } else { i.to_string() },
        f.to_string(),
    ))
}

/// ICU derives a rounding increment from digits `1`-`9` in the pattern (`0.02#` => `0.02`).
pub(crate) fn pattern_increment(int_pat: &str, frac_pat: &str) -> Option<String> {
    let ip: String = int_pat.chars().filter(char::is_ascii_digit).collect();
    let fp: String = frac_pat.chars().filter(char::is_ascii_digit).collect();
    if !ip.bytes().chain(fp.bytes()).any(|b| (b'1'..=b'9').contains(&b)) {
        return None;
    }
    Some(format!("{}.{}", if ip.is_empty() { "0" } else { &ip }, fp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rf(int: &str, frac: &str, keep: usize, mode: Mode, neg: bool) -> String {
        let (mut i, mut f) = (int.to_string(), frac.to_string());
        round_fraction(&mut i, &mut f, keep, mode, neg);
        format!("{}.{}", i, f)
    }

    #[test]
    fn half_even_ties_go_to_even_neighbour() {
        assert_eq!(rf("0", "125", 2, Mode::RoundHalfEven, false), "0.12");
        assert_eq!(rf("0", "135", 2, Mode::RoundHalfEven, false), "0.14");
        assert_eq!(rf("1235", "5", 0, Mode::RoundHalfEven, false), "1236.");
        assert_eq!(rf("10", "9", 0, Mode::RoundHalfEven, false), "11.");
    }

    #[test]
    fn every_mode_on_positive_and_negative_ties() {
        let r = |m, neg| rf("2", "5", 0, m, neg);
        assert_eq!(r(Mode::RoundDown, false), "2.");
        assert_eq!(r(Mode::RoundUp, false), "3.");
        assert_eq!(r(Mode::RoundCeiling, false), "3.");
        assert_eq!(r(Mode::RoundCeiling, true), "2.");
        assert_eq!(r(Mode::RoundFloor, true), "3.");
        assert_eq!(r(Mode::RoundFloor, false), "2.");
        assert_eq!(r(Mode::RoundHalfUp, false), "3.");
        assert_eq!(r(Mode::RoundHalfDown, false), "2.");
        assert_eq!(r(Mode::RoundHalfEven, false), "2.");
    }

    #[test]
    fn carry_propagates_into_integer_part() {
        assert_eq!(rf("99", "99", 1, Mode::RoundHalfUp, false), "100.0");
    }

    #[test]
    fn exact_discard_is_not_rounded() {
        assert_eq!(rf("1", "2500", 2, Mode::RoundUp, false), "1.25");
    }

    #[test]
    fn increment_rounds_to_nearest_multiple() {
        let r = |i, f, inc| round_to_increment(i, f, inc, Mode::RoundHalfEven, false);
        assert_eq!(r("0", "128", "0.02"), Some(("0".into(), "120".into())));
        assert_eq!(r("10", "9", "0.1"), Some(("10".into(), "9".into())));
        assert_eq!(r("8", "6", "1"), Some(("9".into(), "0".into())));
        assert_eq!(r("1", "0", "0"), None);
        assert_eq!(r("1", "0", "x"), None);
    }

    #[test]
    fn pattern_increment_from_nonzero_digits() {
        assert_eq!(pattern_increment("#,##0", "02#").as_deref(), Some("0.02"));
        assert_eq!(pattern_increment("#,##0", "020").as_deref(), Some("0.020"));
        assert_eq!(pattern_increment("##0", "00#"), None);
    }

    #[test]
    fn exponent_notation_expands_to_plain_digits() {
        assert_eq!(expand_exponent("8.6E-3"), "0.0086");
        assert_eq!(expand_exponent("1.5E3"), "1500");
        assert_eq!(expand_exponent("1.2345E2"), "123.45");
        assert_eq!(expand_exponent("NaN"), "NaN");
        assert_eq!(expand_exponent("1E999999"), "1E999999");
    }

    #[test]
    fn explicit_rounding_only_when_requested() {
        let mut p = ResolvedProperties {
            text_number_rounding_mode: Mode::RoundDown,
            text_number_rounding_increment: Some("0.5".to_string()),
            ..ResolvedProperties::default()
        };
        assert_eq!(NumberRounding::from_props(&p).mode, Mode::RoundHalfEven);
        p.text_number_rounding_explicit = true;
        let r = NumberRounding::from_props(&p);
        assert_eq!((r.mode, r.increment), (Mode::RoundDown, Some("0.5")));
    }
}
