//! Parsing a configuration document without quoting it (#2617): the one
//! wording every configuration reader gives a malformed document, the same
//! fixed categories the infrastructure's parse helper
//! (`infrastructure::persistence::file::json_failure`) gives, pinned
//! equal by its tests. A document parses into a JSON value, so only its
//! syntax can fail, and the position says where.

use serde_json::Value;

/// `bytes` as a JSON value, or what kind of failure and where.
pub fn parse_document(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes).map_err(|error| describe(&error))
}

/// `error` as its fixed category and, when known, its line and column.
pub fn describe(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let category = match error.classify() {
        Category::Syntax => "malformed JSON",
        Category::Eof => "the file ends early",
        Category::Io => "the file could not be read",
        Category::Data => "a field has the wrong type or value",
    };
    match error.line() {
        0 => category.to_string(),
        line => format!("{category} at line {line} column {}", error.column()),
    }
}

#[cfg(test)]
#[path = "document_parse_tests.rs"]
mod tests;
