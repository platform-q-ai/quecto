//! Describing a JSON parse failure without its content (#2617).
//!
//! serde's own messages quote the offending value (`invalid type: integer
//! `98765432123``, `unknown variant `sk-…``), and a wrong-typed field of
//! a credential or configuration file may hold part of a secret. A
//! description says what kind of failure it was and where — by field
//! path when known, else by line and column — and never what was there.

/// What kind of failure `error` is, in fixed words.
pub fn category(error: &serde_json::Error) -> &'static str {
    use serde_json::error::Category;
    match error.classify() {
        Category::Syntax => "malformed JSON",
        Category::Data => "a field has the wrong type or value",
        Category::Eof => "the file ends early",
        Category::Io => "the file could not be read",
    }
}

/// `error` as its category and position: line and column when it has
/// them (a parse of text), nothing more when it does not (a document).
pub fn describe(error: &serde_json::Error) -> String {
    match error.line() {
        0 => category(error).to_string(),
        line => format!(
            "{} at line {line} column {}",
            category(error),
            error.column()
        ),
    }
}

/// `error`, met at the field `path` (dotted, `[n]` for an index): its
/// category, the path when one is known, and the line and column when the
/// failure has them (a parse of text).
pub fn describe_at(path: &serde_path_to_error::Path, error: &serde_json::Error) -> String {
    let path = path.to_string();
    // A path with an unknown segment (`?`, e.g. a truncated map) names no
    // field: the position says where instead.
    let field = match path.as_str() {
        "" | "." => None,
        field if field.split('.').any(|segment| segment == "?") => None,
        field => Some(field),
    };
    match (field, error.line()) {
        (None, _) => describe(error),
        (Some(field), 0) => format!("{} at {field}", category(error)),
        (Some(field), line) => format!(
            "{} at {field} (line {line} column {})",
            category(error),
            error.column()
        ),
    }
}

/// `T` from `value`, a parse failure described by [`describe_at`].
pub fn from_value<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
    serde_path_to_error::deserialize(value)
        .map_err(|error| describe_at(&error.path().clone(), error.inner()))
}

/// `T` from `text`, a parse failure described by [`describe_at`].
pub fn from_str<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let parsed: T = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| describe_at(&error.path().clone(), error.inner()))?;
    deserializer.end().map_err(|error| describe(&error))?;
    Ok(parsed)
}

#[cfg(test)]
#[path = "json_failure_tests.rs"]
mod tests;
