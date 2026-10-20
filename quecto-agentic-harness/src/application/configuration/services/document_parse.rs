//! Parsing a configuration document without quoting it (#2617): every
//! configuration reader words a malformed document through
//! [`DocumentTextFailure`], as the infrastructure's parse helper does. A
//! document parses into a JSON value, so only its text can fail, and the
//! position says where.

use serde_json::Value;

use crate::application::configuration::dto::DocumentTextFailure;

/// `bytes` as a JSON value, or what kind of failure and where.
pub fn parse_document(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|error| describe(&error))
}

/// `error` as its fixed category and, when known, its line and column.
pub fn describe(error: &serde_json::Error) -> String {
    let category = DocumentTextFailure::of(error).words();
    match error.line() {
        0 => category.to_string(),
        line => format!("{category} at line {line} column {}", error.column()),
    }
}

#[cfg(test)]
#[path = "document_parse_tests.rs"]
mod tests;
