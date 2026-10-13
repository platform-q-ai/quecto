//! Board text as it enters the board (#2481, decision D4): the spelling of
//! what a person or agent typed that the board keeps. It changes only
//! spacing a reader never shows:
//!
//! - line ends become one LF (a CRLF or a lone CR; a one-line field and a
//!   message also take a line or paragraph separator, a next line and a
//!   vertical tab as one);
//! - a space chat writes between words (no-break, narrow no-break,
//!   figure, ideographic: French keyboards put them before `!`, `?` and
//!   `:`) becomes a plain space: in a one-line field and a message always,
//!   in markdown text only where a reader sees the same (past a prose
//!   line's first word, between characters a plain space beside them makes
//!   no markup of, never in code, math, HTML or a link);
//! - blank lines go from the text's start and end, the final newline
//!   included;
//! - trailing spaces and tabs go from a prose line only where nothing can
//!   change: the run is a single space, or comes before a blank line or
//!   the end (no hard break forms); the character before it is a safe one
//!   (a letter, a digit, or punctuation but a backslash, which a line end
//!   would make a hard break); its paragraph so far holds no backtick or
//!   dollar (a code span or math may run on across lines, showing the
//!   space), no `<` (inline HTML may span lines) and no link or
//!   reference (`](`, `]:`: a title or label may span lines); and the line
//!   cannot be indented code.
//!
//! No NFC is taken (it would rewrite link labels, addresses and fence
//! info, and the Kelvin sign into an ASCII `K`): text that is not NFC is
//! left as written, and the codec's NFC rule refuses it with its line.
//! Text from agents and phones is NFC in practice. Everything else (tabs in
//! markdown, a chat space beside a block marker or in code, any other
//! space, selectors, indents, hard breaks, inner blank lines, fenced code,
//! HTML blocks, footnote definitions) is kept as written: the domain's
//! schema refuses a space other than a plain one, a tab or a line end with
//! its field (`PLAIN_SPACES_ONLY`), and the codec holds the rest or
//! refuses it with its field and line.
//!
//! The write paths take only normalised text, so no entry point skips
//! normalising: `Task::entered` (create) and the task's `edit_draft` check
//! it with [`entered_text`] and [`entered_line`]; a plan's `propose` and
//! `edit_draft` check its approach, task item titles and criteria; and a
//! discussion's `begin` and `append` check its title and every message
//! with [`entered_message`] (a message is normalised by [`message`]).
//! Reading an artifact back (`Task::new`, `Plan::new`, `Discussion::new`)
//! stays lenient by design, so what earlier builds stored still reads.
use super::schema_error::SchemaError;

/// Why text a write path takes is refused: it was never normalised.
const NOT_ENTERED: &str = "must be board text as it enters (normalised: one LF between \
     lines, no blank lines at its edges, no trailing space where no hard break forms)";
/// Why one line a write path takes is refused: it was never normalised
/// (owner-approved wording, #2481 S7).
const NOT_ENTERED_LINE: &str = "must be one line with no spaces or tabs at its start or end";
/// Why a message a write path takes is refused: it was never normalised
/// (owner-approved wording, #2481 S7).
const NOT_ENTERED_MESSAGE: &str = "has spacing the board removes: use one LF between lines, no \
     blank lines, and no spaces or tabs at the start or end of a line";
/// The spaces chat writes between words that a message keeps as a plain
/// space: no-break, narrow no-break (French before `:`), figure and
/// ideographic. A file holds no other space.
const CHAT_SPACES: [char; 4] = ['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}'];
/// What chat and imports break a line with besides LF: a lone CR, a line
/// or paragraph separator, a next line and a vertical tab. A line or a
/// message takes each as one LF (markdown text keeps them: a reader does
/// not break a line there).
const LINE_BREAKS: [char; 5] = ['\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}'];
/// The ASCII punctuation a trailing space may follow: all but a backslash,
/// which a line end after it makes a hard break.
const SAFE_PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[]^_`{|}~";

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
    /// After a fence opened inside a quote, a list item or an indent (which
    /// may end with its container), or a line that may start an HTML block
    /// (which keeps its lines as written): the rest is kept as written.
    Unknown,
}

/// Markdown text (a description, an approach) as the board keeps it. A
/// discussion message is one line per row, normalised by [`message`].
pub fn text(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
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
                None if may_start_html_block(line) || defines_footnote(line) => {
                    (Place::Unknown, (*line).to_string())
                }
                // A blank line ends a paragraph; its own spacing may still
                // count (an indented blank line stays in a list item).
                None if blank(line) => (Place::Prose { marked: false }, (*line).to_string()),
                None => {
                    let marked = marked || line.contains(['`', '$', '<']) || links(line);
                    let written = match marked {
                        false => without_trailing(&plain_between_words(line), before_blank),
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
    debug_assert_eq!(
        unchanged(&normal),
        unchanged(raw),
        "only spacing, line ends and blank lines change"
    );
    normal
}

/// `line`, a prose line no code span, math, HTML or link marks, with each
/// of [`CHAT_SPACES`] a plain space where a reader sees the same: past the
/// line's first word (its leading block marks, indent, quote marks, list
/// markers and heading hashes, are never touched, so no list, heading,
/// quote or indent forms) and between two characters a plain space beside
/// them makes no markup of ([`spaced_after`], [`spaced_before`]). CommonMark
/// takes a chat space as Unicode whitespace already, so emphasis reads
/// alike. A line that may be indented code is kept as written.
fn plain_between_words(line: &str) -> String {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let first_word = first_word_at(line);
    let plain = |at: usize| -> bool {
        let (index, c) = chars[at];
        let left = at.checked_sub(1).map(|before| chars[before].1);
        let right = chars.get(at + 1).map(|(_, after)| *after);
        CHAT_SPACES.contains(&c)
            && first_word.is_some_and(|word| index > word)
            && left.is_some_and(spaced_after)
            && right.is_some_and(spaced_before)
    };
    match may_be_indented_code(line) {
        true => line.to_string(),
        false => (0..chars.len())
            .map(|at| match plain(at) {
                true => ' ',
                false => chars[at].1,
            })
            .collect(),
    }
}

/// Where `line`'s first word starts (its first character past its block
/// marks that is not whitespace), if it has one.
fn first_word_at(line: &str) -> Option<usize> {
    let mut rest = line;
    loop {
        let before = rest.len();
        rest = rest.trim_start_matches([' ', '\t', '>', '#', '-', '*', '+']);
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && rest[digits..].starts_with(['.', ')']) {
            rest = &rest[digits + 1..];
        }
        if rest.len() == before {
            break;
        }
    }
    // Past any whitespace (a chat space beside a marker included).
    let word = rest.trim_start();
    word.chars().next().map(|_| line.len() - word.len())
}

/// A character a plain space may follow without making markup: a letter,
/// a digit, or the punctuation a word or a number ends with, and the
/// guillemets.
fn spaced_after(c: char) -> bool {
    c.is_alphanumeric() || ".,!?:;%)\u{ab}\u{bb}".contains(c)
}

/// A character a plain space may come before without making markup: a
/// letter, a digit, or the punctuation French and number spacing puts
/// after a space.
fn spaced_before(c: char) -> bool {
    c.is_alphanumeric() || "!?:;%(\u{ab}\u{bb}\u{20ac}".contains(c)
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
        false => Err(SchemaError::new(field, NOT_ENTERED_LINE)),
    }
}

/// One line of text (a title, a criterion, an owner, a message line) as
/// the board keeps it: each line end one LF (CRLF, and each of
/// [`LINE_BREAKS`]), the spaces chat writes between words
/// ([`CHAT_SPACES`]) and tabs plain spaces, and spaces and line ends gone
/// from both edges (one line has no structure for them to start). A line
/// end left inside makes it two lines, which the field's schema refuses.
pub fn line(raw: &str) -> String {
    plain_spaces(&line_ends(raw))
        .replace('\t', " ")
        .trim_matches([' ', '\n'])
        .to_string()
}

/// A discussion message as the board keeps it (#2481 D3): split at each
/// line end (CRLF, LF and each of [`LINE_BREAKS`]), each line as [`line`]
/// makes it (chat spaces and tabs plain; a message line is a table cell,
/// which shows no edge spacing), and blank lines dropped (a message is its
/// non-empty lines). Empty when nothing but spacing was said.
pub fn message(raw: &str) -> String {
    let lines: Vec<String> = line_ends(raw)
        .split('\n')
        .map(line)
        .filter(|kept| kept.chars().next().is_some())
        .collect();
    let normal = lines.join("\n");
    debug_assert_eq!(
        unchanged(&normal),
        unchanged(raw),
        "only spacing, line ends and blank lines change"
    );
    normal
}

/// A message field `field` holds as it enters the board: itself once
/// normalised by [`message`].
///
/// # Errors
/// A message no write path normalised.
pub fn entered_message(field: &str, held: &str) -> Result<(), SchemaError> {
    match message(held) == held {
        true => Ok(()),
        false => Err(SchemaError::new(field, NOT_ENTERED_MESSAGE)),
    }
}

/// `raw` with each line end one LF: a CRLF, and each of [`LINE_BREAKS`]
/// (a lone CR, a line or paragraph separator, a next line, a vertical tab).
/// For a line and a message only: markdown text breaks lines at CR and LF
/// alone, as a reader does.
fn line_ends(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .chars()
        .map(|c| match LINE_BREAKS.contains(&c) {
            true => '\n',
            false => c,
        })
        .collect()
}

/// `text` with each of [`CHAT_SPACES`] a plain space.
fn plain_spaces(text: &str) -> String {
    text.chars()
        .map(|c| match CHAT_SPACES.contains(&c) {
            true => ' ',
            false => c,
        })
        .collect()
}

/// `text`'s characters the normaliser never changes, in order: all but
/// spacing, chat spaces and line ends.
fn unchanged(text: &str) -> String {
    text.split(normalised_spacing).collect()
}

/// What a normaliser may change: spacing, a chat space, a line end.
fn normalised_spacing(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n') || CHAT_SPACES.contains(&c) || LINE_BREAKS.contains(&c)
}

fn spacing(c: char) -> bool {
    matches!(c, ' ' | '\t')
}

/// A blank line, as a reader takes it: only spaces and tabs.
fn blank(line: &str) -> bool {
    line.chars().all(spacing)
}

/// Whether a stripped trailing space after `c` changes nothing (an
/// allowlist): a letter, a digit, or ASCII punctuation but a backslash.
fn safe_before_line_end(c: char) -> bool {
    c.is_alphanumeric() || SAFE_PUNCTUATION.contains(c)
}

/// Whether `line` may start an HTML block: a `<` after spaces, quote marks
/// or list markers (read liberally: it only keeps more lines as written).
fn may_start_html_block(line: &str) -> bool {
    line.trim_start_matches(container_mark).starts_with('<')
}

/// Whether `line` holds a link or a reference definition (`](` or `]:`):
/// its title may start on the next line, or its label span lines.
fn links(line: &str) -> bool {
    line.contains("](") || line.contains("]:")
}

/// Whether `line` defines a footnote (`[^n]:` after any container marks):
/// a container whose lines may be code or HTML, so the rest is kept as
/// written.
fn defines_footnote(line: &str) -> bool {
    let rest = line.trim_start_matches(container_mark);
    rest.starts_with("[^") && rest.contains("]:")
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
