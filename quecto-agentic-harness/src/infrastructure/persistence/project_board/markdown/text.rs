//! The rules every line and every piece of text keeps. Matching uses the
//! `regex` crate alone, which never backtracks.
use super::MarkdownError;
use regex::Regex;
use std::sync::LazyLock;

/// Format, line and paragraph separator, private-use and unassigned
/// characters: invisible, or of no known meaning.
static INVISIBLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{Cf}\p{Zl}\p{Zp}\p{Co}\p{Cn}]$").expect("a valid class"));
static PICTOGRAPHIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Extended_Pictographic}$").expect("a valid class"));

fn is(class: &Regex, c: char) -> bool {
    class.is_match(c.encode_utf8(&mut [0; 4]))
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// A zero-width joiner between two emoji (the first may carry its
/// presentation selector), as an emoji sequence writes one.
fn joins_emoji(before: Option<char>, after: Option<char>) -> bool {
    let emoji = |c: Option<char>, selector: bool| {
        c.is_some_and(|c| is(&PICTOGRAPHIC, c) || (selector && c == '\u{fe0f}'))
    };
    emoji(before, true) && emoji(after, false)
}

/// The character rules every line of a file keeps.
///
/// # Errors
/// The first character the line must not hold, or a trailing space.
pub fn check_line(line: &str, at: usize) -> Result<(), MarkdownError> {
    let chars: Vec<char> = line.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        let before = index.checked_sub(1).and_then(|i| chars.get(i).copied());
        match c {
            '\r' => return Err(MarkdownError::CarriageReturn { line: at }),
            '\u{200d}' if joins_emoji(before, chars.get(index + 1).copied()) => {}
            c if c.is_control() => return Err(MarkdownError::Control { line: at }),
            c if is_bidi(c) => return Err(MarkdownError::Bidi { line: at }),
            c if is(&INVISIBLE, c) => return Err(MarkdownError::Invisible { line: at }),
            _ => {}
        }
    }
    match line.ends_with(' ') {
        true => Err(MarkdownError::TrailingSpace { line: at }),
        false => Ok(()),
    }
}

/// The rules a title, a prose line or a cell keeps: no raw HTML or comment
/// (`<` before a letter, `/`, `!` or `?`), and code spans that close.
///
/// # Errors
/// Raw HTML, or a code span left open.
pub fn check_text(text: &str, at: usize) -> Result<(), MarkdownError> {
    let html = text.as_bytes().windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    });
    match (html, code_spans_close(text)) {
        (false, true) => Ok(()),
        (true, _) => Err(MarkdownError::Html { line: at }),
        (false, false) => Err(MarkdownError::UnbalancedCode { line: at }),
    }
}

/// Each run of backticks that opens a code span meets a run of the same
/// length that closes it.
fn code_spans_close(text: &str) -> bool {
    let runs: Vec<usize> = text
        .split(|c| c != '`')
        .map(str::len)
        .filter(|run| *run > 0)
        .collect();
    let mut at = 0;
    while let Some(open) = runs.get(at) {
        match runs[at + 1..].iter().position(|run| run == open) {
            Some(close) => at += close + 2,
            None => return false,
        }
    }
    true
}

/// Whether a prose line starts as a block might: a heading, quote, list,
/// table, thematic break, setext underline, link reference, HTML, fence,
/// indentation, or a backslash (the escape itself).
pub fn starts_like_block(line: &str) -> bool {
    line.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || "#|>=-+*_:[<\\`~ ".contains(c))
}

/// A cell's text in a row: `\` and `|` escaped.
pub fn escaped_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

/// The rules a cell keeps, beyond [`check_text`]: one line, not empty, no
/// space at either end.
///
/// # Errors
/// Why the cell is refused.
pub fn check_cell(value: &str, at: usize) -> Result<(), MarkdownError> {
    let cell = |why| Err(MarkdownError::Cell { line: at, why });
    match () {
        () if value.is_empty() => cell("an empty cell"),
        () if value.contains('\n') => cell("a newline in a cell"),
        () if value.starts_with(' ') || value.ends_with(' ') => cell("space at a cell's edge"),
        () => check_text(value, at),
    }
}
