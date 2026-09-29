//! Python's `==` and truthiness over the values the board compares (#2271
//! round-1 review M1, M2): a stored cell as Python's `sqlite3` fetches it
//! (`None`, `int`, `float`, `str`) and a member's argument as `json.loads`
//! gives it (`None`, `bool`, `int`, `float`, `str`, `list`, `dict`), both
//! as JSON values. The board's refusals turn on these comparisons
//! (`row['reservation'] != reservation`, `(row['pid'], row['started']) !=
//! (pid, started)`, `reservation or uuid4().hex`), so they must answer
//! exactly as Python does.
//!
//! This models Python's *semantics* over values already read; the board's
//! Python-compatible JSON *text* (reading and writing columns byte for byte,
//! with values a `serde_json::Value` cannot hold) is
//! `infrastructure::persistence::swarm_board::py_json`. The two are separate
//! because the domain must not depend on that infrastructure codec. These
//! comparisons are only as exact as their inputs: an argument must be decoded
//! by `py_json` (correctly rounded floats), not by `serde_json`, before it
//! reaches them.
use serde_json::{Number, Value};

use super::python_unassigned::UNASSIGNED;

/// A number as Python holds it: `bool` and `int` are exact integers (the
/// argument's range is i64 ∪ u64), `float` a double.
#[derive(Clone, Copy)]
enum Numeric {
    Integer(i128),
    Float(f64),
}

fn numeric(value: &Value) -> Option<Numeric> {
    match value {
        Value::Bool(flag) => Some(Numeric::Integer(i128::from(*flag))),
        Value::Number(number) => Some(number_value(number)),
        Value::Null | Value::String(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn number_value(number: &Number) -> Numeric {
    match (number.as_i64(), number.as_u64(), number.as_f64()) {
        (Some(integer), _, _) => Numeric::Integer(i128::from(integer)),
        (None, Some(integer), _) => Numeric::Integer(i128::from(integer)),
        (None, None, Some(float)) => Numeric::Float(float),
        // serde_json holds every number as one of the three.
        (None, None, None) => Numeric::Float(f64::NAN),
    }
}

/// The bounds of the integers an argument holds, i64 ∪ u64, as doubles:
/// -2^63 and 2^64, both exact. A float outside `[-2^63, 2^64)` equals no
/// such integer.
const LOWEST: f64 = -9_223_372_036_854_775_808.0;
const BEYOND: f64 = 18_446_744_073_709_551_616.0;

/// Python's exact `int == float`: the float is integral and is that
/// integer. The range check comes first, so a float beyond the integers
/// (2^63 against i64::MAX, say) is never saturated into one; within it
/// the conversion to i128 is exact.
fn integer_equals_float(integer: i128, float: f64) -> bool {
    let integral = float.is_finite() && float.fract() == 0.0;
    let within = (LOWEST..BEYOND).contains(&float);
    integral && within && float as i128 == integer
}

fn numbers_equal(left: Numeric, right: Numeric) -> bool {
    match (left, right) {
        (Numeric::Integer(left), Numeric::Integer(right)) => left == right,
        (Numeric::Float(left), Numeric::Float(right)) => left == right,
        (Numeric::Integer(integer), Numeric::Float(float))
        | (Numeric::Float(float), Numeric::Integer(integer)) => {
            integer_equals_float(integer, float)
        }
    }
}

/// Python's `left == right`: `None` equals only `None`; `bool`, `int` and
/// `float` compare as numbers (`True == 1 == 1.0`); text equals only the
/// same text (`'5' != 5`); a list equals a list of equal items, a dict a
/// dict of the same keys with equal values.
pub fn python_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| python_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, value)| {
                    right
                        .get(key)
                        .is_some_and(|other| python_equal(value, other))
                })
        }
        (Value::Bool(_) | Value::Number(_), Value::Bool(_) | Value::Number(_)) => {
            match (numeric(left), numeric(right)) {
                (Some(left), Some(right)) => numbers_equal(left, right),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Python's `bool(value)`: `None`, `False`, a zero, and an empty text,
/// list or dict are false; everything else is true.
pub fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => match number_value(number) {
            Numeric::Integer(integer) => integer != 0,
            Numeric::Float(float) => float != 0.0,
        },
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

/// Python's `repr()` of a `str` (#2277): quoted with `'`, or with `"`
/// when the text holds a `'` and no `"`; a backslash, the quote, `\t`,
/// `\n` and `\r` escaped; every other character Python does not print
/// written as `\xhh`, `\uhhhh` or `\Uhhhhhhhh`. Printability follows
/// Python 3.14's `str.isprintable` ([`not_printed`]): the code points it
/// refuses whatever its Unicode version, and those Unicode 16.0, its
/// version, has not assigned (#2277 review L2).
pub fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut written = String::with_capacity(text.len() + 2);
    written.push(quote);
    for character in text.chars() {
        match character {
            '\\' => written.push_str("\\\\"),
            '\t' => written.push_str("\\t"),
            '\n' => written.push_str("\\n"),
            '\r' => written.push_str("\\r"),
            c if c == quote => {
                written.push('\\');
                written.push(c);
            }
            c if not_printed(c) => {
                let code = u32::from(c);
                let escaped = match code {
                    0..=0xff => format!("\\x{code:02x}"),
                    0x100..=0xffff => format!("\\u{code:04x}"),
                    _ => format!("\\U{code:08x}"),
                };
                written.push_str(&escaped);
            }
            c => written.push(c),
        }
    }
    written.push(quote);
    written
}

/// The code points Python 3.14's `str.isprintable` refuses: those that do
/// not depend on its Unicode version, listed here (the controls (Cc), the
/// separators other than the space (Zs, Zl, Zp), the format characters
/// (Cf) and the private-use planes (Co), and the noncharacters of planes
/// 15 and 16), and those Unicode 16.0 has not assigned ([`unassigned`]).
/// Swept against Python's own answer for every code point by
/// `not_printed_is_exactly_what_python_does_not_print`.
fn not_printed(character: char) -> bool {
    version_independent(character) || unassigned(character)
}

/// Whether Unicode 16.0 (Python 3.14's `unicodedata`) has not assigned
/// `character`: a binary search of the generated [`UNASSIGNED`] ranges.
fn unassigned(character: char) -> bool {
    let point = u32::from(character);
    debug_assert!(
        UNASSIGNED.len() % 2 == 0,
        "the table holds (first, last) pairs"
    );
    // The bounds are sorted (each range's first, then its last, below the
    // next range's first). The first bound not below `point` is a range's
    // last (an odd index) only when `point` is inside that range, or its
    // first only when that first is `point`.
    let at = UNASSIGNED.partition_point(|&bound| bound < point);
    at % 2 == 1 || UNASSIGNED.get(at) == Some(&point)
}

/// The code points `str.isprintable` refuses in every Unicode version.
fn version_independent(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00..=0x1f
            | 0x7f..=0xa0
            | 0xad
            | 0x0600..=0x0605
            | 0x061c
            | 0x06dd
            | 0x070f
            | 0x0890..=0x0891
            | 0x08e2
            | 0x1680
            | 0x180e
            | 0x2000..=0x200f
            | 0x2028..=0x202f
            | 0x205f..=0x2064
            | 0x2066..=0x206f
            | 0x3000
            | 0xe000..=0xf8ff
            | 0xfeff
            | 0xfff9..=0xfffb
            | 0x110bd
            | 0x110cd
            | 0x13430..=0x1343f
            | 0x1bca0..=0x1bca3
            | 0x1d173..=0x1d17a
            | 0xe0001
            | 0xe0020..=0xe007f
            | 0xf0000..=0x10ffff
    )
}

#[cfg(test)]
#[path = "python_value_tests.rs"]
mod tests;
