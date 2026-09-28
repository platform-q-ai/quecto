//! `json.dumps`, reproduced (#2268): Python's two board styles, its
//! `ensure_ascii` escaping and its float `repr`.

use std::fmt::Write as _;

use super::{ENCODE_MAX_DEPTH, PyJson, PyJsonError, PyStr};

/// The separators and key order of one of Python's two board styles.
#[derive(Clone, Copy)]
pub(super) struct Style {
    item: &'static str,
    key: &'static str,
    sort_keys: bool,
}

impl Style {
    pub(super) const ENCODE: Self = Self {
        item: ",",
        key: ":",
        sort_keys: true,
    };
    pub(super) const DUMPS: Self = Self {
        item: ", ",
        key: ": ",
        sort_keys: false,
    };
}

pub(super) fn write(value: &PyJson, style: Style) -> Result<String, PyJsonError> {
    let _ = (
        value,
        style.item,
        style.key,
        style.sort_keys,
        ENCODE_MAX_DEPTH,
    );
    Err(PyJsonError::TooDeep {
        limit: 0,
        action: "unimplemented",
    })
}

/// A string as `ensure_ascii=True` writes it: printable ASCII as is, the
/// short escapes Python uses, and `\u00e9`-style escapes (four lowercase
/// hex digits; UTF-16 surrogate pairs above U+FFFF, lone surrogates as
/// themselves) for everything else, including DEL.
pub(super) fn write_str(out: &mut String, text: &PyStr) {
    out.push('"');
    for point in text.code_points() {
        match point {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x20..=0x7E => out.push(char::from_u32(point).expect("printable ASCII")),
            0..=0xFFFF => push_unit(out, point),
            _ => {
                let offset = point - 0x1_0000;
                push_unit(out, 0xD800 + (offset >> 10));
                push_unit(out, 0xDC00 + (offset & 0x3FF));
            }
        }
    }
    out.push('"');
}

fn push_unit(out: &mut String, unit: u32) {
    assert!(unit <= 0xFFFF, "a UTF-16 code unit: {unit:#x}");
    write!(out, "\\u{unit:04x}").expect("writing to a String cannot fail");
}

/// See [`super::float_repr`].
pub(super) fn float_repr(value: f64) -> String {
    if value.is_finite() {
        finite_repr(value)
    } else if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_sign_positive() {
        "Infinity".to_owned()
    } else {
        "-Infinity".to_owned()
    }
}
/// Python's `repr` of a finite float, from `ryu`'s shortest round-trip
/// digits (ties broken to even, like Python's `repr`).
fn finite_repr(value: f64) -> String {
    let mut buffer = ryu::Buffer::new();
    let shortest = buffer.format_finite(value);
    let (sign, unsigned) = match shortest.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", shortest),
    };
    let (digits, exponent) = scientific(unsigned);
    assert!(
        (1..=17).contains(&digits.len()),
        "a shortest round-trip f64 has 1 to 17 significant digits: {shortest}"
    );
    let body = if (-4..16).contains(&exponent) {
        positional(&digits, exponent)
    } else {
        exponential(&digits, exponent)
    };
    format!("{sign}{body}")
}

/// The significant digits of an unsigned decimal (`ryu` writes `0.1`,
/// `1e16` or `1.5e-7`), and the power of ten of the first one: `1.5e-7`
/// gives `("15", -7)`, `0.001` gives `("1", -3)`, `0.0` gives `("0", 0)`.
fn scientific(unsigned: &str) -> (String, i32) {
    let (mantissa, power) = match unsigned.split_once('e') {
        Some((mantissa, power)) => (
            mantissa,
            power.parse::<i32>().expect("ryu writes a decimal exponent"),
        ),
        None => (unsigned, 0),
    };
    let (integral, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    assert!(
        integral
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit()),
        "ryu writes a plain decimal mantissa: {unsigned}"
    );
    let all = format!("{integral}{fraction}");
    let significant = all.trim_start_matches('0');
    let leading_zeros = all.len() - significant.len();
    let digits = significant.trim_end_matches('0');
    if digits.is_empty() {
        return ("0".to_owned(), 0);
    }
    let integral_len = i32::try_from(integral.len()).expect("a short mantissa");
    let leading_zeros = i32::try_from(leading_zeros).expect("a short mantissa");
    (digits.to_owned(), power + integral_len - 1 - leading_zeros)
}

/// `digits` × 10^(`exponent` − len + 1) written without an exponent, always
/// with a fractional part (`5.0`, `0.0001`, `1790000000.123456`).
fn positional(digits: &str, exponent: i32) -> String {
    if exponent >= 0 {
        let integral = usize::try_from(exponent).expect("non-negative exponent") + 1;
        if digits.len() > integral {
            format!("{}.{}", &digits[..integral], &digits[integral..])
        } else {
            format!("{digits}{}.0", "0".repeat(integral - digits.len()))
        }
    } else {
        let zeros = usize::try_from(-exponent - 1).expect("exponent is below zero");
        format!("0.{}{digits}", "0".repeat(zeros))
    }
}

/// `d.ddde±XX`, as Python's `repr` writes an exponent form.
fn exponential(digits: &str, exponent: i32) -> String {
    let (lead, rest) = digits.split_at(1);
    let sign = if exponent < 0 { '-' } else { '+' };
    let magnitude = exponent.unsigned_abs();
    if rest.is_empty() {
        format!("{lead}e{sign}{magnitude:02}")
    } else {
        format!("{lead}.{rest}e{sign}{magnitude:02}")
    }
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
