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
//!
//! Deliberate divergence (#2269 review M1): a `float` is bound as REAL, as
//! Python binds it, but where SQLite turns a REAL into TEXT (TEXT affinity
//! on store or comparison, `CAST`, `||`) the text is the linked library's.
//! The bundled SQLite (3.53) writes 17 significant digits (`1/3` is
//! `0.33333333333333332`), as Arch's system 3.53 does; Debian's 3.46.1 and
//! Ubuntu 24.04's 3.45.1 write 15 (`0.333333333333333`). Python's text thus
//! already differs between hosts, and board SQL reaches the conversion only
//! for a loosely typed float meeting a TEXT column (a float `recipient` in
//! `send`, which Python does not type-check), so the bundled conversion is
//! kept rather than emulating one host's library.
//! `binding_tests.rs` pins it.

use rusqlite::{Connection, Statement, types::Value};

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
pub fn bind(value: &PyJson) -> Result<Value, BindingError> {
    match value {
        PyJson::Null => Ok(Value::Null),
        PyJson::Bool(flag) => Ok(Value::Integer(i64::from(*flag))),
        PyJson::Int(integer) => integer
            .as_i64()
            .map(Value::Integer)
            .ok_or(BindingError::IntegerTooLarge),
        PyJson::Float(float) => Ok(Value::Real(*float)),
        PyJson::Str(text) => text
            .as_str()
            .map(|text| Value::Text(text.to_owned()))
            .ok_or(BindingError::LoneSurrogate),
        PyJson::List(_) => Err(BindingError::Unsupported("list")),
        PyJson::Object(_) => Err(BindingError::Unsupported("dict")),
    }
}

/// [`bind`] for each parameter, in order.
///
/// # Errors
/// The first parameter's [`BindingError`].
pub fn bind_all(values: &[PyJson]) -> Result<Vec<Value>, BindingError> {
    values.iter().map(bind).collect()
}

/// `sql` prepared on `connection` with `parameters` bound in order, as
/// Python's `cursor.execute(sql, parameters)` prepares and binds it. Board
/// SQL goes through here rather than rusqlite's own binding: Python checks
/// the count before binding any parameter and reports every surplus one,
/// where rusqlite's check stops counting at the first.
///
/// # Errors
/// The SQLite error preparing `sql` (one statement only);
/// [`rusqlite::Error::InvalidParameterCount`] with the true counts, which
/// the store reports as Python's "Incorrect number of bindings supplied".
pub fn bound_statement<'c>(
    connection: &'c Connection,
    sql: &str,
    parameters: &[Value],
) -> rusqlite::Result<Statement<'c>> {
    let mut statement = connection.prepare(sql)?;
    let needed = statement.parameter_count();
    if parameters.len() == needed {
        for (index, parameter) in parameters.iter().enumerate() {
            statement.raw_bind_parameter(index + 1, parameter)?;
        }
        Ok(statement)
    } else {
        Err(rusqlite::Error::InvalidParameterCount(
            parameters.len(),
            needed,
        ))
    }
}

#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
