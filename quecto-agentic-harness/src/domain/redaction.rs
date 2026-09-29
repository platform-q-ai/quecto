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

/// Credential-shaped patterns scrubbed to `[REDACTED]`.
///
/// Covers both named spans (`<api_key|token|password|secret|access_token>=
/// <value>`) and tokens that have recognisable prefixes (`Bearer <tok>`,
/// `sk-` incl. `sk-proj-`/`sk-ant-`/`sk-or-`, AWS `AKIA`, GitHub
/// `gh[pousr]_`, Slack `xox[baprs]-`, Google `AIza`, GitLab `glpat-`), and
/// then the command-line shapes of [`COMMAND_LINE_SHAPES`]. This is best-effort:
/// tokens with no distinguishing shape (bare passwords passed positionally,
/// opaque JWTs) can still slip through, so callers must not treat output as
/// guaranteed clean.
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
/// more than an over-redacted word. A named span keeps matching inside an
/// identifier: `GITHUB_TOKEN=` and `DB_PASSWORD:` are secret labels
/// however they are prefixed.
static PATTERNS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(concat!(
        r"(?i)(?:bearer\s+\S+",
        r"|(?P<lead>(?-i:^|[^A-Za-z]",
        r"|(?:\x1b|\\e|\\x1[bB]|\\033|\\u001[bB]|\^\[)\[[0-9;?]*[A-Za-z]",
        r"|\\[abefnrtv]|\\x[0-9A-Fa-f]{2}|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8}",
        r"|%(?:25)*[0-9A-Fa-f]{2}|=[0-9A-Fa-f]{2}|`[0befnrtv]))",
        r"sk-[A-Za-z0-9_-]{8,}",
        r"|AKIA[0-9A-Z]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}",
        r"|AIza[A-Za-z0-9_-]{20,}|glpat-[A-Za-z0-9_-]{20,}",
        r"|(?:api[_-]?key|token|password|secret|access[_-]?token)\s*[=:]\s*\S+)",
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

/// A credential in a command line's shape (#2304 review): the flag or the
/// name it follows (`keep`) is kept, the credential (`secret`) redacted.
/// Each is tried after [`PATTERNS`], in order, over the whole text:
///
/// - `curl -u user:pass`, `--user user:pass`, `--user=…`, `-uuser:pass`:
///   only a `user:password` (a `-u` naming a user alone, as `ps -u root`
///   or `sort -u`, is no credential), and never a `uid:gid` of digits
///   (`-u 1000:1000`);
/// - `Authorization: Basic …` (and `Proxy-Authorization`), any case;
/// - `--password X`, `--password=X`, `--passwd …`, `-pass …` (openssl's
///   `-pass pass:…`): a flag that is exactly one of these (`--passes=3` is
///   not);
/// - `-p<password>` glued to its flag, only after one of the mysql-family
///   clients, in the same command (`mysql`, `mysqldump`, `mysqladmin`,
///   `mysqlimport`, `mysqlshow`, `mysqlcheck`, `mysqlsh`, `mariadb`,
///   `mariadb-dump`): for them `-p pw` with a space prompts and names a
///   database, and for every other tool `-p` is a port (`ssh -p 2222`,
///   `psql -p 5432`), a flag (`git log -p`, `cp -p`) or a mode (`mkdir
///   -p`), so none of those is touched;
/// - `sshpass -p <password>`, spaced or glued;
/// - an upper-case environment name ending in `_KEY`, `_KEY_ID`, `_PASS`,
///   `_PASSWD`, `_PWD`, `_CREDENTIAL(S)` or `_AUTH`, assigned (`export
///   STRIPE_KEY=…`, `AWS_ACCESS_KEY_ID=…`); `*_TOKEN=`, `*_SECRET=`,
///   `*_PASSWORD=` and `*_API_KEY=` are [`PATTERNS`]' own;
/// - `aws_secret_access_key`, any case, then its value after `=`, `:` or
///   a space (`aws configure set aws_secret_access_key …`).
static COMMAND_LINE_SHAPES: LazyLock<[CommandLineShape; 7]> = LazyLock::new(|| {
    let shape = |pattern: &str, is_credential: fn(&str) -> bool| CommandLineShape {
        pattern: regex::Regex::new(&pattern.replace("SECRET", r#"(?P<secret>[^\s"']+)"#))
            .expect("static command-line regex is valid"),
        is_credential,
    };
    let any = |_: &str| true;
    [
        shape(
            r#"(?P<keep>(?:^|[\s;&|(])(?:-u\s*|--user(?:\s+|=))["']?)(?P<secret>[^\s:"']+:[^\s"']+)"#,
            is_user_password,
        ),
        shape(r"(?i)(?P<keep>authorization\s*:\s*basic\s+)SECRET", any),
        shape(
            r"(?P<keep>(?:^|\s)(?:--password|--passwd|-pass)(?:\s+|=))SECRET",
            any,
        ),
        shape(
            r"(?P<keep>(?:^|[\s;&|(])(?:mysql|mysqldump|mysqladmin|mysqlimport|mysqlshow|mysqlcheck|mysqlsh|mariadb|mariadb-dump)(?:\s+[^\s;&|]+)*?\s+-p)SECRET",
            any,
        ),
        shape(
            r"(?P<keep>(?:^|[\s;&|(])sshpass\s+(?:-[A-Za-z]\S*\s+)*?-p\s*)SECRET",
            any,
        ),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:KEY|KEY_ID|PASS|PASSWD|PWD|CREDENTIALS?|AUTH)\s*=\s*)SECRET",
            any,
        ),
        shape(
            r"(?i)(?P<keep>aws_secret_access_key(?:\s*[=:]\s*|\s+))SECRET",
            any,
        ),
    ]
});

/// One of [`COMMAND_LINE_SHAPES`]: its pattern, with a `keep` and a
/// `secret` group, and whether a `secret` it matched is a credential.
struct CommandLineShape {
    pattern: regex::Regex,
    is_credential: fn(&str) -> bool,
}

/// Whether a `-u`/`--user` value is a credential: `user:password`, not a
/// `uid:gid`.
fn is_user_password(value: &str) -> bool {
    value.split_once(':').is_some_and(|(user, password)| {
        let uid_gid = user
            .chars()
            .chain(password.chars())
            .all(|c| c.is_ascii_digit());
        !uid_gid
    })
}

/// Replace known secret shapes in `input` with `[REDACTED]`.
///
/// Non-secret tokens are preserved so the redacted string stays useful: an
/// `sk-` key's `lead` is written back in front of it, and a command-line
/// credential's flag or name ([`COMMAND_LINE_SHAPES`]).
pub(crate) fn redact_secrets(input: &str) -> String {
    let named = PATTERNS
        .replace_all(input, |caps: &regex::Captures<'_>| {
            let lead = caps.name("lead").map_or("", |lead| lead.as_str());
            debug_assert!(
                is_key_lead(lead),
                "an sk- key's lead is a word start: {lead:?}"
            );
            format!("{lead}[REDACTED]")
        })
        .into_owned();
    COMMAND_LINE_SHAPES.iter().fold(named, |text, shape| {
        shape
            .pattern
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                match (shape.is_credential)(&caps["secret"]) {
                    true => format!("{}[REDACTED]", &caps["keep"]),
                    false => caps[0].to_string(),
                }
            })
            .into_owned()
    })
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
        assert_eq!(redacted.as_str(), "prefix [REDACTED] suffix");
        assert_eq!(redacted.to_string(), "prefix [REDACTED] suffix");
        assert_eq!(&*redacted, "prefix [REDACTED] suffix");
    }
}
#[cfg(test)]
#[path = "redaction_command_line_tests.rs"]
mod command_line_tests;
#[cfg(test)]
#[path = "redaction_encodings_tests.rs"]
mod encodings_tests;
