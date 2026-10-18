//! The structured parts the board's artifacts share: their limits and the
//! checks they share (a finding's place is one rule for every artifact
//! that has findings).
use super::super::value_objects::schema_error::{SchemaError, distinct, line};
use regex::Regex;
use std::sync::LazyLock;

pub const MAX_ITEMS: usize = 128;
/// Criteria, evidence and dependencies per task item.
pub const MAX_PER_ITEM: usize = 16;
pub const MAX_LINE_CHARS: usize = 500;
pub const MAX_OWNER_CHARS: usize = 100;
/// The largest integer a JSON reader in any language keeps exactly.
pub const MAX_BUDGET_TOKENS: u64 = (1 << 53) - 1;
pub const MAX_FINDING_CHARS: usize = 4_000;

/// `path:line` or `path:line:col`, numbers from 1 without leading zeros.
/// The path is relative: `/`-separated segments, none empty, `.` or `..`
/// (a segment starts with a name character, or with one `.` followed by
/// one, as in `.github`), and the first does not start with `-` (an option
/// to a tool).
static LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    let rest = r"[A-Za-z0-9._@+-]*";
    let first = format!(r"(?:[A-Za-z0-9_@+]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let segment = format!(r"(?:[A-Za-z0-9_@+-]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let pattern = format!(r"^{first}(?:/{segment})*:[1-9][0-9]{{0,8}}(?::[1-9][0-9]{{0,8}})?$");
    Regex::new(&pattern).expect("location")
});

/// Distinct single lines, at most `max`.
pub(super) fn lines(field: &str, entries: &[String], max: usize) -> Result<(), SchemaError> {
    distinct(field, entries, max)?;
    let mut indexed = entries.iter().enumerate();
    indexed.try_for_each(|(index, entry)| line(&format!("{field}/{index}"), entry, MAX_LINE_CHARS))
}

pub(super) fn shaped(
    field: &str,
    text: &str,
    max_bytes: usize,
    shape: &Regex,
) -> Result<(), SchemaError> {
    if text.len() <= max_bytes && shape.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("{text:?} is not an allowed reference of at most {max_bytes} bytes"),
        ))
    }
}

/// A finding's place, `path:line` or `path:line:col`: one rule for the
/// task's findings and a review's.
pub(super) fn location(field: &str, text: &str) -> Result<(), SchemaError> {
    shaped(field, text, MAX_LINE_CHARS, &LOCATION)
}

#[cfg(test)]
#[path = "task_parts_tests.rs"]
mod tests;
