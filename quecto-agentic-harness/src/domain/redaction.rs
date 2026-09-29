//! Shared secret redaction for audit logs and subagent error surfacing.
//!
//! A single source of truth for scrubbing credential-shaped spans out of
//! strings before they are persisted (audit log, #790) or propagated to the
//! parent agent context (subagent error cause, #752). Keeping one regex avoids
//! the two-copy drift risk where tightening one redactor silently leaves the
//! other leaking. Pure; no I/O.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// A string guaranteed to have passed through [`redact_secrets`].
///
/// Its only text constructors ([`Redacted::new`], `From<&str>`/`From<String>`)
/// redact on the way in, so a value of this type cannot carry an un-scrubbed
/// secret-shaped span. This turns the "redact before persistence" invariant for
/// [`crate::domain::audit::AuditEvent::ProviderError`] into a property the type
/// system enforces rather than a convention an emitter could bypass by building
/// the raw variant directly (#937 review). Serializes transparently as the inner
/// string, so the on-disk/wire format is unchanged. Deserialization (reading the
/// audit file back) trusts the already-redacted on-disk value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Redacted(String);

impl Redacted {
    /// Redact `raw` and wrap it. The sole entry point from untrusted text.
    pub(crate) fn new(raw: &str) -> Self {
        Self(redact_secrets(raw))
    }

    /// Borrow the redacted contents as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// At most `max_bytes` of it, cut on a character boundary (#2304): a
    /// prefix of redacted text holds no secret it did not.
    pub fn truncated(mut self, max_bytes: usize) -> Self {
        let mut end = self.0.len().min(max_bytes);
        while !self.0.is_char_boundary(end) {
            end -= 1;
        }
        self.0.truncate(end);
        self
    }
}

impl From<&str> for Redacted {
    fn from(raw: &str) -> Self {
        Self::new(raw)
    }
}

impl From<String> for Redacted {
    fn from(raw: String) -> Self {
        Self::new(&raw)
    }
}

impl std::ops::Deref for Redacted {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Credential-shaped tokens scrubbed to `[REDACTED]`: those with
/// recognisable prefixes (`Bearer <tok>`, `sk-` incl.
/// `sk-proj-`/`sk-ant-`/`sk-or-`, AWS `AKIA`, GitHub `gh[pousr]_`, Slack
/// `xox[baprs]-`, Google `AIza`, GitLab `glpat-`). They are tried after
/// the named spans ([`named`]) and before the command-line shapes
/// ([`shapes`]). This is best-effort: tokens with no
/// distinguishing shape (bare passwords passed positionally, opaque JWTs)
/// can still slip through, so callers must not treat output as guaranteed
/// clean.
///
/// `Bearer`'s token is a run of anything but spaces and quotes: the quote
/// closing a header (`-H "Authorization: Bearer …"`) is kept.
///
/// `sk-` is held to one extra rule (#2241): it starts a word, so
/// `task-runner01`, `disk-usage` or `risk-assessment` is text, never
/// `ta[REDACTED]`. The `lead` before it (kept in the output) is one of:
///
/// - the start of the text;
/// - any character but an ASCII letter (a digit included: a digit glued to
///   `sk-` is a concatenated dump far more often than prose);
/// - an escape or encoding a key travels behind, which may end in a letter
///   (#2249 review): an ANSI CSI colour code (`ESC[01;31m`, also written
///   `\e[`, `\x1b[`, `\033[`, `\u001b[` or `cat -v`'s `^[[`), a
///   C/JSON/shell escape (`\n` `\r` `\t` `\b` `\f` `\a` `\e` `\v`), a hex
///   escape (`\x3d`), a Unicode escape (`\u00e9`, `\U0000003D`), a URL
///   escape, once or more encoded (`%3D`, `%253D`), a quoted-printable
///   escape (`=3D`, either case) or a PowerShell backtick escape (`` `n ``;
///   not `` `a ``, the bell, whose `` `ask-… `` is markdown code far more
///   often than a key).
///
/// The lead is case-sensitive (`(?-i:…)`), so `ſ` or the Kelvin sign, which
/// case-fold to `s` and `k`, still lead a key, and `\N` is no `\n`. The
/// `sk-` minimum stays at eight characters: every real key is far longer,
/// and a shorter floor than theirs costs only over-redaction, never a
/// missed key.
///
/// Every other prefixed shape matches wherever it appears, as it always
/// has: no false positive of theirs was ever shown, and a missed key costs
/// more than an over-redacted word.
static PATTERNS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(concat!(
        r#"(?i)(?:bearer[ \t]+[^\s"']+"#,
        r"|(?P<lead>(?-i:^|[^A-Za-z]",
        r"|(?:\x1b|\\e|\\x1[bB]|\\033|\\u001[bB]|\^\[)\[[0-9;?]*[A-Za-z]",
        r"|\\[abefnrtv]|\\x[0-9A-Fa-f]{2}|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8}",
        r"|%(?:25)*[0-9A-Fa-f]{2}|=[0-9A-Fa-f]{2}|`[0befnrtv]))",
        r"sk-[A-Za-z0-9_-]{8,}",
        r"|AKIA[0-9A-Z]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}",
        r"|AIza[A-Za-z0-9_-]{20,}|glpat-[A-Za-z0-9_-]{20,})",
    ))
    .expect("static redaction regex is valid")
});

/// Whether `lead` is one of the word starts [`PATTERNS`] allows before an
/// `sk-` key, written out independently of the pattern so the assertion
/// below checks it: an allowlist of exactly the shapes documented there.
pub(super) fn is_key_lead(lead: &str) -> bool {
    let mut chars = lead.chars();
    let single =
        matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_ascii_alphabetic());
    lead.is_empty()
        || single
        || is_ansi_csi(lead)
        || escaped(lead, "\\", 1, |c| "abefnrtv".contains(c))
        || escaped(lead, "\\x", 2, |c| c.is_ascii_hexdigit())
        || escaped(lead, "\\u", 4, |c| c.is_ascii_hexdigit())
        || escaped(lead, "\\U", 8, |c| c.is_ascii_hexdigit())
        || is_percent_escape(lead)
        || escaped(lead, "=", 2, |c| c.is_ascii_hexdigit())
        || escaped(lead, "`", 1, |c| "0befnrtv".contains(c))
}

/// Whether `text` is `prefix` followed by exactly `count` characters that
/// all satisfy `allowed`.
fn escaped(text: &str, prefix: &str, count: usize, allowed: impl Fn(char) -> bool) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|rest| rest.chars().count() == count && rest.chars().all(allowed))
}

/// Whether `text` is a URL escape, encoded once or more: `%`, any number
/// of `25` (a `%` encoded again), then two hex digits.
fn is_percent_escape(text: &str) -> bool {
    let Some(mut rest) = text.strip_prefix('%') else {
        return false;
    };
    while rest.len() > 2 {
        match rest.strip_prefix("25") {
            Some(inner) => rest = inner,
            None => return false,
        }
    }
    rest.len() == 2 && rest.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `text` is one ANSI CSI sequence: an introducer (the ESC
/// character or its `\e`, `\x1b`, `\033`, `\u001b` or caret `^[`
/// spelling), `[`,
/// parameters (`0-9`, `;`, `?`) and one final ASCII letter.
fn is_ansi_csi(text: &str) -> bool {
    [
        "\x1b[", "\\e[", "\\x1b[", "\\x1B[", "\\033[", "\\u001b[", "\\u001B[", "^[[",
    ]
    .iter()
    .filter_map(|intro| text.strip_prefix(intro))
    .any(|rest| {
        rest.strip_suffix(|c: char| c.is_ascii_alphabetic())
            .is_some_and(|params| {
                params
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ';' || c == '?')
            })
    })
}

#[path = "redaction_named.rs"]
mod named;
#[path = "redaction_shapes.rs"]
mod shapes;

/// Replace known secret shapes in `input` with `[REDACTED]`.
///
/// Non-secret tokens are preserved so the redacted string stays useful: a
/// named secret's label ([`named`]), an `sk-` key's `lead`, and a
/// command-line credential's flag, name or header, and a URL's user and
/// host ([`shapes`]). No rule reaches across a newline, and the output
/// redacts to itself.
pub(crate) fn redact_secrets(input: &str) -> String {
    let named = PATTERNS
        .replace_all(&named::redact_named(input), |caps: &regex::Captures<'_>| {
            let lead = caps.name("lead").map_or("", |lead| lead.as_str());
            debug_assert!(
                is_key_lead(lead),
                "an sk- key's lead is a word start: {lead:?}"
            );
            format!("{lead}[REDACTED]")
        })
        .into_owned();
    shapes::redact_command_lines(named)
}

/// The userinfo of a URL (`scheme://user:token@host/…`), in whatever text
/// it appears.
static URL_USERINFO: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(://)[^/\s]+@").expect("static URL userinfo regex is valid")
});

/// Replace the userinfo of every URL in `input` with `***`, keeping the
/// scheme, host, port and path: a container config's `--repo
/// https://user:ghp_…@host/x/y` must be nameable in a preflight line, a
/// doctor header or a spawn error without handing the token to the model,
/// the terminal or a log (#2024 S4b review). Text without a URL is
/// returned unchanged.
pub fn redact_url_userinfo(input: &str) -> String {
    URL_USERINFO.replace_all(input, "${1}***@").into_owned()
}

#[cfg(test)]
#[path = "redaction_tests.rs"]
mod tests;
#[cfg(test)]
mod redacted_cov_tests {
    use super::*;

    #[test]
    fn redacted_from_string_and_display_scrub_secret() {
        let redacted = Redacted::from(String::from("prefix token=hunter2 suffix"));
        assert_eq!(redacted.as_str(), "prefix token=[REDACTED] suffix");
        assert_eq!(redacted.to_string(), "prefix token=[REDACTED] suffix");
        assert_eq!(&*redacted, "prefix token=[REDACTED] suffix");
    }
}
#[cfg(test)]
#[path = "redaction_command_line_tests.rs"]
mod command_line_tests;
#[cfg(test)]
#[path = "redaction_corpus_tests.rs"]
mod corpus_tests;
#[cfg(test)]
#[path = "redaction_encodings_tests.rs"]
mod encodings_tests;
#[cfg(test)]
#[path = "redaction_linearity_tests.rs"]
mod linearity_tests;
#[cfg(test)]
#[path = "redaction_newline_tests.rs"]
mod newline_tests;
#[cfg(test)]
#[path = "redaction_round5_tests.rs"]
mod round5_tests;
