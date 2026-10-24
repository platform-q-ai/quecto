//! Board text as it enters the board (#2481, decision D4): the spelling of
//! what a person or agent typed that the board keeps. It changes only
//! spacing, and past chat spaces only spacing a reader never shows:
//!
//! - line ends become one LF (a CRLF or a lone CR; a one-line field and a
//!   message also take a line or paragraph separator, a next line and a
//!   vertical tab as one);
//! - a space chat writes between words (no-break, narrow no-break,
//!   figure, ideographic: French keyboards put them before `!`, `?` and
//!   `:`; rich-text paste and browsers write empty paragraphs, indents and
//!   runs of spaces with them) never stays, so no entry path refuses text
//!   for a chat space (the codec still refuses what it would refuse typed
//!   with plain spaces, with its line). In a one-line field and a message
//!   it becomes a plain space. In markdown text:
//!   - a prose line's leading run of spacing that holds a chat space goes
//!     whole, its plain spaces and tabs too (in fenced code, and what is
//!     kept as written, a chat space there becomes a plain space: an
//!     indent);
//!   - one at a line's end goes (fenced code included), unless going
//!     would leave a prose line ending in a backslash, a setext underline
//!     or thematic break, or plain spacing: then it becomes a plain space,
//!     read as typed text is;
//!   - outside a fence, a line of nothing but spacing, a chat space among
//!     it, becomes empty, and each run of blank lines holding such a line
//!     becomes one empty line, or none between two items of a list with
//!     no other blank line between its items (two lists of one, to the
//!     codec);
//!   - every other one becomes a plain space: beside a block marker (`-`
//!     then a no-break space starts a list item), in code spans, math, HTML
//!     and links. This PR normalises inside code and math too: an NBSP
//!     between words there is never meaningful, and refusing it blocks
//!     pasted text.
//!
//!   Typed plain spaces keep the rules below;
//! - blank lines go from the text's start and end, the final newline
//!   included;
//! - trailing spaces and tabs go from a prose line only where nothing can
//!   change: the run is a single space, or comes before a blank line or
//!   the end (no hard break forms); the line is no lone `-` under
//!   paragraph text (`- ` is text there, `-` a setext underline); the
//!   character before
//!   it is a safe one (a letter, a digit, or punctuation but a backslash,
//!   which a line end would make a hard break); its paragraph so far holds no backtick or
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
//! lines, fenced code, HTML blocks (opened by `<` then a letter, `/`, `!`
//! or `?`; ended by a blank line, but for a comment, `<pre>` and the like),
//! footnote definitions) is kept as written, but for its chat spaces: the domain's schema refuses a space
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
use std::collections::BTreeSet;

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
    /// An HTML block that a blank line ends (CommonMark's types 6 and 7):
    /// its lines are kept as written.
    Html,
    /// After a fence opened inside a quote, a list item or an indent (which
    /// may end with its container), an HTML block only its own close ends
    /// (a comment, `<pre>`, `<script>` and the like), or a footnote
    /// definition: the rest is kept as written.
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
/// it, or `None` where it goes: a blank line at the text's edges, or one
/// of a run of blank lines holding an emptied chat line ([`settled_gaps`]).
fn kept_lines(raw: &str) -> Vec<Option<String>> {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = unified.split('\n').collect();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    // Whether each line is a prose line of spacing and chat spaces only,
    // made empty; and whether it is a blank prose line at all.
    let mut gap: Vec<bool> = Vec::with_capacity(lines.len());
    let mut prose_blank: Vec<bool> = Vec::with_capacity(lines.len());
    let mut place = Place::Prose { marked: false };
    // Whether the line before is paragraph text a `-` would underline as a
    // setext heading (not an item, a heading, a quote or an indent, in a
    // paragraph no list item opened).
    let (mut under_text, mut item_in_paragraph) = (false, false);
    for (at, raw_line) in lines.iter().enumerate() {
        // A line of chat spaces is blank, wherever it stands.
        let before_blank = lines
            .get(at + 1)
            .is_none_or(|next| blank(&plain_spaces(next)));
        // Each line's chat spaces first, as where it stands takes them.
        let edged = match place {
            Place::Prose { .. } => without_chat(raw_line, Edge::Prose),
            Place::Fenced(_) | Place::Html | Place::Unknown => without_chat(raw_line, Edge::Code),
        };
        let line = edged.as_str();
        let in_prose = matches!(place, Place::Prose { .. });
        let emptied = in_prose && line.is_empty() && raw_line.contains(CHAT_SPACES);
        let (next, written) = match place {
            Place::Fenced(open) if open.closed_by(line) => {
                (Place::Prose { marked: false }, line.to_string())
            }
            // A fence, or a block only its own close ends, inside may be a
            // reader's, if the block is no HTML to it (a lone `<span` is
            // text): the rest is kept as written.
            Place::Html
                if Fence::opened_by(line).is_some()
                    || html_block(line) == Some(HtmlEnd::OwnClose)
                    || defines_footnote(line) =>
            {
                (Place::Unknown, line.to_string())
            }
            Place::Html if blank(line) => (Place::Prose { marked: false }, line.to_string()),
            Place::Fenced(_) | Place::Html | Place::Unknown => (place, line.to_string()),
            Place::Prose { marked } => match (Fence::opened_by(line), html_block(line)) {
                (Some(open), _) if open.at_margin => (Place::Fenced(open), line.to_string()),
                (Some(_), _) => (Place::Unknown, line.to_string()),
                (None, Some(HtmlEnd::BlankLine)) => (Place::Html, line.to_string()),
                (None, Some(HtmlEnd::OwnClose)) => (Place::Unknown, line.to_string()),
                (None, None) if defines_footnote(line) => (Place::Unknown, line.to_string()),
                // A blank line ends a paragraph; its own spacing may still
                // count (an indented blank line stays in a list item).
                (None, None) if blank(line) => (Place::Prose { marked: false }, line.to_string()),
                (None, None) => {
                    let marked = marked || line.contains(['`', '$', '<']) || links(line);
                    let written = match marked {
                        false => without_trailing(line, before_blank, under_text),
                        true => line.to_string(),
                    };
                    (Place::Prose { marked }, written)
                }
            },
        };
        let paragraph = in_prose && matches!(next, Place::Prose { .. }) && !blank(&written);
        item_in_paragraph = paragraph && (item_in_paragraph || list_item(&written));
        under_text = paragraph && !item_in_paragraph && paragraph_text(&written);
        place = next;
        prose_blank.push(in_prose && blank(&written));
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
        Place::Fenced(_) | Place::Html | Place::Unknown => match kept.as_slice() {
            [.., before, last] if last.is_empty() && !before.is_empty() => kept.len() - 1,
            _ => kept.len(),
        },
    };
    let settled = settled_gaps(kept, &gap, &prose_blank);
    settled
        .into_iter()
        .enumerate()
        .map(|(at, line)| match (first..end.max(first)).contains(&at) {
            true => line,
            false => None,
        })
        .collect()
}

/// `kept` with each run of blank prose lines that holds an emptied chat
/// line (`gap`) made one empty line, or none when it stands between two
/// items of a list with no other blank line between its items (rich-text
/// paste's empty paragraphs, which would make that list loose: two lists
/// to the codec). Other lines are kept. One pass each way, so a long run
/// costs no more than a short one.
fn settled_gaps(kept: Vec<String>, gap: &[bool], prose_blank: &[bool]) -> Vec<Option<String>> {
    assert!(kept.len() == gap.len() && gap.len() == prose_blank.len());
    let lines = kept.len();
    // The list each line that holds something belongs to, read loosely: an
    // item starts or continues one; any other line continues it unless it
    // comes after a blank line at the margin.
    let mut owner: Vec<Option<usize>> = vec![None; lines];
    let (mut list, mut lists, mut after_blank) = (None, 0, false);
    for at in 0..lines {
        let line = &kept[at];
        match (prose_blank[at], list_item(line)) {
            (true, _) => after_blank = true,
            (false, item) => {
                let indented = line.starts_with([' ', '\t']);
                list = match (list, item, after_blank && !indented) {
                    (Some(open), _, false) | (Some(open), true, true) => Some(open),
                    (None, true, _) => {
                        lists += 1;
                        Some(lists)
                    }
                    (Some(_), false, true) | (None, false, _) => None,
                };
                owner[at] = list;
                after_blank = false;
            }
        }
    }
    // Each run of blank prose lines: where it starts and stops, whether it
    // holds an emptied chat line, and the list it stands inside, if any.
    let mut runs: Vec<(usize, usize, bool, Option<usize>)> = Vec::new();
    let mut at = 0;
    while at < lines {
        let start = at;
        while at < lines && prose_blank[at] {
            at += 1;
        }
        let inside = match (start.checked_sub(1), kept.get(at)) {
            (Some(before), Some(after)) if list_item(after) && owner[before] == owner[at] => {
                owner[before]
            }
            _ => None,
        };
        if at > start {
            runs.push((start, at, gap[start..at].contains(&true), inside));
        }
        at = at.max(start + 1);
    }
    // A list some typed blank line already makes loose stays loose.
    let loose: BTreeSet<usize> = runs
        .iter()
        .filter_map(|&(_, _, chat, inside)| match chat {
            true => None,
            false => inside,
        })
        .collect();
    let mut settled: Vec<Option<String>> = kept.into_iter().map(Some).collect();
    for (start, stop, chat, inside) in runs {
        let tight = inside.is_some_and(|list| !loose.contains(&list));
        match (chat, tight) {
            (true, true) => settled[start..stop].fill(None),
            (true, false) => {
                settled[start] = Some(String::new());
                settled[start + 1..stop].fill(None);
            }
            (false, _) => {}
        }
    }
    settled
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

/// `line` with no chat space left:
/// - in prose, a leading run of spacing that holds a chat space goes whole,
///   and a line of nothing but spacing, a chat space among it, is empty; in
///   code a chat space at the start becomes plain (an indent);
/// - those at its end go, but in prose only where nothing forms: when going
///   would leave the line ending in a backslash or plain spacing, or a
///   setext underline or thematic break, the run becomes plain, read as a
///   typed one is ([`without_trailing`] then decides);
/// - every other one becomes plain.
fn without_chat(line: &str, edge: Edge) -> String {
    let chat = |c: char| CHAT_SPACES.contains(&c);
    let spaced = |c: char| spacing(c) || chat(c);
    let body = line.trim_matches(spaced);
    if edge == Edge::Prose && body.is_empty() && line.contains(CHAT_SPACES) {
        return String::new();
    }
    let start = line.len() - line.trim_start_matches(spaced).len();
    let (head, tail) = match body.is_empty() {
        // Spacing only: all of it is the line's end.
        true => ("", line),
        false => (&line[..start], &line[start + body.len()..]),
    };
    // An edge run's plain spaces and tabs (an allowlist).
    let plain_only = |run: &str| run.chars().filter(|c| spacing(*c)).collect::<String>();
    let head = match (edge, head.contains(CHAT_SPACES)) {
        (Edge::Prose, true) => String::new(),
        (Edge::Prose, false) => head.to_string(),
        (Edge::Code, _) => plain_spaces(head),
    };
    let content = format!("{head}{}", plain_spaces(body));
    let marks =
        rule_line(&content) || content.ends_with('\\') || plain_only(tail).chars().next().is_some();
    let tail = match (edge, tail.contains(CHAT_SPACES) && marks) {
        (Edge::Prose, true) => plain_spaces(tail),
        (Edge::Prose, false) | (Edge::Code, _) => plain_only(tail),
    };
    let normal = format!("{content}{tail}");
    debug_assert!(!normal.contains(CHAT_SPACES), "no chat space stays");
    debug_assert!(normal.len() <= line.len(), "only spacing changes");
    normal
}

/// Whether `content` is a lone `-` (at most three spaces in): under
/// paragraph text it underlines a setext heading, which `- ` (an empty
/// item, or text) does not.
fn setext_dash(content: &str) -> bool {
    let dash = content.trim_start_matches(' ');
    content.len() - dash.len() <= CLOSING_INDENT && dash == "-"
}

/// Whether `line`, a prose line that holds something, is paragraph text at
/// the margin: not a list item, an ATX heading, a quote or an indent.
fn paragraph_text(line: &str) -> bool {
    let hashes = line.len() - line.trim_start_matches('#').len();
    let heading = (1..=6).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_none_or(|c| matches!(c, ' ' | '\t'));
    let opens = line
        .chars()
        .next()
        .is_some_and(|c| !matches!(c, ' ' | '\t' | '>'));
    opens && !heading && !list_item(line)
}

/// Whether `content` reads as a setext underline or a thematic break once
/// its end goes: only `-`, `=`, `*` and `_` (spaced or not), read
/// liberally (taking a line for one only keeps a chat space as plain).
fn rule_line(content: &str) -> bool {
    let marks = content.trim_matches(' ');
    marks.chars().next().is_some()
        && marks
            .chars()
            .all(|c| matches!(c, '-' | '=' | '*' | '_' | ' '))
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

/// What ends an HTML block.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HtmlEnd {
    /// A blank line (CommonMark's types 6 and 7).
    BlankLine,
    /// Its own close: a comment, a processing instruction, a declaration,
    /// CDATA, `<pre>`, `<script>`, `<style>` or `<textarea>` (types 1 to 5).
    OwnClose,
}

/// The HTML block `line` may open, read liberally (it only keeps more lines
/// as written): a `<` then a letter, `/`, `!` or `?`, after at most
/// spaces, quote marks and bullet markers. A `<` before a digit or a space
/// (`< 5`, `1 < 2`, `- <200 ms`) opens none.
fn html_block(line: &str) -> Option<HtmlEnd> {
    let rest = line.trim_start_matches([' ', '\t', '>', '-', '*', '+']);
    let opener = rest.strip_prefix('<')?;
    let tag = opener
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'));
    let lower = opener.to_ascii_lowercase();
    let lasting = ["!", "?", "pre", "script", "style", "textarea"]
        .iter()
        .any(|start| lower.starts_with(start));
    match (tag, lasting) {
        (true, true) => Some(HtmlEnd::OwnClose),
        (true, false) => Some(HtmlEnd::BlankLine),
        (false, _) => None,
    }
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
/// a safe character, on a line that cannot be indented code, and that is
/// no lone `-` `under_text` (paragraph text above, which `-` would
/// underline as a heading where `- ` is text).
fn without_trailing(line: &str, before_blank: bool, under_text: bool) -> String {
    let content = line.trim_end_matches(spacing);
    let run = &line[content.len()..];
    let breakless = run == " " || before_blank;
    let safe = content.chars().last().is_some_and(safe_before_line_end);
    // Indented code, or a `-` its space keeps from underlining the text
    // above, reads its trailing spacing.
    let read = may_be_indented_code(line) || (under_text && setext_dash(content));
    match (breakless && safe, read) {
        (true, false) => content.to_string(),
        (false, _) | (true, true) => line.to_string(),
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
