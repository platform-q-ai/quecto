//! Fuzzy matching — finds substring matches with scoring.
//!
//! All query characters must appear in order in the target text.
//! Rewards consecutive matches, word boundary matches. Penalizes gaps.
//! Supports space-separated multi-token queries (all tokens must match).

/// Result of a fuzzy match attempt.
#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

/// Fuzzy match a single query against text.
///
/// Returns whether all characters in `query` appear in order in `text`,
/// and a score (lower = better match).
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = lowercase_chars(query);
    fuzzy_match_lowered(&query_lower, text)
}

fn fuzzy_match_lowered(query_lower: &[char], text: &str) -> FuzzyMatch {
    if query_lower.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }

    let mut query_idx = 0;
    let mut score: f64 = 0.0;
    let mut last_match_idx: Option<usize> = None;
    let mut consecutive = 0;
    let mut prev_ch: Option<char> = None;

    for (i, ch) in text.chars().flat_map(char::to_lowercase).enumerate() {
        if query_idx < query_lower.len() && ch == query_lower[query_idx] {
            // Word boundary bonus.
            let is_boundary = i == 0 || prev_ch.is_some_and(is_boundary_separator);
            if is_boundary {
                score -= 10.0;
            }

            // Consecutive match bonus.
            if i > 0 && last_match_idx == Some(i - 1) {
                consecutive += 1;
                score -= consecutive as f64 * 5.0;
            } else {
                consecutive = 0;
                // Gap penalty.
                if let Some(last) = last_match_idx {
                    score += (i - last - 1) as f64 * 2.0;
                }
            }

            // Slight penalty for later positions.
            score += i as f64 * 0.1;

            last_match_idx = Some(i);
            query_idx += 1;
        }
        prev_ch = Some(ch);
    }

    if query_idx < query_lower.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

fn lowercase_chars(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

fn is_boundary_separator(ch: char) -> bool {
    matches!(ch, ' ' | '-' | '_' | '.' | '/' | ':')
}

/// Filter and sort items by fuzzy match quality (best first).
///
/// Supports space-separated tokens: all tokens must match.
pub fn fuzzy_filter<'a, T, F>(items: &'a [T], query: &str, get_text: F) -> Vec<&'a T>
where
    F: Fn(&T) -> &str,
{
    fuzzy_filter_scored(items, query, get_text, None)
}

/// Filter and sort items by fuzzy match quality, returning only the best
/// `limit` matches. This preserves the same ordering as [`fuzzy_filter`] for
/// the returned prefix while avoiding result storage that grows with every
/// matching candidate.
pub fn fuzzy_filter_limited<'a, T, F>(
    items: &'a [T],
    query: &str,
    limit: usize,
    get_text: F,
) -> Vec<&'a T>
where
    F: Fn(&T) -> &str,
{
    fuzzy_filter_scored(items, query, get_text, Some(limit))
}

fn fuzzy_filter_scored<'a, T, F>(
    items: &'a [T],
    query: &str,
    get_text: F,
    limit: Option<usize>,
) -> Vec<&'a T>
where
    F: Fn(&T) -> &str,
{
    if limit == Some(0) {
        return Vec::new();
    }

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return match limit {
            Some(limit) => items.iter().take(limit).collect(),
            None => items.iter().collect(),
        };
    }

    let tokens: Vec<Vec<char>> = trimmed.split_whitespace().map(lowercase_chars).collect();
    if tokens.is_empty() {
        return match limit {
            Some(limit) => items.iter().take(limit).collect(),
            None => items.iter().collect(),
        };
    }

    let mut results: Vec<(&T, f64, usize)> = Vec::with_capacity(limit.unwrap_or(0));

    for (idx, item) in items.iter().enumerate() {
        let text = get_text(item);
        let mut total_score = 0.0;
        let mut all_match = true;

        for token in &tokens {
            let m = fuzzy_match_lowered(token, text);
            if m.matches {
                total_score += m.score;
            } else {
                all_match = false;
                break;
            }
        }

        if !all_match {
            continue;
        }

        match limit {
            Some(limit) => insert_limited_match(&mut results, (item, total_score, idx), limit),
            None => results.push((item, total_score, idx)),
        }
    }

    if limit.is_none() {
        results.sort_by(match_order);
    }
    results.into_iter().map(|(item, _, _)| item).collect()
}

fn insert_limited_match<'a, T>(
    results: &mut Vec<(&'a T, f64, usize)>,
    item: (&'a T, f64, usize),
    limit: usize,
) {
    let insert_at = results.partition_point(|existing| match_order(existing, &item).is_lt());
    if insert_at < limit {
        results.insert(insert_at, item);
        if results.len() > limit {
            results.pop();
        }
    }
}

fn match_order<T>(a: &(&T, f64, usize), b: &(&T, f64, usize)) -> std::cmp::Ordering {
    a.1.partial_cmp(&b.1)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.2.cmp(&b.2))
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::*;

    struct CountingAllocator;

    #[cfg(not(coverage))]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct AllocationMetrics {
        count: usize,
        bytes: usize,
    }

    thread_local! {
        static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
        static ALLOCATION_COUNT: Cell<usize> = const { Cell::new(0) };
        static ALLOCATION_BYTES: Cell<usize> = const { Cell::new(0) };
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if TRACK_ALLOCATIONS.with(Cell::get) {
                ALLOCATION_COUNT.with(|count| count.set(count.get() + 1));
                ALLOCATION_BYTES.with(|bytes| bytes.set(bytes.get() + layout.size()));
            }
            // SAFETY: Delegates directly to the system allocator with the caller's layout.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: Delegates directly to the system allocator with the caller's pointer/layout.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[test]
    fn exact_match() {
        let m = fuzzy_match("model", "model");
        assert!(m.matches);
    }

    #[test]
    fn prefix_match() {
        let m = fuzzy_match("mod", "model");
        assert!(m.matches);
    }

    #[test]
    fn substring_match() {
        let m = fuzzy_match("del", "model");
        assert!(m.matches);
    }

    #[test]
    fn no_match() {
        let m = fuzzy_match("xyz", "model");
        assert!(!m.matches);
    }

    #[test]
    fn case_insensitive() {
        let m = fuzzy_match("MODEL", "model");
        assert!(m.matches);
    }

    #[test]
    fn empty_query_matches() {
        let m = fuzzy_match("", "anything");
        assert!(m.matches);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn query_longer_than_text_fails() {
        let m = fuzzy_match("longquery", "short");
        assert!(!m.matches);
    }

    #[test]
    fn word_boundary_scores_better() {
        let exact = fuzzy_match("cl", "claude");
        let mid = fuzzy_match("cl", "include");
        assert!(exact.matches);
        assert!(mid.matches);
        // Word boundary (start of word) should score better (lower).
        assert!(
            exact.score < mid.score,
            "boundary match should score better: {} vs {}",
            exact.score,
            mid.score
        );
    }

    #[test]
    fn filter_returns_all_on_empty_query() {
        let items = vec!["a", "b", "c"];
        let results = fuzzy_filter(&items, "", |s| s);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn filter_by_prefix() {
        let items = vec!["settings", "model", "clear", "quit"];
        let results = fuzzy_filter(&items, "set", |s| s);
        assert_eq!(results.len(), 1);
        assert_eq!(*results[0], "settings");
    }

    #[test]
    fn filter_multi_token() {
        let items = vec!["claude-sonnet-4", "gpt-4o", "claude-opus-4"];
        let results = fuzzy_filter(&items, "claude opus", |s| s);
        assert_eq!(results.len(), 1);
        assert_eq!(*results[0], "claude-opus-4");
    }

    #[test]
    fn filter_sorts_by_score() {
        let items = vec!["abcmodel", "model", "modeler"];
        let results = fuzzy_filter(&items, "model", |s| s);
        assert_eq!(results, vec![&"model", &"modeler", &"abcmodel"]);
    }

    #[test]
    fn match_handles_case_folding_that_expands_to_multiple_chars() {
        let m = fuzzy_match("i̇st", "İstanbul.rs");
        assert!(m.matches);
    }

    #[test]
    fn filter_handles_cjk_candidates() {
        let items = vec!["src/東京.rs", "src/京都.rs"];
        let results = fuzzy_filter(&items, "東京", |s| s);
        assert_eq!(results, vec![&"src/東京.rs"]);
    }

    #[cfg(not(coverage))]
    #[test]
    fn limited_filter_allocations_stay_constant_with_candidate_count() {
        for query in ["file", "missing", "file module"] {
            let small = workspace_files(100);
            let capped = workspace_files(5_000);

            let small_allocations = allocations_during(|| {
                let _ = fuzzy_filter_limited(&small, query, 32, |s| s.as_str());
            });
            let capped_allocations = allocations_during(|| {
                let _ = fuzzy_filter_limited(&capped, query, 32, |s| s.as_str());
            });

            assert_eq!(
                capped_allocations, small_allocations,
                "limited filtering should not add allocations as candidates grow for {query:?}: small={small_allocations:?}, capped={capped_allocations:?}"
            );
            assert!(
                capped_allocations.count <= 5,
                "limited filtering should only allocate fixed query/result bookkeeping for {query:?}: capped={capped_allocations:?}"
            );
            assert!(
                capped_allocations.bytes <= 1_024,
                "limited filtering should keep fixed bookkeeping bytes bounded for {query:?}: capped={capped_allocations:?}"
            );
        }
    }

    #[test]
    fn limited_filter_pins_order_for_boundaries_and_case() {
        let items = vec![
            "abcmodel.rs",
            "src/Model.rs",
            "src/foo-model.rs",
            "docs/model.rs",
            "x/MODELER.rs",
        ];
        let limited = fuzzy_filter_limited(&items, "MODEL", 4, |s| s);
        assert_eq!(
            limited,
            vec![
                &"x/MODELER.rs",
                &"src/Model.rs",
                &"docs/model.rs",
                &"src/foo-model.rs",
            ]
        );
    }

    #[test]
    fn limited_filter_pins_order_with_expanding_case_fold() {
        let items = vec![
            "src/xİstanbul.rs",
            "src/İstanbul.rs",
            "docs/istanbul.rs",
            "src/foo-İstanbul.rs",
        ];
        let limited = fuzzy_filter_limited(&items, "i̇st", 3, |s| s);
        assert_eq!(
            limited,
            vec![
                &"src/İstanbul.rs",
                &"src/foo-İstanbul.rs",
                &"src/xİstanbul.rs",
            ]
        );
    }

    #[test]
    fn limited_filter_pins_order_with_multiple_cjk_matches() {
        let items = vec!["src/x東京.rs", "src/東京.rs", "docs/東京_notes.md"];
        let limited = fuzzy_filter_limited(&items, "東京", 2, |s| s);
        assert_eq!(limited, vec![&"src/東京.rs", &"docs/東京_notes.md"]);
    }

    #[cfg(not(coverage))]
    fn workspace_files(file_count: usize) -> Vec<String> {
        (0..file_count)
            .map(|i| format!("src/module_{i:04}/file_{i:04}.rs"))
            .collect()
    }

    #[cfg(not(coverage))]
    fn allocations_during(work: impl FnOnce()) -> AllocationMetrics {
        ALLOCATION_COUNT.with(|count| count.set(0));
        ALLOCATION_BYTES.with(|bytes| bytes.set(0));
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(true));
        work();
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
        AllocationMetrics {
            count: ALLOCATION_COUNT.with(Cell::get),
            bytes: ALLOCATION_BYTES.with(Cell::get),
        }
    }
}
