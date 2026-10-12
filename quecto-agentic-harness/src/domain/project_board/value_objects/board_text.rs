//! Board text as it enters the board (#2481, decision D4): the spelling of
//! what a person or agent typed that the board keeps. It changes only what
//! cannot change what a reader shows:
//!
//! - line ends become one LF (a CRLF or a lone CR);
//! - text is made Unicode NFC (canonically equal, so shown the same);
//! - blank lines go from the text's start and end, the final newline
//!   included;
//! - trailing spaces and tabs go from a line outside fenced code, only
//!   where no hard break can form there: a single space, or a run before a
//!   blank line or the end. A line that may be indented code (four columns
//!   of indent, or a tab, before its content) is left alone.
//!
//! Everything else (tabs, no-break and other spaces, selectors, indents,
//! hard breaks, blank-line runs, all of fenced code) is kept as written:
//! the board's codec then holds it, or refuses it with its field and line.
//! Task validation takes only normalised text, so no entry point skips
//! this. Discussions (M4) run it before `append`.
use unicode_normalization::UnicodeNormalization;

/// The most spaces before a fence's closing run a reader still takes as
/// its close.
const CLOSING_INDENT: usize = 3;
/// Columns of indent that start indented code.
const CODE_INDENT: usize = 4;

/// Where a line stands, as far as this reads markdown.
#[derive(Clone, Copy)]
enum Place {
    /// Prose, outside any fence.
    Prose,
    /// Inside fenced code opened at the margin, which only its close ends.
    Fenced(Fence),
    /// After a fence opened inside a quote, a list item or an indent, which
    /// may end with its container: the rest is kept as written.
    Unknown,
}

/// Markdown text (a description, an approach, a message) as the board
/// keeps it.
pub fn text(raw: &str) -> String {
    let unified: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .nfc()
        .collect();
    let lines: Vec<&str> = unified.split('\n').collect();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut place = Place::Prose;
    // The backtick run of a code span open at the end of the last line: a
    // span may run on across lines of one paragraph, its line ends shown
    // as spaces, so the spaces before them are its own.
    let mut span: Option<usize> = None;
    for (at, line) in lines.iter().enumerate() {
        let before_blank = lines.get(at + 1).is_none_or(|next| blank(next));
        let (next, written) = match place {
            Place::Fenced(open) if open.closed_by(line) => (Place::Prose, (*line).to_string()),
            Place::Fenced(_) | Place::Unknown => (place, (*line).to_string()),
            Place::Prose => match Fence::opened_by(line) {
                Some(open) if open.at_margin => (Place::Fenced(open), (*line).to_string()),
                Some(_) => (Place::Unknown, (*line).to_string()),
                None => {
                    span = match blank(line) {
                        true => None,
                        false => open_span(line, span),
                    };
                    let line = match span {
                        None => without_trailing(line, before_blank),
                        Some(_) => (*line).to_string(),
                    };
                    (Place::Prose, line)
                }
            },
        };
        span = match next {
            Place::Prose => span,
            Place::Fenced(_) | Place::Unknown => None,
        };
        place = next;
        kept.push(written);
    }
    let first = kept
        .iter()
        .position(|line| !blank(line))
        .unwrap_or(kept.len());
    let end = match place {
        Place::Prose => kept
            .iter()
            .rposition(|line| !blank(line))
            .map_or(first, |last| last + 1),
        // Code may run to the end, its blank lines shown: only a final
        // line end after a line that holds something goes, which no reader
        // shows.
        Place::Fenced(_) | Place::Unknown => match kept.as_slice() {
            [.., before, last] if last.is_empty() && !before.is_empty() => kept.len() - 1,
            _ => kept.len(),
        },
    };
    kept[first..end.max(first)].join("\n")
}

/// One line of text (a title, a criterion, an owner) as the board keeps
/// it: line ends and NFC as [`text`] makes them, and spaces and tabs gone
/// from both edges (one line has no structure for them to start).
pub fn line(raw: &str) -> String {
    let unified: String = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .nfc()
        .collect();
    unified.trim_matches(spacing).to_string()
}

fn spacing(c: char) -> bool {
    matches!(c, ' ' | '\t')
}

/// A blank line, as a reader takes it: only spaces and tabs.
fn blank(line: &str) -> bool {
    line.chars().all(spacing)
}

/// The backtick run of the code span open at the end of `line`, given the
/// one `open` at its start: each run opens a span or closes the open one
/// of its own length (backslashes are not escapes inside a span; outside,
/// counting an escaped run as an opener only keeps more lines as written).
fn open_span(line: &str, open: Option<usize>) -> Option<usize> {
    let mut open = open;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '`' {
            continue;
        }
        let mut run = 1;
        while chars.next_if_eq(&'`').is_some() {
            run += 1;
        }
        open = match open {
            Some(length) if length == run => None,
            Some(length) => Some(length),
            None => Some(run),
        };
    }
    open
}

/// `line` without its trailing spaces and tabs where no hard break can
/// form: a single trailing space, or any run before a blank line or the
/// end; a line that may be indented code keeps them.
fn without_trailing(line: &str, before_blank: bool) -> String {
    let content = line.trim_end_matches(spacing);
    let run = &line[content.len()..];
    let breakless = run == " " || before_blank;
    match breakless && !may_be_indented_code(line) {
        true => content.to_string(),
        false => line.to_string(),
    }
}

/// Whether `line`, after any quote and list markers, is indented enough
/// (or by a tab) to be code: then its trailing spaces may be code's.
fn may_be_indented_code(line: &str) -> bool {
    let prefix: String = line.chars().take_while(|c| container_mark(*c)).collect();
    prefix.contains('\t') || prefix.contains(&" ".repeat(CODE_INDENT))
}

/// What may stand before a block inside a quote or a list item: spaces,
/// tabs, quote marks and list markers.
fn container_mark(c: char) -> bool {
    matches!(c, ' ' | '\t' | '>' | '-' | '*' | '+' | '.' | ')') || c.is_ascii_digit()
}

/// An open fence: its character and how many of it opened it.
#[derive(Clone, Copy)]
struct Fence {
    mark: char,
    length: usize,
    quotes: usize,
    /// Opened at the margin (no indent, no quote or list marker): nothing
    /// but its close ends it.
    at_margin: bool,
}

impl Fence {
    /// The fence `line` opens, read liberally (inside any quote or list
    /// item, at any indent): taking a line for an opener too readily only
    /// leaves more lines as written.
    fn opened_by(line: &str) -> Option<Self> {
        let prefix: String = line.chars().take_while(|c| container_mark(*c)).collect();
        let rest = &line[prefix.len()..];
        let mark = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let length = rest.chars().take_while(|c| *c == mark).count();
        let info = &rest[length..];
        let opens = length >= 3 && (mark == '~' || !info.contains('`'));
        let quotes = prefix.matches('>').count();
        opens.then_some(Self {
            mark,
            length,
            quotes,
            at_margin: prefix.is_empty(),
        })
    }

    /// Whether `line` closes this fence, read strictly (the same quotes, at
    /// most three spaces, a run of the same mark at least as long, then
    /// only spacing): taking a line for a close too readily would let code
    /// be changed.
    fn closed_by(self, line: &str) -> bool {
        let prefix: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '>'))
            .collect();
        let rest = &line[prefix.len()..];
        let indent = prefix.len() - prefix.trim_end_matches(' ').len();
        let run = rest.chars().take_while(|c| *c == self.mark).count();
        prefix.matches('>').count() == self.quotes
            && indent <= CLOSING_INDENT
            && run >= self.length
            && rest[run..].chars().all(spacing)
    }
}

#[cfg(test)]
#[path = "board_text_tests.rs"]
mod tests;
