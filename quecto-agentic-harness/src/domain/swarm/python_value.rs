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
use std::collections::HashMap;

use serde_json::{Number, Value};

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
    integral_within(float) && float as i128 == integer
}

/// Whether `float` is integral and within `[-2^63, 2^64)`, so that the
/// conversion to i128 is exact.
fn integral_within(float: f64) -> bool {
    float.is_finite() && float.fract() == 0.0 && (LOWEST..BEYOND).contains(&float)
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

/// A value's key under Python's `==` and `hash`, for the values Python
/// can hash: `None`, text, and a number, where the equal `bool`, `int` and
/// `float` (`True == 1 == 1.0`, `0 == -0.0`) share one key, as their
/// hashes do. A float no integer equals keys by its bits. A list or a
/// dict (unhashable to Python) has no key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PythonKey<'a> {
    None,
    Integer(i128),
    Float(u64),
    Text(&'a str),
}

fn python_key(value: &Value) -> Option<PythonKey<'_>> {
    match value {
        Value::Null => Some(PythonKey::None),
        Value::String(text) => Some(PythonKey::Text(text)),
        Value::Bool(_) | Value::Number(_) => match numeric(value)? {
            Numeric::Integer(integer) => Some(PythonKey::Integer(integer)),
            // NaN equals nothing, itself included: no key finds it.
            Numeric::Float(float) if float.is_nan() => None,
            Numeric::Float(float) if integral_within(float) => {
                // Exact: the range check bounds the conversion.
                Some(PythonKey::Integer(float as i128))
            }
            Numeric::Float(float) => Some(PythonKey::Float(float.to_bits())),
        },
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// The first of some values equal to a key under Python's `==`, found as
/// Python's dict finds it: by hash for a value with a [`PythonKey`], by
/// `python_equal` among the values without one (lists, dicts). So
/// `position` answers as a linear search with `python_equal` does, in
/// constant time for a hashable key.
pub struct PythonLookup<'a> {
    values: Vec<&'a Value>,
    keyed: HashMap<PythonKey<'a>, usize>,
    unkeyed: Vec<usize>,
}

impl<'a> PythonLookup<'a> {
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = &'a Value>) -> Self {
        let values: Vec<&'a Value> = values.into_iter().collect();
        let (mut keyed, mut unkeyed) = (HashMap::with_capacity(values.len()), Vec::new());
        for (index, value) in values.iter().enumerate() {
            match python_key(value) {
                Some(key) => {
                    keyed.entry(key).or_insert(index);
                }
                None => unkeyed.push(index),
            }
        }
        Self {
            values,
            keyed,
            unkeyed,
        }
    }

    /// The index of the first value equal to `key` under Python's `==`.
    /// A keyed value equals only values of the same key, and an unkeyed
    /// one only unkeyed values, so each side is searched alone.
    #[must_use]
    pub fn position(&self, key: &Value) -> Option<usize> {
        let found = match python_key(key) {
            Some(hashed) => {
                let keyed: &HashMap<PythonKey<'_>, usize> = &self.keyed;
                keyed.get(&hashed).copied()
            }
            None => self
                .unkeyed
                .iter()
                .copied()
                .find(|&index| python_equal(self.values[index], key)),
        };
        debug_assert!(found.is_none_or(|index| python_equal(self.values[index], key)));
        found
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

/// `json.dumps(text)` (#2279): the text as Python's `json` writes a `str`
/// with `ensure_ascii`, quoted with `"`. The quote and the backslash are
/// escaped, `\n`, `\r`, `\t`, `\b` and `\f` take their short forms, and
/// every other character outside the printable ASCII range (`' '` to `'~'`)
/// is written as `\u` and four lowercase hex digits, a character beyond the
/// Basic Multilingual Plane as its UTF-16 surrogate pair. No character's
/// escape depends on the Unicode version, so every Python writes the same
/// text (the `repr()` this replaces in a task's contact did not).
pub fn json_text(text: &str) -> String {
    let mut written = String::with_capacity(text.len() + 2);
    written.push('"');
    for character in text.chars() {
        match character {
            '"' => written.push_str("\\\""),
            '\\' => written.push_str("\\\\"),
            '\n' => written.push_str("\\n"),
            '\r' => written.push_str("\\r"),
            '\t' => written.push_str("\\t"),
            '\u{8}' => written.push_str("\\b"),
            '\u{c}' => written.push_str("\\f"),
            ' '..='~' => written.push(character),
            other => {
                let mut units = [0_u16; 2];
                for unit in other.encode_utf16(&mut units) {
                    written.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    written.push('"');
    debug_assert!(written.is_ascii(), "ensure_ascii writes ASCII only");
    written
}

#[cfg(test)]
#[path = "python_value_tests.rs"]
mod tests;
