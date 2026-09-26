//! The switchable event log (#2149, #2150).
use serde::{Deserialize, Serialize};

/// `telemetry` in config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub event_log: EventLogConfig,
}

/// `telemetry.event_log`: when on, every agent writes the detailed audit
/// log (`<base_dir>/audit/<session>.jsonl`). Off unless configured; a
/// repository overlay may switch it on, never off.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventLogConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// Whether the owner's global config (`<base_dir>/config.json`) switches
/// the event log on (#2150): it covers every agent on the machine, one
/// started with its own `--config` (a container member, a sub-agent given a
/// config) included. A missing or unreadable file switches nothing on.
pub fn globally_enabled(base_dir: &std::path::Path) -> bool {
    std::fs::read(base_dir.join("config.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|config| {
            config["telemetry"]["event_log"]["enabled"] == serde_json::json!(true)
        })
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
