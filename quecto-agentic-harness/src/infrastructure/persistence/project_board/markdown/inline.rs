//! The rules a line of text keeps inline (a title, a paragraph's line, a
//! list item, a text cell). Code spans must close, and are taken as they
//! are; everywhere else nothing but plain text, emphasis and links of one
//! form survives: no raw HTML or comment, character reference, dollar
//! math, image, reference, wiki or bare link, and no backslash escape (the
//! renderer adds the only ones a file holds). Matching uses the `regex`
//! crate alone, which never backtracks.
use super::MarkdownError;
use regex::Regex;
use std::sync::LazyLock;

/// The one link a text may hold: visible text, an https address, no title.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[[^\[\]\\]+\]\(https://[^\s()<>\[\]\\`]+\)").expect("a valid pattern")
});
/// A named, decimal or hexadecimal character reference.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:#[0-9]+|#[xX][0-9A-Fa-f]+|[A-Za-z][A-Za-z0-9]*);").expect("a valid pattern")
});

/// Whether `text` opens with a link of the one form a text may hold.
pub fn starts_with_link(text: &str) -> bool {
    LINK.find(text).is_some_and(|link| link.start() == 0)
}

/// `text` with every closed code span replaced by `x`, so what is left is
/// what markdown reads as markup. A code span left open, or a backtick a
/// backslash escapes (markdown opens no span there), is refused.
fn outside_code(text: &str, at: usize) -> Result<String, MarkdownError> {
    let mut outside = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        if rest[..start].ends_with('\\') {
            return Err(MarkdownError::Escape { line: at });
        }
        outside.push_str(&rest[..start]);
        let run = rest[start..].len() - rest[start..].trim_start_matches('`').len();
        let after = &rest[start + run..];
        let fence = "`".repeat(run);
        // The span closes at the next run of exactly as many backticks.
        let close = after.match_indices(&fence).find(|(at, _)| {
            let before = after[..*at].ends_with('`');
            let beyond = after[at + run..].starts_with('`');
            !before && !beyond
        });
        let close = close.ok_or(MarkdownError::UnbalancedCode { line: at })?;
        outside.push('x');
        rest = &after[close.0 + run..];
    }
    outside.push_str(rest);
    Ok(outside)
}

/// The inline rules `text` keeps.
///
/// # Errors
/// The first rule `text` breaks; `at` is the file line it would be on.
pub fn check_inline(text: &str, at: usize) -> Result<(), MarkdownError> {
    let outside = outside_code(text, at)?;
    let bytes = outside.as_bytes();
    let html = bytes.windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    });
    let touches = |at: usize| bytes.get(at).is_some_and(|byte| *byte != b' ');
    let math = bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'$' && (index.checked_sub(1).is_some_and(touches) || touches(index + 1))
    });
    let escaped = bytes
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_punctuation())
        || outside.ends_with('\\');
    let unlinked = LINK.replace_all(&outside, "x");
    let lower = unlinked.to_ascii_lowercase();
    let bare = ["](", "][", "[[", "http://", "https://", "www."]
        .iter()
        .any(|form| lower.contains(form));
    match () {
        () if html => Err(MarkdownError::Html { line: at }),
        () if REFERENCE.is_match(&outside) => Err(MarkdownError::CharacterReference { line: at }),
        () if math => Err(MarkdownError::Math { line: at }),
        () if outside.contains("![") => Err(MarkdownError::Image { line: at }),
        () if bare => Err(MarkdownError::Link { line: at }),
        () if escaped => Err(MarkdownError::Escape { line: at }),
        () => Ok(()),
    }
}
