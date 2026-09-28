//! Python-compatible JSON for the swarm board (#2268).
//!
//! The board stores JSON text in its columns and later compares it byte for
//! byte (an idempotent request retry compares `requests.payload` by string),
//! and the Python implementation shares the same SQLite file. Members reach
//! the board with arbitrary Python, so a column can hold anything Python's
//! `json` writes. This codec reads and writes all of it identically:
//!
//! - [`decode`] is `json.loads`: `NaN`, `Infinity` and `-Infinity` are read,
//!   integers are kept exactly (`-0` is `0`, up to Python's 4300-digit
//!   limit), floats are parsed correctly rounded (`1e400` is `Infinity`),
//!   `\u` escapes may leave lone surrogates, and a repeated key keeps its
//!   first position with its last value.
//! - [`encode`] is `json.dumps(v, sort_keys=True, separators=(',', ':'))`,
//!   the board's `encode()`; keys sort by code point.
//! - [`dumps`] is plain `json.dumps(v)`: insertion order, `", "`/`": "`.
//!
//! Both writers escape as `ensure_ascii=True` does and write a float as
//! Python's `float.__repr__` ([`float_repr`]).
//!
//! Values are [`PyJson`], not `serde_json::Value`, which cannot hold
//! non-finite floats, big integers or lone surrogates. Conversion from a
//! `Value` is lossless; conversion to one refuses what it cannot hold.
//!
//! **Nesting.** CPython bounds `json` nesting by the C stack it has left, so
//! the limit depends on the Python version and the thread (3.14 on an 8 MiB
//! main thread: `loads` reads 52127 nested arrays or objects, `dumps` writes
//! 52126 nested lists; earlier versions stop near the 1000-frame recursion
//! limit). The codec reads up to [`DECODE_MAX_DEPTH`], at least anything a
//! Python writer can produce, and writes up to [`ENCODE_MAX_DEPTH`], so all
//! it writes it can read. Both parser and writer use explicit stacks.

mod parse;
mod value;
mod write;

pub use value::{INT_MAX_STR_DIGITS, PyInt, PyJson, PyObject, PyStr, SERDE_MAX_DEPTH};

/// The deepest nesting [`decode`] reads: CPython 3.14's `json.loads` limit.
pub const DECODE_MAX_DEPTH: usize = 52_127;

/// The deepest nesting [`encode`] and [`dumps`] write: CPython 3.14's
/// `json.dumps` limit for nested lists.
pub const ENCODE_MAX_DEPTH: usize = 52_126;

/// Why a text could not be decoded or a value could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PyJsonError {
    /// `json.JSONDecodeError`, with Python's message and position (in
    /// characters).
    #[error("{message}: line {line} column {column} (char {offset})")]
    Syntax {
        message: String,
        line: usize,
        column: usize,
        offset: usize,
    },
    /// Python's `ValueError` for an integer beyond its digit limit.
    #[error(
        "Exceeds the limit (4300 digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
    )]
    IntegerDigits { digits: usize },
    /// Text given to [`PyInt::parse`] that is not a JSON integer.
    #[error("not a JSON integer: {0:?}")]
    InvalidInteger(String),
    /// A code point above U+10FFFF given to [`PyStr::from_code_points`].
    #[error("not a Unicode code point: {0:#x}")]
    InvalidCodePoint(u32),
    /// A high surrogate followed by a low one given to
    /// [`PyStr::from_code_points`] at `index`: written out, the pair reads
    /// back as one supplementary code point.
    #[error("a high surrogate followed by a low one at index {index}")]
    JoinableSurrogates { index: usize },
    /// Nesting beyond [`DECODE_MAX_DEPTH`] or [`ENCODE_MAX_DEPTH`]
    /// (Python raises `RecursionError`).
    #[error("maximum nesting depth ({limit}) exceeded while {action}")]
    TooDeep { limit: usize, action: &'static str },
    /// A [`PyJson`] that a `serde_json::Value` cannot hold.
    #[error("not representable as a serde_json value: {0}")]
    NotRepresentable(String),
}

/// `json.loads`.
///
/// # Errors
/// [`PyJsonError::Syntax`] for text Python refuses, [`PyJsonError::IntegerDigits`]
/// for an over-long integer and [`PyJsonError::TooDeep`] beyond
/// [`DECODE_MAX_DEPTH`].
pub fn decode(text: &str) -> Result<PyJson, PyJsonError> {
    parse::parse(text)
}

/// `json.dumps(v, sort_keys=True, separators=(',', ':'))`.
///
/// # Errors
/// [`PyJsonError::TooDeep`] beyond [`ENCODE_MAX_DEPTH`].
pub fn encode(value: &PyJson) -> Result<String, PyJsonError> {
    written(value, write::Style::ENCODE)
}

/// Plain `json.dumps(v)`: insertion order, `", "` and `": "`.
///
/// # Errors
/// [`PyJsonError::TooDeep`] beyond [`ENCODE_MAX_DEPTH`].
pub fn dumps(value: &PyJson) -> Result<String, PyJsonError> {
    written(value, write::Style::DUMPS)
}

/// A float as Python's `json` writes it: `float.__repr__` for a finite
/// value, else `NaN`, `Infinity` or `-Infinity`.
///
/// The digits are `ryu`'s shortest round-trip digits (an exact tie between
/// two shortest strings goes to the even one, as in Python), laid out by
/// Python's `repr` rules: positional when the decimal exponent is in
/// `-4..16`, with `.0` for an integral value; otherwise `d.ddde±XX` with
/// the exponent padded to two digits.
pub fn float_repr(value: f64) -> String {
    write::float_repr(value)
}

fn written(value: &PyJson, style: write::Style) -> Result<String, PyJsonError> {
    let text = write::write(value, style)?;
    debug_assert!(
        decode(&text).is_ok_and(|decoded| decoded == *value),
        "Python-compatible JSON must decode back to the value it wrote: {text}"
    );
    Ok(text)
}

#[cfg(test)]
#[path = "py_json_tests.rs"]
mod tests;
