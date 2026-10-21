//! UDS `restart_admission` (#2615): the TUI's `/admission restart`. The
//! connection's reader task serves it, busy or idle (a turn waiting on a
//! stuck broker is exactly when it is needed), off the async runtime. The
//! restart use case does the work; this controller invokes it and presents
//! its report or refusal on the wire.
//!
//! A root's startup negotiation registers the composed use case; a process
//! without one (no admission, or a launch that never negotiated) says so.

use std::sync::{Arc, OnceLock};

use crate::application::admission::dto::negotiation::{
    Restart, RestartError, RestartOutcome, RestartReport,
};
use crate::application::admission::use_cases::RestartAuthority;

/// The wire command this controller answers.
pub const COMMAND: &str = "restart_admission";
const NOT_CONFIGURED: &str = "admission is not configured for this session";

/// One per process, like the admission binding it restarts.
static REGISTERED: OnceLock<Arc<RestartAuthority>> = OnceLock::new();

/// Register the composed restart (first registration wins, as the binding).
pub fn register(use_case: Arc<RestartAuthority>) {
    let _ = REGISTERED.set(use_case);
}

/// The restart this process registered, if any.
pub fn registered() -> Option<&'static RestartAuthority> {
    REGISTERED.get().map(Arc::as_ref)
}

/// Restart the broker and rejoin it (`confirm`: how many other sessions or
/// sub-agents the user acknowledged sharing the broker): the response
/// data, or the error text.
pub fn respond(
    use_case: Option<&RestartAuthority>,
    confirm: Option<usize>,
) -> Result<serde_json::Value, String> {
    let Some(use_case) = use_case else {
        return Err(NOT_CONFIGURED.into());
    };
    match use_case.execute(confirm).map_err(refusal)? {
        Restart::Done(report) => Ok(present(report)),
        Restart::Unconfirmed { others } => Ok(serde_json::json!({
            "outcome": "unconfirmed",
            "sharedWith": others,
        })),
    }
}

fn outcome_name(outcome: RestartOutcome) -> &'static str {
    match outcome {
        RestartOutcome::ServiceRestarted => "service_restarted",
        RestartOutcome::Restarted => "restarted",
        RestartOutcome::Started => "started",
    }
}

fn present(report: RestartReport) -> serde_json::Value {
    serde_json::json!({
        "outcome": outcome_name(report.outcome),
        "authorityStatus": report.health,
    })
}

fn refusal(error: RestartError) -> String {
    match error {
        RestartError::NotConfigured => NOT_CONFIGURED.into(),
        RestartError::Child => {
            "a child session cannot restart the shared admission broker; its root session can"
                .into()
        }
        RestartError::InProgress => "an admission restart is already in progress".into(),
        RestartError::Restart(reason) => format!("admission broker restart failed: {reason}"),
        RestartError::Rejoin { outcome, reason } => format!(
            "the admission broker is back ({}), but this session could not rejoin it: {reason}",
            outcome_name(outcome)
        ),
    }
}

#[cfg(test)]
#[path = "restart_controller_tests.rs"]
mod tests;
