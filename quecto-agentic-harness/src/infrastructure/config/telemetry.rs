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

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
