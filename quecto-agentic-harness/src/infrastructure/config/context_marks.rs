//! #2403/#2414: the watermark marks — `agents.defaults.context_high_tokens`
//! and `context_low_tokens`, their environment overrides and their
//! load-time validation (split from `config.rs`, line cap). The watermark
//! pass is the only context mode, so the marks are plain settings.

use super::{AgentDefaults, ConfigError};
use crate::domain::conversation::watermark::Watermark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The marks' keys, flattened into `agents.defaults`. Unset, a mark is the
/// owner's (`Watermark::default()`: 256k, down to 70k).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextMarksConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_high_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_low_tokens: Option<usize>,
    /// An environment value that is no count, refused at validation.
    #[serde(skip)]
    invalid_env: Option<String>,
}

impl ContextMarksConfig {
    /// The marks: the configured ones, or the owner's.
    fn marks(&self) -> Result<Watermark, ConfigError> {
        let owners = Watermark::default();
        let high = self.context_high_tokens.unwrap_or(owners.high());
        let low = self.context_low_tokens.unwrap_or(owners.low());
        match (high, low) {
            (0, _) => Err(refused("context_high_tokens must be above 0")),
            (_, 0) => Err(refused("context_low_tokens must be above 0")),
            (high, low) => Watermark::new(high, low).map_err(|error| {
                refused(&format!(
                    "context_high_tokens {high} and context_low_tokens {low}: {error}"
                ))
            }),
        }
    }
}

fn refused(reason: &str) -> ConfigError {
    ConfigError::ContextBudget(reason.to_string())
}

impl AgentDefaults {
    /// The watermark marks the configuration sets. Validated at load, so
    /// marks it refuses (only in a configuration built in code without
    /// loading) fall back to the owner's.
    pub fn context_marks(&self) -> Watermark {
        let marks = self.context_marks.marks();
        debug_assert!(marks.is_ok(), "the marks were validated at load: {marks:?}");
        marks.unwrap_or_default()
    }
}

/// `QUECTO_CONTEXT_HIGH_TOKENS` and `QUECTO_CONTEXT_LOW_TOKENS`. A mark
/// that is no count is kept as refused, so the load fails naming it.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    let config = &mut defaults.context_marks;
    for (key, mark) in [
        (
            "QUECTO_CONTEXT_HIGH_TOKENS",
            &mut config.context_high_tokens,
        ),
        ("QUECTO_CONTEXT_LOW_TOKENS", &mut config.context_low_tokens),
    ] {
        match env.get(key).map(|value| (value, value.parse::<usize>())) {
            Some((_, Ok(count))) => *mark = Some(count),
            Some((value, Err(_))) => {
                config.invalid_env.get_or_insert_with(|| {
                    format!("{key} must be a whole number of tokens, not {value:?}")
                });
            }
            None => {}
        }
    }
}

/// A mark that is no positive count, and marks the watermark refuses, are
/// refused.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let config = &defaults.context_marks;
    match &config.invalid_env {
        Some(reason) => Err(refused(reason)),
        None => config.marks().map(|_| ()),
    }
}

#[cfg(test)]
#[path = "context_marks_tests.rs"]
mod tests;
