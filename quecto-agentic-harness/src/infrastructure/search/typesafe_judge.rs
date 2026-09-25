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
/// Retries of a rate-limited (429) or overloaded (529) request.
const RATE_LIMIT_RETRIES: u32 = 2;
/// The first wait before such a retry, doubling, unless `Retry-After` says.
const RATE_LIMIT_BACKOFF: Duration = Duration::from_millis(250);
/// The longest wait before such a retry.
const RATE_LIMIT_MAX_WAIT: Duration = Duration::from_secs(2);

/// Why one hit could not be judged.
enum Unjudged {
    /// No request can succeed (a rejected key): stop judging.
    Stop(String),
    /// This hit could not be judged; others may.
    Skip(String),
}

/// A `Retry-After` given in seconds.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Requests in flight per search unless configured.
const DEFAULT_CONCURRENCY: usize = 8;

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
    concurrency: usize,
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
        // The client's own bound sits past the judging deadline, so the
        // deadline decides (keeping the scores already judged).
        let client = reqwest::Client::builder()
            .timeout(timeout + Duration::from_secs(1))
            .build()
            .map_err(|e| format!("the TypeSafe HTTP client could not be built: {e}"))?;
        Ok(Self {
            client,
            endpoint: endpoint.to_string(),
            key,
            model: model.to_string(),
            timeout,
            concurrency: DEFAULT_CONCURRENCY,
        })
    }

    /// Requests in flight at most.
    #[cfg(test)]
    pub(crate) fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// At most `concurrency` requests in flight (at least one).
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        assert!(concurrency >= 1, "at least one request in flight");
        self.concurrency = concurrency;
        self
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

    async fn score(&self, query: &str, candidate: &RelevanceCandidate) -> Result<f64, Unjudged> {
        let mut retries = 0;
        loop {
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.key)
                .json(&self.request(query, candidate))
                .send()
                .await
                .map_err(|e| Unjudged::Skip(format!("TypeSafe could not be reached: {e}")))?;
            let code = response.status().as_u16();
            match code {
                200..=299 => {
                    let body: Value = response.json().await.map_err(|e| {
                        Unjudged::Skip(format!("TypeSafe's answer was not JSON: {e}"))
                    })?;
                    return match body["answers"]["relevant"]["noul"].as_f64() {
                        Some(noul) if (0.0..=1.0).contains(&noul) => Ok(noul),
                        _ => Err(Unjudged::Skip(
                            "TypeSafe answered without a relevance score".to_string(),
                        )),
                    };
                }
                // No request with this key can succeed: stop asking.
                401 => {
                    return Err(Unjudged::Stop(
                        "TypeSafe rejected the API key (401)".to_string(),
                    ));
                }
                403 => {
                    return Err(Unjudged::Stop(
                        "TypeSafe refused the request (403)".to_string(),
                    ));
                }
                // Rate-limited or overloaded: a short, bounded retry.
                429 | 529 if retries < RATE_LIMIT_RETRIES => {
                    let wait =
                        retry_after(&response).unwrap_or(RATE_LIMIT_BACKOFF * 2u32.pow(retries));
                    // Asked to wait longer than a search can: give up this
                    // hit rather than retry early.
                    if wait > RATE_LIMIT_MAX_WAIT {
                        return Err(Unjudged::Skip(format!(
                            "TypeSafe asked to retry after {} s (HTTP {code})",
                            wait.as_secs()
                        )));
                    }
                    tokio::time::sleep(wait).await;
                    retries += 1;
                }
                429 | 529 => {
                    return Err(Unjudged::Skip(format!(
                        "TypeSafe is rate-limiting or overloaded (HTTP {code})"
                    )));
                }
                code => return Err(Unjudged::Skip(format!("TypeSafe answered HTTP {code}"))),
            }
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
            let mut pending = futures::stream::iter(requests).buffer_unordered(self.concurrency);
            let deadline = tokio::time::Instant::now() + self.timeout;
            let mut judged = Vec::with_capacity(candidates.len());
            let mut timed_out = false;
            loop {
                match tokio::time::timeout_at(deadline, pending.next()).await {
                    // A rejected key: stop, and dropping the stream cancels
                    // the requests still in flight.
                    Ok(Some((index, Err(Unjudged::Stop(error))))) => {
                        judged.push((index, Err(Unjudged::Stop(error))));
                        break;
                    }
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
                    // A rejected key outranks other errors as the reason.
                    Err(Unjudged::Stop(error)) => first_error = Some(error),
                    Err(Unjudged::Skip(error)) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            // Why some were not judged: the deadline, the first error, or
            // both (a timeout never hides a rejected key).
            let deadline = timed_out
                .then(|| format!("TypeSafe did not answer within {}", human(self.timeout)));
            let reason = match (deadline, first_error) {
                (Some(deadline), Some(error)) => Some(format!("{deadline}; first error: {error}")),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            };
            match (scores.iter().any(Option::is_some), reason) {
                (_, None) => Relevance::Scored(scores),
                (true, Some(reason)) => Relevance::Partial(scores, reason),
                (false, Some(reason)) => Relevance::Unavailable(reason),
            }
        })
    }
}

/// `10 s`, or `700 ms` below a second.
fn human(duration: Duration) -> String {
    if duration.as_secs() >= 1 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

#[cfg(test)]
#[path = "typesafe_judge_tests.rs"]
mod tests;
