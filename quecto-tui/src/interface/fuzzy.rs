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
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();

    if query_lower.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }

    if query_lower.len() > text_lower.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    let mut query_idx = 0;
    let mut score: f64 = 0.0;
    let mut last_match_idx: Option<usize> = None;
    let mut consecutive = 0;

    for (i, &ch) in text_lower.iter().enumerate() {
        if query_idx < query_lower.len() && ch == query_lower[query_idx] {
            // Word boundary bonus.
            let is_boundary =
                i == 0 || (i > 0 && matches!(text_lower[i - 1], ' ' | '-' | '_' | '.' | '/' | ':'));
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

/// Filter and sort items by fuzzy match quality (best first).
///
/// Supports space-separated tokens: all tokens must match.
pub fn fuzzy_filter<'a, T, F>(items: &'a [T], query: &str, get_text: F) -> Vec<&'a T>
where
    F: Fn(&T) -> &str,
{
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return items.iter().collect();
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return items.iter().collect();
    }

    let mut results: Vec<(&T, f64)> = Vec::new();

    for item in items {
        let text = get_text(item);
        let mut total_score = 0.0;
        let mut all_match = true;

        for token in &tokens {
            let m = fuzzy_match(token, text);
            if m.matches {
                total_score += m.score;
            } else {
                all_match = false;
                break;
            }
        }

        if all_match {
            results.push((item, total_score));
        }
    }

    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(results.len() >= 2);
        // "model" (exact) should come before "abcmodel" (later position).
        assert_eq!(*results[0], "model");
    }
}
