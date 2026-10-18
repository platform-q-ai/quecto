//! Wire rendering of a runtime-configuration reload (#1849): the `reload`
//! reply is a bare success when the runtime was reloaded cleanly or nothing
//! had to be, the rebuild error otherwise. A reload that found persisted
//! `tools.policy` entries naming no tool carries them (#2247 round 2 N2):
//! `{"unknownPolicyTools": [...], "warnings": [...]}`. The dispatch loop
//! wraps the verdict in its response envelope.

use crate::application::catalogue::dto::ReloadOutcome;
use crate::domain::tool_policy::services::tool_policy_catalogue::unknown_policy_entry_warning;

pub const NOT_CONFIGURED: &str = "provider reload is not configured";

/// `Ok(data)` renders as a success carrying `data` (none for a clean
/// reload), `Err(message)` as the error reply carrying `message`.
pub fn render(outcome: &ReloadOutcome) -> Result<Option<serde_json::Value>, String> {
    match outcome {
        ReloadOutcome::Reloaded {
            unknown_policy_tools,
        } => Ok(match unknown_policy_tools.as_slice() {
            [] => None,
            unknown => Some(serde_json::json!({
                "unknownPolicyTools": unknown,
                "warnings": unknown
                    .iter()
                    .map(|stable_id| unknown_policy_entry_warning(stable_id))
                    .collect::<Vec<_>>(),
            })),
        }),
        ReloadOutcome::Unchanged => Ok(None),
        ReloadOutcome::Failed(error) => Err(error.clone()),
        ReloadOutcome::NotConfigured => Err(NOT_CONFIGURED.to_string()),
    }
}

#[cfg(test)]
#[path = "reload_presenter_tests.rs"]
mod tests;
