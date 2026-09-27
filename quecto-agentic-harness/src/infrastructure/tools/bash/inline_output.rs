//! What a command's output shows inline: its last 2000 lines or 50KB, and a
//! note on what was left out and where the whole output was saved (#2167).
//! Output that fits the byte budget keeps every line whole, however long —
//! one-line JSON from `gh api`, `curl` or `jq -c` comes back as it was.
//! Only output over the budget has each line over 8KB cut to its start and
//! end before the tail is taken (#2196), so one huge line never fills the
//! tail or shows as a whole line when it is not.
use super::long_lines::{self, CutLine};
use super::saved_output::{self, TailView, save_to_temp_file};
use crate::infrastructure::tools::truncate::{TruncatedBy, TruncationResult, truncate_tail};

const TAIL_MAX_LINES: usize = 2000;
const TAIL_MAX_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

/// The inline view of `combined`, saving the whole to a temp file when any
/// of it is left out; `capture_cut` says the capture itself already dropped
/// part of the middle.
pub(super) async fn truncate_output(combined: String, capture_cut: bool) -> String {
    let whole = truncate_tail(&combined, TAIL_MAX_LINES, TAIL_MAX_BYTES);
    debug_assert_eq!(whole.truncated, whole.truncated_by.is_some());
    match whole.truncated_by {
        None => whole.content,
        // Within the byte budget the kept lines are all whole.
        Some(TruncatedBy::Lines) => {
            let note = Note::of(&whole, Vec::new());
            with_note(whole.content, note, combined, capture_cut).await
        }
        Some(TruncatedBy::Bytes) => windowed_tail(combined, capture_cut).await,
    }
}

/// Output over the byte budget: over-long lines cut to their windows, then
/// the tail of that.
async fn windowed_tail(combined: String, capture_cut: bool) -> String {
    // A cut line, marker included, always fits the tail's byte budget.
    const { assert!(2 * long_lines::LINE_MAX_BYTES < TAIL_MAX_BYTES) };
    let windowed = long_lines::window_long_lines(&combined);
    let tail = truncate_tail(&windowed.text, TAIL_MAX_LINES, TAIL_MAX_BYTES);
    // Every line now fits the byte budget, so the tail never shows part of
    // a line as if it were whole (#2196).
    assert!(!tail.last_line_partial, "a windowed line fits the tail");
    let start_line = Note::of(&tail, Vec::new()).start_line;
    let cuts: Vec<CutLine> = windowed
        .cut
        .iter()
        .copied()
        .filter(|cut| cut.number >= start_line)
        .collect();
    assert!(
        tail.truncated || !cuts.is_empty(),
        "output over the budget leaves something out"
    );
    // Windowed, the output may be cut by line count alone: then the range
    // is not labelled as the 50KB limit's.
    let note = Note::of(&tail, cuts);
    with_note(tail.content, note, combined, capture_cut).await
}

/// What the note after a cut output says.
struct Note {
    start_line: usize,
    total: usize,
    /// The cut lines the tail shows.
    cuts: Vec<CutLine>,
    /// The tail was cut by the 50KB budget, not by line count.
    by_bytes: bool,
}

impl Note {
    /// The note on `tail`. Its range is the byte budget's unless line count
    /// cut it (#2196 review): a tail cut by bytes, or windowed output that
    /// the windows alone brought within the budget. (Uncut output that fits
    /// the budget gets no note at all.)
    fn of(tail: &TruncationResult, cuts: Vec<CutLine>) -> Self {
        Self {
            start_line: tail.total_lines.saturating_sub(tail.output_lines) + 1,
            total: tail.total_lines,
            cuts,
            by_bytes: matches!(tail.truncated_by, None | Some(TruncatedBy::Bytes)),
        }
    }
}

/// `shown`, then the note on what was left out and where the whole
/// `combined` output was saved.
async fn with_note(mut shown: String, note: Note, combined: String, capture_cut: bool) -> String {
    let view = TailView {
        start_line: note.start_line,
        end_line: note.total,
        total: note.total,
        by_bytes: note.by_bytes,
        capture_cut,
        combined_len: combined.len(),
        long_lines: match note.cuts.as_slice() {
            [] => None,
            cuts => Some(long_lines::cut_note(cuts)),
        },
    };
    let saved_to = save_to_temp_file(combined).await;
    shown.push_str(&saved_output::truncation_hint(saved_to.as_deref(), &view));
    shown
}

#[cfg(test)]
#[path = "inline_output_tests.rs"]
mod tests;
