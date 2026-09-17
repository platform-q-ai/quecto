//! Wire rendering of a runtime-configuration reload (#1849): the `reload`
//! reply carries no data — a bare success when the runtime was reloaded or
//! nothing had to be, the rebuild error otherwise. The dispatch loop wraps
//! the verdict in its response envelope.

use crate::application::catalogue::dto::ReloadOutcome;

pub const NOT_CONFIGURED: &str = "provider reload is not configured";

/// `Ok(())` renders as a data-less success, `Err(message)` as the error
/// reply carrying `message`.
pub fn render(outcome: &ReloadOutcome) -> Result<(), String> {
    match outcome {
        ReloadOutcome::Reloaded { .. } | ReloadOutcome::Unchanged => Ok(()),
        ReloadOutcome::Failed(error) => Err(error.clone()),
        ReloadOutcome::NotConfigured => Err(NOT_CONFIGURED.to_string()),
    }
}

#[cfg(test)]
#[path = "reload_presenter_tests.rs"]
mod tests;
