//! #2414: watermark is the only context mode. The keys of the pruning
//! rules it replaced, and the switch between the two, are refused at load,
//! naming the key: no silent ignore, no compatibility shim.

use super::{AgentDefaults, ConfigError};
use crate::application::configuration::removed_keys::{REMOVED_DEFAULTS_KEYS, WHY_REMOVED};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

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
    /// The removed environment overrides that are set.
    #[serde(skip)]
    env: Vec<&'static str>,
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
    /// Every removed key the configuration sets, in the policy's order.
    fn set_keys(&self) -> Vec<&'static str> {
        let set = [
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
        ];
        debug_assert!(
            set.iter().map(|(key, _)| *key).eq(REMOVED_DEFAULTS_KEYS),
            "one field per removed key, in the policy's order"
        );
        set.into_iter()
            .filter_map(|(key, set)| set.then_some(key))
            .collect()
    }
}

/// Records every removed environment override that is set.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    defaults.removed_context_keys.env = REMOVED_ENV
        .into_iter()
        .filter(|key| env.contains_key(*key))
        .collect();
}

/// A configuration that sets removed keys or overrides is refused, naming
/// every one, with what removes it.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let removed = &defaults.removed_context_keys;
    let keys = removed.set_keys();
    let mut repairs: Vec<String> = keys
        .iter()
        .map(|key| format!("quecto config unset agents.defaults.{key} --global (or --local)"))
        .collect();
    repairs.extend(removed.env.iter().map(|key| format!("unset {key}")));
    let named: Vec<String> = keys
        .iter()
        .map(|key| format!("agents.defaults.{key}"))
        .chain(
            removed
                .env
                .iter()
                .map(|key| format!("the {key} environment override")),
        )
        .collect();
    match named.is_empty() {
        true => Ok(()),
        false => Err(ConfigError::RemovedKey(format!(
            "{} {WHY_REMOVED}; remove each one: {}",
            named.join(", "),
            repairs.join("; ")
        ))),
    }
}

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
