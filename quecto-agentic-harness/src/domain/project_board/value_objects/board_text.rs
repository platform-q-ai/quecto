//! Board text as it enters the board (#2481, decision D4): the one spelling
//! of what a person or agent typed (chat, an import, an editor) that the
//! board's markdown files can hold. Only how the text is spelt changes,
//! never what it says or shows:
//!
//! - line ends become one LF (a CRLF or a lone CR);
//! - text is made Unicode NFC (canonically equal, so shown the same);
//! - trailing whitespace goes from every line, and blank lines from the
//!   text's edges;
//! - in prose, a tab, a no-break space or any other space separator becomes
//!   a space; a presentation selector (U+FE0E, U+FE0F) that selects nothing
//!   (no emoji before it, nor, for U+FE0F, a keycap) goes; a run of blank
//!   lines becomes one; and up to three spaces before a block's first line
//!   go, which a reader shows as nothing;
//! - in fenced code only the first three apply: a tab, a no-break space, a
//!   selector or a blank line there is left as written, for the codec to
//!   refuse with its field and line rather than have code changed.
//!
//! Whatever else the codec refuses (a structure or markup) it refuses with
//! the field and line. The normaliser runs before the schema reads text:
//! a task's title and description now, a plan's text and a discussion's
//! messages where they enter (M4).
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// A space separator (`\p{Zs}`): U+0020, no-break, wide and narrow spaces.
static SPACE_SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Zs}$").expect("a valid class"));
/// What a presentation selector may follow: an emoji.
static EMOJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Extended_Pictographic}$").expect("a valid class"));
/// What a keycap is made on (`1`, U+FE0F, U+20E3).
const KEYCAP_BASES: &str = "0123456789#*";
const KEYCAP: char = '\u{20e3}';
/// The most spaces before a block's first line that a reader ignores; a
/// fourth starts indented code.
const IGNORED_INDENT: usize = 3;

fn is(class: &Regex, c: char) -> bool {
    class.is_match(c.encode_utf8(&mut [0; 4]))
}

/// Markdown text (a description, an approach, a message) as the board
/// keeps it.
pub fn text(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut kept: Vec<String> = vec![];
    let mut in_fence = false;
    for line in unified.split('\n') {
        if in_fence {
            let code = nfc(line.trim_end());
            in_fence = !closes_fence(&code);
            kept.push(code);
            continue;
        }
        let fixed = nfc(&prose(line));
        let block_start = kept.last().is_none_or(String::is_empty);
        match (fixed.is_empty(), block_start) {
            // A leading blank line, or one more in a run, says nothing.
            (true, true) => {}
            (true, false) => kept.push(fixed),
            (false, true) => {
                let line = without_ignored_indent(&fixed);
                in_fence = opens_fence(&line);
                kept.push(line);
            }
            (false, false) => {
                in_fence = opens_fence(&fixed);
                kept.push(fixed);
            }
        }
    }
    while kept.last().is_some_and(String::is_empty) {
        kept.pop();
    }
    let normal = kept.join("\n");
    debug_assert!(
        !normal.contains('\r') && !normal.starts_with('\n') && !normal.ends_with('\n'),
        "one LF between lines, none at the edges"
    );
    normal
}

/// One line of text (a title, a criterion, an owner) as the board keeps
/// it: fixed as prose is, and trimmed at both edges.
pub fn line(raw: &str) -> String {
    nfc(prose(raw).trim()).to_string()
}

fn nfc(text: &str) -> String {
    text.nfc().collect()
}

/// A line of prose: each space separator and tab a space, each selector
/// that selects nothing gone, and its trailing whitespace gone.
fn prose(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut fixed = String::with_capacity(line.len());
    for (at, &c) in chars.iter().enumerate() {
        match c {
            '\t' => fixed.push(' '),
            c if is(&SPACE_SEPARATOR, c) => fixed.push(' '),
            '\u{fe0e}' | '\u{fe0f}' if selects(&fixed, c, chars.get(at + 1)) => fixed.push(c),
            '\u{fe0e}' | '\u{fe0f}' => {}
            c => fixed.push(c),
        }
    }
    fixed.trim_end().to_string()
}

/// Whether selector `c`, after `before` and before `next`, selects
/// something: it follows an emoji, or (U+FE0F) it is a keycap's.
fn selects(before: &str, c: char, next: Option<&char>) -> bool {
    let last = before.chars().last();
    let emoji = last.is_some_and(|last| is(&EMOJI, last));
    let keycap = c == '\u{fe0f}'
        && last.is_some_and(|last| KEYCAP_BASES.contains(last))
        && next == Some(&KEYCAP);
    emoji || keycap
}

/// `line` without the spaces before it when they are ones a reader
/// ignores (at most [`IGNORED_INDENT`]); four or more start indented code
/// and are kept, for the codec to refuse.
fn without_ignored_indent(line: &str) -> String {
    let indent = line.len() - line.trim_start_matches(' ').len();
    match indent {
        0..=IGNORED_INDENT => line[indent..].to_string(),
        _ => line.to_string(),
    }
}

/// A line a reader opens fenced code on: three backticks after at most
/// three spaces, the info holding no backtick.
fn opens_fence(line: &str) -> bool {
    let unindented = without_ignored_indent(line);
    unindented
        .strip_prefix("```")
        .is_some_and(|info| !info.contains('`'))
}

/// A line a reader closes fenced code on: three or more backticks alone,
/// after at most three spaces.
fn closes_fence(line: &str) -> bool {
    let unindented = without_ignored_indent(line);
    unindented.len() >= 3 && unindented.chars().all(|c| c == '`')
}

#[cfg(test)]
#[path = "board_text_tests.rs"]
mod tests;
