//! The structured parts the board's artifacts share: their limits, a
//! task's review findings (until reviews become their own artifact, M1b),
//! and the checks they share. Reviews are capped; a write past the cap is
//! refused.
use super::super::value_objects::code_ref::RepoName;
use super::super::value_objects::schema_error::{SchemaError, distinct, filled_markdown, line};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{Severity, Verdict};
use super::claim::Identity;
use regex::Regex;
use std::sync::LazyLock;

pub const MAX_ITEMS: usize = 128;
/// Criteria, evidence and dependencies per item.
pub const MAX_PER_ITEM: usize = 16;
pub const MAX_REVIEWS: usize = 128;
pub const MAX_LINE_CHARS: usize = 500;
pub const MAX_OWNER_CHARS: usize = 100;
/// The largest integer a JSON reader in any language keeps exactly.
pub const MAX_BUDGET_TOKENS: u64 = (1 << 53) - 1;
pub const MAX_FINDING_CHARS: usize = 4_000;

/// A review finding, at `path:line` or `path:line:col` when it has a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub id: Slug,
    pub round: u8,
    pub reviewer: Identity,
    pub at: Timestamp,
    pub severity: Severity,
    pub location: Option<String>,
    /// Markdown.
    pub finding: String,
    pub verdict: Verdict,
    /// The test that proves a fix.
    pub proving_test: Option<String>,
}

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

/// The longest reference.
pub const MAX_REFERENCE_BYTES: usize = 2_048;

/// An issue or pull request, `#n` or `owner/repo#n`.
static ISSUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:([A-Za-z0-9._-]{1,100}/[A-Za-z0-9._-]{1,100}))?#[1-9][0-9]{0,9}$")
        .expect("issue reference")
});

/// An https URL: a host, an optional port, then printable ASCII with no
/// space, `|`, backtick or `\`.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https://[A-Za-z0-9](?:[A-Za-z0-9.-]{0,252})(?::[0-9]{1,5})?(?:[/?#][\x21-\x5B\x5D-\x5F\x61-\x7B\x7D\x7E]*)?$",
    )
    .expect("url reference")
});

/// A plain pointer to work outside the board's tasks: `#n`,
/// `owner/repo#n` (a repository as [`RepoName`] names it), or an https
/// URL; at most [`MAX_REFERENCE_BYTES`].
pub(super) fn reference(field: &str, text: &str) -> Result<(), SchemaError> {
    let repo_ok = |repo: Option<regex::Match<'_>>| {
        repo.is_none_or(|repo| RepoName::parse(repo.as_str()).is_ok())
    };
    let pointer = match ISSUE.captures(text) {
        Some(issue) => repo_ok(issue.get(1)),
        None => URL.is_match(text),
    };
    match text.len() <= MAX_REFERENCE_BYTES && pointer {
        true => Ok(()),
        false => Err(SchemaError::new(
            field,
            format!(
                "{text:?} is not #n, owner/repo#n or an https URL of at most {MAX_REFERENCE_BYTES} bytes"
            ),
        )),
    }
}

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

impl Review {
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        if !matches!(self.round, 1..) {
            return Err(SchemaError::new(
                format!("{field}/round"),
                "must be at least 1",
            ));
        }
        self.reviewer.validate(&format!("{field}/reviewer"))?;
        if let Some(location) = &self.location {
            shaped(
                &format!("{field}/location"),
                location,
                MAX_LINE_CHARS,
                &LOCATION,
            )?;
        }
        filled_markdown(
            &format!("{field}/finding"),
            &self.finding,
            MAX_FINDING_CHARS,
        )?;
        match &self.proving_test {
            Some(test) => line(&format!("{field}/proving_test"), test, MAX_LINE_CHARS),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "task_parts_tests.rs"]
mod tests;
