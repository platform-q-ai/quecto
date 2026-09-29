//! The board's `encode()` behind the application's `BoardEncoding` port
//! (#2270): the Python-compatible codec's sorted, compact, ASCII-escaped
//! JSON, so a bound on an argument's stored size is Python's bound.
use serde_json::Value;

use super::py_json::{self, PyJson};
use crate::application::swarm::ports::BoardEncoding;
use crate::domain::swarm::{BoardError, RefusalKind};

#[derive(Clone, Copy, Debug, Default)]
pub struct PyJsonEncoding;

impl BoardEncoding for PyJsonEncoding {
    fn encode(&self, value: &Value) -> Result<String, BoardError> {
        PyJson::try_from(value)
            .and_then(|value| py_json::encode(&value))
            .map_err(|error| BoardError::new(RefusalKind::Invalid, error.to_string()))
    }
}

#[cfg(test)]
#[path = "encoding_tests.rs"]
mod tests;
