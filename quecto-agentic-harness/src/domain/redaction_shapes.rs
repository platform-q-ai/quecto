//! A credential in a command line's or a header's shape (#2304 review
//! rounds 1 to 4): the flag, name or header it follows (`keep`) is kept,
//! the credential (`value`) redacted, and what closes it (`tail`, when a
//! shape names one) kept after it.

use std::sync::LazyLock;

/// The shapes, each tried in order over the whole text, after the named
/// spans and the prefixed keys.
///
/// A value is a bare word, or quoted: a quoted one runs to its closing
/// quote (a double-quoted one skipping `\"`), spaces included, or to its
/// line's end when it never closes there, and its quotes are kept around
/// `[REDACTED]`. The shapes:
///
/// - a connection URL's password (`scheme://user:password@host`, the user
///   empty or not), up to the last `@` before the host: the scheme, user,
///   host and path are kept (`postgres://app:[REDACTED]@db/shop`);
/// - `-u`/`--user user:pass` (also `--user=…`, glued `-uuser:pass`) only
///   after an HTTP client in the same command (`curl`, `wget`, `http`,
///   `https`, `httpie`, `xh`, a path before it allowed), and httpie's and
///   xh's `-a`/`--auth` (curl's `-a` is `--append`), each only a
///   `user:password` (`curl -u alice` prompts): for every other tool `-u
///   user:group` names a user (a container's `run -u app:app`, `sudo -u`);
/// - `Authorization: Basic …` or `Authorization: token …` (GitHub's), and
///   `Proxy-Authorization`, any case, a JSON key's quotes allowed;
/// - `--password X`, `--password=X`, `--passwd …`, `-pass …` (openssl's
///   `-pass pass:…`): a flag that is exactly one of these (`--passes=3` is
///   not), whatever follows it;
/// - a flag whose name ends in `token`, `api-key`, `apikey`, `secret`,
///   `password`, `passwd` or `access-key`, any case (`--token`,
///   `--auth-token`, `--client-secret`, `--API-KEY`), then spaces and a
///   value that does not look like a flag (`--token --verbose` is kept);
///   `--password-stdin` and `--token-file` name no secret;
/// - `-p<password>` glued to its flag, only after one of the mysql-family
///   clients, in the same command (`mysql`, `mysqldump`, `mysqladmin`,
///   `mysqlimport`, `mysqlshow`, `mysqlcheck`, `mysqlsh`, `mariadb`,
///   `mariadb-dump`): for every other tool `-p` is a port (`ssh -p 2222`),
///   a flag (`git log -p`) or a mode (`mkdir -p`);
/// - `sshpass -p <password>`, spaced or glued;
/// - `-p <password>` in a registry-style login, spaced or glued: a
///   command whose first word (after `sudo`, a path allowed) is followed by
///   `login` or `registry login`, or any `login` subcommand whose command
///   also has `-u`/`--username`. `gh auth login -p https` (a protocol)
///   and a commit message's `login -p` are neither;
/// - `redis-cli`'s `-a <password>` and `--pass <password>`;
/// - `gh secret set`'s `-b`/`--body` value;
/// - a `Cookie:` or `Set-Cookie:` header's value, to its closing quote or
///   line's end;
/// - an upper-case environment name ending in `_PASS`, `_PASSWD`, `_PWD`
///   or `_CREDENTIAL(S)`, assigned any value (`export DB_PASS=…`);
/// - one ending in `_KEY`, `_KEY_ID` or `_AUTH`, only when its value looks
///   secret ([`looks_secret`]: 16 characters or more, or a key's prefix),
///   so `SORT_KEY=name` and `USE_AUTH=true` survive. An assignment's value
///   never starts with `=`: `FOO_KEY == x` is a test;
/// - `aws_secret_access_key` then `=` or `:` and any value, or a space and
///   a value shaped like an AWS secret ([`looks_aws_secret`]), so prose
///   naming it survives.
///
/// The deliberate false positives (each pinned by a test): a long `*_KEY`
/// value that is no secret (`CACHE_KEY=user-profile-cache-v2`, a key's
/// path), any `*_PASS` value (`SKIP_PASS=1`), the word after a password
/// flag, a secret flag or `Authorization: Basic` whatever it is (`build
/// --secret id=npm,src=…`), and a `curl -u` value of digits.
///
/// No shape spans a line: a flag, label or name is separated from its
/// value by spaces or tabs only, a tool's arguments ([`TOOL_ARGUMENTS`])
/// stay on its line, and a quoted value stops at the line's end.
///
/// Every pattern is a `regex` one: matching is linear in the text.
static COMMAND_LINE_SHAPES: LazyLock<[CommandLineShape; 16]> = LazyLock::new(|| {
    let shape = |pattern: &str, is_credential: IsCredential| CommandLineShape {
        pattern: regex::Regex::new(
            &pattern
                .replace("ARGS", TOOL_ARGUMENTS)
                .replace("TOOL", TOOL_START)
                .replace("ASSIGNED", ASSIGNED_VALUE)
                .replace("FLAG_VALUE", FLAG_VALUE)
                .replace("VALUE", VALUE),
        )
        .expect("static command-line regex is valid"),
        is_credential,
    };
    let any: IsCredential = |_, _| true;
    [
        shape(
            r#"(?P<keep>\b[A-Za-z][A-Za-z0-9+.-]*://[^\s:/@"'<>]*:)(?P<value>[^\s/?#"'<>]+)(?P<tail>@)"#,
            any,
        ),
        shape(
            r"(?P<keep>TOOL(?:curl|wget|https?|httpie|xh)ARGS(?:-u[ \t]*|--user(?:[ \t]+|=)))VALUE",
            is_user_password,
        ),
        shape(
            r"(?P<keep>TOOL(?:https?|httpie|xh)ARGS(?:-a[ \t]*|--auth(?:[ \t]+|=)))VALUE",
            is_user_password,
        ),
        shape(
            r#"(?i)(?P<keep>authorization["']?[ \t]*:[ \t]*["']?(?:basic|token)[ \t]+)VALUE"#,
            any,
        ),
        shape(
            r"(?P<keep>(?:(?m:^)|[ \t])(?:--password|--passwd|-pass)(?:[ \t]+|=))VALUE",
            any,
        ),
        shape(
            r"(?i)(?P<keep>(?:(?m:^)|[ \t(])--?[a-z0-9-]*?(?:token|api-?key|secret|password|passwd|access-key)[ \t]+)FLAG_VALUE",
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
            r"(?P<keep>(?:(?m:^)|[;&|(])[ \t]*(?:sudo[ \t]+)?(?:[^\s;&|()]*/)?[A-Za-z][\w.-]*[ \t]+(?:registry[ \t]+)?loginARGS-p[ \t]*)VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOL[A-Za-z][\w.-]*[ \t]+loginARGS-p[ \t]*)VALUE(?P<tail>(?:[ \t]+[^\s;&|]+)*?[ \t]+(?:-u|--username))?",
            names_a_user,
        ),
        shape(
            r"(?P<keep>TOOLredis-cliARGS(?:-a[ \t]*|--pass(?:[ \t]+|=)))VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOLgh[ \t]+secret[ \t]+setARGS(?:-b[ \t]*|--body(?:[ \t]+|=)))VALUE",
            any,
        ),
        shape(
            r#"(?i)(?P<keep>\b(?:set-)?cookie[ \t]*:[ \t]*)(?P<value>[^\s"'](?:[^\r\n"']*[^\s"'])?)"#,
            any,
        ),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:PASS|PASSWD|PWD|CREDENTIALS?)[ \t]*=[ \t]*)ASSIGNED",
            any,
        ),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:KEY|KEY_ID|AUTH)[ \t]*=[ \t]*)ASSIGNED",
            |_, value| looks_secret(value),
        ),
        shape(
            r"(?i)(?P<keep>aws_secret_access_key(?:[ \t]*[=:][ \t]*|[ \t]+))ASSIGNED",
            |caps, value| caps["keep"].contains(['=', ':']) || looks_aws_secret(value),
        ),
    ]
});

/// Where a tool's name starts: a line's start, or after a space, a tab, a
/// command separator, a subshell's `(` or a path's `/`.
const TOOL_START: &str = r"(?:(?m:^)|[ \t;&|(/])";

/// A tool's arguments up to the flag a shape names: any words, on the
/// tool's own line and in its own command (never past `;`, `&` or `|`),
/// then the spaces before the flag.
const TOOL_ARGUMENTS: &str = r"(?:[ \t]+[^\s;&|]+)*?[ \t]+";

/// A shape's value: a double-quoted run (skipping `\"`) or a
/// single-quoted one, each stopping at its line's end and with its
/// closing quote optional (for a text cut inside it), or a bare word,
/// which ends where the shell's would: at a space, a quote or one of
/// `; & | ( ) < >`, so `login -p pw; next` keeps its `;`.
const VALUE: &str = r#"(?P<value>"(?:[^"\\\r\n]|\\.)*"?|'[^'\r\n]*'?|[^\s"';&|()<>]+)"#;

/// A secret flag's [`VALUE`]: a bare one never starts with `-`, so the
/// flag after a flag is kept.
const FLAG_VALUE: &str =
    r#"(?P<value>"(?:[^"\\\r\n]|\\.)*"?|'[^'\r\n]*'?|[^\s"';&|()<>-][^\s"';&|()<>]*)"#;

/// An assignment's [`VALUE`]: a bare one starts with anything but `=`, so
/// `FOO_KEY == x` is a comparison, never an assignment of `= x`.
const ASSIGNED_VALUE: &str =
    r#"(?P<value>"(?:[^"\\\r\n]|\\.)*"?|'[^'\r\n]*'?|[^\s"';&|()<>=][^\s"';&|()<>]*)"#;

/// Whether a shape's match is a credential, given its captures and its
/// value without quotes.
type IsCredential = fn(&regex::Captures<'_>, &str) -> bool;

/// One of [`COMMAND_LINE_SHAPES`]: its pattern, with a `keep` and a
/// `value` group (and an optional `tail`), and whether a match of it is a
/// credential.
struct CommandLineShape {
    pattern: regex::Regex,
    is_credential: IsCredential,
}

/// Redact every [`COMMAND_LINE_SHAPES`] credential in `text`, keeping
/// each one's `keep`, quotes and `tail`.
pub(super) fn redact_command_lines(text: String) -> String {
    COMMAND_LINE_SHAPES.iter().fold(text, |text, shape| {
        shape
            .pattern
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                let keep = &caps["keep"];
                let (open, inner, close) = unquoted(&caps["value"]);
                let tail = caps.name("tail").map_or("", |tail| tail.as_str());
                match (shape.is_credential)(caps, inner) {
                    true => format!("{keep}{open}[REDACTED]{close}{tail}"),
                    false => caps[0].to_string(),
                }
            })
            .into_owned()
    })
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
fn is_user_password(_caps: &regex::Captures<'_>, value: &str) -> bool {
    value.contains(':')
}

/// Whether a `login` subcommand's command names a user: a `-u` or
/// `--username` flag after `login` and before its `-p`, or after the
/// password (the shape's `tail`).
fn names_a_user(caps: &regex::Captures<'_>, _value: &str) -> bool {
    caps.name("tail").is_some()
        || caps["keep"]
            .split_whitespace()
            .skip_while(|word| *word != "login")
            .any(|word| word.starts_with("-u") || word.starts_with("--username"))
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
