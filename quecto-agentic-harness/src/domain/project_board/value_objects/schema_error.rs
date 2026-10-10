//! Why a task was refused, and the bounded-text checks every field shares.
use regex::Regex;
use std::fmt;
use std::sync::LazyLock;

/// A task that breaks the schema: the field and what it must be. `field`
/// is the full path to the value, keys and list indexes joined by `/`
/// (`depends_on/3`, `claim/holder/email`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub field: String,
    pub problem: String,
}

impl SchemaError {
    pub fn new(field: impl Into<String>, problem: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            problem: problem.into(),
        }
    }

    /// The same refusal, for the value at `field`.
    pub fn at(self, field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            ..self
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.problem)
    }
}

impl std::error::Error for SchemaError {}

/// Letters, marks, numbers, punctuation, symbols and the space separators
/// (Unicode L*, M*, N*, P*, S*, Zs): no controls, format characters (bidi
/// overrides, zero-width characters, the BOM), private-use or unassigned.
static ON_ONE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}\p{Zs}]*$").expect("text allowlist"));
/// The same, plus `\n` between lines (CRLF is converted at the board's entry).
static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}\p{Zs}\n]*$").expect("markdown allowlist")
});
/// Something to read: a letter, number, punctuation mark or symbol.
static VISIBLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}\p{P}\p{S}]").expect("visible allowlist"));

/// Single-line text with something visible, at most `max_chars` characters.
pub fn line(field: &str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    let fits = text.chars().count() <= max_chars;
    if fits && ON_ONE_LINE.is_match(text) && VISIBLE.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be 1 to {max_chars} visible characters on one line"),
        ))
    }
}

/// Markdown of at most `max_chars` characters; it may be empty.
pub fn markdown(field: &str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    if text.chars().count() <= max_chars && MARKDOWN.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be at most {max_chars} characters of text and newlines"),
        ))
    }
}

/// At most `max` entries, no two equal.
pub fn distinct<T: Ord>(field: &str, entries: &[T], max: usize) -> Result<(), SchemaError> {
    let unique: std::collections::BTreeSet<&T> = entries.iter().collect();
    if entries.len() <= max && unique.len() == entries.len() {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must hold at most {max} distinct entries"),
        ))
    }
}

#[cfg(test)]
#[path = "schema_error_tests.rs"]
mod tests;
