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
use crate::domain::search_ranking::relevance_order;
use crate::infrastructure::security::sandbox::Sandbox;

use super::{RgMatch, read_file_for_cache};

/// Context lines either side of a match in the text a judge sees.
const CANDIDATE_CONTEXT: usize = 1;
/// The most matched lines of one match a judge sees.
const CANDIDATE_MATCH_LINES: usize = 10;
/// The most characters of any one line a judge sees: each line is cut on
/// its own, so a long neighbour never pushes the match out.
const CANDIDATE_LINE_CHARS: usize = 400;
/// Characters kept before a match when a long line is cut around it.
const CANDIDATE_LEAD_CHARS: usize = 100;

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
    sandbox: &Sandbox,
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
    debug_assert!(
        ranking.max_candidates >= 1,
        "ranking judges at least one match"
    );
    let judged = matches.len().min(ranking.max_candidates);
    let locations: Vec<String> = matches[..judged]
        .iter()
        .map(|m| location(m, workspace))
        .collect();
    // What the judge may see of each: `None` for a path the sandbox
    // refuses or a match whose line could not be read (not judged).
    let texts = candidate_texts(&matches[..judged], sandbox).await;
    let sendable: Vec<(usize, RelevanceCandidate)> = texts
        .into_iter()
        .enumerate()
        .filter_map(|(index, text)| {
            text.map(|text| {
                (
                    index,
                    RelevanceCandidate {
                        location: locations[index].clone(),
                        text,
                    },
                )
            })
        })
        .collect();
    let candidates: Vec<RelevanceCandidate> = sendable.iter().map(|(_, c)| c.clone()).collect();
    let started = Instant::now();
    let relevance = match candidates.as_slice() {
        [] => Relevance::Scored(Vec::new()),
        candidates => ranking.judge.judge(query, candidates).await,
    };
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (answered, unjudged_reason) = match relevance {
        Relevance::Scored(answered) => (answered, None),
        Relevance::Partial(answered, reason) => (answered, Some(reason)),
        Relevance::Unavailable(reason) => return unavailable(matches, query, reason, elapsed_ms),
    };
    if answered.len() == sendable.len() {
        let mut scores = vec![None; judged];
        for ((index, _), score) in sendable.iter().zip(answered) {
            scores[*index] = score;
        }
        ranked(
            matches,
            &scores,
            &locations,
            query,
            unjudged_reason,
            elapsed_ms,
        )
    } else {
        unavailable(
            matches,
            query,
            format!(
                "the judge answered for {} of {} matches",
                answered.len(),
                sendable.len()
            ),
            elapsed_ms,
        )
    }
}

/// The judged matches in relevance order, then the rest in rg order.
fn ranked(
    matches: Vec<RgMatch>,
    scores: &[Option<f64>],
    locations: &[String],
    query: &str,
    unjudged_reason: Option<String>,
    elapsed_ms: u64,
) -> Ranked {
    let judged = scores.len();
    let total = matches.len();
    let mut matches: Vec<Option<RgMatch>> = matches.into_iter().map(Some).collect();
    let order = relevance_order(scores);
    let mut ordered = Vec::with_capacity(total);
    let mut record = Vec::with_capacity(judged);
    for index in order {
        let mut m = matches[index]
            .take()
            .expect("each position is ordered once");
        m.score = scores[index];
        record.push(ScoredLocation {
            location: locations[index].clone(),
            score: scores[index],
        });
        ordered.push(m);
    }
    ordered.extend(matches.into_iter().flatten());
    debug_assert_eq!(
        ordered.len(),
        total,
        "ranking neither drops nor adds matches"
    );
    let unscored = scores.iter().filter(|score| score.is_none()).count();
    let mut notes = Vec::new();
    if unscored > 0 {
        let why = unjudged_reason
            .as_deref()
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        notes.push(format!(
            "{unscored} of {judged} matches could not be judged{why} and follow the ranked ones"
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
            scores: record,
            unjudged_reason,
            elapsed_ms,
        },
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

/// What the judge sees of each match: its matched line(s) (at most
/// [`CANDIDATE_MATCH_LINES`]) with a line of context either side, each line
/// cut on its own. `None` when the sandbox refuses the path or the matched
/// line could not be read (past the file cache, or unreadable).
async fn candidate_texts(matches: &[RgMatch], sandbox: &Sandbox) -> Vec<Option<String>> {
    let mut files: std::collections::HashMap<PathBuf, Option<Vec<String>>> = Default::default();
    for m in matches {
        if let std::collections::hash_map::Entry::Vacant(slot) = files.entry(m.file_path.clone()) {
            let lines = match sandbox.validate_path(&m.file_path.to_string_lossy()) {
                Ok(_) => {
                    let path = m.file_path.clone();
                    Some(
                        tokio::task::spawn_blocking(move || read_file_for_cache(&path))
                            .await
                            .unwrap_or_default(),
                    )
                }
                Err(_) => None,
            };
            slot.insert(lines);
        }
    }
    matches
        .iter()
        .map(|m| {
            let lines = files.get(&m.file_path)?.as_deref()?;
            lines.get(m.line_number.checked_sub(1)?)?;
            let matched_last = m.line_number + m.line_count.clamp(1, CANDIDATE_MATCH_LINES) - 1;
            let first = m.line_number.saturating_sub(CANDIDATE_CONTEXT).max(1);
            let last = matched_last + CANDIDATE_CONTEXT;
            let text = (first..=last)
                .filter_map(|n| {
                    let line = lines.get(n - 1)?;
                    Some(match (n == m.line_number, m.column) {
                        // The match's own line: a window around the match.
                        (true, Some(column)) => around(line, column),
                        (true, None) | (false, _) => {
                            line.chars().take(CANDIDATE_LINE_CHARS).collect()
                        }
                    })
                })
                .collect::<Vec<String>>()
                .join("\n");
            Some(text)
        })
        .collect()
}

/// At most [`CANDIDATE_LINE_CHARS`] of `line` around the match starting at
/// byte `column` (PR #2138 review: a match past the cut must still be what
/// the judge sees), with `…` marking text cut before it.
fn around(line: &str, column: usize) -> String {
    if line.chars().count() <= CANDIDATE_LINE_CHARS {
        return line.to_string();
    }
    let mut at = column.min(line.len());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    let begin = line[..at]
        .chars()
        .count()
        .saturating_sub(CANDIDATE_LEAD_CHARS);
    let window: String = line
        .chars()
        .skip(begin)
        .take(CANDIDATE_LINE_CHARS)
        .collect();
    match begin {
        0 => window,
        _ => format!("…{window}"),
    }
}

/// A match's location as the tool shows it: `path:line`, relative to the
/// workspace, without `./`.
fn location(m: &RgMatch, workspace: &Path) -> String {
    let relative = m.file_path.strip_prefix(workspace).unwrap_or(&m.file_path);
    let relative = relative.strip_prefix(".").unwrap_or(relative);
    format!("{}:{}", relative.to_string_lossy(), m.line_number)
}

#[cfg(test)]
#[path = "grep_rank_tests.rs"]
mod tests;
