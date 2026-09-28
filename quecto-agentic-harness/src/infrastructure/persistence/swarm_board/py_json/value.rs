//! The value model of board JSON: everything Python's `json` module reads
//! and writes, including what `serde_json::Value` cannot hold (`NaN` and
//! `±Infinity`, integers of any size up to Python's 4300-digit limit,
//! strings with lone UTF-16 surrogates, nesting beyond 128 levels).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Number, Value};

use super::PyJsonError;

/// CPython's `sys.int_info.default_max_str_digits`: `int()` refuses decimal
/// text with more digits (the sign is not counted), and so does
/// `json.loads`.
pub const INT_MAX_STR_DIGITS: usize = 4300;

/// The nesting serde_json's own parser allows. Conversions to and from
/// `serde_json::Value` stay within it, so they need no explicit stack.
pub const SERDE_MAX_DEPTH: usize = 128;

/// A JSON value as Python's `json.loads` returns it.
///
/// Equality is structural identity: objects compare by key regardless of
/// order, an integer never equals a float (`5 != 5.0`), floats compare by
/// bits (`-0.0 != 0.0`) except that every `NaN` equals every `NaN`. Two
/// equal values therefore `encode` to the same text.
///
/// Dropping, comparing and formatting (`Debug` prints the `dumps` text) use
/// explicit stacks, so a value nested as deep as the codec allows never
/// overflows the thread's stack.
pub enum PyJson {
    Null,
    Bool(bool),
    Int(PyInt),
    Float(f64),
    Str(PyStr),
    List(Vec<PyJson>),
    Object(PyObject),
}

/// An exact Python `int`, kept as canonical decimal text (`-0` is `0`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PyInt {
    decimal: String,
}

impl PyInt {
    /// Reads JSON integer text (`-?(0|[1-9][0-9]*)`) exactly, as Python's
    /// `int()` does for `json.loads`.
    ///
    /// # Errors
    /// [`PyJsonError::InvalidInteger`] for other text, and
    /// [`PyJsonError::IntegerDigits`] beyond [`INT_MAX_STR_DIGITS`] digits.
    pub fn parse(text: &str) -> Result<Self, PyJsonError> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(digits) => (true, digits),
            None => (false, text),
        };
        let well_formed = digits == "0"
            || (digits.starts_with(|first: char| ('1'..='9').contains(&first))
                && digits.bytes().all(|byte| byte.is_ascii_digit()));
        if !well_formed {
            return Err(PyJsonError::InvalidInteger(text.to_owned()));
        }
        if digits.len() > INT_MAX_STR_DIGITS {
            return Err(PyJsonError::IntegerDigits {
                digits: digits.len(),
            });
        }
        let decimal = if negative && digits != "0" {
            format!("-{digits}")
        } else {
            digits.to_owned()
        };
        Ok(Self { decimal })
    }

    /// The canonical decimal text, as Python's `json` writes it.
    pub fn as_str(&self) -> &str {
        &self.decimal
    }

    /// The value when it fits an `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        self.decimal.parse().ok()
    }

    /// The value when it fits a `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        self.decimal.parse().ok()
    }
}

impl From<i64> for PyInt {
    fn from(value: i64) -> Self {
        Self {
            decimal: value.to_string(),
        }
    }
}

impl From<u64> for PyInt {
    fn from(value: u64) -> Self {
        Self {
            decimal: value.to_string(),
        }
    }
}

/// A Python `str`: Unicode code points, which may include lone UTF-16
/// surrogates (`"\ud800"` from `json.loads`) that a Rust `String` cannot
/// hold. Ordered by code point, as Python orders `str`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyStr(Repr);

/// `CodePoints` is used only when the string holds a surrogate, so each
/// string has exactly one representation and the derived `Eq`/`Hash` hold.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Repr {
    Text(String),
    CodePoints(Vec<u32>),
}

impl PyStr {
    /// A string from code points, surrogates included.
    ///
    /// # Errors
    /// [`PyJsonError::InvalidCodePoint`] for a value above U+10FFFF.
    pub fn from_code_points(points: Vec<u32>) -> Result<Self, PyJsonError> {
        if let Some(&invalid) = points.iter().find(|&&point| point > 0x10_FFFF) {
            return Err(PyJsonError::InvalidCodePoint(invalid));
        }
        let text: Option<String> = points.iter().map(|&point| char::from_u32(point)).collect();
        Ok(match text {
            Some(text) => Self(Repr::Text(text)),
            None => Self(Repr::CodePoints(points)),
        })
    }

    /// The text, when the string holds no lone surrogate.
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            Repr::Text(text) => Some(text),
            Repr::CodePoints(_) => None,
        }
    }

    /// The code points in order.
    pub fn code_points(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match &self.0 {
            Repr::Text(text) => Box::new(text.chars().map(u32::from)),
            Repr::CodePoints(points) => Box::new(points.iter().copied()),
        }
    }
}

impl From<&str> for PyStr {
    fn from(text: &str) -> Self {
        Self(Repr::Text(text.to_owned()))
    }
}

impl From<String> for PyStr {
    fn from(text: String) -> Self {
        Self(Repr::Text(text))
    }
}

impl Ord for PyStr {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            // UTF-8 byte order is code point order.
            (Repr::Text(left), Repr::Text(right)) => left.cmp(right),
            _ => self.code_points().cmp(other.code_points()),
        }
    }
}

impl PartialOrd for PyStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for PyStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        super::write::write_str(&mut text, self);
        formatter.write_str(&text)
    }
}

/// A Python `dict` with `str` keys: insertion order, unique keys; setting
/// an existing key replaces its value in place, as `d[k] = v` does.
#[derive(Default)]
pub struct PyObject {
    entries: Vec<(PyStr, PyJson)>,
}

impl PyObject {
    /// An empty object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key`, keeping its position when present; returns the value it
    /// replaced.
    pub fn insert(&mut self, key: PyStr, value: PyJson) -> Option<PyJson> {
        match self.entries.iter_mut().find(|(present, _)| *present == key) {
            Some((_, slot)) => Some(std::mem::replace(slot, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    /// The value under `key`.
    pub fn get(&self, key: &PyStr) -> Option<&PyJson> {
        self.entries
            .iter()
            .find(|(present, _)| present == key)
            .map(|(_, value)| value)
    }

    /// The entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&PyStr, &PyJson)> {
        self.entries.iter().map(|(key, value)| (key, value))
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the object has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// An object from entries whose keys the caller has made unique.
    pub(super) fn from_unique(entries: Vec<(PyStr, PyJson)>) -> Self {
        debug_assert!(
            {
                let mut keys: Vec<&PyStr> = entries.iter().map(|(key, _)| key).collect();
                keys.sort_unstable();
                keys.windows(2).all(|pair| pair[0] != pair[1])
            },
            "object keys are unique"
        );
        Self { entries }
    }
}

impl Drop for PyJson {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_children(self, &mut pending);
        while let Some(mut node) = pending.pop() {
            take_children(&mut node, &mut pending);
        }
    }
}

/// Moves a container's children out, so dropping it recurses no further.
fn take_children(node: &mut PyJson, pending: &mut Vec<PyJson>) {
    match node {
        PyJson::List(items) => pending.append(items),
        PyJson::Object(object) => pending.extend(
            std::mem::take(&mut object.entries)
                .into_iter()
                .map(|(_, value)| value),
        ),
        PyJson::Null | PyJson::Bool(_) | PyJson::Int(_) | PyJson::Float(_) | PyJson::Str(_) => {}
    }
}

impl PartialEq for PyJson {
    fn eq(&self, other: &Self) -> bool {
        let mut pending = vec![(self, other)];
        while let Some((left, right)) = pending.pop() {
            let same = match (left, right) {
                (Self::Null, Self::Null) => true,
                (Self::Bool(left), Self::Bool(right)) => left == right,
                (Self::Int(left), Self::Int(right)) => left == right,
                (Self::Float(left), Self::Float(right)) => {
                    left.to_bits() == right.to_bits() || (left.is_nan() && right.is_nan())
                }
                (Self::Str(left), Self::Str(right)) => left == right,
                (Self::List(left), Self::List(right)) => {
                    pending.extend(left.iter().zip(right));
                    left.len() == right.len()
                }
                (Self::Object(left), Self::Object(right)) => {
                    same_keys_pending(left, right, &mut pending)
                }
                _ => false,
            };
            if !same {
                return false;
            }
        }
        true
    }
}

/// Whether two objects have the same keys; queues their paired values.
fn same_keys_pending<'a>(
    left: &'a PyObject,
    right: &'a PyObject,
    pending: &mut Vec<(&'a PyJson, &'a PyJson)>,
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let index: HashMap<&PyStr, &PyJson> = right.iter().collect();
    for (key, value) in left.iter() {
        match index.get(key) {
            Some(other) => pending.push((value, other)),
            None => return false,
        }
    }
    true
}

impl fmt::Debug for PyJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match super::dumps(self) {
            Ok(text) => formatter.write_str(&text),
            Err(error) => write!(formatter, "<unwritable PyJson: {error}>"),
        }
    }
}

impl PyJson {
    /// The equivalent `serde_json::Value`. This direction is lossy in
    /// general, so it refuses rather than approximates: a caller that
    /// writes a value back to the board must keep the `PyJson`.
    ///
    /// # Errors
    /// [`PyJsonError::NotRepresentable`] for `NaN`/`±Infinity`, an integer
    /// outside `i64`/`u64`, a string with a lone surrogate, or nesting
    /// deeper than [`SERDE_MAX_DEPTH`].
    pub fn to_value(&self) -> Result<Value, PyJsonError> {
        let _ = self;
        Err(PyJsonError::NotRepresentable("unimplemented".to_owned()))
    }
}

impl TryFrom<&Value> for PyJson {
    type Error = PyJsonError;

    /// Lossless for every `Value` nested at most [`SERDE_MAX_DEPTH`] deep.
    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let _ = (value, Map::<String, Value>::new(), Number::from(0));
        Err(PyJsonError::NotRepresentable("unimplemented".to_owned()))
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
