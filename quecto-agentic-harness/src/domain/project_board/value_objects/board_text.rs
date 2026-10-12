//! Board text as it enters the board (#2481, decision D4): the spelling of
//! what a person or agent typed that the board keeps. It changes only what
//! cannot change what a reader shows:
//!
//! - line ends become one LF (a CRLF or a lone CR);
//! - text is made Unicode NFC, only where NFC keeps the text's ASCII
//!   punctuation exactly (composing `>` and U+0338 into `≯`, say, would
//!   unmake a quote; U+1FEF and U+037E would become a backtick and `;`):
//!   otherwise the text is left as written, for the codec's NFC rule to
//!   refuse with its line. Inside code NFC may change bytes, but the glyphs
//!   shown are canonically equal;
//! - blank lines go from the text's start and end, the final newline
//!   included;
//! - trailing spaces and tabs go from a line outside fenced code only where
//!   nothing can change: the run is a single space, or comes before a
//!   blank line or the end (no hard break forms); the character before it
//!   is safe (anything but a backslash, which a stripped space would turn
//!   into a hard break); its paragraph so far holds no backtick or dollar
//!   (a code span or math may run on across lines, showing the space); and
//!   the line cannot be indented code (four columns of indent, or a tab).
//!
//! Everything else (tabs, no-break and other spaces, selectors, indents,
//! hard breaks, blank-line runs, all of fenced code) is kept as written:
//! the board's codec then holds it, or refuses it with its field and line.
//!
//! The write paths take only normalised text: `Task::entered` (create) and
//! `edit_draft` check it with [`entered_text`] and [`entered_line`], so no
//! entry point skips normalising. Reading a task back (`Task::new`) stays
//! lenient by design, so what earlier builds stored still reads.
//! Discussions (M4) run [`text`] before `append`.
use super::schema_error::SchemaError;
use unicode_normalization::UnicodeNormalization;

/// Why text a write path takes is refused: it was never normalised.
const NOT_ENTERED: &str = "must be board text as it enters (normalised: one LF between \
     lines, NFC, no blank lines at its edges, no trailing space where no hard break forms)";

/// The most spaces before a fence's closing run a reader still takes as
/// its close.
const CLOSING_INDENT: usize = 3;
/// Columns of indent that start indented code.
const CODE_INDENT: usize = 4;

/// Where a line stands, as far as this reads markdown.
#[derive(Clone, Copy)]
enum Place {
    /// Prose, outside any fence; `marked` once its paragraph so far holds
    /// a backtick or a dollar.
    Prose { marked: bool },
    /// Inside fenced code opened at the margin, which only its close ends.
    Fenced(Fence),
    /// After a fence opened inside a quote, a list item or an indent, which
    /// may end with its container: the rest is kept as written.
    Unknown,
}

/// Markdown text (a description, an approach, a message) as the board
/// keeps it.
pub fn text(raw: &str) -> String {
    let unified = canonical(&raw.replace("\r\n", "\n").replace('\r', "\n"));
    let lines: Vec<&str> = unified.split('\n').collect();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut place = Place::Prose { marked: false };
    for (at, line) in lines.iter().enumerate() {
        let before_blank = lines.get(at + 1).is_none_or(|next| blank(next));
        let (next, written) = match place {
            Place::Fenced(open) if open.closed_by(line) => {
                (Place::Prose { marked: false }, (*line).to_string())
            }
            Place::Fenced(_) | Place::Unknown => (place, (*line).to_string()),
            Place::Prose { marked } => match Fence::opened_by(line) {
                Some(open) if open.at_margin => (Place::Fenced(open), (*line).to_string()),
                Some(_) => (Place::Unknown, (*line).to_string()),
                // A blank line ends a paragraph; its own spacing may still
                // count (an indented blank line stays in a list item).
                None if blank(line) => (Place::Prose { marked: false }, (*line).to_string()),
                None => {
                    let marked = marked || line.contains(['`', '$']);
                    let written = match marked {
                        false => without_trailing(line, before_blank),
                        true => (*line).to_string(),
                    };
                    (Place::Prose { marked }, written)
                }
            },
        };
        place = next;
        kept.push(written);
    }
    let first = kept
        .iter()
        .position(|line| !blank(line))
        .unwrap_or(kept.len());
    let end = match place {
        Place::Prose { .. } => kept
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
    let normal = kept[first..end.max(first)].join("\n");
    debug_assert!(!normal.contains('\r'), "one LF between lines");
    normal
}

/// Markdown text field `field` holds as it enters the board: itself once
/// normalised (an allowlist: nothing else is taken).
///
/// # Errors
/// Text no write path normalised.
pub fn entered_text(field: &str, held: &str) -> Result<(), SchemaError> {
    match text(held) == held {
        true => Ok(()),
        false => Err(SchemaError::new(field, NOT_ENTERED)),
    }
}

/// One line field `field` holds as it enters the board: itself once
/// normalised.
///
/// # Errors
/// A line no write path normalised.
pub fn entered_line(field: &str, held: &str) -> Result<(), SchemaError> {
    match line(held) == held {
        true => Ok(()),
        false => Err(SchemaError::new(field, NOT_ENTERED)),
    }
}

/// One line of text (a title, a criterion, an owner) as the board keeps
/// it: line ends and NFC as [`text`] makes them, and spaces and tabs gone
/// from both edges (one line has no structure for them to start).
pub fn line(raw: &str) -> String {
    canonical(&raw.replace("\r\n", "\n").replace('\r', "\n"))
        .trim_matches(spacing)
        .to_string()
}

/// `text` made NFC, only where NFC keeps its ASCII punctuation exactly (an
/// allowlist): otherwise as written.
fn canonical(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let punctuation =
        |text: &str| -> Vec<char> { text.chars().filter(char::is_ascii_punctuation).collect() };
    match punctuation(&nfc) == punctuation(text) {
        true => nfc,
        false => text.to_string(),
    }
}

fn spacing(c: char) -> bool {
    matches!(c, ' ' | '\t')
}

/// A blank line, as a reader takes it: only spaces and tabs.
fn blank(line: &str) -> bool {
    line.chars().all(spacing)
}

/// Whether a stripped trailing space after `c` changes nothing: anything
/// a reader shows as itself, but a backslash (which a line end after it
/// makes a hard break).
fn safe_before_line_end(c: char) -> bool {
    match c {
        '\\' => false,
        c => !spacing(c),
    }
}

/// `line` without its trailing spaces and tabs where nothing can change: a
/// single trailing space or any run before a blank line or the end, after
/// a safe character, on a line that cannot be indented code.
fn without_trailing(line: &str, before_blank: bool) -> String {
    let content = line.trim_end_matches(spacing);
    let run = &line[content.len()..];
    let breakless = run == " " || before_blank;
    let safe = content.chars().last().is_some_and(safe_before_line_end);
    match breakless && safe && !may_be_indented_code(line) {
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
        opens.then_some(Self {
            mark,
            length,
            quotes: prefix.matches('>').count(),
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
