//! Discovery rows as the picker presents them (#2009): newest first, stable
//! `session:<key>` IDs, safe copy, and the affirmative allowlist of keys the
//! shell may activate. No eligibility is decided here — the backend's
//! `resumeEligible` is carried, never inferred.
use std::collections::BTreeSet;

use crate::components::select_list::SelectItem;
use crate::protocol::session_payloads::ResumeSessionSummary;

pub const SESSION_ROW_PREFIX: &str = "session:";
const NO_HOME: &str = "Unassociated / unavailable home";
const ELIGIBLE: &str = "Resume";
const INELIGIBLE: &str = "Needs a decision (Enter)";

pub struct ResumeRows {
    pub items: Vec<SelectItem>,
    /// Keys the backend admitted; a selection outside this set is refused.
    pub eligible_keys: BTreeSet<String>,
    /// The home version each row was listed at (#2011), echoed on selection.
    pub home_versions: std::collections::BTreeMap<String, String>,
    /// The status line to show when there is nothing to pick.
    pub empty_hint: Option<&'static str>,
}

impl ResumeRows {
    /// Project parsed summaries: most recently active first (unknown times
    /// sink), one row per session, `when` formatting the update time.
    pub fn project(
        mut sessions: Vec<ResumeSessionSummary>,
        had_entries: bool,
        when: impl Fn(u64) -> String,
    ) -> Self {
        sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_unix_secs.unwrap_or(0)));
        let empty_hint = match (sessions.is_empty(), had_entries) {
            (false, _) => None,
            (true, true) => Some("No resumable CLI sessions found."),
            (true, false) => Some("No persisted sessions found."),
        };
        let eligible_keys = sessions
            .iter()
            .filter(|s| s.resume_eligible)
            .map(|s| s.key.clone())
            .collect();
        let home_versions = sessions
            .iter()
            .filter_map(|s| Some((s.key.clone(), s.home_version.clone()?)))
            .collect();
        let items = sessions
            .into_iter()
            .map(|session| {
                let when = session
                    .updated_unix_secs
                    .map(&when)
                    .unwrap_or_else(|| "unknown time".to_string());
                let action = if session.resume_eligible {
                    ELIGIBLE
                } else {
                    INELIGIBLE
                };
                SelectItem {
                    value: format!("{SESSION_ROW_PREFIX}{}", session.key),
                    label: session.title,
                    description: Some(format!(
                        "{} · {when} ({} msgs) · {action}",
                        session.execution_dir.as_deref().unwrap_or(NO_HOME),
                        session.message_count,
                    )),
                }
            })
            .collect();
        Self {
            items,
            eligible_keys,
            home_versions,
            empty_hint,
        }
    }
}
