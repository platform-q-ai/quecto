//! Presenter of the scoped `list_sessions` response (#2009, #2011): rows carry
//! only application-approved metadata — home state and version, execution path,
//! the backend's eligibility — with untrusted text bounded and made safe.
use super::session_summary_to_json;

/// Present only application-approved metadata; never infer resume eligibility here.
pub(in crate::interface::cli) fn discovery_json(
    result: &crate::application::sessions::dto::ListSessionsResult,
    scope: crate::interface::cli::protocol::SessionListScopeCommand,
) -> serde_json::Value {
    use crate::domain::session_home::SessionHomeScope;
    let sessions: Vec<_> = result
        .sessions
        .iter()
        .map(|row| {
            let mut value = session_summary_to_json(&row.summary);
            let (state, execution_path) = match &row.home {
                SessionHomeScope::Scoped(home) => (
                    "scoped",
                    Some(safe_display(&home.execution_dir.to_string_lossy())),
                ),
                SessionHomeScope::LegacyUnscoped => ("legacy_unscoped", None),
                SessionHomeScope::Unavailable(_) => ("unavailable", None),
            };
            value["homeState"] = serde_json::json!(state);
            value["executionPath"] = serde_json::json!(execution_path);
            value["resumeEligible"] = serde_json::json!(row.resume_eligible);
            value["homeVersion"] = serde_json::json!(row.home_version().as_str());
            value
        })
        .collect();
    serde_json::json!({
        "sessions": sessions,
        "scope": scope,
        "diagnostics": result.diagnostics.iter().map(|s| safe_display(s)).collect::<Vec<_>>(),
        "rebuilt": result.rebuilt,
    })
}

#[path = "uds_safe_display.rs"]
mod uds_safe_display;
pub(in crate::interface::cli) use uds_safe_display::safe_display;

#[cfg(test)]
#[path = "uds_dispatch_discovery_tests.rs"]
mod tests;
