//! JSON values bound as SQL parameters the way Python's `sqlite3` binds
//! them (#2269).
//!
//! Members pass loosely typed JSON to the board, and SQLite's column
//! affinity then decides what matches: a task id given as the string `"3"`
//! finds INTEGER row 3, and `True` is stored as `1`. Binding each JSON type
//! to the SQLite type Python's `sqlite3` gives the Python value keeps those
//! comparisons identical: `None` is NULL, `bool` is INTEGER 0/1, `int` is
//! INTEGER, `float` is REAL and `str` is TEXT. Python refuses a `list` or a
//! `dict`, an `int` beyond 64 bits and a `str` holding a lone surrogate, so
//! these are errors here too (with this module's own text).

use rusqlite::types::Value;

use super::py_json::PyJson;

/// Why a JSON value cannot be bound as an SQL parameter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    /// An integer outside SQLite's 64-bit INTEGER (Python's `OverflowError`).
    #[error("Python int too large to convert to SQLite INTEGER")]
    IntegerTooLarge,
    /// A string holding a lone UTF-16 surrogate, which has no UTF-8 form
    /// (Python's `UnicodeEncodeError`).
    #[error("a string holding a lone surrogate cannot be bound")]
    LoneSurrogate,
    /// A list or an object (Python's `ProgrammingError`).
    #[error("type '{0}' is not supported")]
    Unsupported(&'static str),
}

/// The SQLite value Python's `sqlite3` binds for `value`.
///
/// # Errors
/// [`BindingError`] for a list, an object, an integer beyond i64 or a
/// string holding a lone surrogate.
pub fn bind(_value: &PyJson) -> Result<Value, BindingError> {
    Err(BindingError::Unsupported("unimplemented"))
}

/// [`bind`] for each parameter, in order.
///
/// # Errors
/// The first parameter's [`BindingError`].
pub fn bind_all(values: &[PyJson]) -> Result<Vec<Value>, BindingError> {
    values.iter().map(bind).collect()
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
