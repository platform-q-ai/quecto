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
#[path = "fuzzy_tests.rs"]
mod tests;
