//! The board's `encode()` behind the application's `BoardEncoding` port
//! (#2270): the Python-compatible codec's sorted, compact, ASCII-escaped
//! JSON, so a bound on an argument's stored size is Python's bound.
use serde_json::Value;

use crate::application::swarm::ports::BoardEncoding;
use crate::domain::swarm::BoardError;

#[derive(Clone, Copy, Debug, Default)]
pub struct PyJsonEncoding;

impl BoardEncoding for PyJsonEncoding {
    fn encode(&self, value: &Value) -> Result<String, BoardError> {
        let _ = value;
        Err(BoardError::new("not implemented yet (#2270)"))
    }
}

#[cfg(test)]
#[path = "encoding_tests.rs"]
mod tests;
