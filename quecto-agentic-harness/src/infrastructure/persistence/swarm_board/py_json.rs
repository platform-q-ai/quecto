//! Python-compatible JSON text for the swarm board (#2268).
//!
//! The board stores JSON text in its columns and later compares it byte for
//! byte (an idempotent request retry compares `requests.payload` by string),
//! and the Python implementation shares the same SQLite file. So the text
//! this module writes is exactly the text Python's `json` module writes:
//!
//! - [`encode`] is `json.dumps(v, sort_keys=True, separators=(',', ':'))`,
//!   the board's `encode()`.
//! - [`dumps`] is plain `json.dumps(v)`: insertion order (serde_json's
//!   `preserve_order` is enabled), `", "` and `": "` separators.
//!
//! Both escape as `ensure_ascii=True` does, and write a float as Python's
//! `float.__repr__` ([`float_repr`]).
//!
//! [`decode`] is `serde_json::from_str`. It differs from `json.loads` in two
//! documented ways, neither reachable from text the board writes: it refuses
//! `NaN`/`Infinity`/`-Infinity`, and it reads an integer outside
//! `[-2**63, 2**64-1]` as a float (permitted divergence P3, epic #2265).

use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

/// `json.dumps(v, sort_keys=True, separators=(',', ':'))`.
pub fn encode(value: &Value) -> String {
    written(value, Style::ENCODE)
}

/// Plain `json.dumps(v)`: insertion order, `", "` and `": "`.
pub fn dumps(value: &Value) -> String {
    written(value, Style::DUMPS)
}

/// `json.loads`, through `serde_json::from_str` (see the module notes for
/// the two documented divergences).
///
/// # Errors
/// Returns the parse error for text that is not JSON.
pub fn decode(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(text)
}

/// A float as Python's `json` writes it: `float.__repr__` for a finite
/// value, else `NaN`, `Infinity` or `-Infinity`.
///
/// The digits are `ryu`'s shortest round-trip digits, laid out by
/// Python's `repr` rules: positional when the decimal exponent is in
/// `-4..16`, with `.0` for an integral value; otherwise `d.ddde±XX` with
/// the exponent padded to two digits.
pub fn float_repr(value: f64) -> String {
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

/// The separators and key order of one of Python's two board styles.
#[derive(Clone, Copy)]
struct Style {
    item: &'static str,
    key: &'static str,
    sort_keys: bool,
}

impl Style {
    const ENCODE: Self = Self {
        item: ",",
        key: ":",
        sort_keys: true,
    };
    const DUMPS: Self = Self {
        item: ", ",
        key: ": ",
        sort_keys: false,
    };
}

fn written(value: &Value, style: Style) -> String {
    let mut out = String::new();
    write_value(&mut out, value, style);
    debug_assert!(
        decode(&out).is_ok_and(|decoded| decoded == *value),
        "Python-compatible JSON must decode back to the value it encodes: {out}"
    );
    out
}

fn write_value(out: &mut String, value: &Value, style: Style) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => write_number(out, number),
        Value::String(text) => write_string(out, text),
        Value::Array(items) => write_array(out, items, style),
        Value::Object(map) => write_object(out, map, style),
    }
}

fn write_number(out: &mut String, number: &Number) {
    if let Some(float) = number.as_f64().filter(|_| number.is_f64()) {
        out.push_str(&float_repr(float));
    } else if let Some(signed) = number.as_i64() {
        out.push_str(&signed.to_string());
    } else if let Some(unsigned) = number.as_u64() {
        out.push_str(&unsigned.to_string());
    } else {
        // Without `arbitrary_precision` a Number is always one of the three.
        unreachable!("a serde_json Number is a float, an i64 or a u64: {number}");
    }
}

fn write_array(out: &mut String, items: &[Value], style: Style) {
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(style.item);
        }
        write_value(out, item, style);
    }
    out.push(']');
}

fn write_object(out: &mut String, map: &Map<String, Value>, style: Style) {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    if style.sort_keys {
        // Python sorts `str` keys by code point; `str`'s byte order is the
        // same order for UTF-8.
        entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    }
    out.push('{');
    for (index, (key, item)) in entries.into_iter().enumerate() {
        if index > 0 {
            out.push_str(style.item);
        }
        write_string(out, key);
        out.push_str(style.key);
        write_value(out, item, style);
    }
    out.push('}');
}

/// A string as `ensure_ascii=True` writes it: printable ASCII as is, the
/// short escapes Python uses, and `\u00e9`-style escapes (four lowercase
/// hex digits; UTF-16 surrogate pairs above U+FFFF) for everything else,
/// including DEL.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            _ => {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    write!(out, "\\u{unit:04x}").expect("writing to a String cannot fail");
                }
            }
        }
    }
    out.push('"');
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
#[path = "py_json_tests.rs"]
mod tests;
