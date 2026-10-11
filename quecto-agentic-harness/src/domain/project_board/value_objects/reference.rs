//! How the board points at work by number or address (#2481's artifact
//! model): an issue or pull request by its number, and a reference to any
//! issue, pull request or page, on the board or not, by `#n`,
//! `owner/repo#n` or an https URL. The codec, the importer and M4 share
//! these, so a reference is judged and keyed one way everywhere.
use super::code_ref::RepoName;
use super::schema_error::SchemaError;
use regex::Regex;
use std::sync::LazyLock;

/// The highest issue or pull request number: ten digits.
pub const MAX_ISSUE_NUMBER: u64 = 9_999_999_999;
/// The longest reference.
pub const MAX_REFERENCE_BYTES: usize = 2_048;

/// An issue (or pull request) number, from 1 to [`MAX_ISSUE_NUMBER`]: the
/// host numbers issues and pull requests from one sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueNumber(u64);

impl IssueNumber {
    /// # Errors
    /// 0, or more than ten digits.
    pub fn new(number: u64) -> Result<Self, SchemaError> {
        match number {
            1..=MAX_ISSUE_NUMBER => Ok(Self(number)),
            _ => Err(SchemaError::new(
                "issue",
                format!("an issue number is from 1 to {MAX_ISSUE_NUMBER}"),
            )),
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// An issue or pull request, `#n` or `owner/repo#n`: the repository is
/// whatever stands before the `#`, and [`RepoName`] decides it.
static ISSUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:([^#/]+/[^#]+))?#([1-9][0-9]{0,9})$").expect("issue reference")
});

/// One label of a host name.
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$").expect("host label")
});

/// What a reference points at, as its kind is told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Points {
    /// `#n`: an issue or pull request of the board's own repository.
    Own(IssueNumber),
    /// `owner/repo#n`.
    Elsewhere(IssueNumber),
    /// An https URL.
    Page,
}

/// A plain pointer to an issue, pull request or page, on the board or not
/// (closed work, or work outside the board): `#n`, `owner/repo#n` (a
/// repository as [`RepoName`] names it) or an https URL, at most
/// [`MAX_REFERENCE_BYTES`]. It keeps the text as given and its key, what
/// makes two the same pointer ([`Reference::key`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    shown: String,
    key: String,
    points: Points,
}

impl Reference {
    /// # Errors
    /// Anything but `#n`, `owner/repo#n` or an https URL that holds.
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let read = match (
            text.len() <= MAX_REFERENCE_BYTES,
            text.starts_with("https://"),
        ) {
            (false, _) => None,
            (true, true) => url_key(text).map(|key| (key, Points::Page)),
            (true, false) => issue(text),
        };
        match read {
            Some((key, points)) => Ok(Self {
                shown: text.to_string(),
                key,
                points,
            }),
            None => Err(SchemaError::new(
                "reference",
                format!(
                    "{text:?} is not #n, owner/repo#n or an https URL of at most \
                     {MAX_REFERENCE_BYTES} bytes"
                ),
            )),
        }
    }

    /// The text as given.
    pub fn as_str(&self) -> &str {
        &self.shown
    }

    /// What makes two references the same pointer: an issue's repository
    /// lowercased (the host names repositories without regard to case);
    /// a URL with its host lowercased, port 443 dropped and leading zeros
    /// gone from another, an empty path as `/`, and its escapes' hex in
    /// upper case. A task does not know its own repository, so
    /// `owner/repo#n` for its own is a different key from `#n`.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn points(&self) -> Points {
        self.points
    }
}

/// `#n` or `owner/repo#n`, and its key.
fn issue(text: &str) -> Option<(String, Points)> {
    let captures = ISSUE.captures(text)?;
    let number = captures.get(2)?.as_str().parse::<u64>().ok()?;
    let number = IssueNumber::new(number).ok()?;
    match captures.get(1) {
        None => Some((text.to_string(), Points::Own(number))),
        Some(repo) => {
            let repo = RepoName::parse(repo.as_str()).ok()?;
            let key = format!("{}#{}", repo.as_str().to_ascii_lowercase(), number.get());
            Some((key, Points::Elsewhere(number)))
        }
    }
}

/// The characters an https URL's path, query and fragment may hold (RFC
/// 3986): unreserved, sub-delims, `:`, `@`, `/`, `?`, `#` (once, starting
/// the fragment) and `%` (starting an escape).
fn url_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/?#%".contains(&byte)
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

/// `text` with every escape's hex in upper case.
fn escapes_upper(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut upper = 0;
    for character in text.chars() {
        match (character, upper) {
            ('%', _) => {
                upper = 2;
                out.push('%');
            }
            (_, 1..=2) => {
                upper -= 1;
                out.push(character.to_ascii_uppercase());
            }
            _ => out.push(character),
        }
    }
    out
}

/// `text` as an https URL and its key: a host of dot-joined labels, the
/// last not all digits (no IP literal), at most 253 bytes; an optional
/// port from 1 to 65535; then a path, query and fragment of
/// [`url_char`]s, with at most one `#` and every `%` an escape.
fn url_key(text: &str) -> Option<String> {
    let rest = text.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let labels: Vec<&str> = host.split('.').collect();
    let named = labels
        .last()
        .is_some_and(|last| last.bytes().any(|byte| !byte.is_ascii_digit()));
    let host_holds = (1..=253).contains(&host.len())
        && labels.iter().all(|label| LABEL.is_match(label))
        && named;
    let port = match port {
        None => Some(None),
        Some(port) => (port.len() <= 5 && port.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| port.parse::<u32>().ok())
            .flatten()
            .filter(|number| (1..=65_535).contains(number))
            .map(Some),
    };
    let tail_holds =
        tail.bytes().all(url_char) && tail.matches('#').count() <= 1 && escapes_hold(tail);
    match (host_holds, port, tail_holds) {
        (true, Some(port), true) => {
            let port = match port {
                None | Some(443) => String::new(),
                Some(port) => format!(":{port}"),
            };
            let path = match tail.as_bytes().first() {
                Some(b'/') => "",
                None | Some(_) => "/",
            };
            Some(format!(
                "https://{}{port}{path}{}",
                host.to_ascii_lowercase(),
                escapes_upper(tail)
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
