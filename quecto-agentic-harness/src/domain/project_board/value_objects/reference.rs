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
                    "{:?} is not #n, owner/repo#n or an https URL of at most \
                     {MAX_REFERENCE_BYTES} bytes",
                    echoed(text)
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
    /// a URL normalised as RFC 3986 §6.2.2 does (its host lowercased, port
    /// 443 dropped and leading zeros gone from another, escapes of
    /// unreserved characters decoded and the rest in upper-case hex, dot
    /// segments removed, an empty path as `/`), and, by the board's own
    /// choice, an empty query or fragment as none. A task does not know its own repository, so
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

/// `text` with each escape of an unreserved character decoded and every
/// other escape's hex in upper case (RFC 3986 §6.2.2.2); every `%` in it
/// starts an escape ([`escapes_hold`]).
fn escapes_normal(text: &str) -> String {
    debug_assert!(escapes_hold(text), "every % starts an escape");
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = match (bytes[at], bytes.get(at + 1..at + 3)) {
            (b'%', Some(hex)) => std::str::from_utf8(hex)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok()),
            _ => None,
        };
        match escaped {
            Some(byte) if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) => {
                out.push(char::from(byte));
                at += 3;
            }
            Some(byte) => {
                out.push_str(&format!("%{byte:02X}"));
                at += 3;
            }
            None => {
                out.push(char::from(bytes[at]));
                at += 1;
            }
        }
    }
    out
}

/// `path` without its dot segments (RFC 3986 §5.2.4).
fn without_dots(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());
    let pop = |output: &mut String| {
        let cut = output.rfind('/').unwrap_or(0);
        output.truncate(cut);
    };
    while !input.is_empty() {
        match input {
            _ if input.starts_with("../") => input = &input[3..],
            _ if input.starts_with("./") => input = &input[2..],
            _ if input.starts_with("/./") => input = &input[2..],
            "/." => input = "/",
            _ if input.starts_with("/../") => {
                input = &input[3..];
                pop(&mut output);
            }
            "/.." => {
                input = "/";
                pop(&mut output);
            }
            "." | ".." => input = "",
            _ => {
                let start = usize::from(input.starts_with('/'));
                let end = input[start..]
                    .find('/')
                    .map_or(input.len(), |end| end + start);
                output.push_str(&input[..end]);
                input = &input[end..];
            }
        }
    }
    output
}

/// A host's last label names it, not an address: it is not all digits
/// (IPv4) and not hex as `0x…` (an address in WHATWG's reading).
fn named(last: &str) -> bool {
    let lower = last.to_ascii_lowercase();
    let digits = lower.bytes().all(|byte| byte.is_ascii_digit());
    let hex = lower
        .strip_prefix("0x")
        .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    !digits && !hex
}

/// `text` as an https URL and its key: a host of dot-joined labels, the
/// last naming it ([`named`]), at most 253 bytes; an optional port from 1
/// to 65535 (leading zeros aside); then a path, query and fragment of
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
    let host_holds = (1..=253).contains(&host.len())
        && labels.iter().all(|label| LABEL.is_match(label))
        && labels.last().is_some_and(|last| named(last));
    let port = match port {
        None => Some(None),
        Some(port) => {
            let digits = port.trim_start_matches('0');
            (!port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && digits.len() <= 5)
                .then(|| digits.parse::<u32>().ok())
                .flatten()
                .filter(|number| (1..=65_535).contains(number))
                .map(Some)
        }
    };
    let tail_holds =
        tail.bytes().all(url_char) && tail.matches('#').count() <= 1 && escapes_hold(tail);
    match (host_holds, port, tail_holds) {
        (true, Some(port), true) => {
            let port = match port {
                None | Some(443) => String::new(),
                Some(port) => format!(":{port}"),
            };
            let (before, fragment) = tail.split_once('#').unwrap_or((tail, ""));
            let (path, query) = before.split_once('?').unwrap_or((before, ""));
            let path = match without_dots(&escapes_normal(path)).as_str() {
                "" => "/".to_string(),
                path => path.to_string(),
            };
            let part = |mark: char, text: &str| match text {
                "" => String::new(),
                text => format!("{mark}{}", escapes_normal(text)),
            };
            Some(format!(
                "https://{}{port}{path}{}{}",
                host.to_ascii_lowercase(),
                part('?', query),
                part('#', fragment)
            ))
        }
        _ => None,
    }
}

/// `text` shown in a refusal: at most its first 64 characters.
fn echoed(text: &str) -> String {
    match text.char_indices().nth(64) {
        None => text.to_string(),
        Some((cut, _)) => format!("{}…", &text[..cut]),
    }
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
