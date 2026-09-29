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
/// the named spans of [`NAMED_LABEL`] and before the command-line shapes
/// of [`COMMAND_LINE_SHAPES`]. This is best-effort: tokens with no
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
        r#"(?i)(?:bearer\s+[^\s"']+"#,
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

/// A secret's label (#2304 review round 3): a secret's name, a quote
/// closing it (a JSON or dict key's, `"password":`), then `=` or `:`. The
/// name matches inside an identifier (`GITHUB_TOKEN=`, `DB_PASSWORD:`,
/// `PGPASSWORD=`), which is kept whole, as the label is: only its value is
/// redacted ([`named_value`]).
static NAMED_LABEL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)(?:api[_-]?key|token|password|secret|access[_-]?token)(?P<closed>["']?)\s*[=:]\s*"#,
    )
    .expect("static named-label regex is valid")
});

/// Redact the value after every [`NAMED_LABEL`] in `input`, keeping the
/// label. A label with no value ([`named_value`]) is left as it is, and
/// the search for the next goes on right after it.
fn redact_named(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(label) = NAMED_LABEL.captures_at(input, from) {
        let whole = label.get(0).expect("a match has its whole span");
        let enclosing = match label["closed"].is_empty() {
            true => enclosing_quote(&input[..whole.start()]),
            false => None,
        };
        from = whole.end();
        let Some(value) = named_value(&input[whole.end()..], enclosing) else {
            continue;
        };
        let (start, end) = (whole.end() + value.start, whole.end() + value.end);
        assert!(
            copied <= start && start < end && end <= input.len(),
            "a redacted value lies ahead of what is copied"
        );
        out.push_str(&input[copied..start]);
        out.push_str("[REDACTED]");
        copied = end;
        from = end;
    }
    out.push_str(&input[copied..]);
    out
}

/// The quote a label sits inside, when the name (its identifier
/// included) is glued to one: `'password:'`, `"DB_PASSWORD=`.
fn enclosing_quote(before: &str) -> Option<char> {
    before
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        .chars()
        .next_back()
        .filter(|c| matches!(c, '"' | '\''))
}

/// The byte range, in `rest` (the text after a label), of the value to
/// redact, or `None` when there is none:
///
/// - inside an `enclosing` quote, the run up to that quote (or the line's
///   end): empty, the label is a search pattern (`grep 'password:' f`);
/// - after an opening quote, the run up to its closing quote (or the
///   text's end), when it starts with neither a space nor that quote: a
///   quote followed by a space or closed at once closes a search pattern
///   (`grep "token=" src/`), and the quotes are kept;
/// - else the bare word that follows.
fn named_value(rest: &str, enclosing: Option<char>) -> Option<std::ops::Range<usize>> {
    let end_of = |text: &str, stop: &dyn Fn(char) -> bool| text.find(stop).unwrap_or(text.len());
    let range = match (enclosing, rest.chars().next()) {
        (Some(quote), _) => 0..end_of(rest, &|c| c == quote || c == '\n'),
        (None, Some(quote @ ('"' | '\''))) => {
            let inner = &rest[1..];
            match inner.chars().next() {
                Some(first) if !first.is_whitespace() && first != quote => {
                    1..1 + end_of(inner, &|c| c == quote)
                }
                Some(_) | None => return None,
            }
        }
        (None, Some(_)) => 0..end_of(rest, &char::is_whitespace),
        (None, None) => return None,
    };
    (!range.is_empty()).then_some(range)
}

/// A credential in a command line's shape (#2304 review): the flag or the
/// name it follows (`keep`) is kept, the credential (`value`) redacted.
/// Each is tried after [`PATTERNS`], in order, over the whole text.
///
/// A value is a bare word, or quoted: a quoted one runs to its closing
/// quote, spaces included (to the text's end when it never closes), and
/// its quotes are kept around `[REDACTED]`. The shapes:
///
/// - `-u`/`--user user:pass` (also `--user=…`, glued `-uuser:pass`) only
///   after an HTTP client in the same command (`curl`, `wget`, `http`,
///   `https`, `httpie`, `xh`, a path before it allowed), and httpie's and
///   xh's `-a`/`--auth` (curl's `-a` is `--append`), each only a
///   `user:password` (`curl -u alice` prompts): for every other tool `-u
///   user:group` names a user (a container's `run -u app:app`, `sudo -u`);
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
/// - `-p <password>` after any tool's `login` subcommand (a registry
///   login, `registry login` included), spaced or glued: there `-p` is the
///   password everywhere it is taken (`--password-stdin` is no password);
/// - `redis-cli`'s `-a <password>` and `--pass <password>`;
/// - an upper-case environment name ending in `_PASS`, `_PASSWD`, `_PWD`
///   or `_CREDENTIAL(S)`, assigned any value (`export DB_PASS=…`);
/// - one ending in `_KEY`, `_KEY_ID` or `_AUTH`, only when its value looks
///   secret ([`looks_secret`]: 16 characters or more, or a key's prefix),
///   so `SORT_KEY=name` and `USE_AUTH=true` survive. `*_TOKEN=`,
///   `*_SECRET=`, `*_PASSWORD=` and `*_API_KEY=` are [`PATTERNS`]' own. An
///   assignment's value never starts with `=`: `FOO_KEY == x` is a test;
/// - `aws_secret_access_key` then `=` or `:` and any value, or a space and
///   a value shaped like an AWS secret ([`looks_aws_secret`]), so prose
///   naming it survives.
///
/// The deliberate false positives (each pinned by a test): a long `*_KEY`
/// value that is no secret (`CACHE_KEY=user-profile-cache-v2`, a key's
/// path), any `*_PASS` value (`SKIP_PASS=1`), the word after a password
/// flag or `Authorization: Basic` whatever it is, and a `curl -u` value of
/// digits.
///
/// A tool's arguments ([`TOOL_ARGUMENTS`]) never span a line: the next
/// line is another command.
///
/// Every pattern is a `regex` one: matching is linear in the text.
static COMMAND_LINE_SHAPES: LazyLock<[CommandLineShape; 11]> = LazyLock::new(|| {
    let shape = |pattern: &str, is_credential: fn(&str, &str) -> bool| CommandLineShape {
        pattern: regex::Regex::new(
            &pattern
                .replace("ARGS", TOOL_ARGUMENTS)
                .replace("TOOL", TOOL_START)
                .replace("ASSIGNED", ASSIGNED_VALUE)
                .replace("VALUE", VALUE),
        )
        .expect("static command-line regex is valid"),
        is_credential,
    };
    let any = |_: &str, _: &str| true;
    [
        shape(
            r"(?P<keep>TOOL(?:curl|wget|https?|httpie|xh)ARGS(?:-u[ \t]*|--user(?:[ \t]+|=)))VALUE",
            is_user_password,
        ),
        shape(
            r"(?P<keep>TOOL(?:https?|httpie|xh)ARGS(?:-a[ \t]*|--auth(?:[ \t]+|=)))VALUE",
            is_user_password,
        ),
        shape(r"(?i)(?P<keep>authorization\s*:\s*basic\s+)VALUE", any),
        shape(
            r"(?P<keep>(?:^|\s)(?:--password|--passwd|-pass)(?:\s+|=))VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOL(?:mysql|mysqldump|mysqladmin|mysqlimport|mysqlshow|mysqlcheck|mysqlsh|mariadb|mariadb-dump)ARGS-p)VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOLsshpass(?:[ \t]+-[A-Za-z]\S*)*?[ \t]+-p[ \t]*)VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOL[A-Za-z][\w.-]*[ \t]+loginARGS-p[ \t]*)VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOLredis-cliARGS(?:-a[ \t]*|--pass(?:[ \t]+|=)))VALUE",
            any,
        ),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:PASS|PASSWD|PWD|CREDENTIALS?)\s*=\s*)ASSIGNED",
            any,
        ),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:KEY|KEY_ID|AUTH)\s*=\s*)ASSIGNED",
            |_, value| looks_secret(value),
        ),
        shape(
            r"(?i)(?P<keep>aws_secret_access_key(?:\s*[=:]\s*|\s+))ASSIGNED",
            |keep, value| keep.contains(['=', ':']) || looks_aws_secret(value),
        ),
    ]
});

/// Where a tool's name starts: the text's start, or after a space, a
/// command separator, a subshell's `(` or a path's `/`.
const TOOL_START: &str = r"(?:^|[\s;&|(/])";

/// A tool's arguments up to the flag a shape names: any words, on the
/// tool's own line and in its own command (never past `;`, `&` or `|`),
/// then the spaces before the flag.
const TOOL_ARGUMENTS: &str = r"(?:[ \t]+[^\s;&|]+)*?[ \t]+";

/// A shape's value: a double- or single-quoted run (its closing quote
/// optional, for a text cut inside it), or a bare word.
const VALUE: &str = r#"(?P<value>"[^"]*"?|'[^']*'?|[^\s"']+)"#;

/// An assignment's [`VALUE`]: a bare one starts with anything but `=`, so
/// `FOO_KEY == x` is a comparison, never an assignment of `= x`.
const ASSIGNED_VALUE: &str = r#"(?P<value>"[^"]*"?|'[^']*'?|[^\s"'=][^\s"']*)"#;

/// One of [`COMMAND_LINE_SHAPES`]: its pattern, with a `keep` and a
/// `value` group, and whether a value it matched (unquoted) after that
/// `keep` is a credential.
struct CommandLineShape {
    pattern: regex::Regex,
    is_credential: fn(&str, &str) -> bool,
}

/// A matched value without its quotes: `(open, inner, close)`.
fn unquoted(value: &str) -> (&str, &str, &str) {
    for quote in ["\"", "'"] {
        if let Some(rest) = value.strip_prefix(quote) {
            return match rest.strip_suffix(quote) {
                Some(inner) => (quote, inner, quote),
                None => (quote, rest, ""),
            };
        }
    }
    ("", value, "")
}

/// Whether an HTTP client's `-u`/`--user` (or `-a`/`--auth`) value is a
/// credential: a `user:password`.
fn is_user_password(_keep: &str, value: &str) -> bool {
    value.contains(':')
}

/// The prefixes a key's value starts with: Stripe's, GitHub's, GitLab's,
/// Slack's, AWS's, Google's and `sk-`.
const SECRET_PREFIXES: &[&str] = &[
    "sk_",
    "pk_",
    "rk_",
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xox",
    "AKIA",
    "ASIA",
    "AIza",
];

/// The length from which a `*_KEY` value is taken for a secret whatever
/// its shape.
const SECRET_MIN_CHARS: usize = 16;

/// Whether a `*_KEY`, `*_KEY_ID` or `*_AUTH` value looks secret: at least
/// [`SECRET_MIN_CHARS`] characters, or one of [`SECRET_PREFIXES`].
fn looks_secret(value: &str) -> bool {
    value.chars().count() >= SECRET_MIN_CHARS
        || SECRET_PREFIXES
            .iter()
            .any(|prefix| value.starts_with(prefix))
}

/// Whether a value looks like an AWS secret access key: at least
/// [`SECRET_MIN_CHARS`] of `A-Z a-z 0-9 / + =`.
fn looks_aws_secret(value: &str) -> bool {
    value.chars().count() >= SECRET_MIN_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '+' | '='))
}

/// Replace known secret shapes in `input` with `[REDACTED]`.
///
/// Non-secret tokens are preserved so the redacted string stays useful: a
/// named secret's label ([`NAMED_LABEL`]), an `sk-` key's `lead`, and a
/// command-line credential's flag or name ([`COMMAND_LINE_SHAPES`]).
pub(crate) fn redact_secrets(input: &str) -> String {
    let named = PATTERNS
        .replace_all(&redact_named(input), |caps: &regex::Captures<'_>| {
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
                let keep = &caps["keep"];
                let (open, inner, close) = unquoted(&caps["value"]);
                match (shape.is_credential)(keep, inner) {
                    true => format!("{keep}{open}[REDACTED]{close}"),
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
        assert_eq!(redacted.as_str(), "prefix token=[REDACTED] suffix");
        assert_eq!(redacted.to_string(), "prefix token=[REDACTED] suffix");
        assert_eq!(&*redacted, "prefix token=[REDACTED] suffix");
    }
}
#[cfg(test)]
#[path = "redaction_command_line_tests.rs"]
mod command_line_tests;
#[cfg(test)]
#[path = "redaction_encodings_tests.rs"]
mod encodings_tests;
