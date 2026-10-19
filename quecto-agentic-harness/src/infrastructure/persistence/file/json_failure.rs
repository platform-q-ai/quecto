//! Describing a JSON parse failure without its content (#2617).
//!
//! serde's own messages quote the offending value (`invalid type: integer
//! `98765432123``, `unknown variant `sk-…``), and a wrong-typed field of
//! a credential or configuration file may hold part of a secret. A
//! description says what kind of failure it was and where — by field
//! path when known, else by line and column — and never what was there.
//! Only what the schema itself says crosses: the name of a missing or
//! duplicate field, and the values a variant may take; never a value or a
//! key the file holds.

/// The longest a field-path segment is printed; longer keys are cut.
const SEGMENT_CHARS: usize = 64;

/// What kind of failure `error` is, in fixed words (plus, for a missing
/// or duplicate field, the schema's name for it, and for an unknown
/// variant, the schema's allowed values).
pub fn category(error: &serde_json::Error) -> String {
    use serde_json::error::Category;
    match error.classify() {
        Category::Syntax => "malformed JSON".to_string(),
        Category::Eof => "the file ends early".to_string(),
        Category::Io => "the file could not be read".to_string(),
        Category::Data => data_category(&error.to_string()),
    }
}

/// A data failure's kind from serde's message, by an allowlist of its
/// fixed prefixes; anything else is "a field has the wrong type or value".
fn data_category(message: &str) -> String {
    // serde names the field from the schema: "missing field `token`".
    for prefix in ["missing field `", "duplicate field `"] {
        if let Some(rest) = message.strip_prefix(prefix)
            && let Some((name, _)) = rest.split_once('`')
        {
            let kind = prefix.trim_end_matches(" `").trim_end_matches('`');
            return format!("{kind} `{}`", escaped(name));
        }
    }
    if message.starts_with("unknown field `") {
        // The key is the file's own: the path names where it is.
        return "unknown field".to_string();
    }
    if message.starts_with("unknown variant `") {
        // "unknown variant `<the file's value>`, expected <the schema's>":
        // the schema's list is the last part, and has no "`, expected ".
        return match message.rsplit_once("`, expected ") {
            Some((_, expected)) => {
                format!("unknown variant, expected {}", strip_position(expected))
            }
            None => "unknown variant".to_string(),
        };
    }
    "a field has the wrong type or value".to_string()
}

/// serde appends " at line L column C" to a parse-of-text message; the
/// description places it itself.
fn strip_position(text: &str) -> &str {
    match text.rfind(" at line ") {
        Some(at) => &text[..at],
        None => text,
    }
}

/// `text` printable: control characters (an ANSI escape) escaped, and cut
/// to [`SEGMENT_CHARS`].
fn escaped(text: &str) -> String {
    let mut out: String = text
        .chars()
        .flat_map(char::escape_debug)
        .take(SEGMENT_CHARS)
        .collect();
    if text
        .chars()
        .flat_map(char::escape_debug)
        .nth(SEGMENT_CHARS)
        .is_some()
    {
        out.push('…');
    }
    out
}

/// The field `path` names, dotted with `[n]` for an index and each key
/// escaped; `None` at the root, or when a segment is unknown (the
/// position says where then).
fn field(path: &serde_path_to_error::Path) -> Option<String> {
    use serde_path_to_error::Segment;
    let mut out = String::new();
    for segment in path.iter() {
        match segment {
            Segment::Seq { index } => out.push_str(&format!("[{index}]")),
            Segment::Map { key } => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(&escaped(key));
            }
            Segment::Enum { variant } => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(&escaped(variant));
            }
            Segment::Unknown => return None,
        }
    }
    Some(out).filter(|field| !field.is_empty())
}

/// `error` as its category and position: line and column when it has
/// them (a parse of text), nothing more when it does not (a document).
pub fn describe(error: &serde_json::Error) -> String {
    match error.line() {
        0 => category(error),
        line => format!(
            "{} at line {line} column {}",
            category(error),
            error.column()
        ),
    }
}

/// `error`, met at the field `path`: its category, the field when one is
/// known, and the line and column when the failure has them.
pub fn describe_at(path: &serde_path_to_error::Path, error: &serde_json::Error) -> String {
    match (field(path), error.line()) {
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
