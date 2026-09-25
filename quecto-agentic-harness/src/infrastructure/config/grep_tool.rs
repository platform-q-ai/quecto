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
    /// Judging stops after this long; the search then returns rg's order.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// The judged matches per search a config may ask for.
pub const MAX_CANDIDATES: std::ops::RangeInclusive<usize> = 1..=100;
/// The judging time a config may allow, in seconds.
pub const TIMEOUT_SECS: std::ops::RangeInclusive<u64> = 1..=60;

impl GrepRelevanceConfig {
    /// The candidate cap and judging timeout, when both are in range; a
    /// setting outside its range is refused, never clamped.
    pub fn limits(&self) -> Result<(usize, std::time::Duration), String> {
        match (
            MAX_CANDIDATES.contains(&self.max_candidates),
            TIMEOUT_SECS.contains(&self.timeout_secs),
        ) {
            (true, true) => Ok((
                self.max_candidates,
                std::time::Duration::from_secs(self.timeout_secs),
            )),
            (false, _) => Err(format!(
                "tools.grep.relevance.max_candidates must be {}..={} (got {})",
                MAX_CANDIDATES.start(),
                MAX_CANDIDATES.end(),
                self.max_candidates
            )),
            (true, false) => Err(format!(
                "tools.grep.relevance.timeout_secs must be {}..={} (got {})",
                TIMEOUT_SECS.start(),
                TIMEOUT_SECS.end(),
                self.timeout_secs
            )),
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
    30
}

fn default_timeout_secs() -> u64 {
    10
}

fn default_log_enabled() -> bool {
    true
}

#[cfg(test)]
#[path = "grep_tool_tests.rs"]
mod tests;
