//! The order of judged search hits (#2136): most relevant first, hits that
//! could not be judged after them, ties and the unjudged kept in their
//! original order.

/// Positions of `scores` in display order. A score is a probability.
pub fn relevance_order(scores: &[Option<f64>]) -> Vec<usize> {
    debug_assert!(
        scores
            .iter()
            .flatten()
            .all(|score| (0.0..=1.0).contains(score)),
        "relevance scores are probabilities: {scores:?}"
    );
    let mut order: Vec<usize> = (0..scores.len()).collect();
    // Stable: equal keys keep their original order.
    order.sort_by(|&a, &b| match (scores[a], scores[b]) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    order
}

#[cfg(test)]
#[path = "search_ranking_tests.rs"]
mod tests;
