//! A named secret's value (#2304 review rounds 3 to 5): a secret's label
//! (`password=`, `token: `, `"api_key":`, `<password>`) is kept and only
//! the value after it is redacted. One linear `regex` finds each label; a
//! scanner that only moves forward takes its value. No value reaches past
//! its line.
//!
//! Linearity (round 5, H1): everything a label looks at behind it is its
//! own name and the spaces before that name, which never reach back past
//! the `=` or `:` of the label before it, so each byte is looked at by one
//! label at most.

use std::ops::Range;
use std::sync::LazyLock;

/// The names a secret is labelled by, any case: `api_key`, `token`,
/// `password`, `passwd`, `pwd`, `passphrase`, `secret`, and
/// `access_token`, `access_key`, `secret_key`, `private_key` and
/// `account_key` (each also with `-` or nothing for its `_`, so
/// `accessKey`, `privateKey` and `AccountKey` are names too).
const SECRET_NAMES: &str = concat!(
    r"api[_-]?key|access[_-]?token|access[_-]?key|secret[_-]?key|private[_-]?key",
    r"|account[_-]?key|password|passwd|passphrase|pwd|token|secret",
);

/// A secret's label: either an XML element named for a secret
/// (`<password>`, `<db_password>`), or a secret's name, a quote closing
/// it (a JSON or dict key's, `"password":`, or an escaped JSON body's,
/// `\"token\":`), then `=` or `:`, with spaces or tabs (never a newline)
/// around it. The name matches inside an identifier (`GITHUB_TOKEN=`,
/// `DB_PASSWORD:`, `PGPASSWORD=`), which is kept whole, as the label is.
static NAMED_LABEL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(&format!(
        r#"(?i)<(?P<tag>[A-Za-z0-9_.-]*?(?:{SECRET_NAMES})[A-Za-z0-9_.-]*)>|(?P<name>{SECRET_NAMES})(?P<closed>\\?["']|)[ \t]*[=:][ \t]*"#
    ))
    .expect("static named-label regex is valid")
});

/// Where a label sits, which decides how far its value runs.
#[derive(Clone, Copy)]
enum Context {
    /// Inside a quote glued to its name (`'password: x'`, `"TOKEN=x`):
    /// the value runs to that quote (the string given) or the line's end,
    /// or to a form or connection string's next field (`&grant_type=`,
    /// `;Server=`).
    Enclosed(&'static str),
    /// A key closed by a quote (`"password":`): a value opened by the
    /// same quote is redacted unless it is empty or only spaces.
    Keyed(&'static str),
    /// Starting its line after `:` (`password: two words`, YAML-ish): a
    /// bare value runs to the line's end.
    LineStart,
    /// An XML element's text (`<password>x</password>`): the value runs
    /// to the closing tag, on the same line.
    Element,
    /// Anywhere else: a bare value is one word.
    Inline,
}

/// Redact the value after every [`NAMED_LABEL`] in `input`, keeping the
/// label. A label with no value ([`named_value`]), a name that is a path's
/// segment (`/etc/passwd:`) and a working directory (`PWD=/home/u`) are
/// left as they are, and the search for the next label goes on right
/// after it.
pub(super) fn redact_named(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(label) = NAMED_LABEL.captures_at(input, from) {
        let whole = label.get(0).expect("a match has its whole span");
        from = whole.end();
        let Some(context) = context_of(&input[..whole.start()], &label) else {
            continue;
        };
        let rest = &input[whole.end()..];
        let Some(value) = named_value(rest, context, whole.as_str()) else {
            continue;
        };
        let name = label.name("name").map_or("", |name| name.as_str());
        if names_a_directory(name, &rest[value.clone()]) {
            continue;
        }
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

/// Whether a `pwd` label's value is a working directory (`PWD=/home/u`,
/// `OLDPWD=/tmp`): an absolute path. Every other `pwd` value is a
/// password (`Pwd=…;` in a connection string).
fn names_a_directory(name: &str, value: &str) -> bool {
    name.eq_ignore_ascii_case("pwd") && value.starts_with('/')
}

/// The [`Context`] of a label, from the text `before` it and its
/// captures, or `None` when its name is a path's segment (glued to a `/`,
/// `/etc/passwd:`). Where the label's line starts is only looked for
/// when no quote encloses it, and only in the spaces before its name.
fn context_of(before: &str, label: &regex::Captures<'_>) -> Option<Context> {
    if label.name("tag").is_some() {
        return Some(Context::Element);
    }
    let closed = label.name("closed").map_or("", |closed| closed.as_str());
    if let Some(quote) = quote_named(closed) {
        return Some(Context::Keyed(quote));
    }
    let name_start = before.trim_end_matches(is_identifier);
    if name_start.ends_with('/') {
        return None;
    }
    if let Some(quote) = enclosing_quote(name_start) {
        return Some(Context::Enclosed(quote));
    }
    match (starts_line(name_start), label[0].contains(':')) {
        (true, true) => Some(Context::LineStart),
        (true, false) | (false, _) => Some(Context::Inline),
    }
}

/// Whether `c` belongs to an identifier a secret's name may sit in.
fn is_identifier(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The quote strings a value may be opened and closed by: plain, or
/// escaped inside a JSON string.
const QUOTES: [&str; 4] = ["\"", "'", "\\\"", "\\'"];

/// `text` as one of [`QUOTES`], when it is one.
fn quote_named(text: &str) -> Option<&'static str> {
    QUOTES.into_iter().find(|quote| *quote == text)
}

/// The longest of [`QUOTES`] `text` starts with, when it starts with one.
fn opening_quote(text: &str) -> Option<&'static str> {
    QUOTES
        .into_iter()
        .filter(|quote| text.starts_with(quote))
        .max_by_key(|quote| quote.len())
}

/// The quote a label's identifier is glued to, when there is one:
/// `'password:`, `"DB_PASSWORD=`, `\"token=`.
fn enclosing_quote(name_start: &str) -> Option<&'static str> {
    let quote = name_start
        .chars()
        .next_back()
        .filter(|c| matches!(c, '"' | '\''))?;
    let escaped = name_start[..name_start.len() - 1].ends_with('\\');
    quote_named(&format!("{}{quote}", if escaped { "\\" } else { "" }))
}

/// Whether nothing but spaces, tabs and a YAML list's `-` comes before a
/// label's identifier on its line. It looks only at the end of
/// `name_start` — spaces, one `-`, spaces, then a newline or the text's
/// start — never the whole line (H1: a backward scan per label was
/// quadratic).
fn starts_line(name_start: &str) -> bool {
    let blank = [' ', '\t', '\r'];
    let spaced = name_start.trim_end_matches(blank);
    let line = spaced
        .strip_suffix('-')
        .unwrap_or(spaced)
        .trim_end_matches(blank);
    line.is_empty() || line.ends_with('\n')
}

/// The byte range, in `rest` (the text after a label), of the value to
/// redact, or `None` when there is none:
///
/// - [`Context::Enclosed`]: the run up to the label's closing quote
///   (escapes skipped), the line's end, or a next field
///   ([`next_field`]); empty, the label is a search pattern (`grep
///   'password:' f`);
/// - [`Context::Keyed`], opened by the key's own quote: the run to its
///   closing quote (escapes skipped), unless it is empty or only spaces;
/// - [`Context::Element`]: the run to its closing tag's `</`, on its
///   line (a generic, `Option<Secret>,`, has none);
/// - opened by any other quote: the run to its closing quote, when it
///   starts with neither a space nor that closing quote: either closes a
///   search pattern (`grep "token=" src/`, `rg '"token":' f`);
/// - else the bare value that follows ([`bare_value`]), which for
///   [`Context::LineStart`] is the rest of the line.
///
/// Every scan stops at the line's end.
fn named_value(rest: &str, context: Context, label: &str) -> Option<Range<usize>> {
    let spaced = label.ends_with([' ', '\t']);
    let range = match (context, opening_quote(rest)) {
        (Context::Enclosed(quote), _) => {
            let end = quoted_end(rest, quote);
            0..next_field(&rest[..end]).unwrap_or(end)
        }
        (Context::Element, _) => {
            let end = rest.find(['<', '\n', '\r']).unwrap_or(rest.len());
            match rest[end..].starts_with("</") {
                true => 0..end,
                false => return None,
            }
        }
        (Context::Keyed(key), Some(quote)) if key == quote => {
            let inner = &rest[quote.len()..];
            let end = quoted_end(inner, quote);
            match inner[..end].trim().is_empty() {
                true => return None,
                false => quote.len()..quote.len() + end,
            }
        }
        (_, Some(quote)) => {
            let inner = &rest[quote.len()..];
            match inner.chars().next() {
                Some(first) if !first.is_whitespace() && !inner.starts_with(quote) => {
                    quote.len()..quote.len() + quoted_end(inner, quote)
                }
                Some(_) | None => return None,
            }
        }
        (Context::LineStart, None) => bare_value(rest, spaced, Reach::Line)?,
        (Context::Keyed(_) | Context::Inline, None) => bare_value(rest, spaced, Reach::Word)?,
    };
    (!range.is_empty()).then_some(range)
}

/// Where a form's or a connection string's next field starts in a value
/// (`S3CR&grant_type=x`, `S3CR;Server=db`): the first `&` or `;`
/// followed by a field's name and `=`.
fn next_field(value: &str) -> Option<usize> {
    value
        .char_indices()
        .filter(|(_, c)| matches!(c, '&' | ';'))
        .map(|(at, _)| at)
        .find(|at| {
            let after = &value[at + 1..];
            let name = after.len()
                - after
                    .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || "_.-".contains(c))
                    .len();
            name > 0 && after[name..].starts_with('=')
        })
}

/// Where `text`'s first line ends.
fn line_end(text: &str) -> usize {
    text.find(['\n', '\r']).unwrap_or(text.len())
}

/// Where a quoted value opened by `quote` ends in `inner` (the text after
/// the opening quote): at its closing quote, or the line's end, or the
/// text's end. A closing quote is one not escaped: after an even number
/// of backslashes for a plain quote, and for a JSON-escaped one (`\"`,
/// itself after a backslash) after a number of backslashes one more than
/// a multiple of four, so `\\\"` (an escaped quote inside it) is skipped.
fn quoted_end(inner: &str, quote: &str) -> usize {
    let (mark, escaped) = match quote.strip_prefix('\\') {
        Some(mark) => (mark, true),
        None => (quote, false),
    };
    let mut backslashes = 0usize;
    for (at, c) in inner.char_indices() {
        let closes = match escaped {
            true => backslashes % 4 == 1,
            false => backslashes % 2 == 0,
        };
        match c {
            '\n' | '\r' => return at,
            '\\' => backslashes += 1,
            c if mark.starts_with(c) && closes => return at - usize::from(escaped),
            _ => backslashes = 0,
        }
    }
    inner.len()
}

/// A bare value's end: whitespace, a quote, a backslash, or the
/// punctuation that closes the structure it sits in (`,` `&` `;` `#` `}`
/// `)` `<` `>` `@`), so `?access_token=…&state=x`, `db.password=…,db.user`
/// and `x-access-token:…@host` keep what follows. `]` is not one, so
/// `[REDACTED]` redacts to itself.
fn ends_bare(c: char) -> bool {
    c.is_whitespace() || "\"'\\,&;#})<>@".contains(c)
}

/// The auth schemes whose name comes before a credential in a value.
const AUTH_SCHEMES: [&str; 3] = ["bearer", "basic", "token"];

/// How far a bare value may reach.
#[derive(Clone, Copy)]
enum Reach {
    /// To its line's end ([`Context::LineStart`]).
    Line,
    /// One word ([`Context::Inline`], [`Context::Keyed`]).
    Word,
}

/// The bare value at the start of `rest` (round 5, M2):
///
/// - a word ending at a quote after a wrapper ([`is_wrapper`]:
///   `Some(`, `Secret::new(`, `[`, `b`, `u`, `$` …): the quoted string
///   inside, keeping the wrapper and quotes (`Some("[REDACTED]")`);
/// - a spaced label's code ([`reads_as_code`]): nothing;
/// - reaching a [`Reach::Line`]: the rest of the line;
/// - a word ending at a quote followed by a letter or digit
///   (`abc"S3CR"`): the word, the quoted string and what is glued after
///   it, as one value;
/// - else the word; when it is one of [`AUTH_SCHEMES`] followed, on its
///   line, by another word, both.
///
/// A quote after a word followed by anything else (`"echo token=abc" &&
/// ls`) closes the string the label sits in, and is kept.
fn bare_value(rest: &str, spaced: bool, reach: Reach) -> Option<Range<usize>> {
    let word = rest.find(ends_bare).unwrap_or(rest.len());
    match (opening_quote(&rest[word..]), reach) {
        (Some(quote), _) if is_wrapper(&rest[..word]) => {
            let start = word + quote.len();
            Some(start..start + quoted_end(&rest[start..], quote))
        }
        _ if spaced && reads_as_code(rest) => None,
        (_, Reach::Line) => Some(0..rest[..line_end(rest)].trim_end().len()),
        (Some(_), Reach::Word) if glued_quote(&rest[word..]).is_some() => {
            Some(0..glued_end(rest, word))
        }
        (Some(_) | None, Reach::Word) => Some(scheme_and_credential(rest, word)),
    }
}

/// The quote at the start of `text` when a letter or digit follows it: a
/// quoted string glued into a bare word (`abc"S3CR"`), never the quote
/// closing the string the label sits in.
fn glued_quote(text: &str) -> Option<&'static str> {
    opening_quote(text).filter(|quote| {
        text[quote.len()..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
    })
}

/// The end of a bare word holding glued quoted strings, from its first
/// quote at `at`: each quoted string to its closing quote (or the line's
/// end), then the bare word after it, until neither goes on.
fn glued_end(rest: &str, mut at: usize) -> usize {
    while let Some(quote) = glued_quote(&rest[at..]) {
        let inner = at + quote.len();
        let close = inner + quoted_end(&rest[inner..], quote);
        at = match rest[close..].starts_with(quote) {
            true => close + quote.len(),
            false => close,
        };
        at += rest[at..].find(ends_bare).unwrap_or(rest.len() - at);
        assert!(at <= rest.len(), "a glued value ends inside its text");
    }
    at
}

/// Whether `word` (the bare text before a quote) only wraps the quoted
/// string after it: calls (`Some(`, `Secret::new(`, `vec![`), an opening
/// bracket, and at the end, at most one string prefix (`b`, `u`, `r`,
/// `br`, `f`, `$`).
fn is_wrapper(word: &str) -> bool {
    let prefixes = ["", "b", "u", "r", "br", "rb", "f", "$", "B", "U", "R"];
    prefixes.iter().any(|prefix| {
        word.strip_suffix(prefix)
            .is_some_and(|calls| is_calls(calls) && (!calls.is_empty() || !prefix.is_empty()))
    })
}

/// Whether `text` is nothing but calls and brackets opened one after
/// another: `Some(`, `Some(Secret(`, `Secret::new(`, `vec![`, `[`.
fn is_calls(text: &str) -> bool {
    let mut rest = text;
    while !rest.is_empty() {
        let path = path_len(rest);
        let bang = usize::from(rest[path..].starts_with('!'));
        let opened = &rest[path + bang..];
        let next = match opened.chars().next() {
            Some('(') if path > 0 => 1,
            Some('[') => 1,
            Some(_) | None => return false,
        };
        rest = &opened[next..];
    }
    true
}

/// The length of the identifier path at the start of `text`
/// (`lexer.next_token`, `Secret::new`), or 0.
fn path_len(text: &str) -> usize {
    let ident = |text: &str| {
        let mut chars = text.char_indices();
        match chars.next() {
            Some((_, c)) if c.is_ascii_alphabetic() || c == '_' => chars
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map_or(text.len(), |(at, _)| at),
            Some(_) | None => 0,
        }
    };
    let mut len = ident(text);
    while len > 0 {
        let separator = [".", "::"]
            .into_iter()
            .find(|separator| text[len..].starts_with(separator));
        let Some(separator) = separator else { break };
        let next = ident(&text[len + separator.len()..]);
        if next == 0 {
            break;
        }
        len += separator.len() + next;
    }
    len
}

/// Whether a spaced label's bare value reads as code, not a secret: an
/// identifier path followed by a call, a generic or `?`
/// (`lexer.next_token()?`, `Option<Secret>`), or a type's name (a
/// capital, then letters, one lower-case at least) followed by `,` or
/// `>` (`api_key: Secret,`).
fn reads_as_code(rest: &str) -> bool {
    let path = path_len(rest);
    let after = &rest[path..];
    let is_type = rest[..path].starts_with(|c: char| c.is_ascii_uppercase())
        && rest[..path].chars().all(|c| c.is_ascii_alphabetic())
        && rest[..path].chars().any(|c| c.is_ascii_lowercase());
    path > 0 && (after.starts_with(['(', '<', '?']) || is_type && after.starts_with([',', '>']))
}

/// The word at the start of `rest`, `word` long; when it is one of
/// [`AUTH_SCHEMES`] followed, on its line, by another word, both.
fn scheme_and_credential(rest: &str, word: usize) -> Range<usize> {
    let is_scheme = AUTH_SCHEMES
        .iter()
        .any(|scheme| rest[..word].eq_ignore_ascii_case(scheme));
    let after = &rest[word..];
    let spaced = after.len() - after.trim_start_matches([' ', '\t']).len();
    match (is_scheme, spaced) {
        (true, 1..) => {
            let credential = &after[spaced..];
            let len = credential.find(ends_bare).unwrap_or(credential.len());
            0..word + spaced * usize::from(len > 0) + len
        }
        _ => 0..word,
    }
}
