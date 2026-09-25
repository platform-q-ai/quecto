//! The grep tool's settings (#2136 slice B): relevance ranking of hits
//! through TypeSafe's Jev (off unless enabled, and only with a TypeSafe
//! key), and the search log (on by default: every search is recorded
//! locally so search behaviour can be optimised from logs).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GrepToolConfig {
    #[serde(default)]
    pub relevance: GrepRelevanceConfig,
    #[serde(default)]
    pub log: SearchLogConfig,
}

/// `rank_by` relevance ranking. The key is `TYPESAFE_API_KEY`, else
/// `~/.config/typesafe/api_key`; it is never written to config or logs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrepRelevanceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_model")]
    pub model: String,
    /// At most this many hits are judged per search; the rest follow in rg
    /// order.
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
    /// Judging stops after this long; the matches judged by then are
    /// ranked, the rest follow in rg's order.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Requests to TypeSafe in flight at once.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

/// The judged matches per search a config may ask for: calls cost next to
/// nothing, so every match is judged up to a guard against runaway fan-out.
pub const MAX_CANDIDATES: std::ops::RangeInclusive<usize> = 1..=5000;
/// The judging time a config may allow, in seconds.
pub const TIMEOUT_SECS: std::ops::RangeInclusive<u64> = 1..=120;
/// The requests in flight a config may allow.
pub const CONCURRENCY: std::ops::RangeInclusive<usize> = 1..=64;

/// Ranking's bounds for one search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankingLimits {
    pub max_candidates: usize,
    pub timeout: std::time::Duration,
    pub concurrency: usize,
}

impl GrepRelevanceConfig {
    /// Ranking's bounds, when every setting is in range; a setting outside
    /// its range is refused, never clamped.
    pub fn limits(&self) -> Result<RankingLimits, String> {
        let refused = |key: &str, range: String, got: String| {
            Err(format!(
                "tools.grep.relevance.{key} must be {range} (got {got})"
            ))
        };
        let span = |start: String, end: String| format!("{start}..={end}");
        match (
            MAX_CANDIDATES.contains(&self.max_candidates),
            TIMEOUT_SECS.contains(&self.timeout_secs),
            CONCURRENCY.contains(&self.concurrency),
        ) {
            (true, true, true) => Ok(RankingLimits {
                max_candidates: self.max_candidates,
                timeout: std::time::Duration::from_secs(self.timeout_secs),
                concurrency: self.concurrency,
            }),
            (false, _, _) => refused(
                "max_candidates",
                span(
                    MAX_CANDIDATES.start().to_string(),
                    MAX_CANDIDATES.end().to_string(),
                ),
                self.max_candidates.to_string(),
            ),
            (true, false, _) => refused(
                "timeout_secs",
                span(
                    TIMEOUT_SECS.start().to_string(),
                    TIMEOUT_SECS.end().to_string(),
                ),
                self.timeout_secs.to_string(),
            ),
            (true, true, false) => refused(
                "concurrency",
                span(
                    CONCURRENCY.start().to_string(),
                    CONCURRENCY.end().to_string(),
                ),
                self.concurrency.to_string(),
            ),
        }
    }
}

impl Default for GrepRelevanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_model(),
            max_candidates: default_max_candidates(),
            timeout_secs: default_timeout_secs(),
            concurrency: default_concurrency(),
        }
    }
}

/// The local search log: `<base_dir>/search-log/<YYYY-MM-DD>.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchLogConfig {
    #[serde(default = "default_log_enabled")]
    pub enabled: bool,
}

impl Default for SearchLogConfig {
    fn default() -> Self {
        Self {
            enabled: default_log_enabled(),
        }
    }
}

fn default_model() -> String {
    "jev-latest".to_string()
}

fn default_max_candidates() -> usize {
    1000
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_concurrency() -> usize {
    32
}

fn default_log_enabled() -> bool {
    true
}

#[cfg(test)]
#[path = "grep_tool_tests.rs"]
mod tests;
