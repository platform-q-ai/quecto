// Why `oldText` was not found, when the answer is indentation: a match that
// exists once the leading whitespace of each line is ignored (#2193).
//
// Pure text logic, no I/O. A hint is given only when it is proven: oldText,
// re-indented as the file is, matches.

use super::edit_match::{Location, fuzzy_normalise, is_line_space, locate, occurrences};

/// oldText matches the file once its indentation is the file's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndentMismatch {
    /// Where the match's first line starts, as a byte offset of the text.
    pub(super) start_at: usize,
    /// Where further proven matches start, up to [`LISTED_HINTS`] in all.
    pub(super) also_at: Vec<usize>,
    /// More proven matches exist than are listed.
    pub(super) more: bool,
    /// Where the first line whose indentation differs starts.
    pub(super) differs_at: usize,
    /// That line's indentation in the file.
    pub(super) file_indent: String,
    /// The same line's indentation in oldText.
    pub(super) old_indent: String,
}

/// Candidates tried before giving up: each proof reads its lines again, and
/// a line may be up to the whole 1 MiB file.
const MAX_CANDIDATES: usize = 16;

/// Proven matches a hint lists; the first one's indentation is described.
pub(super) const LISTED_HINTS: usize = 3;

/// Where `old` (BOM-stripped, LF) would match `content` if its lines were
/// indented as the file's are, or `None` when no such match is proven.
///
/// Both sides are fuzzy-normalised and stripped of each line's leading
/// whitespace, then searched. An indented first line of `old` stands for a
/// whole line, so its candidates start at a line start; an unindented one
/// may start mid-line and its indentation is not compared. A candidate is
/// kept only when `old`, re-indented from the candidate's own lines,
/// matches those lines ([`locate`]). Every proven match among the first
/// [`MAX_CANDIDATES`] is counted, so an ambiguous hint says so.
pub(super) fn indent_mismatch(content: &str, old: &str) -> Option<IndentMismatch> {
    let old_lines: Vec<&str> = old.split('\n').collect();
    // A whitespace-only first line indents nothing: it is unindented.
    let first_indented = old_lines
        .first()
        .is_some_and(|line| line.starts_with(is_line_space) && !split_indent(line).1.is_empty());
    let needle = dedent(&fuzzy_normalise(old).ok()?.text);
    let haystack = dedent(&fuzzy_normalise(content).ok()?.text);
    let file_lines: Vec<&str> = content.split('\n').collect();
    let line_starts: Vec<usize> = file_lines
        .iter()
        .scan(0usize, |at, line| {
            let start = *at;
            *at += line.len() + 1;
            Some(start)
        })
        .collect();
    let bytes = haystack.as_bytes();
    let mut line = 0usize;
    let mut scanned = 0usize;
    let mut last_tried = None;
    let mut proven = occurrences(&haystack, &needle)
        .filter(|&start| match first_indented {
            true => start == 0 || bytes.get(start - 1) == Some(&b'\n'),
            false => true,
        })
        .filter_map(|start| {
            line += bytes
                .get(scanned..start)?
                .iter()
                .filter(|b| **b == b'\n')
                .count();
            scanned = start;
            let is_new_line = last_tried != Some(line);
            last_tried = Some(line);
            is_new_line.then_some(line)
        })
        .take(MAX_CANDIDATES)
        .filter_map(|at| prove(&file_lines, &line_starts, at, &old_lines, first_indented));
    let mut first = proven.next()?;
    for other in proven {
        match first.also_at.len() + 1 < LISTED_HINTS {
            true => first.also_at.push(other.start_at),
            false => {
                first.more = true;
                break;
            }
        }
    }
    Some(first)
}

/// Re-indent `old_lines` from the file's lines starting at `at` (0-based)
/// and name the first line whose indentation differs, when the re-indented
/// text matches those lines.
fn prove(
    file_lines: &[&str],
    line_starts: &[usize],
    at: usize,
    old_lines: &[&str],
    first_indented: bool,
) -> Option<IndentMismatch> {
    let lines = file_lines.get(at..at.checked_add(old_lines.len())?)?;
    let mut first_difference = None;
    let mut reindented = Vec::with_capacity(old_lines.len());
    for (i, (old_line, file_line)) in old_lines.iter().zip(lines).enumerate() {
        let (old_indent, body) = split_indent(old_line);
        let (file_indent, _) = split_indent(file_line);
        let is_compared = (i > 0 || first_indented) && !body.is_empty();
        if is_compared {
            if old_indent != file_indent && first_difference.is_none() {
                first_difference = Some(IndentMismatch {
                    start_at: line_starts.get(at).copied()?,
                    differs_at: line_starts.get(at + i).copied()?,
                    also_at: Vec::new(),
                    more: false,
                    file_indent: file_indent.to_string(),
                    old_indent: old_indent.to_string(),
                });
            }
            reindented.push(format!("{file_indent}{body}"));
        } else {
            reindented.push((*old_line).to_string());
        }
    }
    let found = first_difference?;
    let matched = locate(&lines.join("\n"), &reindented.join("\n"));
    matches!(matched, Ok(Location::Unique(_) | Location::Ambiguous(_))).then_some(found)
}

/// A line's leading in-line whitespace, and the rest.
fn split_indent(line: &str) -> (&str, &str) {
    let body = line.trim_start_matches(is_line_space);
    line.split_at(line.len() - body.len())
}

/// Drop the leading in-line whitespace of every line; newlines are kept, so
/// line numbers are unchanged.
fn dedent(text: &str) -> String {
    text.split('\n')
        .map(|line| split_indent(line).1)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Indentation in words, run by run: "1 tab", "4 spaces", "2 tabs then 2
/// spaces", "no indentation".
pub(super) fn describe_indent(indent: &str) -> String {
    let mut runs: Vec<(&str, usize)> = Vec::new();
    for c in indent.chars() {
        let kind = match c {
            '\t' => "tab",
            ' ' => "space",
            _ => "other whitespace character",
        };
        match runs.last_mut() {
            Some((last, count)) if *last == kind => *count += 1,
            Some(_) | None => runs.push((kind, 1)),
        }
    }
    if runs.is_empty() {
        return "no indentation".to_string();
    }
    runs.iter()
        .map(|(kind, count)| match count {
            1 => format!("1 {kind}"),
            many => format!("{many} {kind}s"),
        })
        .collect::<Vec<_>>()
        .join(" then ")
}

#[cfg(test)]
#[path = "edit_indent_tests.rs"]
mod tests;
