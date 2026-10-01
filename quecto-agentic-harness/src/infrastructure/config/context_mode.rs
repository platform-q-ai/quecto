//! #2403: the context mode and its marks — `agents.defaults.context_mode`,
//! `context_high_tokens` and `context_low_tokens`, their environment
//! overrides and their load-time validation (split from `config.rs`, line
//! cap).

use super::{AgentDefaults, ConfigError};
use crate::domain::conversation::ContextMode;
use crate::domain::conversation::watermark::Watermark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The mode values, and the only ones accepted.
const DEFAULT: &str = "default";
const WATERMARK: &str = "watermark";
/// The owner's marks (#2401): cut at 256k, down to 70k. The context
/// ceiling (`max_context_tokens`, the model's window, a swarm member's cap)
/// still wins: under a lower ceiling both marks scale down with it.
const DEFAULT_HIGH_TOKENS: usize = 256_000;
const DEFAULT_LOW_TOKENS: usize = 70_000;

/// The context mode's keys, flattened into `agents.defaults`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextModeConfig {
    /// `"default"` (or unset): the pruning rules. `"watermark"`: append
    /// only, one cut at `context_high_tokens` down to `context_low_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_high_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_low_tokens: Option<usize>,
    /// An environment value that is no count, refused at validation.
    #[serde(skip)]
    invalid_env: Option<String>,
    /// Whether `QUECTO_CONTEXT_MODE` set the mode.
    #[serde(skip)]
    mode_from_env: bool,
    /// Whether the launching agent's mode filled an unset mode.
    #[serde(skip)]
    mode_inherited: bool,
}

impl ContextModeConfig {
    /// The marks: the configured ones, or the owner's.
    fn marks(&self) -> Result<Watermark, ConfigError> {
        let high = self.context_high_tokens.unwrap_or(DEFAULT_HIGH_TOKENS);
        let low = self.context_low_tokens.unwrap_or(DEFAULT_LOW_TOKENS);
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
    /// The context mode the configuration selects. Validated at load, so a
    /// configuration that names no valid mode (only one built in code
    /// without loading) keeps the default.
    pub fn context_mode(&self) -> ContextMode {
        let config = &self.context_mode;
        debug_assert!(
            config.marks().is_ok(),
            "the marks were validated at load: {:?}",
            config.marks()
        );
        match config.context_mode.as_deref() {
            Some(WATERMARK) => config
                .marks()
                .map_or(ContextMode::Default, ContextMode::Watermark),
            Some(_) | None => ContextMode::Default,
        }
    }
}

/// Where the effective context mode came from (#2403 final review L4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextModeSource {
    OwnConfiguration,
    OwnEnvironment,
    Inherited,
    Default,
}

impl AgentDefaults {
    /// Adopt what the launching agent handed down with
    /// `--inherited-context-mode` (`default`, or `watermark:<high>:<low>`).
    pub fn inherit_context_mode(&mut self, value: &str) -> Result<(), ConfigError> {
        let refusal = || {
            refused(&format!(
                "--inherited-context-mode must be \"{DEFAULT}\" or \"{WATERMARK}:<high>:<low>\", not {value:?}"
            ))
        };
        let parts: Vec<&str> = value.split(':').collect();
        let (mode, marks) = match parts.as_slice() {
            [DEFAULT] => (DEFAULT, None),
            [WATERMARK, high, low] => {
                let high = high.parse::<usize>().map_err(|_| refusal())?;
                let low = low.parse::<usize>().map_err(|_| refusal())?;
                (WATERMARK, Some((high, low)))
            }
            _ => return Err(refusal()),
        };
        // For each of the mode and the marks, this agent's own setting wins.
        let config = &mut self.context_mode;
        if config.context_mode.is_none() {
            config.context_mode = Some(mode.to_string());
            config.mode_inherited = true;
        }
        if let Some((high, low)) = marks {
            config.context_high_tokens.get_or_insert(high);
            config.context_low_tokens.get_or_insert(low);
        }
        validate(self)
    }

    /// Where the effective context mode came from.
    pub fn context_mode_source(&self) -> ContextModeSource {
        let config = &self.context_mode;
        match (
            config.context_mode.is_some(),
            config.mode_from_env,
            config.mode_inherited,
        ) {
            (true, true, _) => ContextModeSource::OwnEnvironment,
            (true, false, true) => ContextModeSource::Inherited,
            (true, false, false) => ContextModeSource::OwnConfiguration,
            (false, _, _) => ContextModeSource::Default,
        }
    }
}

/// The `--inherited-context-mode` value of `mode`.
pub fn inherited_value(mode: ContextMode) -> String {
    match mode {
        ContextMode::Default => DEFAULT.to_string(),
        ContextMode::Watermark(marks) => {
            format!("{WATERMARK}:{}:{}", marks.high(), marks.low())
        }
    }
}

/// `QUECTO_CONTEXT_MODE`, `QUECTO_CONTEXT_HIGH_TOKENS` and
/// `QUECTO_CONTEXT_LOW_TOKENS`. A mark that is no count is kept as
/// refused, so the load fails naming it.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    let config = &mut defaults.context_mode;
    if let Some(mode) = env.get("QUECTO_CONTEXT_MODE") {
        config.context_mode = Some(mode.clone());
        config.mode_from_env = true;
    }
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

/// An unknown mode, a mark that is no positive count, and marks the
/// watermark refuses are refused. The marks are checked in the default
/// mode too, so a mistyped mark is caught before the mode is switched on.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let config = &defaults.context_mode;
    if let Some(reason) = &config.invalid_env {
        return Err(refused(reason));
    }
    match config.context_mode.as_deref() {
        None | Some(DEFAULT) | Some(WATERMARK) => config.marks().map(|_| ()),
        Some(other) => Err(refused(&format!(
            "context_mode must be \"{DEFAULT}\" or \"{WATERMARK}\", not {other:?}"
        ))),
    }
}

#[cfg(test)]
#[path = "context_mode_tests.rs"]
mod tests;
