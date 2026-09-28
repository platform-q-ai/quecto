//! The board's idempotency ledger and event log (#2269): `Store.retry` and
//! `Store.event` from `swarm_helpers/swarm_store.py`, run inside a
//! [`BoardStore`](super::store::BoardStore) transaction. Stored JSON is
//! written with the Python-compatible codec, so a Python and a Rust client
//! replay each other's requests.

use rusqlite::{Connection, types::ValueRef};

use super::py_json::{self, PyJson};
use super::store::{CONTENDED, TransactionError};

/// The most requests the idempotency ledger holds (`Store.retry`).
pub const REQUEST_LEDGER_CAPACITY: i64 = 10_000;

/// The longest request id, in UTF-8 bytes (`bounded(request, 'request id', 128)`).
pub const REQUEST_ID_MAX_BYTES: usize = 128;

/// `Store.retry`: the idempotency ledger. A request seen before replays its
/// stored result when the payload matches; a new one runs `action` and
/// records its result, while the ledger has room.
///
/// # Errors
/// A board refusal for an empty or over-long request id, a reused id with
/// a different payload, or a full ledger; `action`'s error; SQLite errors.
pub fn retry(
    transaction: &Connection,
    actor: &str,
    request: &str,
    payload: &PyJson,
    action: impl FnOnce() -> Result<PyJson, TransactionError>,
) -> Result<PyJson, TransactionError> {
    bounded_request(request)?;
    let payload = encoded(payload)?;
    let stored = transaction
        .prepare("SELECT * FROM requests WHERE actor=? AND request=?")?
        .query_row([actor, request], |row| {
            Ok(replayed(
                row.get_ref("payload")?,
                row.get_ref("result")?,
                &payload,
            ))
        });
    match stored {
        Ok(replay) => replay,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            let held: i64 =
                transaction.query_row("SELECT count(*) FROM requests", [], |row| row.get(0))?;
            if held < REQUEST_LEDGER_CAPACITY {
                let result = action()?;
                transaction.execute(
                    "INSERT INTO requests VALUES(?,?,?,?)",
                    [actor, request, payload.as_str(), encoded(&result)?.as_str()],
                )?;
                Ok(result)
            } else {
                Err(TransactionError::board(format!(
                    "coordination request ledger full ({REQUEST_LEDGER_CAPACITY})"
                )))
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// A stored request: its result, as `json.loads(old['result'])` reads it,
/// when the payload text is the same.
fn replayed(
    stored_payload: ValueRef<'_>,
    stored_result: ValueRef<'_>,
    payload: &str,
) -> Result<PyJson, TransactionError> {
    // Python's sqlite3 decodes the whole row before either is compared.
    let stored_payload = Stored::read("payload", stored_payload)?;
    let stored_result = Stored::read("result", stored_result)?;
    match stored_payload {
        Stored::Text(stored) if stored == payload => stored_result.loads(),
        _ => Err(TransactionError::board(
            "request id reused with different payload",
        )),
    }
}

/// A column as Python's `sqlite3` returns it: `str`, `bytes`, `int`,
/// `float` or `None`.
enum Stored<'a> {
    Text(&'a str),
    Blob(&'a [u8]),
    Other(&'static str),
}

impl<'a> Stored<'a> {
    /// The column's Python value. A TEXT that is not UTF-8 fails the fetch
    /// with Python's `OperationalError`, which the transaction reports as
    /// contended; Python formats the C string, so it ends at a NUL.
    fn read(column: &str, value: ValueRef<'a>) -> Result<Self, TransactionError> {
        match value {
            ValueRef::Text(bytes) => std::str::from_utf8(bytes).map(Self::Text).map_err(|_| {
                let text = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
                TransactionError::board(format!(
                    "{CONTENDED}: Could not decode to UTF-8 column '{column}' with text '{}'",
                    replaced_per_byte(text)
                ))
            }),
            ValueRef::Blob(bytes) => Ok(Self::Blob(bytes)),
            ValueRef::Integer(_) => Ok(Self::Other("int")),
            ValueRef::Real(_) => Ok(Self::Other("float")),
            ValueRef::Null => Ok(Self::Other("NoneType")),
        }
    }

    /// `json.loads(value)`.
    fn loads(self) -> Result<PyJson, TransactionError> {
        let text = match self {
            Self::Text(text) => text,
            Self::Blob(bytes) => utf8_json(bytes)?,
            Self::Other(kind) => {
                return Err(TransactionError::board(format!(
                    "the JSON object must be str, bytes or bytearray, not {kind}"
                )));
            }
        };
        py_json::decode(text).map_err(|error| TransactionError::board(error.to_string()))
    }
}

/// `bytes` as CPython's error message reads them: valid UTF-8 as itself and
/// one U+FFFD for every byte of an invalid sequence (not one per maximal
/// subpart, as `String::from_utf8_lossy` replaces).
fn replaced_per_byte(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                return text;
            }
            Err(error) => {
                let (valid, invalid) = rest.split_at(error.valid_up_to());
                // The first `valid_up_to` bytes are UTF-8 by definition.
                text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                let skipped = error.error_len().unwrap_or(invalid.len());
                debug_assert!(skipped > 0, "an invalid sequence has at least one byte");
                text.extend(std::iter::repeat_n('\u{fffd}', skipped));
                rest = &invalid[skipped..];
            }
        }
    }
}

/// The text `json.loads(bytes)` decodes, for bytes its `detect_encoding`
/// reads as UTF-8 (a UTF-8 BOM is dropped). Bytes it reads as UTF-16 or
/// UTF-32, or UTF-8 holding an encoded surrogate, are refused here with
/// this module's own text (a permitted divergence, epic #2265 P3).
///
/// Permitted divergence (#2269 review N2): Python raises a non-`SwarmError`
/// for a BLOB it cannot read, with other text: for a double UTF-8 BOM
/// (only one goes with the encoding) `JSONDecodeError` "Expecting value:
/// line 1 column 1 (char 0)", where [`py_json::decode`] reports the second
/// BOM as `json.loads(str)` does, and for invalid UTF-8 `UnicodeDecodeError`
/// "'utf-8' codec can't decode byte ...", where this is "the stored result
/// is not UTF-8 JSON". `ledger_tests.rs` pins both.
fn utf8_json(bytes: &[u8]) -> Result<&str, TransactionError> {
    let wide = bytes.starts_with(b"\xff\xfe")
        || bytes.starts_with(b"\xfe\xff")
        || bytes.starts_with(b"\x00\x00\xfe\xff")
        || match bytes {
            [first, second, _, _, ..] => *first == 0 || *second == 0,
            [first, second] => *first == 0 || *second == 0,
            _ => false,
        };
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match (wide, std::str::from_utf8(text)) {
        (false, Ok(text)) => Ok(text),
        _ => Err(TransactionError::board(
            "the stored result is not UTF-8 JSON",
        )),
    }
}

/// `Store.event`: one `events` row with the encoded detail.
///
/// # Errors
/// A board refusal for a detail too deep to encode; SQLite errors.
pub fn event(
    transaction: &Connection,
    actor: &str,
    clock_now: f64,
    action: &str,
    detail: &PyJson,
) -> Result<(), TransactionError> {
    transaction.execute(
        "INSERT INTO events(actor,time,action,detail) VALUES(?,?,?,?)",
        rusqlite::params![actor, clock_now, action, encoded(detail)?],
    )?;
    Ok(())
}

/// The board's `encode()`.
fn encoded(value: &PyJson) -> Result<String, TransactionError> {
    py_json::encode(value).map_err(|error| TransactionError::board(error.to_string()))
}

/// `bounded(request, 'request id', 128)`: nonblank by Python's `str.strip`
/// and at most 128 UTF-8 bytes.
fn bounded_request(request: &str) -> Result<(), TransactionError> {
    if has_content(request) && request.len() <= REQUEST_ID_MAX_BYTES {
        Ok(())
    } else {
        Err(TransactionError::board(format!(
            "request id must be nonempty and at most {REQUEST_ID_MAX_BYTES} bytes"
        )))
    }
}

/// Whether `text.strip()` leaves anything, by Python's whitespace.
fn has_content(text: &str) -> bool {
    text.chars().any(|c| !python_whitespace(c))
}

/// Python's `str.isspace`: Unicode White_Space plus the separators
/// U+001C..U+001F, which Python also strips.
fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
