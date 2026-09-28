//! `json.loads`, reproduced (#2268). RED stub.

use super::{PyJson, PyJsonError, PyObject};

pub(super) fn parse(text: &str) -> Result<PyJson, PyJsonError> {
    let _ = PyObject::from_unique;
    Err(PyJsonError::Syntax {
        message: format!("unimplemented ({} bytes)", text.len()),
        line: 1,
        column: 1,
        offset: 0,
    })
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
