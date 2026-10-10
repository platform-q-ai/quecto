//! Why a task was refused, and the bounded-text checks every field shares.
use std::fmt;

/// A task that breaks the schema: the field and what it must be. `field`
/// is the JSON key, nested keys joined by `/` (`team/roles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub field: &'static str,
    pub problem: String,
}

impl SchemaError {
    pub fn new(field: &'static str, problem: impl Into<String>) -> Self {
        Self {
            field,
            problem: problem.into(),
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.problem)
    }
}

impl std::error::Error for SchemaError {}

/// Shown on one line: ASCII graphic or space, or any non-ASCII character
/// that is neither a control nor a line or paragraph separator.
fn on_one_line(c: char) -> bool {
    c == ' '
        || c.is_ascii_graphic()
        || (!c.is_ascii() && !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}'))
}

/// Non-blank single-line text of at most `max_chars` characters.
pub fn line(field: &'static str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    let fits = !text.trim().is_empty() && text.chars().count() <= max_chars;
    if fits && text.chars().all(on_one_line) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be 1 to {max_chars} printable characters on one line"),
        ))
    }
}

/// Markdown of at most `max_bytes`: printable lines, tabs and newlines.
pub fn text(field: &'static str, text: &str, max_bytes: usize) -> Result<(), SchemaError> {
    let printable = text
        .chars()
        .all(|c| on_one_line(c) || matches!(c, '\n' | '\t'));
    if text.len() <= max_bytes && printable {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be at most {max_bytes} bytes of printable text"),
        ))
    }
}

/// At most `max` entries, no two equal.
pub fn distinct<T: Ord>(field: &'static str, entries: &[T], max: usize) -> Result<(), SchemaError> {
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
