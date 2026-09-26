//! Formatting a content search's matches into blocks (#2163): each match
//! with its context, every line shown once per run of blocks in a file, and
//! a matching line always shown as a match, with rg's own text for it.
use std::collections::HashMap;
use std::path::PathBuf;

use super::{BEYOND_CACHE, RgMatch, read_file_for_cache, truncate_line};

/// Format parsed matches with file-cache-based context extraction (Quecto compatibility).
/// Configuration shared across all match blocks during formatting.
pub(super) struct BlockConfig<'a> {
    pub(super) ws_str: &'a str,
    pub(super) ws_prefix_slash: &'a str,
    pub(super) context_lines: usize,
    pub(super) max_line_bytes: usize,
    pub(super) max_output_bytes: usize,
    /// Every line a shown match spans, by file (#2163): such a line is
    /// shown as a match wherever it appears, never as another's context.
    pub(super) matched: &'a HashMap<PathBuf, HashMap<usize, MatchedLine>>,
}

/// A line some shown match spans.
pub(super) struct MatchedLine {
    /// The match's score, on its first line only.
    score: Option<f64>,
    /// The line as rg reported it.
    text: Option<String>,
}

/// The lines each match in `matches` spans, by file.
pub(super) fn matched_lines(matches: &[RgMatch]) -> HashMap<PathBuf, HashMap<usize, MatchedLine>> {
    let mut by_file: HashMap<PathBuf, HashMap<usize, MatchedLine>> = HashMap::new();
    for m in matches {
        let lines = by_file.entry(m.file_path.clone()).or_default();
        for offset in 0..m.line_count.max(1) {
            lines.insert(
                m.line_number + offset,
                MatchedLine {
                    score: m.score.filter(|_| offset == 0),
                    text: m.text.as_ref().and_then(|text| text.get(offset).cloned()),
                },
            );
        }
    }
    by_file
}

/// State accumulated while formatting matches.
pub(super) struct FormatState {
    pub(super) output_lines: Vec<String>,
    pub(super) byte_total: usize,
    pub(super) lines_truncated: bool,
    pub(super) truncated_bytes: bool,
    /// The lines already shown, by file: no line is shown twice, whatever
    /// order the blocks come in (#2163; rank order, #2174 review).
    pub(super) shown: HashMap<PathBuf, std::collections::HashSet<usize>>,
    /// The file and line printed last.
    pub(super) last_printed: Option<(PathBuf, usize)>,
}

/// Format one match block (match line + optional context lines) into `state`.
/// Returns `false` when the byte limit is exceeded and formatting should stop.
pub(super) fn format_match_block(
    m: &RgMatch,
    file_cache: &mut HashMap<PathBuf, Vec<String>>,
    cfg: &BlockConfig<'_>,
    state: &mut FormatState,
) -> bool {
    let file_lines = file_cache
        .entry(m.file_path.clone())
        .or_insert_with(|| read_file_for_cache(&m.file_path));

    let raw_path = m.file_path.to_string_lossy();
    let rel_path = if let Some(rest) = raw_path.strip_prefix(cfg.ws_prefix_slash) {
        rest
    } else if let Some(rest) = raw_path.strip_prefix(cfg.ws_str) {
        rest
    } else {
        raw_path.as_ref()
    };
    // A search of "." reports `<workspace>/./x`: show `x`, as files mode does.
    let rel_path = rel_path.strip_prefix("./").unwrap_or(rel_path);

    let total_lines = file_lines.len();
    let last_matched = m.line_number + m.line_count.max(1) - 1;
    let start = m.line_number.saturating_sub(cfg.context_lines).max(1);
    let end = (last_matched + cfg.context_lines)
        .min(total_lines.max(last_matched))
        .max(m.line_number);
    let matched_here = cfg.matched.get(&m.file_path);
    // A match whose own lines were all shown earlier continues only when
    // the output has just come from it (rg order: its context follows on);
    // otherwise its context alone would stand apart from it (a better
    // ranked block showed it, #2174 review).
    if let Some(shown) = state.shown.get(&m.file_path) {
        let all_shown = (m.line_number..=last_matched).all(|line| shown.contains(&line));
        let continues = matches!(
            &state.last_printed,
            Some((path, line)) if path == &m.file_path && *line >= m.line_number && *line <= last_matched + cfg.context_lines
        );
        if all_shown && !continues {
            return true;
        }
    }

    for current in start..=end {
        // A line this file already showed is not shown again (#2163).
        if state
            .shown
            .get(&m.file_path)
            .is_some_and(|lines| lines.contains(&current))
        {
            continue;
        }
        let matched_line = matched_here.and_then(|lines| lines.get(&current));
        // This block's own match counts whether or not the map lists it.
        let own = (m.line_number..=last_matched).contains(&current);
        let matched = own || matched_line.is_some();
        let reported = matched_line
            .and_then(|line| line.text.as_deref())
            .or_else(|| {
                own.then(|| {
                    m.text
                        .as_ref()?
                        .get(current - m.line_number)
                        .map(String::as_str)
                })
                .flatten()
            });
        let line_text = match (reported, file_lines.get(current - 1), matched) {
            // A matching line as rg reported it, however far into its file.
            (Some(text), _, _) => text,
            (None, Some(line), _) => line.as_str(),
            // Past what the context cache read (the first 1 MB): say so
            // rather than print an empty match line; skip empty context.
            (None, None, true) => BEYOND_CACHE,
            (None, None, false) => continue,
        };
        let sanitized = line_text.trim_end_matches('\n');
        let (display_text, was_truncated) = truncate_line(sanitized, cfg.max_line_bytes);
        if was_truncated {
            state.lines_truncated = true;
        }

        let formatted = if matched {
            let own_score = m.score.filter(|_| current == m.line_number);
            let score = match matched_line.and_then(|line| line.score).or(own_score) {
                Some(score) => format!("[{score:.2}] "),
                None => String::new(),
            };
            format!("{score}{}:{}: {}", rel_path, current, display_text)
        } else {
            format!("{}-{}- {}", rel_path, current, display_text)
        };

        state.byte_total += formatted.len() + 1;
        if state.byte_total > cfg.max_output_bytes {
            state.truncated_bytes = true;
            return false;
        }
        state.output_lines.push(formatted);
        state
            .shown
            .entry(m.file_path.clone())
            .or_default()
            .insert(current);
        state.last_printed = Some((m.file_path.clone(), current));
    }
    true
}
