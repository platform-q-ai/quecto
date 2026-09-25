//! Search capability ports (#2136 slice B). A [`RelevanceJudge`] scores
//! search hits against what the agent is looking for; a [`SearchLog`]
//! records every search, so search behaviour can be optimised from logs.
//! Both are optional: a search never fails because either is missing or
//! down.

use std::future::Future;
use std::pin::Pin;

/// The capability's port future.
pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One search hit offered for judging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelevanceCandidate {
    /// Where it is, as the tool shows it (`path:line`).
    pub location: String,
    /// The matching line(s) with a little surrounding context.
    pub text: String,
}

/// A judge's answer for a set of candidates.
#[derive(Debug, Clone, PartialEq)]
pub enum Relevance {
    /// One score per candidate, in order: the probability (0–1) that it is
    /// what the query describes; `None` for a candidate that could not be
    /// judged.
    Scored(Vec<Option<f64>>),
    /// Like `Scored`, but some candidates could not be judged, and why (a
    /// deadline, or the first error).
    Partial(Vec<Option<f64>>, String),
    /// No candidate could be judged, and why (for the agent and the log).
    Unavailable(String),
}

/// Scores search hits against a natural-language description of what the
/// searcher wants.
pub trait RelevanceJudge: Send + Sync {
    fn judge<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance>;
}

/// How ranking went for one search.
#[derive(Debug, Clone, PartialEq)]
pub enum RankingRecord {
    /// Hits were scored; the scores as judged, by location.
    Ranked {
        query: String,
        candidates: usize,
        scores: Vec<ScoredLocation>,
        /// Why some were not judged, when some were not.
        unjudged_reason: Option<String>,
        elapsed_ms: u64,
    },
    /// Ranking was asked for but could not be done.
    Unavailable {
        query: String,
        reason: String,
        elapsed_ms: u64,
    },
    /// Ranking was asked for but is not configured.
    NotConfigured { query: String },
}

/// One judged hit.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredLocation {
    pub location: String,
    pub score: Option<f64>,
}

/// One search, as recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRecord {
    /// The tool's arguments as the model sent them (JSON text, or whatever
    /// it sent when that was not JSON), cut at [`MAX_RECORDED_ARGUMENTS`].
    pub arguments: String,
    /// `content`, `files`, `count`, or `refused` for arguments the tool
    /// could not honour.
    pub output: String,
    /// Matches (content) or files (files/count) found before the limit.
    pub found: usize,
    /// The result is known to be incomplete (the read cap, an rg error, a
    /// signal, a pipe held open).
    pub incomplete: bool,
    /// The error returned instead of results, if any.
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub ranking: Option<RankingRecord>,
}

/// The most characters of a search's arguments a record keeps.
pub const MAX_RECORDED_ARGUMENTS: usize = 2000;

/// Records searches. Best effort: recording never fails a search.
pub trait SearchLog: Send + Sync {
    fn record(&self, record: &SearchRecord);
}
