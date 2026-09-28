//! A member session's telemetry (#2287): each [`SessionRecord`] is one
//! structured `tracing` event under [`TELEMETRY_TARGET`], beside the
//! process adapter's own (#2286). E2-S11 (#2304) adds the event-log record.

use super::claude_code::process::TELEMETRY_TARGET;
use crate::application::external_agent::dto::SessionRecord;
use crate::application::external_agent::ports::ExternalAgentTelemetry;

/// Logs a member session's records.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingExternalAgentTelemetry;

impl ExternalAgentTelemetry for TracingExternalAgentTelemetry {
    fn record(&self, record: &SessionRecord) {
        tracing::info!(
            target: TELEMETRY_TARGET,
            kind = record.kind(),
            record = ?record,
            "external agent session"
        );
    }
}
