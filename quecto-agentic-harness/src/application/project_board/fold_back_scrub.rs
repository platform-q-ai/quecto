//! What a fold-back keeps of text from outside the board (S4): each owner
//! and evidence part scrubbed, or dropped when in doubt, and each bare
//! `https://` address written as a link. The rules are the parent
//! module's to tell.
use crate::domain::project_board::value_objects::board_text;
use crate::domain::shared::redaction::redact_secrets;
use regex::{Captures, Regex};
use std::cell::Cell;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// The scheme of the one address a board line links.
const HTTPS: &str = "https://";
/// The text of a link written for a bare address.
const LINK_TEXT: &str = "link";
/// What may close a query and is given back to the text around it: a code
/// span's backtick, a link's or quote's close, punctuation.
const QUERY_CLOSERS: [char; 12] = ['`', ')', ']', '>', '"', '\'', '.', ',', ';', ':', '!', '?'];
/// An address anywhere in text (in a code span or a link included), to
/// the next space: a `scheme://` one (`scheme`, then `rest`); a
/// scheme-relative `//host…` one after the start, a space or an opening
/// mark (`lead`, then `relative`); or a bare `host.tld[:port]` one
/// followed by a `/`, `?` or `#` (`bare`).
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?P<scheme>[A-Za-z][A-Za-z0-9+.\-]*://)(?P<rest>\S*)",
        r#"|(?P<lead>^|[\s`(\[<"'])(?P<relative>//[^\s/]\S*)"#,
        r"|\b(?P<bare>[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+(?::[0-9]+)?[/?#]\S*)",
    ))
    .expect("a valid pattern")
});
/// What an address holding an `@`, or whose authority is ambiguous, is
/// written as past its scheme.
const SCRUBBED: &str = "***";
/// The at signs: ASCII, fullwidth and small.
const AT_SIGNS: [char; 3] = ['@', '\u{ff20}', '\u{fe6b}'];
/// The only words outside an address that may hold an at sign: a name and
/// a number (`worker@1`), or a reflog reference (`HEAD@{1}`).
static AT_EXEMPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Za-z0-9._\-]+@[0-9]+|[A-Za-z0-9._/\-]*@\{[0-9]+\})$")
        .expect("a valid pattern")
});
/// A percent-encoded at sign, encoded once or more (`%40`, `%2540`, …).
static AT_ENCODED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%(?:25)*40").expect("a valid pattern"));
/// The words that name a credential: a line holding one, in any case and
/// as any part of a word, is dropped (a line that merely mentions one
/// too: the board is public). `basic` is [`BASIC`]'s.
const CREDENTIAL_WORDS: [&str; 13] = [
    "authorization",
    "bearer",
    "password",
    "passwd",
    "pwd=",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "credential",
    "private_key",
    "private-key",
];
/// `basic` before anything but a letter, or at the end, in folded text
/// (so "basically" is no credential).
static BASIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"basic(?:[^a-z]|$)").expect("a valid pattern"));
/// An invisible character: a format character (`Cf`: a soft hyphen, a
/// direction mark, a zero-width space, a tag) or any other default
/// ignorable one (a grapheme joiner, a variation selector). A part holding
/// one is in doubt: it can hide what a word says.
static INVISIBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{Cf}\p{Default_Ignorable_Code_Point}]").expect("a valid pattern")
});
/// The longest fragment an address keeps: a short anchor.
pub(super) const MAX_FRAGMENT_CHARS: usize = 64;
/// What may open a word, set apart before the word is judged.
const WORD_OPENERS: [char; 6] = ['`', '(', '[', '<', '"', '\''];
/// The line breaks [`board_text::line`] reads as one: kept by [`spaced`],
/// so a line they break is still refused as two.
const LINE_BREAKS: [char; 6] = ['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}'];
/// What may stand before a bare address and stay outside its link.
const OPENERS: [char; 2] = ['<', '('];
/// What may follow a bare address and stay outside its link.
const CLOSERS: [char; 8] = ['>', ')', '.', ',', ';', ':', '!', '?'];

/// `raw` from outside the board as one board line, or none when in doubt
/// (the board is public: a doubtful line is dropped, never rewritten
/// further). In doubt: `raw` past `cap` characters, its board limit (never
/// cut, since a cut may strip the `@` that marks a secret; and nothing past
/// it is scrubbed, so no work grows past it); an [`INVISIBLE`] character;
/// a credential word ([`names_a_credential`]); an
/// address [`kept`] doubts; or, once secrets are redacted, an at sign
/// (`%40` too) outside an address that is no [`AT_EXEMPT`] word.
/// Otherwise its spaces are made plain ([`spaced`]), each address kept to
/// what [`kept`] keeps, and every secret-shaped span redacted.
pub(super) fn scrubbed(raw: &str, cap: usize) -> Option<String> {
    match raw.chars().nth(cap) {
        None => {}
        Some(_) => return None,
    }
    match INVISIBLE.is_match(raw) {
        false => {}
        true => return None,
    }
    let spaced = spaced(raw);
    match names_a_credential(&spaced) {
        false => {}
        true => return None,
    }
    let doubt = Cell::new(false);
    let addressed = ADDRESS.replace_all(&spaced, |found: &Captures<'_>| {
        kept_address(found).unwrap_or_else(|| {
            doubt.set(true);
            found[0].to_string()
        })
    });
    let redacted = redact_secrets(&addressed);
    match (doubt.get(), at_signs_exempt(&redacted)) {
        (false, true) => {}
        _ => return None,
    }
    debug_assert!(
        redacted
            .split(' ')
            .filter(|word| word.contains("://"))
            .all(|word| !holds_at(word)),
        "no userinfo survives: {redacted:?}"
    );
    Some(board_text::line(&redacted))
}

/// Whether `text` names a credential: once folded (NFKC, so a fullwidth
/// or other compatibility form reads as its letters, then lower case), it
/// holds one of [`CREDENTIAL_WORDS`] as any part of a word, or [`BASIC`].
fn names_a_credential(text: &str) -> bool {
    let folded = text.nfkc().collect::<String>().to_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| folded.contains(word)) || BASIC.is_match(&folded)
}

/// Whether `text` holds an at sign: `@`, fullwidth, small, or a
/// percent-encoded one ([`AT_ENCODED`]).
fn holds_at(text: &str) -> bool {
    text.contains(AT_SIGNS) || AT_ENCODED.is_match(text)
}

/// Whether every word of `text` that holds an at sign ([`holds_at`]), its
/// opening and closing marks set apart, is an [`AT_EXEMPT`] one (an
/// allowlist).
fn at_signs_exempt(text: &str) -> bool {
    text.split(' ')
        .map(|word| {
            word.trim_start_matches(WORD_OPENERS)
                .trim_end_matches(QUERY_CLOSERS)
        })
        .all(|word| !holds_at(word) || AT_EXEMPT.is_match(word))
}

/// `raw` with every space but a line break a plain one, so an address ends
/// where a reader sees it end (a no-break space cannot hide the rest of a
/// password past it).
fn spaced(raw: &str) -> String {
    raw.chars()
        .map(|c| match c.is_whitespace() && !LINE_BREAKS.contains(&c) {
            true => ' ',
            false => c,
        })
        .collect()
}

/// What an address in text keeps of itself ([`kept`]), its scheme, its
/// `//` or the start of its bare host as they were, and the text before
/// it (`lead`) left alone; none when [`kept`] doubts it.
fn kept_address(found: &Captures<'_>) -> Option<String> {
    let part = |name: &str| found.name(name).map(|part| part.as_str());
    match (part("scheme"), part("relative"), part("bare")) {
        (Some(scheme), _, _) => kept(scheme, part("rest").unwrap_or(""), Form::Named),
        (None, Some(relative), _) => {
            let rest = relative.strip_prefix("//").unwrap_or(relative);
            let kept = kept("//", rest, Form::Named)?;
            Some(format!("{}{kept}", part("lead").unwrap_or("")))
        }
        (None, None, Some(bare)) => kept("", bare, Form::Bare),
        (None, None, None) => unreachable!("ADDRESS matches one of its forms"),
    }
}

/// How an address was written: with a scheme or `//`, or as a bare host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Named,
    Bare,
}

/// What came of judging an address.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Judged {
    /// Kept: its unambiguous authority, path and anchor.
    Kept(String),
    /// Written [`SCRUBBED`]: it holds an at sign, or an ambiguous
    /// authority.
    Scrubbed,
    /// In doubt: its line is dropped.
    Doubt,
}

/// What an address keeps of `span` (all of it past `prefix`, up to the
/// next space), an allowlist ([`judged`]): kept, written `prefix***`, or
/// none when in doubt. The [`QUERY_CLOSERS`] that end the span belong to
/// the text around it (a code span's backtick, a link's close) and are
/// given back, but for a parenthesis or bracket that closes one opened in
/// what was dropped.
fn kept(prefix: &str, span: &str, form: Form) -> Option<String> {
    let core = span.trim_end_matches(QUERY_CLOSERS);
    let address = match judged(core, form) {
        Judged::Kept(address) => address,
        Judged::Scrubbed => SCRUBBED.to_string(),
        Judged::Doubt => return None,
    };
    let closing = balanced(core, &address, &span[core.len()..]);
    Some(format!("{prefix}{address}{closing}"))
}

/// What `core` (an address past its scheme) comes to. Kept only when it
/// holds no at sign and its authority is unambiguous ([`unambiguous`]):
/// that authority, its path up to its first `?`, `#`, `;` or `%3F`, and a
/// short anchor fragment straight after that path; a query or matrix
/// parameters (and any fragment after them) are dropped. An at sign may
/// end a userinfo anywhere (a password may hold `/`, `?` or `#`), so one
/// anywhere, or an ambiguous authority, is [`Judged::Scrubbed`]. In doubt:
/// a fragment that is no short anchor ([`anchor_shaped`]), a `%25` (a
/// doubly encoded mark) in its path, or a `;` in a bare host's word.
fn judged(core: &str, form: Form) -> Judged {
    let (authority, rest) = core.split_at(core.find(['/', '?', '#']).unwrap_or(core.len()));
    let at_free = !holds_at(core);
    match (at_free, unambiguous(authority)) {
        (true, true) => {}
        _ => return Judged::Scrubbed,
    }
    let (path, after) = rest.split_at(path_end(rest));
    let fragment = core.split_once('#').map(|(_, fragment)| fragment);
    let plain = !path.contains("%25")
        && fragment.is_none_or(anchor_shaped)
        && (form == Form::Named || !core.contains(';'));
    match plain {
        true => {}
        false => return Judged::Doubt,
    }
    Judged::Kept(match after.strip_prefix('#') {
        Some(fragment) => format!("{authority}{path}#{fragment}"),
        None => format!("{authority}{path}"),
    })
}

/// Where a path ends in `rest` (what follows an authority): at its first
/// `?`, `#` or `;`, or an encoded `?` (`%3F`, either case).
fn path_end(rest: &str) -> usize {
    let marked = rest.find(['?', '#', ';']).unwrap_or(rest.len());
    let encoded = rest.to_ascii_lowercase().find("%3f").unwrap_or(rest.len());
    marked.min(encoded)
}

/// `closing`, the marks after an address, less each `)` or `]` that
/// closes a `(` or `[` opened in what was dropped of `core` to keep
/// `kept`: the link or text around it stays balanced. One pass.
pub(super) fn balanced(core: &str, kept: &str, closing: &str) -> String {
    let count = |text: &str, mark: char| text.matches(mark).count();
    let unclosed = |open: char, close: char| {
        let opened = count(core, open).saturating_sub(count(kept, open));
        let closed = count(core, close).saturating_sub(count(kept, close));
        opened.saturating_sub(closed)
    };
    let (mut parens, mut brackets) = (unclosed('(', ')'), unclosed('[', ']'));
    closing
        .chars()
        .filter(|c| match (c, parens, brackets) {
            (')', 1.., _) => {
                parens -= 1;
                false
            }
            (']', _, 1..) => {
                brackets -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

/// Whether `host`, an authority, is unambiguous: an ASCII host name and,
/// perhaps, a port of one or more digits, and nothing else (no userinfo,
/// no IPv6 or non-ASCII host, which are scrubbed rather than read).
fn unambiguous(host: &str) -> bool {
    let (name, port) = host.split_once(':').unwrap_or((host, "0"));
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
}

/// Whether `fragment` is a short anchor: 1 to [`MAX_FRAGMENT_CHARS`]
/// letters, digits, `_`, `.` and `-` (`L10-L20`, `readme.md`), and nothing
/// else.
fn anchor_shaped(fragment: &str) -> bool {
    (1..=MAX_FRAGMENT_CHARS).contains(&fragment.chars().count())
        && fragment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// `text` with each word that is a whole bare address written as a link
/// to it ([`linked_word`]).
pub(super) fn linked(text: &str) -> String {
    text.split(' ')
        .map(linked_word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word` with the address it holds, once any of [`OPENERS`] before it
/// and [`CLOSERS`] after it are set apart, written as a link to that
/// address (angle brackets, an autolink's, dropped); `word` as it is when
/// what is left is no [`address`].
pub(super) fn linked_word(word: &str) -> String {
    let opened = word.trim_start_matches(OPENERS);
    let trimmed = opened.trim_end_matches(CLOSERS);
    // A close that balances an open inside the address is the address's
    // (`…/Rust_(programming_language)`): counted once, taken in one step.
    let unclosed = trimmed
        .matches('(')
        .count()
        .saturating_sub(trimmed.matches(')').count());
    let closes = opened[trimmed.len()..]
        .chars()
        .take_while(|c| *c == ')')
        .count();
    let core = &opened[..trimmed.len() + unclosed.min(closes)];
    let (lead, trail) = (&word[..word.len() - opened.len()], &opened[core.len()..]);
    match address(core) {
        Some(address) => format!(
            "{}[{LINK_TEXT}]({address}){}",
            lead.replace('<', ""),
            trail.replace('>', "")
        ),
        None => word.to_string(),
    }
}

/// `core` as the address a link holds: an `https://` address holding no
/// `@`, whose authority (up to its first `/`, `?` or `#`) is unambiguous;
/// its path to its first `?`, `#`, `;` or `%3F`, and an anchor fragment
/// straight after it, kept; its parentheses percent-encoded; and every
/// character one the codec's address takes (an allowlist). None
/// otherwise, so the word is left for the codec to judge.
pub(super) fn address(core: &str) -> Option<String> {
    let rest = core.strip_prefix(HTTPS)?;
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    match !rest.contains('@') && unambiguous(authority) {
        true => {}
        false => return None,
    }
    let (path, after) = tail.split_at(path_end(tail));
    let kept = match after
        .strip_prefix('#')
        .filter(|fragment| anchor_shaped(fragment))
    {
        Some(fragment) => format!("{path}#{fragment}"),
        None => path.to_string(),
    };
    let encoded = kept.replace('(', "%28").replace(')', "%29");
    encoded
        .chars()
        .all(path_char)
        .then(|| format!("{HTTPS}{authority}{encoded}"))
}

/// A character the codec's address takes in its host part.
fn host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:!&'*+,;=%".contains(c)
}

/// A character the codec's address takes past its host, but an `@`
/// (refused in any address here).
fn path_char(c: char) -> bool {
    host_char(c) || "/?#".contains(c)
}
