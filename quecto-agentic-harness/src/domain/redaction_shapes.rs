//! A credential in a command line's, a header's or a URL's shape (#2304
//! review rounds 1 to 5): the flag, name, header or user it follows
//! (`keep`) is kept, and the credential (`value`) redacted.

use std::sync::LazyLock;

/// The shapes, each tried in order over the whole text, after the named
/// spans and the prefixed keys, and after a URL's credentials
/// ([`redact_url_credentials`]); a user-named login's `-p`
/// ([`redact_user_logins`]) comes last.
///
/// A value is a bare word, or quoted: a quoted one runs to its closing
/// quote (a double-quoted one skipping `\"`), spaces included, or to its
/// line's end when it never closes there, and its quotes are kept around
/// `[REDACTED]`. A one-letter password flag's value may also follow `=`
/// (`-p=…`), which is kept. The shapes:
///
/// - `-u`/`--user user:pass` (also `--user=…`, glued `-uuser:pass`) only
///   after an HTTP client in the same command (`curl`, `wget`, `http`,
///   `https`, `httpie`, `xh`, a path before it allowed), and httpie's and
///   xh's `-a`/`--auth` (curl's `-a` is `--append`), each only a
///   `user:password` (`curl -u alice` prompts): for every other tool `-u
///   user:group` names a user (a container's `run -u app:app`, `sudo -u`);
/// - `Authorization: Basic …` or `Authorization: token …` (GitHub's), and
///   `Proxy-Authorization`, any case, a JSON key's quotes allowed;
/// - `--password X`, `--password=X`, `--passwd …`, `-pass …`, `-passin …`,
///   `-passout …` (openssl's `-pass pass:…`): a flag that is exactly one
///   of these (`--passes=3` is not), whatever follows it;
/// - a flag whose name ends in `token`, `api-key`, `apikey`, `secret`,
///   `password`, `passwd`, `passphrase`, `access-key`, `account-key`,
///   `creds` or `bearer`, any case (`--token`, `--auth-token`,
///   `--client-secret`, `--API-KEY`, `--dest-creds`), then spaces or `=`
///   and a value that does not look like a flag (`--token --verbose` is
///   kept); `--password-stdin`, `--token-file` and `--passphrase-file`
///   name no secret;
/// - `-p<password>` glued to its flag, only after one of the mysql-family
///   clients, in the same command (`mysql`, `mysqldump`, `mysqladmin`,
///   `mysqlimport`, `mysqlshow`, `mysqlcheck`, `mysqlsh`, `mariadb`,
///   `mariadb-dump`), or after `7z` (`7za`, `7zr`, `7zz`): for every other
///   tool `-p` is a port (`ssh -p 2222`), a flag (`git log -p`) or a mode
///   (`mkdir -p`);
/// - `sshpass -p <password>` and `twine … -p <password>`, spaced or
///   glued;
/// - `-p <password>` in a registry-style login, spaced or glued: a
///   command whose first word (after `sudo`, a path allowed) is followed by
///   `login` or `registry login`. `gh auth login -p https` (a protocol)
///   and a commit message's `login -p` are not;
/// - `redis-cli`'s `-a <password>` and `--pass <password>`;
/// - `gh secret set`'s `-b`/`--body` value;
/// - `zip`'s and `unzip`'s `-P`, `sqlcmd`'s `-P`, `ssh-keygen`'s `-N`
///   and `-P` (an empty one, `-N ''`, is kept) and `doctl`'s `-t`;
/// - `cargo login <token>` (a `--registry` before it skipped) and `vault
///   login <token>` (flags before it skipped), when the word looks like a
///   token ([`looks_like_login_token`]), so `cargo login to …` prose and
///   `vault login username=me` survive;
/// - a `Cookie:` or `Set-Cookie:` header's value, to its closing quote or
///   line's end;
/// - an upper-case environment name ending in `_PASS`, `_PASSWD`, `_PWD`
///   or `_CREDENTIAL(S)`, holding `SECRET_KEY` (`DJANGO_SECRET_KEY`,
///   `SECRET_KEY_BASE`), or `SSHPASS`, assigned any value (`export
///   DB_PASS=…`);
/// - one ending in `_KEY`, `_KEY_ID` or `_AUTH`, only when its value looks
///   secret ([`looks_secret`]: 16 characters or more, or a key's prefix),
///   so `SORT_KEY=name` and `USE_AUTH=true` survive. An assignment's value
///   never starts with `=`: `FOO_KEY == x` is a test;
/// - fish's `set -x NAME value` (a flag or more, then a value that is no
///   flag), when the name is a secret's ([`names_a_secret`]): `set -x
///   PATH /usr/bin` and `gh secret set NAME --body …` are no such shape;
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
/// Every pattern is a `regex` one, and none looks past its value for
/// something that may follow it (the one shape that did, a login's `-u`
/// after its `-p`, is [`redact_user_logins`]'s scan, round 5 M1): a
/// search finds each match without reading beyond it, and the next
/// search starts after it, so each shape is linear in the text.
static COMMAND_LINE_SHAPES: LazyLock<[CommandLineShape; 24]> = LazyLock::new(|| {
    let shape = |pattern: &str, is_credential: IsCredential| CommandLineShape {
        pattern: regex::Regex::new(&expand(pattern)).expect("static command-line regex is valid"),
        is_credential,
    };
    let any: IsCredential = |_, _| true;
    let non_empty: IsCredential = |_, value| !value.is_empty();
    [
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
            r"(?P<keep>(?:(?m:^)|[ \t])(?:--password|--passwd|-pass|-passin|-passout)(?:[ \t]+|=))VALUE",
            any,
        ),
        shape(
            r"(?i)(?P<keep>(?:(?m:^)|[ \t(])--?[a-z0-9-]*?(?:token|api-?key|secret|password|passwd|passphrase|access-key|account-key|creds|bearer)(?:[ \t]+|=))FLAG_VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOL(?:mysql|mysqldump|mysqladmin|mysqlimport|mysqlshow|mysqlcheck|mysqlsh|mariadb|mariadb-dump)ARGS-p=?)VALUE",
            any,
        ),
        shape(r"(?P<keep>TOOL7z[arz]?ARGS-p)VALUE", any),
        shape(
            r"(?P<keep>TOOLsshpass(?:[ \t]+-[A-Za-z]\S*)*?[ \t]+-pSPACED)VALUE",
            any,
        ),
        shape(r"(?P<keep>TOOLtwineARGS-pSPACED)VALUE", any),
        shape(
            r"(?P<keep>(?:(?m:^)|[;&|(])[ \t]*(?:sudo[ \t]+)?(?:[^\s;&|()]*/)?[A-Za-z][\w.-]*[ \t]+(?:registry[ \t]+)?loginARGS-pSPACED)VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOLredis-cliARGS(?:-aSPACED|--pass(?:[ \t]+|=)))VALUE",
            any,
        ),
        shape(
            r"(?P<keep>TOOLgh[ \t]+secret[ \t]+setARGS(?:-bSPACED|--body(?:[ \t]+|=)))VALUE",
            any,
        ),
        shape(r"(?P<keep>TOOL(?:zip|unzip|sqlcmd)ARGS-PSPACED)VALUE", any),
        shape(r"(?P<keep>TOOLssh-keygenARGS-[NP]SPACED)VALUE", non_empty),
        shape(r"(?P<keep>TOOLdoctlARGS-tSPACED)VALUE", any),
        shape(
            r"(?P<keep>TOOLcargo[ \t]+login(?:[ \t]+(?:--registry(?:[ \t]+|=)[^\s;&|]+|-[^\s;&|]+))*[ \t]+)FLAG_VALUE",
            |_, value| looks_like_login_token(value),
        ),
        shape(
            r"(?P<keep>TOOLvault[ \t]+login(?:[ \t]+-[^\s;&|]+)*[ \t]+)FLAG_VALUE",
            |_, value| looks_like_login_token(value),
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
            r"(?P<keep>\b(?:[A-Z][A-Z0-9_]*_)?SECRET_KEY(?:_[A-Z0-9_]*)?[ \t]*=[ \t]*)ASSIGNED",
            any,
        ),
        shape(r"(?P<keep>\bSSHPASS[ \t]*=[ \t]*)ASSIGNED", any),
        shape(
            r"(?P<keep>\b[A-Z][A-Z0-9_]*_(?:KEY|KEY_ID|AUTH)[ \t]*=[ \t]*)ASSIGNED",
            |_, value| looks_secret(value),
        ),
        shape(
            r"(?P<keep>TOOLset(?:[ \t]+-[A-Za-z]+)+[ \t]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)[ \t]+)FLAG_VALUE",
            |caps, _| names_a_secret(&caps["name"]),
        ),
        shape(
            r"(?i)(?P<keep>aws_secret_access_key(?:[ \t]*[=:][ \t]*|[ \t]+))ASSIGNED",
            |caps, value| caps["keep"].contains(['=', ':']) || looks_aws_secret(value),
        ),
    ]
});

/// `pattern` with its placeholders filled in: `ARGS`, `TOOL`, `ASSIGNED`,
/// `FLAG_VALUE`, `VALUE`, and `SPACED` (a one-letter flag's `=`, spaces
/// or nothing before its value).
fn expand(pattern: &str) -> String {
    pattern
        .replace("ARGS", TOOL_ARGUMENTS)
        .replace("TOOL", TOOL_START)
        .replace("ASSIGNED", ASSIGNED_VALUE)
        .replace("FLAG_VALUE", FLAG_VALUE)
        .replace("VALUE", VALUE)
        .replace("SPACED", r"(?:=|[ \t]*)")
}

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
/// `value` group, and whether a match of it is a
/// credential.
struct CommandLineShape {
    pattern: regex::Regex,
    is_credential: IsCredential,
}

/// Redact a URL's credentials ([`redact_url_credentials`]), then every
/// [`COMMAND_LINE_SHAPES`] credential in `text`, keeping each one's `keep`
/// and quotes, then a user-named login's `-p` ([`redact_user_logins`]).
pub(super) fn redact_command_lines(text: String) -> String {
    let text = redact_url_credentials(&text);
    let text = COMMAND_LINE_SHAPES.iter().fold(text, |text, shape| {
        shape
            .pattern
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                let keep = &caps["keep"];
                let (open, inner, close) = unquoted(&caps["value"]);
                match (shape.is_credential)(caps, inner) {
                    true => format!("{keep}{open}[REDACTED]{close}"),
                    false => caps[0].to_string(),
                }
            })
            .into_owned()
    });
    redact_user_logins(&text)
}

/// A `login` subcommand's `-p` (spaced, glued or after `=`), any tool's.
static USER_LOGIN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&expand(
        r"(?P<keep>TOOL[A-Za-z][\w.-]*[ \t]+loginARGS-pSPACED)VALUE",
    ))
    .expect("static login regex is valid")
});

/// Redact the `-p` value of every `login` subcommand ([`USER_LOGIN`])
/// whose command names a user ([`names_a_user`]): `az acr login -n r -u
/// me -p pw`, `… login -p pw -u me`.
///
/// Linear (round 5, M1): each command is scanned once, from its first
/// `login` match to its end (the first separator after that match's
/// value, so a quoted `;` in the password is no end), for its user; every
/// later match in the same command reuses that answer, and the search for
/// the next match starts after the last one.
fn redact_user_logins(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut from) = (0, 0);
    let mut command: Option<(usize, bool)> = None;
    while let Some(caps) = USER_LOGIN.captures_at(text, from) {
        let whole = caps.get(0).expect("a match has its whole span");
        let value = caps.name("value").expect("a login match has its value");
        from = whole.end();
        let names_user = match command {
            Some((end, names_user)) if whole.start() < end => names_user,
            Some(_) | None => {
                let end = command_end(text, value.end());
                let names_user = names_a_user(&text[whole.start()..end]);
                command = Some((end, names_user));
                names_user
            }
        };
        let (open, inner, close) = unquoted(value.as_str());
        if names_user && !inner.is_empty() {
            out.push_str(&text[copied..value.start()]);
            out.push_str(&format!("{open}[REDACTED]{close}"));
            copied = value.end();
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Where the command around `from` ends: at its line's end or the first
/// command separator (`;`, `&`, `|`) at or after `from`.
fn command_end(text: &str, from: usize) -> usize {
    text[from..]
        .find(['\n', '\r', ';', '&', '|'])
        .map_or(text.len(), |at| from + at)
}

/// Whether a `login` command names a user: a `-u` or `--username` flag
/// after its `login`, before or after its `-p`.
fn names_a_user(command: &str) -> bool {
    command
        .split_whitespace()
        .skip_while(|word| *word != "login")
        .any(|word| word.starts_with("-u") || word.starts_with("--username"))
}

/// Where a URL's authority starts: after its scheme's `://`.
static URL_SCHEME: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\b[A-Za-z][A-Za-z0-9+.-]*://").expect("static URL scheme regex is valid")
});

/// Redact the credential in every URL's userinfo (`scheme://userinfo@host`,
/// round 5 M3), keeping the scheme, the user, `@` and the host:
///
/// - a `user:password` (the user empty or not): the password, when it is
///   neither empty (`mysql://root:@db`) nor a port before a path
///   ([`is_port`], `http://localhost:8080/@user`);
/// - a userinfo with no `:`: all of it, unless it is a plain username
///   ([`is_plain_username`]) or already redacted ([`REDACTED_USERINFO`]):
///   `https://TOKEN@github.com` is redacted, `ssh://git@host`,
///   `https://user@host` and `https://***@host` are kept.
///
/// The userinfo runs to the last `@` before the host
/// ([`userinfo_len`]), so a password holding `@`, `/`, `#`, `?` or `"`
/// is redacted whole. Linear: a URL with no userinfo is skipped to where
/// its scan stopped, and one with a userinfo to its `@`.
fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut from) = (0, 0);
    while let Some(scheme) = URL_SCHEME.find_at(text, from) {
        let start = scheme.end();
        let (scanned, userinfo) = userinfo_len(&text[start..]);
        from = start + scanned;
        let Some(len) = userinfo else { continue };
        from = start + len + 1;
        let Some(secret) = userinfo_secret(&text[start..start + len]) else {
            continue;
        };
        let (secret_start, secret_end) = (start + secret.start, start + secret.end);
        assert!(
            copied <= secret_start && secret_start < secret_end && secret_end < from,
            "a URL's secret lies inside its userinfo, ahead of what is copied"
        );
        out.push_str(&text[copied..secret_start]);
        out.push_str("[REDACTED]");
        copied = secret_end;
    }
    out.push_str(&text[copied..]);
    out
}

/// How far a URL's authority was scanned in `authority` (the text after
/// `://`), and its userinfo's length (to its last `@`) when it has one.
/// Before any `@`, a `user:password` runs to whitespace (a password may
/// hold `/`, `#`, `?` or `"`), and a userinfo with no `:` stops at a
/// host's end too; after an `@`, the host ends at whitespace or one of
/// `/ ? # " ' < > ) ] ,`.
fn userinfo_len(authority: &str) -> (usize, Option<usize>) {
    let ends_host = |c: char| "/?#\"'<>)],".contains(c);
    let (mut colon, mut at) = (false, None);
    for (index, c) in authority.char_indices() {
        match (c, at, colon) {
            (c, _, _) if c.is_whitespace() => return (index, at),
            ('@', _, _) => at = Some(index),
            (':', None, _) => colon = true,
            (c, Some(_), _) | (c, None, false) if ends_host(c) => return (index, at),
            _ => {}
        }
    }
    (authority.len(), at)
}

/// The range of a userinfo's secret, when it holds one: a
/// `user:password`'s password ([`is_port`] aside), or the whole of a
/// userinfo with no `:` that is no plain username.
fn userinfo_secret(userinfo: &str) -> Option<std::ops::Range<usize>> {
    match userinfo.split_once(':') {
        Some((user, password)) if holds_password(password) => Some(user.len() + 1..userinfo.len()),
        Some(_) => None,
        None if is_plain_username(userinfo) || userinfo == REDACTED_USERINFO => None,
        None => (!userinfo.is_empty()).then_some(0..userinfo.len()),
    }
}

/// The userinfo [`super::redact_url_userinfo`] leaves in place of one:
/// already redacted, so kept.
const REDACTED_USERINFO: &str = "***";

/// Whether the text after a userinfo's `:` is a password: not empty,
/// and not a port ([`is_port`]).
fn holds_password(password: &str) -> bool {
    !password.is_empty() && !is_port(password)
}

/// Whether a would-be password is a host's port and what follows it
/// (`8080/@user`, `80","a`): one to five digits, then one of `/ ? # " '
/// , ] ) > ;`.
fn is_port(password: &str) -> bool {
    let digits = password.len()
        - password
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    (1..=5).contains(&digits)
        && password[digits..].starts_with(['/', '?', '#', '"', '\'', ',', ']', ')', '>', ';'])
}

/// Whether a userinfo with no `:` is a plain username, kept: `git`,
/// `ssh`, `anonymous`, or one to 32 of lower-case ASCII letters, `.`,
/// `_` and `-`, starting with a letter (`user`, `deploy-bot`). A token
/// has digits or capitals, so it is redacted.
fn is_plain_username(userinfo: &str) -> bool {
    matches!(userinfo, "git" | "ssh" | "anonymous")
        || (userinfo.len() <= 32
            && userinfo.starts_with(|c: char| c.is_ascii_lowercase())
            && userinfo
                .chars()
                .all(|c| c.is_ascii_lowercase() || matches!(c, '.' | '_' | '-')))
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

/// Whether a `cargo login` or `vault login` word is a token: at least
/// [`SECRET_MIN_CHARS`] of `A-Z a-z 0-9 . _ -` (`cio…`, `hvs.…`), so
/// `cargo login to publish` and `vault login username=me` are kept.
fn looks_like_login_token(value: &str) -> bool {
    value.chars().count() >= SECRET_MIN_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The endings of a variable's name that make it a secret's in fish's
/// `set -x NAME value`.
const SECRET_NAME_ENDINGS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "_PASS",
    "_PWD",
    "PASSPHRASE",
    "API_KEY",
    "APIKEY",
    "SECRET_KEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "ACCOUNT_KEY",
    "CREDENTIAL",
    "CREDENTIALS",
];

/// Whether a variable's name is a secret's: `SSHPASS`, or one ending in
/// any of [`SECRET_NAME_ENDINGS`], any case (`GITHUB_TOKEN`, `api_key`).
fn names_a_secret(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name == "SSHPASS"
        || SECRET_NAME_ENDINGS
            .iter()
            .any(|ending| name.ends_with(ending))
}

/// Whether a value looks like an AWS secret access key: at least
/// [`SECRET_MIN_CHARS`] of `A-Z a-z 0-9 / + =`.
fn looks_aws_secret(value: &str) -> bool {
    value.chars().count() >= SECRET_MIN_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '+' | '='))
}
