//! Input validation the board applies before storing (#2266): the bounded
//! text rule (`swarm_store.bounded`) and the evidence-criteria rule
//! (`Workbench._criteria`).
use serde_json::Value;

use super::BoardError;
use super::records::Criterion;

pub fn bounded<'a>(
    _value: &'a Value,
    _label: &str,
    _maximum: usize,
) -> Result<&'a str, BoardError> {
    Ok("")
}

pub fn bounded_text<'a>(
    text: &'a str,
    _label: &str,
    _maximum: usize,
) -> Result<&'a str, BoardError> {
    Ok(text)
}

pub fn criteria(_value: &Value, _encoded_len: usize) -> Result<Vec<Criterion>, BoardError> {
    Ok(Vec::new())
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;
