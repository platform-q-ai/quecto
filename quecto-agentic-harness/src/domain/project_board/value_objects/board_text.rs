//! Board text as it enters the board (#2481, decision D4): the spelling of
//! what a person or agent typed that the board keeps. It changes only
//! spacing, and past chat spaces only spacing a reader never shows:
//!
//! - line ends become one LF (a CRLF or a lone CR; a one-line field and a
//!   message also take a line or paragraph separator, a next line and a
//!   vertical tab as one);
//! - a space chat writes between words (no-break, narrow no-break,
//!   figure, ideographic: French keyboards put them before `!`, `?` and
//!   `:`, and rich-text paste writes empty paragraphs and indents with
//!   them) never stays (epic #2705 T1), so no entry path refuses one. In a
//!   one-line field and a message it becomes a plain space. In markdown
//!   text one at a line's end goes (fenced code included); one at a line's
//!   start, before anything else, goes too (in fenced code, and what is
//!   kept as written, it becomes a plain space, an indent); a line of
//!   nothing but plain and chat spaces becomes empty outside a fence, and
//!   goes between two list items (a pasted empty paragraph would make two
//!   lists of one, which the codec refuses); and
//!   every other one becomes a plain space: beside a block marker (`-`
//!   then a no-break space starts a list item), in code spans, math, HTML
//!   and links. Normalising inside code and math is accepted in the
//!   approved #2705 plan (T1): an NBSP between words there is never
//!   meaningful. Typed plain spaces keep the rules below;
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
//! markdown, any other space, selectors, indents, hard breaks, inner blank
//! lines, fenced code, HTML blocks, footnote definitions) is kept as
//! written, but for its chat spaces: the domain's schema refuses a space
//! other than a plain one, a tab or a line end with its field
//! (`PLAIN_SPACES_ONLY`, which stays for what is read back), and the codec
//! holds the rest or refuses it with its field and line.
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
use super::telemetry::TextChange;

/// Why text a write path takes is refused: it was never normalised.
const NOT_ENTERED: &str = "must be board text as it enters (normalised: one LF between \
     lines, no blank lines at its edges, no trailing space where no hard break forms)";
/// Why one line a write path takes is refused: it was never normalised
/// (owner-approved wording, #2481 S7).
const NOT_ENTERED_LINE: &str =
    "must be one line, use plain spaces only, and have no spaces or tabs at its start or end";
/// Why a message a write path takes is refused: it was never normalised
/// (owner-approved wording, #2481 S7).
const NOT_ENTERED_MESSAGE: &str = "has spacing the board removes: use one LF between lines, no \
     blank lines, and no spaces or tabs at the start or end of a line";
/// The spaces chat writes between words, which entry never keeps:
/// no-break, narrow no-break (French before `:`), figure and ideographic.
/// A file holds no other space.
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
    /// a backtick, a dollar, a `<` or a link.
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
    let kept: Vec<String> = kept_lines(raw).into_iter().flatten().collect();
    let normal = kept.join("\n");
    debug_assert!(!normal.contains('\r'), "one LF between lines");
    debug_assert!(!normal.contains(CHAT_SPACES), "no chat space stays");
    debug_assert_eq!(
        unchanged(&normal),
        unchanged(raw),
        "only spacing, line ends and blank lines change"
    );
    normal
}

/// Each line of `raw` (CRLF and a lone CR read as LF) as [`text`] keeps
/// it, or `None` where it goes: a blank line at the text's edges, or a
/// gap of chat spaces between two list items (rich-text paste's empty
/// paragraph, which would make two lists of one to the codec).
fn kept_lines(raw: &str) -> Vec<Option<String>> {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = unified.split('\n').collect();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    // Whether each line is a prose line of chat (and plain) spaces only,
    // made empty.
    let mut gap: Vec<bool> = Vec::with_capacity(lines.len());
    let mut place = Place::Prose { marked: false };
    for (at, raw_line) in lines.iter().enumerate() {
        // A line of chat spaces is blank, wherever it stands.
        let before_blank = lines
            .get(at + 1)
            .is_none_or(|next| blank(&plain_spaces(next)));
        // Each line's chat spaces first, as where it stands takes them.
        let edged = match place {
            Place::Prose { .. } => without_chat(raw_line, Edge::Prose),
            Place::Fenced(_) | Place::Unknown => without_chat(raw_line, Edge::Code),
        };
        let line = edged.as_str();
        let emptied = matches!(place, Place::Prose { .. })
            && line.is_empty()
            && raw_line.contains(CHAT_SPACES);
        let (next, written) = match place {
            Place::Fenced(open) if open.closed_by(line) => {
                (Place::Prose { marked: false }, line.to_string())
            }
            Place::Fenced(_) | Place::Unknown => (place, line.to_string()),
            Place::Prose { marked } => match Fence::opened_by(line) {
                Some(open) if open.at_margin => (Place::Fenced(open), line.to_string()),
                Some(_) => (Place::Unknown, line.to_string()),
                None if may_start_html_block(line) || defines_footnote(line) => {
                    (Place::Unknown, line.to_string())
                }
                // A blank line ends a paragraph; its own spacing may still
                // count (an indented blank line stays in a list item).
                None if blank(line) => (Place::Prose { marked: false }, line.to_string()),
                None => {
                    let marked = marked || line.contains(['`', '$', '<']) || links(line);
                    let written = match marked {
                        false => without_trailing(line, before_blank),
                        true => line.to_string(),
                    };
                    (Place::Prose { marked }, written)
                }
            },
        };
        place = next;
        kept.push(written);
        gap.push(emptied);
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
    let gapped = list_gaps(&kept, &gap);
    debug_assert_eq!(gapped.len(), kept.len());
    kept.into_iter()
        .enumerate()
        .map(
            |(at, line)| match ((first..end.max(first)).contains(&at), gapped[at]) {
                (true, false) => Some(line),
                (false, _) | (true, true) => None,
            },
        )
        .collect()
}

/// Which of `kept` go as a gap between list items: each run of lines
/// `gap` marks whose nearest other lines, before and after, are both list
/// items. Two passes, so a long run costs no more than a short one.
fn list_gaps(kept: &[String], gap: &[bool]) -> Vec<bool> {
    // The nearest line before each that is no gap: a list item or not.
    let mut item_before = Vec::with_capacity(kept.len());
    let mut last = false;
    for (line, gapped) in kept.iter().zip(gap) {
        item_before.push(last);
        last = match gapped {
            true => last,
            false => list_item(line),
        };
    }
    let mut item_after = vec![false; kept.len()];
    let mut next = false;
    for at in (0..kept.len()).rev() {
        item_after[at] = next;
        next = match gap[at] {
            true => next,
            false => list_item(&kept[at]),
        };
    }
    (0..kept.len())
        .map(|at| gap[at] && item_before[at] && item_after[at])
        .collect()
}

/// Whether `line` starts a list item at the margin (at most three spaces
/// in): `-`, `*` or `+`, or one to nine digits and `.` or `)`, then a
/// space.
fn list_item(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let marker = match digits {
        0 => rest.starts_with(['-', '*', '+']).then_some(1),
        1..=9 => rest[digits..].starts_with(['.', ')']).then_some(digits + 1),
        _ => None,
    };
    line.len() - rest.len() <= CLOSING_INDENT
        && marker.is_some_and(|width| rest[width..].starts_with(' '))
}

/// Where a line stands, as far as its chat spaces go: prose, or code
/// (fenced, or kept as written), whose indent a reader shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Prose,
    Code,
}

/// `line` with no chat space left: those at its end removed; those at its
/// start removed in prose and made plain in code (an indent); a prose line
/// of nothing but plain and chat spaces, a chat space among them, made
/// empty; and every other one made plain. Plain spaces and tabs are kept.
fn without_chat(line: &str, edge: Edge) -> String {
    let chat = |c: char| CHAT_SPACES.contains(&c);
    let spaced = |c: char| spacing(c) || chat(c);
    let spaces_only = line.chars().all(|c| c == ' ' || chat(c));
    if edge == Edge::Prose && spaces_only && line.contains(CHAT_SPACES) {
        return String::new();
    }
    let body = line.trim_matches(spaced);
    let start = line.len() - line.trim_start_matches(spaced).len();
    let (head, tail) = match body.is_empty() {
        // Spacing only: all of it is the line's end.
        true => ("", line),
        false => (&line[..start], &line[start + body.len()..]),
    };
    // An edge run's plain spaces and tabs (an allowlist).
    let plain_only = |run: &str| run.chars().filter(|c| spacing(*c)).collect::<String>();
    let head = match edge {
        Edge::Prose => plain_only(head),
        Edge::Code => plain_spaces(head),
    };
    let normal = format!("{head}{}{}", plain_spaces(body), plain_only(tail));
    debug_assert!(!normal.contains(CHAT_SPACES), "no chat space stays");
    debug_assert!(normal.len() <= line.len(), "only spacing changes");
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

/// The kinds of change [`text`] makes to `raw` (epic #2481 T4), each
/// once, in [`TextChange`]'s order: none exactly when it keeps `raw` as
/// written. Its steps are [`text`]'s, told apart, line for line with what
/// it keeps: line ends unified and no chat space kept, then each line that
/// holds something losing trailing spacing, then blank lines gone (at the
/// edges, or a gap of chat spaces in a list).
pub fn text_changes(raw: &str) -> Vec<TextChange> {
    let unified = raw.replace("\r\n", "\n");
    // Read with chat spaces plain: a line of them only is blank, as in
    // `text`.
    let plain = plain_spaces(&unified.replace('\r', "\n"));
    let read: Vec<&str> = plain.split('\n').collect();
    let kept = kept_lines(raw);
    assert_eq!(read.len(), kept.len(), "a kept line for each line read");
    let normal = text(raw);
    let trailing = |line: &str| line.len() - line.trim_end_matches(spacing).len();
    let trimmed = read
        .iter()
        .zip(&kept)
        .any(|(was, now)| match (now, blank(was)) {
            (Some(now), false) => trailing(was) > trailing(now),
            (Some(_), true) | (None, _) => false,
        });
    // An empty text is one empty line, which goes, changing nothing.
    let dropped = match raw.is_empty() {
        true => false,
        false => kept.iter().any(Option::is_none),
    };
    let changes = [
        (TextChange::Crlf, unified.len() < raw.len()),
        (TextChange::LoneCr, unified.contains('\r')),
        // `text` keeps no chat space, wherever it stands.
        (TextChange::ChatSpace, raw.contains(CHAT_SPACES)),
        (TextChange::EdgeBlankLines, dropped),
        (TextChange::TrailingSpace, trimmed),
    ];
    told(raw, &normal, &changes)
}

/// The kinds of change [`line`] makes to `raw` (epic #2481 T4), each
/// once, in [`TextChange`]'s order: none exactly when it keeps `raw` as
/// written. Its steps are [`line`]'s, one by one.
pub fn line_changes(raw: &str) -> Vec<TextChange> {
    let crlf = raw.replace("\r\n", "\n");
    let lone_cr = crlf.replace('\r', "\n");
    let broken = line_ends(&lone_cr);
    let plain = plain_spaces(&broken);
    let untabbed = plain.replace('\t', " ");
    let trimmed = untabbed.trim_matches([' ', '\n']);
    debug_assert_eq!(trimmed, line(raw), "line's own steps");
    let changes = [
        (TextChange::Crlf, crlf != raw),
        (TextChange::LoneCr, lone_cr != crlf),
        (TextChange::LineBreak, broken != lone_cr),
        (TextChange::ChatSpace, plain != broken),
        (TextChange::Tab, untabbed != plain),
        (TextChange::EdgeSpace, trimmed != untabbed),
    ];
    told(raw, trimmed, &changes)
}

/// The kinds `changes` tells, in [`TextChange`]'s order; asserted in
/// debug builds to be none exactly when `kept` is `raw` (telemetry never
/// fails a write).
fn told(raw: &str, kept: &str, changes: &[(TextChange, bool)]) -> Vec<TextChange> {
    let told: Vec<TextChange> = TextChange::ALL
        .iter()
        .copied()
        .filter(|kind| changes.contains(&(*kind, true)))
        .collect();
    debug_assert_eq!(
        told.is_empty(),
        raw == kept,
        "a change is told exactly when one was made"
    );
    told
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
