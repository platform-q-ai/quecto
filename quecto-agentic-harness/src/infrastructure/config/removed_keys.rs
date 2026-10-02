//! #2414: watermark is the only context mode. The keys of the pruning
//! rules it replaced, and the switch between the two, are refused at load,
//! naming the key: no silent ignore, no compatibility shim.

use super::{AgentDefaults, ConfigError};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

/// Why every removed key went, for its refusal.
const WHY: &str = "the watermark context is the only context mode: the context only grows \
                   at its end and is cut once at context_high_tokens down to \
                   context_low_tokens";

/// The removed `agents.defaults` keys, flattened into it: each records
/// only whether it was set (to anything, null included). Never written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemovedContextKeys {
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_mode: bool,
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_collapse_after_tool_calls: bool,
    /// The pre-#1017 name of the tool dial.
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_collapse_after_turns: bool,
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_collapse_after_messages: bool,
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_collapse_large_result_tokens: bool,
    #[serde(default, deserialize_with = "set", skip_serializing)]
    context_collapse_large_result_after_turns: bool,
    /// The first removed environment override that is set.
    #[serde(skip)]
    env: Option<&'static str>,
}

/// The removed environment overrides.
const REMOVED_ENV: [&str; 3] = [
    "QUECTO_CONTEXT_MODE",
    "QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS",
    "QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS",
];

/// A key present in the document is set, whatever its value.
fn set<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| true)
}

impl RemovedContextKeys {
    /// The first removed key the configuration sets.
    fn first_set(&self) -> Option<&'static str> {
        [
            ("context_mode", self.context_mode),
            (
                "context_collapse_after_tool_calls",
                self.context_collapse_after_tool_calls,
            ),
            (
                "context_collapse_after_turns",
                self.context_collapse_after_turns,
            ),
            (
                "context_collapse_after_messages",
                self.context_collapse_after_messages,
            ),
            (
                "context_collapse_large_result_tokens",
                self.context_collapse_large_result_tokens,
            ),
            (
                "context_collapse_large_result_after_turns",
                self.context_collapse_large_result_after_turns,
            ),
        ]
        .into_iter()
        .find_map(|(key, set)| set.then_some(key))
    }
}

/// Records the first removed environment override that is set.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    let found = REMOVED_ENV.into_iter().find(|key| env.contains_key(*key));
    if let Some(key) = found {
        defaults.removed_context_keys.env.get_or_insert(key);
    }
}

/// A configuration that sets a removed key or override is refused, naming it.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let removed = &defaults.removed_context_keys;
    match (removed.first_set(), removed.env) {
        (Some(key), _) => Err(ConfigError::RemovedKey(format!(
            "agents.defaults.{key} was removed (#2414): {WHY}; remove the key"
        ))),
        (None, Some(key)) => Err(ConfigError::RemovedKey(format!(
            "the {key} environment override was removed (#2414): {WHY}; unset it"
        ))),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
