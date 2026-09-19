//! Discovery rows as the picker presents them (#2009, #2011, #2010): listed
//! rows newest first, searched rows in the harness's order with their key,
//! stable `session:<key>` IDs, safe copy, and the home version each row was
//! shown at. No eligibility is decided or enforced here — the backend's
//! `resumeEligible` is only worded, and every selection is asked of the harness.
use crate::components::ansi::sanitize_untrusted_label;
use crate::components::select_list::SelectItem;
use crate::protocol::session_payloads::ResumeSessionSummary;

pub const SESSION_ROW_PREFIX: &str = "session:";
const NO_HOME: &str = "No folder on record";
/// A session saved before folders were tracked: said outright, not implied.
const UNSCOPED: &str = "Unscoped · no folder on record";
const ELIGIBLE: &str = "Resume";
/// Why Enter on the row restores nothing at once, in the picker's own words
/// ("Local Folder" / "All Folders"): the session lives in another folder, or
/// no folder is on record for it (the row already says so where a folder
/// would be).
const ELSEWHERE: &str = "Saved in another folder — Enter for options";
const NO_FOLDER: &str = "Enter for options";

pub struct ResumeRows {
    pub items: Vec<SelectItem>,
    /// The home version each row was listed at (#2011), echoed on selection.
    pub home_versions: std::collections::BTreeMap<String, String>,
    /// The title each row is shown with, for the decision dialog to repeat.
    pub titles: std::collections::BTreeMap<String, String>,
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
        Self::rows(sessions, had_entries, when, false)
    }

    /// Project a metadata-search answer (#2010): the harness's order (best
    /// match first) is kept, and each row names its stable key.
    pub fn project_searched(
        sessions: Vec<ResumeSessionSummary>,
        when: impl Fn(u64) -> String,
    ) -> Self {
        Self::rows(sessions, true, when, true)
    }

    fn rows(
        sessions: Vec<ResumeSessionSummary>,
        had_entries: bool,
        when: impl Fn(u64) -> String,
        searched: bool,
    ) -> Self {
        let empty_hint = match (sessions.is_empty(), searched, had_entries) {
            (false, ..) => None,
            (true, true, _) => Some("No sessions match."),
            (true, false, true) => Some("No resumable CLI sessions found."),
            (true, false, false) => Some("No persisted sessions found."),
        };
        let home_versions = sessions
            .iter()
            .filter_map(|s| Some((s.key.clone(), s.home_version.clone()?)))
            .collect();
        let titles = sessions
            .iter()
            .map(|s| (s.key.clone(), s.title.clone()))
            .collect();
        let items = sessions
            .into_iter()
            .map(|session| {
                let when = session
                    .updated_unix_secs
                    .map(&when)
                    .unwrap_or_else(|| "unknown time".to_string());
                let action = match (session.resume_eligible, &session.execution_dir) {
                    (true, _) => ELIGIBLE,
                    (false, Some(_)) => ELSEWHERE,
                    (false, None) => NO_FOLDER,
                };
                let folder = match (&session.execution_dir, session.unscoped) {
                    (Some(folder), _) => folder.as_str(),
                    (None, true) => UNSCOPED,
                    (None, false) => NO_HOME,
                };
                let key = if searched {
                    format!(" · key {}", sanitize_untrusted_label(&session.key, 128))
                } else {
                    String::new()
                };
                SelectItem {
                    value: format!("{SESSION_ROW_PREFIX}{}", session.key),
                    label: sanitize_untrusted_label(&session.title, 512),
                    description: Some(format!(
                        "{} · {when} ({} msgs) · {action}{key}",
                        sanitize_untrusted_label(folder, 512),
                        session.message_count,
                    )),
                }
            })
            .collect();
        Self {
            items,
            home_versions,
            titles,
            empty_hint,
        }
    }
}
