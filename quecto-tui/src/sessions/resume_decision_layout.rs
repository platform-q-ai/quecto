//! Text layout of the resume decision dialog (#2011): pure functions over
//! already-safe, ANSI-free text. Words wrap whole, a folder path keeps both
//! its ends, and every section is bounded so the footer and the border of the
//! dialog are never pushed off a small terminal.
use crate::components::utils::visible_width;

const ELLIPSIS: char = '…';

/// Wrap on whitespace; only a word wider than `width` is broken, at the column.
pub(super) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let gap = usize::from(!line.is_empty());
            if visible_width(&line) + gap + visible_width(&word) <= width {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            let (head, tail) = split_at_width(&word, width);
            lines.push(head);
            word = tail;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` wrapped to at most `max_lines`; a cut is marked with an ellipsis.
pub(super) fn wrap_bounded(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines = wrap_words(text, width);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            let (mut head, _) = split_at_width(last, width.max(2) - 1);
            head.push(ELLIPSIS);
            *last = head;
        }
    }
    lines
}

/// A path on at most `max_lines` lines of `width` columns: broken at the
/// column (a path has no spaces) and, when it is longer than that, elided in
/// the MIDDLE — its root and its last components are what the user acts on.
pub(super) fn path_lines(path: &str, width: usize, max_lines: usize) -> Vec<String> {
    let width = width.max(1);
    let budget = width * max_lines.max(1);
    let mut rest = elide_middle(path, budget);
    let mut lines = Vec::new();
    while !rest.is_empty() {
        let (head, tail) = split_at_width(&rest, width);
        lines.push(head);
        rest = tail;
    }
    lines
}

/// `text` bounded to `max_chars` characters by dropping its MIDDLE: a long
/// path keeps the tail that tells it from its neighbours (never a head-only
/// cut, which shows two different folders as one).
pub(super) fn bounded_ends(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let head: String = text.chars().take(keep / 2).collect();
    let tail: String = text.chars().skip(count - (keep - keep / 2)).collect();
    format!("{head}{ELLIPSIS}{tail}")
}

fn elide_middle(text: &str, budget: usize) -> String {
    if visible_width(text) <= budget {
        return text.to_string();
    }
    let keep = budget.saturating_sub(1);
    let (head, _) = split_at_width(text, keep / 2);
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for ch in text.chars().rev() {
        let w = visible_width(ch.encode_utf8(&mut [0; 4]));
        if used + w > keep - keep / 2 {
            break;
        }
        used += w;
        tail.push(ch);
    }
    let tail: String = tail.into_iter().rev().collect();
    format!("{head}{ELLIPSIS}{tail}")
}

/// The longest prefix of `text` at most `width` columns wide, and the rest.
/// Always makes progress: a first character wider than `width` is taken whole.
fn split_at_width(text: &str, width: usize) -> (String, String) {
    let mut used = 0;
    for (index, ch) in text.char_indices() {
        let w = visible_width(ch.encode_utf8(&mut [0; 4]));
        if used + w > width && index > 0 {
            return (text[..index].to_string(), text[index..].to_string());
        }
        used += w;
    }
    (text.to_string(), String::new())
}

#[cfg(test)]
#[path = "resume_decision_layout_tests.rs"]
mod tests;
