// The `edit` tool's success diff: a Quecto-style, line-numbered unified
// diff via the `similar` crate, bounded to [`DIFF_MAX_BYTES`].
//
// Pure rendering, no I/O. A long changed line is shown as a window around
// the change, never from its start (#2194); every cut is on a character
// boundary.

use std::ops::Range;

use similar::{DiffTag, TextDiff};

pub(super) const DIFF_MAX_BYTES: usize = 4096;
const DIFF_CONTEXT_LINES: usize = 4;

/// A line longer than this is shown in part (#2194).
const LONG_LINE_BYTES: usize = 240;
/// Unchanged text shown on each side of the change in a long line.
const WINDOW_CONTEXT_BYTES: usize = 60;
/// Changed text shown from each end of a long change.
const SPAN_END_BYTES: usize = 60;
/// The start shown of a long line with no changed partner.
const LINE_HEAD_BYTES: usize = 120;
/// Marks text left out of a shown line.
const ELLIPSIS: &str = "\u{2026}";

/// Produce a Quecto-style unified-diff snippet using LCS-based line diffing
/// via `similar`, with per-line numbers and an ellipsis between hunks:
///
/// ```text
///  11 context line
/// -12 removed line
/// +12 added line
///     ...
/// ```
///
/// Context is [`DIFF_CONTEXT_LINES`] lines on each side of each hunk. A long
/// changed line is shown as a window around the part of `change` it holds,
/// and a note gives the column; other long lines show their start (#2194).
/// If the rendered diff still exceeds [`DIFF_MAX_BYTES`], a bounded prefix
/// of concrete diff lines is returned with a truncation notice; a note is
/// never cut from its line.
pub(super) fn make_edit_diff(
    path: &str,
    old_content: &str,
    new_content: &str,
    change: &Change,
) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    let max_line = old_content.lines().count().max(new_content.lines().count());
    let mut lines = DiffLines {
        old: Side::of(diff.old_slices(), old_content, change.old.clone()),
        new: Side::of(diff.new_slices(), new_content, change.new.clone()),
        width: max_line.max(1).to_string().len(),
        out: Vec::new(),
        changed: 0,
    };

    let ops = diff.grouped_ops(DIFF_CONTEXT_LINES);
    let total_hunks = ops.len();
    let mut hunk_end_line_indexes = Vec::new();

    for (group_idx, group) in ops.iter().enumerate() {
        for op in group {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            match tag {
                DiffTag::Equal => lines.context(old_range),
                DiffTag::Delete | DiffTag::Insert | DiffTag::Replace => {
                    lines.changes(old_range, new_range);
                }
            }
        }
        hunk_end_line_indexes.push(lines.out.len());
        // Add ellipsis between hunks (not after the last one).
        if group_idx + 1 < total_hunks {
            let separator = lines.under_numbers("...");
            lines.out.push(separator);
        }
    }

    let changed_lines_total = lines.changed;
    render_bounded_edit_diff(
        path,
        lines.out,
        hunk_end_line_indexes,
        total_hunks,
        changed_lines_total,
    )
}

/// What an edit changed: a byte range of the old text and the range of the
/// new text that replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Change {
    pub(super) old: Range<usize>,
    pub(super) new: Range<usize>,
}

impl Change {
    /// The splice of `inserted` bytes over `splice` of `old`, narrowed to
    /// the part that differs: text the replacement kept at either end is
    /// not part of the change.
    pub(super) fn of_splice(old: &str, new: &str, splice: Range<usize>, inserted: usize) -> Self {
        let replaced = splice.start..splice.start.saturating_add(inserted);
        match (old.get(splice.clone()), new.get(replaced)) {
            (Some(was), Some(now)) => {
                let (old_span, new_span) = changed_spans(was, now);
                let shift = |span: Range<usize>| span.start + splice.start..span.end + splice.start;
                Self {
                    old: shift(old_span),
                    new: shift(new_span),
                }
            }
            _ => {
                debug_assert!(false, "splice {splice:?} is not in the texts");
                Self::between(old, new)
            }
        }
    }

    /// Everything between the texts' common prefix and common suffix.
    pub(super) fn between(old: &str, new: &str) -> Self {
        let (old, new) = changed_spans(old, new);
        Self { old, new }
    }
}

/// One side of the diff: its lines, where each starts, and its part of the
/// change.
struct Side<'a> {
    lines: &'a [&'a str],
    starts: Vec<usize>,
    change: Range<usize>,
}

impl<'a> Side<'a> {
    fn of(lines: &'a [&'a str], text: &str, change: Range<usize>) -> Self {
        let starts: Vec<usize> = lines
            .iter()
            .scan(0usize, |at, line| {
                let start = *at;
                *at += line.len();
                Some(start)
            })
            .collect();
        debug_assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), text.len());
        debug_assert!(text.get(change.clone()).is_some(), "{change:?}");
        Self {
            lines,
            starts,
            change,
        }
    }

    fn line(&self, index: usize) -> &'a str {
        let line = self.lines.get(index).copied();
        debug_assert!(line.is_some(), "diff line {index} out of range");
        line.unwrap_or_default().trim_end_matches('\n')
    }

    fn start(&self, index: usize) -> usize {
        let start = self.starts.get(index).copied();
        debug_assert!(start.is_some(), "diff line {index} out of range");
        start.unwrap_or_default()
    }

    /// A changed line with no partner on the other side. A long one that
    /// holds only part of the edit's change is windowed on that part, with
    /// the column it starts at; one wholly inside the change shows its two
    /// ends; one outside it shows its start.
    fn unpaired_view(&self, index: usize) -> (String, Option<usize>) {
        let line = self.line(index);
        let start = self.start(index);
        let end = start + line.len();
        let change = &self.change;
        let (low, high) = (change.start.max(start), change.end.min(end));
        let holds_change = match change.is_empty() {
            true => start <= change.start && change.start <= end,
            false => low < high,
        };
        let is_whole = change.start <= start && end <= change.end;
        match (line.len() > LONG_LINE_BYTES, holds_change, is_whole) {
            (true, true, false) => {
                let span =
                    floor_boundary(line, low - start)..ceil_boundary(line, high.max(low) - start);
                (
                    window(line, span.clone()),
                    Some(column_of(line, span.start)),
                )
            }
            (true, true, true) => (elide_middle(line), None),
            (true, false, _) | (false, _, _) => (head_view(line), None),
        }
    }
}

/// The 1-based character column of byte `at` of `line`.
fn column_of(line: &str, at: usize) -> usize {
    line.get(..at).unwrap_or_default().chars().count() + 1
}

/// How many new lines ahead a removed line looks for its partner.
const PARTNER_LOOKAHEAD: usize = 8;

/// Bytes two lines share at their start and at their end.
fn shared_bytes(old: &str, new: &str) -> usize {
    let (old_span, _) = changed_spans(old, new);
    old_span.start + (old.len() - old_span.end)
}

/// Each removed line's partner among the added ones, when either is long:
/// the added line (in order, a few ahead) that shares the most text with it
/// at its start and end. A line that shares nothing has no partner.
fn partners(
    old: &Side<'_>,
    new: &Side<'_>,
    old_range: Range<usize>,
    new_range: Range<usize>,
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut next = new_range.start;
    for i in old_range {
        let old_line = old.line(i);
        let best = (next..new_range.end)
            .take(PARTNER_LOOKAHEAD)
            .filter_map(|j| {
                let new_line = new.line(j);
                let is_long = old_line.len() > LONG_LINE_BYTES || new_line.len() > LONG_LINE_BYTES;
                let shared = is_long.then(|| shared_bytes(old_line, new_line))?;
                (shared > 0).then_some((shared, j))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        if let Some((_, j)) = best {
            pairs.push((i, j));
            next = j + 1;
        }
    }
    pairs
}

/// A removed line and its added partner, each windowed on its own
/// difference, and the note for the added one.
fn pair_views(old: &str, new: &str, number: usize) -> (String, String, String) {
    let (old_span, new_span) = changed_spans(old, new);
    let note = match old == new {
        true => format!("[the text of line {number} is unchanged: only its line break changed]"),
        false => {
            let was = match old.len() {
                same if same == new.len() => String::new(),
                other => format!(", was {other} bytes"),
            };
            format!(
                "[the change starts at column {}; line {number} is {} bytes{was}; \
                 {ELLIPSIS} marks text left out]",
                column_of(new, new_span.start),
                new.len()
            )
        }
    };
    (window(old, old_span), window(new, new_span), note)
}

/// The rendered lines of a diff, as they are built. A note shares its
/// line's entry, so the byte cap keeps or drops the two together.
struct DiffLines<'a> {
    old: Side<'a>,
    new: Side<'a>,
    width: usize,
    out: Vec<String>,
    changed: usize,
}

impl DiffLines<'_> {
    fn push(&mut self, marker: char, number: usize, text: &str) {
        self.out.push(format!(
            "{marker}{number:>width$} {text}",
            width = self.width
        ));
    }

    /// Push a line with a note under it, as one entry.
    fn push_noted(&mut self, marker: char, number: usize, text: &str, note: &str) {
        let note = self.under_numbers(note);
        self.push(marker, number, text);
        if let Some(entry) = self.out.last_mut() {
            entry.push('\n');
            entry.push_str(&note);
        }
    }

    /// Text on a line of its own under the line numbers.
    fn under_numbers(&self, text: &str) -> String {
        format!(" {:>width$} {text}", "", width = self.width)
    }

    fn context(&mut self, old_range: Range<usize>) {
        for i in old_range {
            let line = self.old.line(i);
            self.push(' ', i + 1, &head_view(line));
        }
    }

    /// Removed lines, then added ones. A long line paired with a partner
    /// ([`partners`]) is windowed on its own difference from it, and a note
    /// follows the added one; a line with no partner is shown by
    /// [`Side::unpaired_view`], with a note when it is windowed.
    fn changes(&mut self, old_range: Range<usize>, new_range: Range<usize>) {
        let mut old_views: Vec<Option<String>> = vec![None; old_range.len()];
        let mut new_views: Vec<Option<(String, String)>> = vec![None; new_range.len()];
        for (i, j) in partners(&self.old, &self.new, old_range.clone(), new_range.clone()) {
            let (old_view, new_view, note) = pair_views(self.old.line(i), self.new.line(j), j + 1);
            if let Some(slot) = old_views.get_mut(i - old_range.start) {
                *slot = Some(old_view);
            }
            if let Some(slot) = new_views.get_mut(j - new_range.start) {
                *slot = Some((new_view, note));
            }
        }
        for (k, i) in old_range.enumerate() {
            let view = old_views.get_mut(k).and_then(Option::take);
            let (text, note) = match view {
                Some(text) => (text, None),
                None => self.unpaired('-', i),
            };
            self.push_line('-', i + 1, &text, note);
        }
        for (k, j) in new_range.enumerate() {
            let view = new_views.get_mut(k).and_then(Option::take);
            let (text, note) = match view {
                Some((text, note)) => (text, Some(note)),
                None => self.unpaired('+', j),
            };
            self.push_line('+', j + 1, &text, note);
        }
    }

    /// An unpaired line's view, and a note when it is windowed.
    fn unpaired(&self, marker: char, index: usize) -> (String, Option<String>) {
        let side = match marker {
            '+' => &self.new,
            _ => &self.old,
        };
        let (text, column) = side.unpaired_view(index);
        let note = column.map(|column| {
            format!(
                "[the change starts at column {column}; line {} is {} bytes; \
                 {ELLIPSIS} marks text left out]",
                index + 1,
                side.line(index).len()
            )
        });
        (text, note)
    }

    fn push_line(&mut self, marker: char, number: usize, text: &str, note: Option<String>) {
        match note {
            Some(note) => self.push_noted(marker, number, text, &note),
            None => self.push(marker, number, text),
        }
        self.changed += 1;
    }
}

/// A line whole, or its start and an ellipsis when it is long.
fn head_view(line: &str) -> String {
    if line.len() <= LONG_LINE_BYTES {
        return line.to_string();
    }
    let end = floor_boundary(line, LINE_HEAD_BYTES);
    format!("{}{ELLIPSIS}", line.get(..end).unwrap_or_default())
}

fn render_bounded_edit_diff(
    path: &str,
    diff_lines: Vec<String>,
    hunk_end_line_indexes: Vec<usize>,
    total_hunks: usize,
    changed_lines_total: usize,
) -> String {
    let full = format!("Successfully edited {}\n\n{}", path, diff_lines.join("\n"));
    if full.len() <= DIFF_MAX_BYTES {
        return full;
    }

    let mut shown = Vec::new();
    for line in &diff_lines {
        shown.push(line.clone());
        let hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
        let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
        let prefix = truncated_success_prefix(path, notice.len(), 1);
        let candidate = format!("{}{}\n{}", prefix, shown.join("\n"), notice);
        if candidate.len() > DIFF_MAX_BYTES {
            shown.pop();
            break;
        }
    }

    let mut hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
    let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
    if shown.is_empty() {
        shown = truncated_first_change_pair(path, &diff_lines, notice.len());
        hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
    }
    let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
    let prefix = truncated_success_prefix(path, notice.len(), shown.join("\n").len());
    format!("{}{}\n{}", prefix, shown.join("\n"), notice)
}

fn truncated_first_change_pair(
    path: &str,
    diff_lines: &[String],
    notice_len: usize,
) -> Vec<String> {
    let Some(first_line) = diff_lines.first() else {
        return Vec::new();
    };
    let second_change_line = diff_lines
        .iter()
        .skip(1)
        .find(|line| line.starts_with('+') || line.starts_with('-'));
    let reserved_second_len = second_change_line
        .map(|line| line.len().min(8))
        .unwrap_or_default();
    let reserved_diff_len =
        first_line.len().min(16) + reserved_second_len + usize::from(second_change_line.is_some());
    let prefix = truncated_success_prefix(path, notice_len, reserved_diff_len);
    let mut budget = DIFF_MAX_BYTES.saturating_sub(prefix.len() + notice_len + 1);

    let mut shown = Vec::new();
    let first_budget =
        budget.saturating_sub(reserved_second_len + usize::from(second_change_line.is_some()));
    shown.push(truncate_to_byte_budget(first_line, first_budget));
    budget = budget.saturating_sub(shown[0].len());

    if let Some(second_line) = second_change_line {
        budget = budget.saturating_sub(1);
        let second = truncate_to_byte_budget(second_line, budget);
        if !second.is_empty() {
            shown.push(second);
        }
    }

    shown
}

fn truncated_success_prefix(path: &str, notice_len: usize, diff_len: usize) -> String {
    let boilerplate_len = "Successfully edited \n\n\n".len();
    let path_budget = DIFF_MAX_BYTES.saturating_sub(boilerplate_len + notice_len + diff_len);
    format!(
        "Successfully edited {}\n\n",
        truncate_to_byte_budget(path, path_budget)
    )
}

fn count_complete_hunks_shown(shown_lines: usize, hunk_end_line_indexes: &[usize]) -> usize {
    hunk_end_line_indexes
        .iter()
        .take_while(|end| shown_lines >= **end)
        .count()
}

fn truncate_to_byte_budget(line: &str, budget: usize) -> String {
    line.char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(line.len()))
        .take_while(|idx| *idx <= budget)
        .last()
        .and_then(|end| line.get(..end))
        .map(str::to_string)
        .unwrap_or_default()
}

fn diff_truncated_notice(
    hunks_shown: usize,
    total_hunks: usize,
    changed_lines_total: usize,
) -> String {
    format!(
        "[diff truncated: {} of {} hunks shown, {} lines changed total]",
        hunks_shown, total_hunks, changed_lines_total
    )
}

/// The byte ranges of `old` and `new` that differ: what is left after
/// their longest common prefix and then their longest common suffix, both
/// cut on character boundaries. The two never overlap.
fn changed_spans(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let prefix: usize = common_len(old.chars().zip(new.chars()));
    let old_rest = old.get(prefix..).unwrap_or_default();
    let new_rest = new.get(prefix..).unwrap_or_default();
    let suffix: usize = common_len(old_rest.chars().rev().zip(new_rest.chars().rev()));
    let spans = (prefix..old.len() - suffix, prefix..new.len() - suffix);
    debug_assert!(old.get(spans.0.clone()).is_some() && new.get(spans.1.clone()).is_some());
    spans
}

/// Bytes of the leading run of equal character pairs.
fn common_len(pairs: impl Iterator<Item = (char, char)>) -> usize {
    pairs
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}

/// `line` from [`WINDOW_CONTEXT_BYTES`] before `span` to as far after it,
/// with the span's middle left out when it is long, and an ellipsis where
/// text is left out.
fn window(line: &str, span: Range<usize>) -> String {
    let start = floor_boundary(line, span.start.saturating_sub(WINDOW_CONTEXT_BYTES));
    let end = ceil_boundary(line, span.end.saturating_add(WINDOW_CONTEXT_BYTES));
    let mut shown = String::new();
    if start > 0 {
        shown.push_str(ELLIPSIS);
    }
    shown.push_str(line.get(start..span.start).unwrap_or_default());
    shown.push_str(&elide_middle(line.get(span.clone()).unwrap_or_default()));
    shown.push_str(line.get(span.end..end).unwrap_or_default());
    if end < line.len() {
        shown.push_str(ELLIPSIS);
    }
    shown
}

/// `text` whole, or its first and last [`SPAN_END_BYTES`] around an
/// ellipsis.
fn elide_middle(text: &str) -> String {
    if text.len() <= 2 * SPAN_END_BYTES {
        return text.to_string();
    }
    let head_end = floor_boundary(text, SPAN_END_BYTES);
    let tail_start = ceil_boundary(text, text.len() - SPAN_END_BYTES);
    debug_assert!(head_end <= tail_start);
    format!(
        "{}{ELLIPSIS}{}",
        text.get(..head_end).unwrap_or_default(),
        text.get(tail_start..).unwrap_or_default()
    )
}

/// The largest character boundary of `text` at or before `at`.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The smallest character boundary of `text` at or after `at`.
fn ceil_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

#[cfg(test)]
#[path = "edit_diff_tests.rs"]
mod tests;
