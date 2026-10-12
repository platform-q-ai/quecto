//! The rules a line of text keeps inline (a title, a paragraph's line, a
//! list item, a text cell). Nothing but plain text, closed code spans,
//! emphasis and links of one form survives: no raw HTML, comment or
//! autolink, character reference, dollar math, image, footnote, reference,
//! wiki or bare link (any `scheme://`, `www.`, `mailto:`, `xmpp:` or
//! email), and no backslash escape (the renderer adds the only ones a file
//! holds). A text, and a link's text, shows something that is not an
//! emphasis marker; a link's text shows no address of its own.
//!
//! The text is read left to right as a reader reads it, so what binds
//! tighter than a code span (an autolink, raw HTML, a link's address) is
//! met before a backtick inside it could open one: `<` stands only before a
//! space or at the end, and `](` only before an allowed address.
//!
//! An address must not show in or around a link, read with the markers
//! dropped and again with every underscore kept: which underscores a reader
//! takes for emphasis is not guessed, so `_[@ada](..)_` and a full stop or
//! a literal marker that joins a link's word into an address are refused
//! by design.
//!
//! Emoji shortcodes (`:smile:`) are kept as written: GitHub draws them as
//! the emoji, a visible change that hides nothing, and refusing them would
//! refuse times such as `12:00:00`.
//!
//! Matching uses the `regex` crate alone, which never backtracks.
use super::MarkdownError;
use super::chars::{mark, seen};
use regex::Regex;
use std::sync::LazyLock;

/// An https address of URL characters (an allowlist: no space, `<`,
/// backtick, `$`, bracket or parenthesis), with no userinfo: an `@` only
/// in its path, query or fragment, never in its host part.
const URL: &str = r"https://[A-Za-z0-9\-._~:!&'*+,;=%]+(?:[/?#][A-Za-z0-9\-._~:/?#@!&'*+,;=%]*)?";
/// The one link a text may hold: text, then a [`URL`], no title.
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\[([^\[\]\\]+)\]\({URL}\)")).expect("a valid pattern"));
/// An email a reader would link: an `@` straight after a character an
/// address's local part may hold.
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9.+_-]@").expect("a valid pattern"));
/// What may follow a link's `]`: its address, as [`LINK`] allows it.
static ADDRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^\({URL}\)")).expect("a valid pattern"));
/// A named, decimal or hexadecimal character reference.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:#[0-9]+|#[xX][0-9A-Fa-f]+|[A-Za-z][A-Za-z0-9]*);").expect("a valid pattern")
});

/// Whether `text`, read as a reader reads it (code spans first), opens
/// with a link of the one form a text may hold.
pub fn starts_with_link(text: &str) -> bool {
    outside_code(text, 0)
        .is_ok_and(|(outside, _)| LINK.find(&outside).is_some_and(|link| link.start() == 0))
}

/// Whether `text` shows a reader something: a character [`seen`] that is
/// not an emphasis or strikethrough marker, which a reader may draw as
/// nothing.
fn shows(text: &str) -> bool {
    text.chars().any(|c| seen(c) && !"*_~".contains(c))
}

/// Whether `text` holds an address a reader would link: any `scheme://`,
/// `www.`, `mailto:`, `xmpp:` or an email. These are the forms GitHub turns
/// into links, so naming them is inherent to finding them; what a pattern
/// here misses, the oracle's cmark-gfm reader (comrak, with GFM's
/// autolinks) catches in the tests.
fn holds_address(text: &str) -> bool {
    // A combining mark inside an address hides it from a pattern, not a
    // reader.
    let text: String = text.chars().filter(|c| !mark(*c)).collect();
    let lower = text.to_ascii_lowercase();
    ["://", "www.", "mailto:", "xmpp:"]
        .iter()
        .any(|form| lower.contains(form))
        || EMAIL.is_match(&text)
}

/// What a closed code span holding nothing a reader sees stands for in
/// [`outside_code`]: a character that is not itself seen.
const BLANK_SPAN: char = '\u{7f}';

/// The closed code span `from` opens with (`from` starts at its first
/// backtick): what it stands for, and the text after it. A code span left
/// open is refused.
fn code_span(from: &str, at: usize) -> Result<(char, &str), MarkdownError> {
    let run = from.len() - from.trim_start_matches('`').len();
    let after = &from[run..];
    let fence = "`".repeat(run);
    // The span closes at the next run of exactly as many backticks.
    let close = after.match_indices(&fence).find(|(at, _)| {
        let before = after[..*at].ends_with('`');
        let beyond = after[at + run..].starts_with('`');
        !before && !beyond
    });
    let (close, _) = close.ok_or(MarkdownError::UnbalancedCode { line: at })?;
    let stands_for = match after[..close].chars().any(seen) {
        true => 'x',
        false => BLANK_SPAN,
    };
    Ok((stands_for, &after[close + run..]))
}

/// `text` as markdown reads it outside code: each closed code span
/// replaced by `x` (or by [`BLANK_SPAN`] when it shows nothing). Read left
/// to right, a `<` before anything but a space (raw HTML, a comment, an
/// autolink), a `](` before anything but an allowed address, a backtick a
/// backslash escapes and a code span left open are refused.
fn outside_code(text: &str, at: usize) -> Result<(String, Vec<(usize, String)>), MarkdownError> {
    let mut outside = String::new();
    let mut spans = vec![];
    let mut rest = text;
    while let Some(next) = rest.find(['`', '<', ']']) {
        let (before, from) = rest.split_at(next);
        outside.push_str(before);
        rest = match from.as_bytes()[0] {
            b'`' if before.ends_with('\\') => return Err(MarkdownError::Escape { line: at }),
            b'`' => {
                let (stands_for, after) = code_span(from, at)?;
                let run = from.len() - from.trim_start_matches('`').len();
                let code = &from[run..from.len() - after.len() - run];
                spans.push((outside.len(), shown_code(code).to_string()));
                outside.push(stands_for);
                after
            }
            b'<' => match from[1..].chars().next() {
                None | Some(' ') => {
                    outside.push('<');
                    &from[1..]
                }
                Some(_) => return Err(MarkdownError::Html { line: at }),
            },
            // `]`: a link's address is read before any backtick in it.
            _ => match ADDRESS.find(&from[1..]) {
                Some(address) => {
                    outside.push(']');
                    outside.push_str(address.as_str());
                    &from[1 + address.end()..]
                }
                None if from[1..].starts_with('(') => {
                    return Err(MarkdownError::Link { line: at });
                }
                None => {
                    outside.push(']');
                    &from[1..]
                }
            },
        };
    }
    outside.push_str(rest);
    Ok((outside, spans))
}

/// A code span's contents as a reader shows them: one space taken from
/// each end when both ends have one and it is not spaces alone.
fn shown_code(code: &str) -> &str {
    let padded = code.len() > 1 && code.starts_with(' ') && code.ends_with(' ');
    match padded && code.bytes().any(|byte| byte != b' ') {
        true => &code[1..code.len() - 1],
        false => code,
    }
}

/// How a reading of a link's words treats the emphasis and strikethrough
/// markers.
#[derive(Clone, Copy)]
enum Markers {
    /// Every `*`, `_` and `~` taken out.
    Dropped,
    /// `*` and `~` taken out, and every `_` kept, since `_` is also a
    /// character an email holds. Which underscores a reader takes for
    /// emphasis is not guessed: an approximation of GFM's pairing missed
    /// shapes (runs of unequal length, the rule of three, other markers
    /// between, a leftover opener), so an underscore around a link is read
    /// as shown, and `_[@ada](..)_` is refused by design.
    UnderscoresKept,
}

/// The words a reader sees in and around each link of `outside`, each
/// word once: the text split at its spaces, with every link's brackets and
/// address taken out, each code span shown as a reader shows it (`spans`,
/// by their offset in `outside`), and the markers taken out as `markers`
/// says, so an address split by a link's edge, by code or by a marker reads
/// whole. One pass, in order: linear in the text.
fn link_words(outside: &str, spans: &[(usize, String)], markers: Markers) -> Vec<String> {
    let links: Vec<(usize, std::ops::Range<usize>, usize)> = LINK
        .captures_iter(outside)
        .map(|link| {
            let whole = link.get(0).expect("a match");
            let text = link.get(1).expect("LINK has a text group");
            (whole.start(), text.range(), whole.end())
        })
        .collect();
    let dropped = |c: char| matches!((markers, c), (_, '*' | '~') | (Markers::Dropped, '_'));
    let (mut words, mut word, mut touched) = (vec![], String::new(), false);
    let (mut next, mut skip_to) = (0, 0);
    for (offset, c) in outside.char_indices() {
        match links.get(next) {
            _ if offset < skip_to => continue,
            Some((open, _, _)) if offset == *open => continue,
            Some((_, text, end)) if offset == text.end => {
                (skip_to, next) = (*end, next + 1);
                continue;
            }
            _ => {}
        }
        // A link's text marks its word even where it opens or closes with
        // a marker, which the reading drops.
        touched |= links
            .get(next)
            .is_some_and(|(open, text, _)| offset > *open && offset < text.end);
        match spans.binary_search_by_key(&offset, |(at, _)| *at) {
            Ok(span) => word.push_str(&spans[span].1),
            Err(_) if dropped(c) => continue,
            Err(_) if c == ' ' => {
                match touched {
                    true => words.push(std::mem::take(&mut word)),
                    false => word.clear(),
                }
                touched = false;
                continue;
            }
            Err(_) => word.push(c),
        }
    }
    if touched {
        words.push(word);
    }
    words
}

/// The inline rules `text` keeps.
///
/// # Errors
/// The first rule `text` breaks; `at` is the file line it would be on.
pub fn check_inline(text: &str, at: usize) -> Result<(), MarkdownError> {
    let (outside, spans) = outside_code(text, at)?;
    let bytes = outside.as_bytes();
    let touches = |at: usize| bytes.get(at).is_some_and(|byte| *byte != b' ');
    let math = bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'$' && (index.checked_sub(1).is_some_and(touches) || touches(index + 1))
    });
    let escaped = bytes
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_punctuation())
        || outside.ends_with('\\');
    // A link's text shows something, with no space at its edges.
    let shown = LINK.captures_iter(&outside).all(|link| {
        let text = link.get(1).expect("LINK has a text group").as_str();
        text.trim() == text && shows(text)
    });
    // No address shows in or around a link, read twice: with every marker
    // dropped, and with every underscore kept, since `_` is both a marker
    // and a character an email holds (the only one an address pattern
    // shares with them).
    let around = [Markers::Dropped, Markers::UnderscoresKept]
        .into_iter()
        .any(|markers| {
            link_words(&outside, &spans, markers)
                .iter()
                .any(|word| holds_address(word))
        });
    // The text as shown: each link's address taken out, its text kept, so
    // the address rule below reads link texts too (a link showing
    // `www.evil.example` is refused).
    let unaddressed = LINK.replace_all(&outside, " $1 ");
    let unlinked = LINK.replace_all(&outside, "x");
    let bare = ["](", "][", "[[", "[^"]
        .iter()
        .any(|form| unlinked.contains(form))
        || holds_address(&unaddressed);
    match () {
        () if REFERENCE.is_match(&outside) => Err(MarkdownError::CharacterReference { line: at }),
        () if math => Err(MarkdownError::Math { line: at }),
        () if outside.contains("![") => Err(MarkdownError::Image { line: at }),
        () if bare || !shown || around => Err(MarkdownError::Link { line: at }),
        () if escaped => Err(MarkdownError::Escape { line: at }),
        () if !shows(&unaddressed) => Err(MarkdownError::Invisible { line: at }),
        () => Ok(()),
    }
}
