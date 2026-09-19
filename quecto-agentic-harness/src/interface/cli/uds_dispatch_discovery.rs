//! Presenter of the scoped `list_sessions` response (#2009, #2011): rows carry
//! only application-approved metadata — home state and version, execution path,
//! the backend's eligibility — with untrusted text bounded and made safe.
use super::session_summary_to_json;
use crate::application::sessions::dto::{ListSessionsResult, ListedSession};
use crate::domain::{session_home::SessionHomeScope, session_metadata_text::display_path};

/// One discovery row, shared by `list_sessions` and the metadata search (#2010).
pub(in crate::interface::cli) fn listed_row_json(row: &ListedSession) -> serde_json::Value {
    let mut value = session_summary_to_json(&row.summary);
    let (state, execution_path) = match &row.home {
        SessionHomeScope::Scoped(home) => (
            "scoped",
            Some(safe_display(&display_path(&home.execution_dir))),
        ),
        SessionHomeScope::LegacyUnscoped => ("legacy_unscoped", None),
        SessionHomeScope::Unavailable(_) => ("unavailable", None),
    };
    value["homeState"] = serde_json::json!(state);
    value["executionPath"] = serde_json::json!(execution_path);
    value["resumeEligible"] = serde_json::json!(row.resume_eligible);
    value["homeVersion"] = serde_json::json!(row.home_version().as_str());
    value
}

/// Present only application-approved metadata; never infer resume eligibility here.
pub(in crate::interface::cli) fn discovery_json(
    result: &ListSessionsResult,
    scope: crate::interface::cli::protocol::SessionListScopeCommand,
) -> serde_json::Value {
    let sessions: Vec<_> = result.sessions.iter().map(listed_row_json).collect();
    let body = serde_json::json!({"sessions": sessions, "scope": scope});
    freshened(body, &result.diagnostics, result.rebuilt)
}

#[path = "uds_freshness_json.rs"]
mod uds_freshness_json;
pub(in crate::interface::cli) use uds_freshness_json::{freshened, safe_display};

#[cfg(test)]
#[path = "uds_dispatch_discovery_tests.rs"]
mod tests;
