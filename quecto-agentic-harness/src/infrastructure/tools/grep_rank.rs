//! `rank_by` for the grep tool (#2136 slice B): the first matches (up to
//! the configured number) are judged for relevance to what the agent is
//! looking for and returned best first, each with its score; the rest
//! follow in rg's order. Ranking never fails a search: when it is not
//! configured or the judge cannot answer, the matches keep rg's order and
//! a one-line notice says why.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::application::search::ports::{
    RankingRecord, Relevance, RelevanceCandidate, RelevanceJudge, ScoredLocation,
};

use super::{RgMatch, read_file_for_cache};

/// Context lines either side of a match in the text a judge sees.
const CANDIDATE_CONTEXT: usize = 1;
/// The most characters of one candidate's text a judge sees.
const CANDIDATE_CHARS: usize = 1200;

/// Ranking as configured: the judge and how many matches it sees.
pub(super) struct Ranking {
    pub judge: Arc<dyn RelevanceJudge>,
    pub max_candidates: usize,
}

/// Matches in the order to show them, what to tell the agent, and what to
/// record.
pub(super) struct Ranked {
    pub matches: Vec<RgMatch>,
    pub notice: Option<String>,
    pub record: RankingRecord,
}

pub(super) const NOT_CONFIGURED: &str = "rank_by is not configured here (tools.grep.relevance and a TypeSafe key); results are in rg order";

pub(super) async fn rank(
    ranking: Option<&Ranking>,
    query: &str,
    matches: Vec<RgMatch>,
    workspace: &Path,
) -> Ranked {
    let Some(ranking) = ranking else {
        return Ranked {
            matches,
            notice: Some(NOT_CONFIGURED.to_string()),
            record: RankingRecord::NotConfigured {
                query: query.to_string(),
            },
        };
    };
    let judged = matches.len().min(ranking.max_candidates);
    let candidates = candidates(&matches[..judged], workspace).await;
    let started = Instant::now();
    let relevance = ranking.judge.judge(query, &candidates).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match relevance {
        Relevance::Scored(scores) if scores.len() == judged => {
            let total = matches.len();
            let (ordered, record_scores) = order(matches, &scores, &candidates);
            let unscored = scores.iter().filter(|score| score.is_none()).count();
            let mut notes = Vec::new();
            if unscored > 0 {
                notes.push(format!(
                    "{unscored} of {judged} matches could not be judged and follow the ranked ones"
                ));
            }
            if total > judged {
                notes.push(format!(
                    "ranked the first {judged} of {total} matches; the rest follow in rg order: narrow the search to rank them"
                ));
            }
            Ranked {
                matches: ordered,
                notice: match notes.as_slice() {
                    [] => None,
                    notes => Some(notes.join(". ")),
                },
                record: RankingRecord::Ranked {
                    query: query.to_string(),
                    candidates: judged,
                    scores: record_scores,
                    elapsed_ms,
                },
            }
        }
        Relevance::Scored(scores) => unavailable(
            matches,
            query,
            format!(
                "the judge answered for {} of {judged} matches",
                scores.len()
            ),
            elapsed_ms,
        ),
        Relevance::Unavailable(reason) => unavailable(matches, query, reason, elapsed_ms),
    }
}

fn unavailable(matches: Vec<RgMatch>, query: &str, reason: String, elapsed_ms: u64) -> Ranked {
    Ranked {
        matches,
        notice: Some(format!(
            "rank_by unavailable: {reason}; results are in rg order"
        )),
        record: RankingRecord::Unavailable {
            query: query.to_string(),
            reason,
            elapsed_ms,
        },
    }
}

/// The judged matches best first (unscored after, in rg order), then the
/// rest in rg order; and the judged scores by location, in that order.
fn order(
    matches: Vec<RgMatch>,
    scores: &[Option<f64>],
    candidates: &[RelevanceCandidate],
) -> (Vec<RgMatch>, Vec<ScoredLocation>) {
    let mut judged: Vec<(RgMatch, String)> = Vec::with_capacity(scores.len());
    let mut rest = Vec::new();
    for (index, mut m) in matches.into_iter().enumerate() {
        match (scores.get(index), candidates.get(index)) {
            (Some(score), Some(candidate)) => {
                m.score = *score;
                judged.push((m, candidate.location.clone()));
            }
            (Some(_), None) | (None, _) => rest.push(m),
        }
    }
    // Stable: equal scores (and all unscored) keep rg's order.
    judged.sort_by(|(a, _), (b, _)| {
        let key = |m: &RgMatch| m.score.unwrap_or(-1.0);
        key(b).total_cmp(&key(a))
    });
    let record = judged
        .iter()
        .map(|(m, location)| ScoredLocation {
            location: location.clone(),
            score: m.score,
        })
        .collect();
    let mut ordered: Vec<RgMatch> = judged.into_iter().map(|(m, _)| m).collect();
    ordered.extend(rest);
    (ordered, record)
}

/// What the judge sees of each match: its location as the tool shows it
/// and its line(s) with a little context, bounded.
async fn candidates(matches: &[RgMatch], workspace: &Path) -> Vec<RelevanceCandidate> {
    let mut files: std::collections::HashMap<PathBuf, Vec<String>> = Default::default();
    for m in matches {
        if let std::collections::hash_map::Entry::Vacant(slot) = files.entry(m.file_path.clone()) {
            let path = m.file_path.clone();
            let lines = tokio::task::spawn_blocking(move || read_file_for_cache(&path))
                .await
                .unwrap_or_default();
            slot.insert(lines);
        }
    }
    matches
        .iter()
        .map(|m| {
            let lines = files.get(&m.file_path).map(Vec::as_slice).unwrap_or(&[]);
            let first = m.line_number.saturating_sub(CANDIDATE_CONTEXT).max(1);
            let last = m.line_number + m.line_count.max(1) - 1 + CANDIDATE_CONTEXT;
            let text: String = (first..=last)
                .filter_map(|n| lines.get(n - 1))
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(CANDIDATE_CHARS)
                .collect();
            RelevanceCandidate {
                location: format!("{}:{}", shown_path(&m.file_path, workspace), m.line_number),
                text,
            }
        })
        .collect()
}

/// A path as the tool shows it: relative to the workspace, without `./`.
fn shown_path(path: &Path, workspace: &Path) -> String {
    let relative = path.strip_prefix(workspace).unwrap_or(path);
    let relative = relative.strip_prefix(".").unwrap_or(relative);
    relative.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "grep_rank_tests.rs"]
mod tests;
