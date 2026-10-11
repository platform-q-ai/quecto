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

/// One visible character (Unicode L*, M*, N*, P*, S*), or an emoji tag
/// sequence: a black flag U+1F3F4, then tag characters U+E0020 to U+E007E,
/// ended by the cancel tag U+E007F (subdivision flags such as England's).
const GLYPH: &str = r"(?:[\p{L}\p{M}\p{N}\p{P}\p{S}]|\x{1F3F4}[\x{E0020}-\x{E007E}]+\x{E007F})";
/// ZWNJ U+200C and ZWJ U+200D, each only ever between two glyphs (👩‍💻,
/// 🏳️‍🌈, Persian ZWNJ). Every other format character (bidi marks such as
/// LRM, RLM and ALM, the soft hyphen, zero-width spaces, the BOM), every
/// control, private-use and unassigned character is refused.
const JOINER: &str = r"[\x{200C}\x{200D}]";

/// Glyphs, possibly joined, and the given separators.
fn text(separators: &str) -> Regex {
    Regex::new(&format!("^(?:{GLYPH}(?:{JOINER}{GLYPH})*|{separators})*$")).expect("text allowlist")
}

/// Single-line text: glyphs and the space separators (Unicode Zs).
static ON_ONE_LINE: LazyLock<Regex> = LazyLock::new(|| text(r"\p{Zs}"));
/// The same, plus `\n` between lines and `\t` (CRLF is converted at the
/// board's entry).
static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| text(r"[\p{Zs}\n\t]"));
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
            format!("must be at most {max_chars} characters of text, newlines and tabs"),
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
