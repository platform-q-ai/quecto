//! A named secret's value (#2304 review rounds 3 and 4): a secret's label
//! (`password=`, `token: `, `"api_key":`) is kept and only the value after
//! it is redacted. One linear `regex` finds each label; a scanner that only
//! moves forward takes its value. No value reaches past its line.

use std::ops::Range;
use std::sync::LazyLock;

/// A secret's label: a secret's name, a quote closing it (a JSON or dict
/// key's, `"password":`, or an escaped JSON body's, `\"token\":`), then
/// `=` or `:`, with spaces or tabs (never a newline) around it. The name
/// matches inside an identifier (`GITHUB_TOKEN=`, `DB_PASSWORD:`,
/// `PGPASSWORD=`), which is kept whole, as the label is.
static NAMED_LABEL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)(?:api[_-]?key|token|password|secret|access[_-]?token)(?P<closed>\\?["']|)[ \t]*[=:][ \t]*"#,
    )
    .expect("static named-label regex is valid")
});

/// Where a label sits, which decides how far its value runs.
#[derive(Clone, Copy)]
enum Context {
    /// Inside a quote glued to its name (`'password: x'`, `"TOKEN=x`):
    /// the value runs to that quote (the string given) or the line's end.
    Enclosed(&'static str),
    /// A key closed by a quote (`"password":`): a value opened by the
    /// same quote is redacted unless it is empty or only spaces.
    Keyed(&'static str),
    /// Starting its line after `:` (`password: two words`, YAML-ish): a
    /// bare value runs to the line's end.
    LineStart,
    /// Anywhere else: a bare value is one word.
    Inline,
}

/// Redact the value after every [`NAMED_LABEL`] in `input`, keeping the
/// label. A label with no value ([`named_value`]) is left as it is, and
/// the search for the next goes on right after it.
pub(super) fn redact_named(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(label) = NAMED_LABEL.captures_at(input, from) {
        let whole = label.get(0).expect("a match has its whole span");
        let context = context_of(&input[..whole.start()], &label["closed"], whole.as_str());
        from = whole.end();
        let Some(value) = named_value(&input[whole.end()..], context) else {
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

/// The [`Context`] of a label, from the text `before` it, the quote that
/// `closed` its name (empty when none did) and the label itself.
fn context_of(before: &str, closed: &str, label: &str) -> Context {
    if let Some(quote) = quote_named(closed) {
        return Context::Keyed(quote);
    }
    let name_start = before.trim_end_matches(is_identifier);
    match (enclosing_quote(name_start), starts_line(name_start)) {
        (Some(quote), _) => Context::Enclosed(quote),
        (None, true) if label.contains(':') => Context::LineStart,
        (None, _) => Context::Inline,
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
/// label's identifier on its line.
fn starts_line(name_start: &str) -> bool {
    let line = name_start.rsplit('\n').next().unwrap_or(name_start);
    matches!(line.trim_matches([' ', '\t', '\r']), "" | "-")
}

/// The byte range, in `rest` (the text after a label), of the value to
/// redact, or `None` when there is none:
///
/// - [`Context::Enclosed`]: the run up to the label's closing quote
///   (escapes skipped) or the line's end; empty, the label is a search
///   pattern (`grep 'password:' f`);
/// - [`Context::Keyed`], opened by the key's own quote: the run to its
///   closing quote (escapes skipped), unless it is empty or only spaces;
/// - opened by any other quote: the run to its closing quote, when it
///   starts with neither a space nor that closing quote: either closes a
///   search pattern (`grep "token=" src/`, `rg '"token":' f`);
/// - [`Context::LineStart`], bare: the rest of the line;
/// - else the bare word that follows ([`bare_word`]), with the credential
///   after it when it is an auth scheme's name (`Bearer eyJ…`).
///
/// Every scan stops at the line's end. A scan returning `None` covers
/// only spaces or nothing, so the caller's search stays linear.
fn named_value(rest: &str, context: Context) -> Option<Range<usize>> {
    let opener = QUOTES
        .into_iter()
        .filter(|quote| rest.starts_with(quote))
        .max_by_key(|quote| quote.len());
    let range = match (context, opener) {
        (Context::Enclosed(quote), _) => 0..quoted_end(rest, quote),
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
        (Context::LineStart, None) => 0..rest[..line_end(rest)].trim_end().len(),
        (Context::Keyed(_) | Context::Inline, None) => bare_word(rest),
    };
    (!range.is_empty()).then_some(range)
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

/// The bare word at the start of `rest`; when it is one of
/// [`AUTH_SCHEMES`] followed, on its line, by another word, both.
fn bare_word(rest: &str) -> Range<usize> {
    let word = rest.find(ends_bare).unwrap_or(rest.len());
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
