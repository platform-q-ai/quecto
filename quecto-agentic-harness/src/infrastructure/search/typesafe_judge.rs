//! [`RelevanceJudge`] over TypeSafe's System One API (Jev): one Noul
//! question per hit ("is this what the searcher wants?"), asked with the
//! hit alone so no judgement sees another (TypeSafe's reranking pattern).
//! Requests run concurrently and the whole judging is time-bounded; a hit
//! whose request fails is left unscored, and when none can be scored the
//! answer is `Unavailable` with the reason.

use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::search::ports::{
    PortFuture, Relevance, RelevanceCandidate, RelevanceJudge,
};

/// TypeSafe's System One endpoint.
pub const TYPESAFE_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// Concurrent requests per search.
const CONCURRENCY: usize = 8;

/// The TypeSafe key: `TYPESAFE_API_KEY`, else `~/.config/typesafe/api_key`.
/// `None` when neither holds one. Never logged.
pub fn typesafe_key() -> Option<String> {
    let usable = |key: String| {
        let key = key.trim().to_string();
        key.chars().next().is_some().then_some(key)
    };
    // An empty variable does not hide the file.
    let from_env = std::env::var("TYPESAFE_API_KEY").ok().and_then(usable);
    from_env.or_else(|| {
        let path = dirs::home_dir()?.join(".config/typesafe/api_key");
        std::fs::read_to_string(path).ok().and_then(usable)
    })
}

pub struct TypeSafeJudge {
    client: reqwest::Client,
    endpoint: String,
    key: String,
    model: String,
    timeout: Duration,
}

impl TypeSafeJudge {
    /// A judge over `endpoint`; `Err` when the HTTP client cannot be built
    /// (its timeout is part of what bounds a search).
    pub fn new(
        endpoint: &str,
        key: String,
        model: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("the TypeSafe HTTP client could not be built: {e}"))?;
        Ok(Self {
            client,
            endpoint: endpoint.to_string(),
            key,
            model: model.to_string(),
            timeout,
        })
    }

    /// The question asked of each hit.
    fn request(&self, query: &str, candidate: &RelevanceCandidate) -> Value {
        json!({
            "model": self.model,
            "state": {
                "query": query,
                "candidate": {"location": candidate.location, "text": candidate.text},
            },
            "questions": {
                "relevant": {
                    "type": "noul",
                    "instructions": "Is `candidate` what the searcher is looking for? `query` says, in their words, what they want to find in a codebase. `candidate.text` is one search hit: the matching line with a little surrounding context, found at `candidate.location`.",
                    "criteria": {
                        "true": "This hit is, or defines, implements, configures or documents, what `query` describes",
                        "false": "It only shares words with `query`, or is about something else"
                    }
                }
            }
        })
    }

    async fn score(&self, query: &str, candidate: &RelevanceCandidate) -> Result<f64, String> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.key)
            .json(&self.request(query, candidate))
            .send()
            .await
            .map_err(|e| format!("TypeSafe could not be reached: {e}"))?;
        let status = response.status();
        match status.as_u16() {
            200..=299 => {}
            401 => return Err("TypeSafe rejected the API key (401)".to_string()),
            code => return Err(format!("TypeSafe answered HTTP {code}")),
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("TypeSafe's answer was not JSON: {e}"))?;
        match body["answers"]["relevant"]["noul"].as_f64() {
            Some(noul) if (0.0..=1.0).contains(&noul) => Ok(noul),
            _ => Err("TypeSafe answered without a relevance score".to_string()),
        }
    }
}

impl RelevanceJudge for TypeSafeJudge {
    fn judge<'a>(
        &'a self,
        query: &'a str,
        candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        Box::pin(async move {
            let requests: Vec<_> = candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let scored = self.score(query, candidate);
                    async move { (index, scored.await) }
                })
                .collect();
            // Scores are kept as they arrive; at the deadline the rest
            // stay unscored rather than discarding what was judged.
            let mut pending = futures::stream::iter(requests).buffer_unordered(CONCURRENCY);
            let deadline = tokio::time::Instant::now() + self.timeout;
            let mut judged = Vec::with_capacity(candidates.len());
            let mut timed_out = false;
            loop {
                match tokio::time::timeout_at(deadline, pending.next()).await {
                    Ok(Some(outcome)) => judged.push(outcome),
                    Ok(None) => break,
                    Err(_) => {
                        timed_out = true;
                        break;
                    }
                }
            }
            let mut scores = vec![None; candidates.len()];
            let mut first_error = None;
            for (index, outcome) in judged {
                match outcome {
                    Ok(score) => scores[index] = Some(score),
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            let timeout = || {
                format!(
                    "TypeSafe did not answer within {} s",
                    self.timeout.as_secs()
                )
            };
            match (scores.iter().any(Option::is_some), first_error, timed_out) {
                (true, _, _) | (false, None, false) => Relevance::Scored(scores),
                (false, Some(error), false) => Relevance::Unavailable(error),
                (false, _, true) => Relevance::Unavailable(timeout()),
            }
        })
    }
}

#[cfg(test)]
#[path = "typesafe_judge_tests.rs"]
mod tests;
