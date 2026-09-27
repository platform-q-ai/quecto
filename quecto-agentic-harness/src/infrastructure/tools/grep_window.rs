//! Showing a line longer than the tool's line budget (#2201): a matching
//! line as windows around its matches, any other line from its start. Text
//! left out is marked `…[N bytes]…`, so every byte of the line is either
//! shown or counted, and the line's size and first match's byte offset
//! follow, so the agent can find the match with bash.

use std::ops::Range;

use crate::infrastructure::tools::truncate::format_size;

/// The most windows one line is shown in.
pub(super) const MAX_WINDOWS: usize = 4;
/// Bytes of context kept either side of a match at the least; matches
/// closer than twice this share a window.
const MIN_CONTEXT: usize = 40;

/// `line` with at most `budget` bytes of its text: whole when it fits;
/// otherwise around `hits` (the byte ranges of its matches), or from its
/// start when it has none. Also whether it was cut.
pub(super) fn show_line(line: &str, hits: &[Range<usize>], budget: usize) -> (String, bool) {
    if line.len() <= budget {
        return (line.to_string(), false);
    }
    let hits = usable(line, hits);
    let windows = match hits.as_slice() {
        [] => std::iter::once(0..budget).collect(),
        hits => around(line.len(), hits, budget),
    };
    let windows: Vec<Range<usize>> = windows
        .into_iter()
        .map(|window| on_boundaries(line, window))
        .collect();
    let kept: usize = windows.iter().map(ExactSizeIterator::len).sum();
    assert!(
        kept <= budget,
        "a line shows at most {budget} bytes of its text, got {kept}"
    );
    (render(line, &windows, &hits), true)
}

/// `hits` on `line`: sorted, on character boundaries, clamped to the line;
/// a hit starting past it (an offset rg cannot have meant) is dropped. On
/// a line that is not UTF-8, rg's offsets are into its raw bytes, which
/// the lossy decoding lengthens, so windows there may sit a little early.
fn usable(line: &str, hits: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut kept: Vec<Range<usize>> = hits
        .iter()
        .filter(|hit| hit.start <= line.len())
        .map(|hit| {
            let start = floor_boundary(line, hit.start);
            start..floor_boundary(line, hit.end.min(line.len())).max(start)
        })
        .collect();
    kept.sort_by_key(|hit| hit.start);
    kept
}

/// Windows (in line order, not overlapping, at most `budget` bytes in all)
/// around the first matches of a line `len` bytes long: matches close
/// together share one; the budget left over widens each evenly.
fn around(len: usize, hits: &[Range<usize>], budget: usize) -> Vec<Range<usize>> {
    assert!(!hits.is_empty(), "windows are placed around matches");
    assert!(len > budget, "only a line longer than its budget is cut");
    let context = MIN_CONTEXT.min(budget / 4);
    let mut clusters: Vec<Range<usize>> = Vec::new();
    for hit in hits {
        match clusters.last_mut() {
            Some(last) if hit.start <= last.end + 2 * context => last.end = last.end.max(hit.end),
            Some(_) | None => clusters.push(hit.clone()),
        }
    }
    let first = &clusters[0];
    if first.len() + 2 * context > budget {
        // Its matches alone are larger than the budget: their start (the
        // window moved back from the line's end to keep the whole budget).
        // At exactly the budget both ways give this same window.
        let start = first.start.saturating_sub(context).min(len - budget);
        return std::iter::once(start..start + budget).collect();
    }
    let mut chosen: Vec<Range<usize>> = Vec::new();
    let mut used = 0;
    for cluster in clusters {
        let need = cluster.len() + 2 * context;
        match chosen.len() < MAX_WINDOWS && used + need <= budget {
            true => {
                used += need;
                chosen.push(cluster);
            }
            false => break,
        }
    }
    let extra = (budget - used) / (2 * chosen.len());
    let mut windows: Vec<Range<usize>> = Vec::new();
    for cluster in chosen {
        let side = context + extra;
        let width = cluster.len() + 2 * side;
        // Clipped at one end of the line, the window grows at the other.
        let start = cluster
            .start
            .saturating_sub(side)
            .min(len.saturating_sub(width));
        let window = start..(start + width).min(len);
        match windows.last_mut() {
            Some(last) if window.start <= last.end => last.end = last.end.max(window.end),
            Some(_) | None => windows.push(window),
        }
    }
    windows
}

/// `window` narrowed to character boundaries: its start up, its end down.
/// A match starts on a boundary, so a window keeps the matches it starts
/// before.
fn on_boundaries(line: &str, window: Range<usize>) -> Range<usize> {
    let end = floor_boundary(line, window.end.min(line.len()));
    let mut start = window.start.min(end);
    while !line.is_char_boundary(start) {
        start += 1;
    }
    start..end.max(start)
}

/// The nearest character boundary at or before `at` (at most the length).
fn floor_boundary(line: &str, at: usize) -> usize {
    let mut at = at.min(line.len());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The windows of `line`, the text between them marked with its size,
/// then the line's size and where its matches are.
fn render(line: &str, windows: &[Range<usize>], hits: &[Range<usize>]) -> String {
    let mut shown = String::new();
    let mut at = 0;
    for window in windows {
        if window.start > at {
            shown.push_str(&format!("…[{} bytes]…", window.start - at));
        }
        shown.push_str(&line[window.clone()]);
        at = window.end;
    }
    if at < line.len() {
        shown.push_str(&format!("…[{} bytes]…", line.len() - at));
    }
    let size = format_size(line.len());
    let shown_hits = hits
        .iter()
        .filter(|hit| windows.iter().any(|window| holds(window, hit)))
        .count();
    let hidden = hits.len() - shown_hits;
    let located = match (hits, hidden) {
        ([], _) => String::new(),
        ([only], _) => format!("; match at byte {}", only.start),
        ([first, ..], 0) => format!("; {} matches, first at byte {}", hits.len(), first.start),
        ([first, ..], hidden) => format!(
            "; {} matches, first at byte {}; {hidden} not shown",
            hits.len(),
            first.start
        ),
    };
    format!("{shown} [line is {size}{located}]")
}

/// Whether `window` shows where `hit` starts (an empty hit at its end too).
fn holds(window: &Range<usize>, hit: &Range<usize>) -> bool {
    window.contains(&hit.start) || (hit.is_empty() && hit.start == window.end)
}

#[cfg(test)]
#[path = "grep_window_tests.rs"]
mod tests;
