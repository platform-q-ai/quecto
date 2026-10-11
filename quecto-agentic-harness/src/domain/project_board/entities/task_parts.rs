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
/// The highest issue or pull request number: ten digits.
pub const MAX_ISSUE_NUMBER: u64 = 9_999_999_999;

/// An issue or pull request, `#n` or `owner/repo#n`: the repository is
/// whatever stands before the `#`, and [`RepoName`] decides it.
static ISSUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:([^#/]+/[^#]+))?#[1-9][0-9]{0,9}$").expect("issue reference")
});

/// One label of a host name.
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$").expect("host label")
});

/// The characters an https URL's path, query and fragment may hold:
/// printable ASCII but a space, `|`, backtick or `\`.
fn url_char(byte: u8) -> bool {
    byte.is_ascii_graphic() && !b"|`\\".contains(&byte)
}

/// Every `%` in `text` starts an escape: two hex digits follow.
fn escapes_hold(text: &str) -> bool {
    let bytes = text.as_bytes();
    let hex = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_hexdigit);
    bytes.iter().enumerate().all(|(index, byte)| match byte {
        b'%' => hex(index + 1) && hex(index + 2),
        _ => true,
    })
}

/// `text` as an https URL: a host of dot-joined labels (at most 253
/// bytes), an optional port from 1 to 65535, then a path, query or
/// fragment of [`url_char`]s whose `%`s escape; its key, the host
/// lowercased, when it is one.
fn url_key(text: &str) -> Option<String> {
    let rest = text.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let host_holds =
        (1..=253).contains(&host.len()) && host.split('.').all(|label| LABEL.is_match(label));
    let port_holds = port.is_none_or(|port| {
        port.len() <= 5
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && matches!(port.parse::<u32>(), Ok(1..=65_535))
    });
    let tail_holds = tail.bytes().all(url_char) && escapes_hold(tail);
    (host_holds && port_holds && tail_holds).then(|| {
        let port = port.map_or(String::new(), |port| format!(":{port}"));
        format!("https://{}{port}{tail}", host.to_ascii_lowercase())
    })
}

/// A plain pointer to work outside the board's tasks: `#n`,
/// `owner/repo#n` (a repository as [`RepoName`] names it), or an https
/// URL; at most [`MAX_REFERENCE_BYTES`]. Its key, what makes two the same
/// pointer: the repository lowercased (the host names repositories
/// without regard to case) or the URL's host lowercased. A task does not
/// know its own repository, so `owner/repo#n` for its own is a different
/// key from `#n`.
pub(super) fn reference(field: &str, text: &str) -> Result<String, SchemaError> {
    let key = match (text.starts_with("https://"), ISSUE.captures(text)) {
        (true, _) => url_key(text),
        (false, Some(issue)) => issue
            .get(1)
            .is_none_or(|repo| RepoName::parse(repo.as_str()).is_ok())
            .then(|| text.to_ascii_lowercase()),
        (false, None) => None,
    };
    match (text.len() <= MAX_REFERENCE_BYTES, key) {
        (true, Some(key)) => Ok(key),
        _ => Err(SchemaError::new(
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
