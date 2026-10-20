//! Parsing a configuration document without quoting it (#2617): every
//! configuration reader words a malformed document through the domain's
//! [`DocumentTextFailure`], as the infrastructure's parse helper does. A
//! document parses into a JSON value, so only its text can fail, and the
//! position says where.

use serde_json::Value;

use crate::domain::shared::document_failure::DocumentTextFailure;

/// `bytes` as a JSON value, or what kind of failure and where.
pub fn parse_document(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|error| describe(&error))
}

/// `error` as its fixed category and, when known, its line and column.
pub fn describe(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let failure = match error.classify() {
        Category::Eof => DocumentTextFailure::EndsEarly,
        // Parsing into a value has no schema to break, and the bytes are in
        // memory: whatever else fails is malformed text.
        Category::Syntax | Category::Io | Category::Data => DocumentTextFailure::Malformed,
    };
    let category = failure.words();
    match error.line() {
        0 => category.to_string(),
        line => format!("{category} at line {line} column {}", error.column()),
    }
}

#[cfg(test)]
#[path = "document_parse_tests.rs"]
mod tests;
