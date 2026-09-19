//! The `/resume` picker's metadata search (#2010): what a scope or search-text
//! edit sends and which answer is shown. The harness owns scope and matching;
//! this shell edge owns only the request id, the flight control and the toast.
//! Child module of `app_session_resume`.
use super::super::super::*;
use crate::protocol::session_payloads::SessionListScope;
use crate::protocol::session_search_payloads::{
    SessionSearchAnswer, SessionSearchRequest, parse_session_search,
};
use crate::sessions::resume_picker::RowsState;
use crate::sessions::session_search::{FlightState, Overdue, Settled};

/// Said once per connection to a harness that predates the command (R1-T5).
const OLD_HARNESS: &str = "Search needs a newer quecto harness — restart the agent";
const FILTERED_HERE: &str = "Filtering the listed sessions here — this harness cannot search";
const NO_ANSWER: &str = "Search did not answer — edit the text or change Scope to retry";

impl App {
    /// The picker's scope or search text changed: visible text is searched by
    /// the harness in the scope on screen, an empty box lists that scope.
    pub(in crate::shell) fn request_session_discovery(&mut self, scope: SessionListScope) {
        let searching = (self.ac().sessions.resume_selector.as_ref())
            .is_some_and(|picker| !picker.query().trim().is_empty());
        let rescoped = scope != self.ac().sessions.scope;
        // An old harness is asked for a listing only; the box filters it here.
        if !searching || (rescoped && self.ac().sessions.search_unsupported) {
            return self.request_session_scope(scope);
        }
        self.ac_mut().sessions.scope = scope;
        // A listing still in flight would overwrite the rows the search shows.
        self.ac_mut().sessions.pending_list_id = None;
        if self.ac().sessions.search_unsupported {
            return self.filter_listed_sessions();
        }
        let flight = &mut self.ac_mut().sessions.search;
        let send = if rescoped {
            flight.scope_changed()
        } else {
            flight.edited()
        };
        if let Some(generation) = send {
            self.send_session_search(generation);
        }
        self.sync_picker_rows_state();
    }

    fn send_session_search(&mut self, generation: u64) {
        let Some(picker) = self.ac().sessions.resume_selector.as_ref() else {
            return;
        };
        let search = SessionSearchRequest {
            query: picker.query().to_string(),
            scope: picker.scope(),
            generation,
        };
        let id = self.ac().namespaced_id(&format!(
            "resume-search-{}",
            super::super::super::app_events::uuid_like()
        ));
        let now = tokio::time::Instant::now();
        self.ac_mut()
            .sessions
            .search
            .sent(id.clone(), generation, now);
        if !self.send_command(Command::SearchSessionMetadata {
            id: Some(id),
            search,
        }) {
            self.ac_mut().sessions.search.unsent();
        }
    }

    /// Only the answer of the latest edit, under this tab's id, SETTLES the
    /// rows. An overtaken one is still newer than what is on screen, so it is
    /// shown as progress (R1-T3) — never for another scope or a cleared box —
    /// while the box stays unsettled and the latest edit goes out.
    pub(in crate::shell) fn handle_session_search_response(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        let answer = data.as_ref().map(parse_session_search).filter(|_| success);
        let flight = &mut self.ac_mut().sessions.search;
        // A failure echoes nothing: it stands for the generation it was sent with.
        let generation = match &answer {
            Some(answer) => answer.generation,
            None => flight.sent_generation(id),
        };
        // The echoed scope must be the one on screen (an absent echo is an
        // older harness's: the generation already covers a scope change).
        let scope = self.ac().sessions.scope;
        let answer = answer.filter(|answer| answer.scope.is_none_or(|echo| echo == scope));
        match self.ac_mut().sessions.search.settle(id, generation) {
            Settled::Foreign => return,
            Settled::Stale { resend, progress } => {
                if let Some(answer) = answer.filter(|_| progress) {
                    self.show_session_search(answer, data.as_ref());
                }
                if resend {
                    let latest = self.ac().sessions.search.latest();
                    self.send_session_search(latest);
                }
            }
            Settled::Fresh => match answer {
                Some(answer) => self.show_session_search(answer, data.as_ref()),
                None => {
                    self.ac_mut().sessions.search.unanswered();
                    self.notify_response_error("Could not search sessions", error);
                }
            },
        }
        self.sync_picker_rows_state();
    }

    /// A harness that does not know the command rejects the line with an
    /// uncorrelated `parse_error`: the search in flight would never be
    /// answered. It is given up, the user is told ONCE per connection, no
    /// further search is sent on it, and the box filters the listed rows here
    /// instead (R1-T5). `parse_error` is broadcast, so it is this tab's only
    /// when it names the command AND a search is in flight here.
    pub(in crate::shell) fn handle_search_parse_error(&mut self, error: Option<&str>) {
        let about_search = error.is_some_and(|e| e.contains("search_session_metadata"));
        if about_search && self.ac().sessions.search.is_in_flight() {
            self.ac_mut().sessions.search.abandon();
            self.ac_mut().sessions.search_unsupported = true;
            self.notify(OLD_HARNESS, NotifyLevel::Warning);
            self.filter_listed_sessions();
        }
    }

    /// The fallback of R1-T5: the listed rows the text names, settled at once.
    pub(in crate::shell) fn filter_listed_sessions(&mut self) {
        let sessions = &self.ac().sessions;
        let Some(picker) = sessions.resume_selector.as_ref() else {
            return;
        };
        let query = picker.query().to_string();
        let found = crate::sessions::local_filter::filter_listed(&sessions.listed, &query);
        let answer = SessionSearchAnswer {
            generation: None,
            total_matches: found.len() as u64,
            sessions: found,
            refused: None,
            truncated: false,
            scope: None,
        };
        self.show_session_search(answer, None);
        let picker = self.ac_mut().sessions.resume_selector.as_mut();
        if let Some(picker) = picker.filter(|picker| picker.item_count() > 0) {
            picker.set_notice(Some(FILTERED_HERE.into()));
        }
        self.sync_picker_rows_state();
    }

    /// Give up a search that was not answered in time (R1-T4): the latest
    /// text is asked once more, then the box says so and waits for an edit.
    /// `true` when something changed on screen.
    pub(in crate::shell) fn service_search_timeout(&mut self, now: tokio::time::Instant) -> bool {
        let Some(overdue) = self.ac_mut().sessions.search.overdue(now) else {
            return false;
        };
        match overdue {
            Overdue::Retry(generation) => self.send_session_search(generation),
            Overdue::GaveUp => self.notify(NO_ANSWER, NotifyLevel::Warning),
            Overdue::Moot => {}
        }
        self.sync_picker_rows_state();
        true
    }

    /// Tell the picker what its rows are worth: loading while a listing is
    /// awaited, else the flight's word. Settling honours an Enter typed ahead
    /// of the answer (R1-T1) through the one selection path.
    pub(in crate::shell) fn sync_picker_rows_state(&mut self) {
        let sessions = &mut self.ac_mut().sessions;
        let state = match (sessions.pending_list_id.is_some(), sessions.search.state()) {
            (true, _) | (false, FlightState::Searching) => RowsState::Searching,
            (false, FlightState::Settled) => RowsState::Settled,
            (false, FlightState::Stalled) => RowsState::Stalled,
        };
        let picker = sessions.resume_selector.as_mut();
        if let Some(choice) = picker.and_then(|picker| picker.set_rows_state(state)) {
            self.resume_picker_selected(&choice);
        }
    }

    /// The connection was lost: the flight and any awaited listing died with
    /// it, the next connection is asked afresh, and an open picker says its
    /// rows are unanswered instead of loading for ever (R1-T4).
    pub(in crate::shell) fn interrupt_session_discovery(&mut self) {
        let sessions = &mut self.ac_mut().sessions;
        let awaited = sessions.pending_list_id.take().is_some();
        sessions.search.interrupted();
        sessions.search_unsupported = false;
        self.sync_picker_rows_state();
        let picker = self.ac_mut().sessions.resume_selector.as_mut();
        if let Some(picker) = picker.filter(|_| awaited) {
            picker.set_rows_state(RowsState::Stalled);
        }
    }

    /// Replace the rows with an answer's. With no picker open the answer is
    /// nobody's: nothing is recorded, nothing is said (R1-T7).
    fn show_session_search(
        &mut self,
        answer: SessionSearchAnswer,
        data: Option<&serde_json::Value>,
    ) {
        let Some(picker) = self.ac().sessions.resume_selector.as_ref() else {
            return;
        };
        let notice = search_notice(&answer, picker.query(), picker.scope());
        if let Some(line) = data.and_then(|data| {
            crate::sessions::discovery_diagnostics::unseen_diagnostics_toast(
                &mut self.ac_mut().sessions.shown_diagnostics,
                &crate::protocol::session_payloads::session_discovery_diagnostics(data),
            )
        }) {
            self.notify(&line, NotifyLevel::Warning);
        }
        let rows = crate::sessions::resume_rows::ResumeRows::project_searched(
            answer.sessions,
            super::super::super::app_time::format_unix_minutes,
        );
        let sessions = &mut self.ac_mut().sessions;
        sessions.home_versions = rows.home_versions;
        sessions.listed_titles = rows.titles;
        sessions.selected_home_version = None;
        if let Some(picker) = sessions.resume_selector.as_mut() {
            picker.sync_items(rows.items);
            picker.set_notice(notice);
        }
    }
}

/// The one status line under a search's rows (R1-T2, R1-T14): refused, no
/// match (scope-aware, in the picker's own words) or cut at the limit.
fn search_notice(
    answer: &SessionSearchAnswer,
    query: &str,
    scope: SessionListScope,
) -> Option<String> {
    use crate::components::ansi::sanitize_untrusted_label;
    let shown = answer.sessions.len() as u64;
    if let Some(refused) = &answer.refused {
        let refused = sanitize_untrusted_label(refused, 120);
        return Some(format!("Search refused: {refused}"));
    }
    if shown == 0 {
        let query = sanitize_untrusted_label(query.trim(), 40);
        return Some(match scope {
            SessionListScope::Local => format!(
                "No sessions match \"{query}\" in Local Folder — Tab to Scope to search All Folders"
            ),
            SessionListScope::Global => format!("No sessions match \"{query}\" in All Folders"),
        });
    }
    answer.truncated.then(|| {
        let total = grouped(answer.total_matches.max(shown));
        format!(
            "Showing {} of {total} — keep typing to narrow",
            grouped(shown)
        )
    })
}

/// `5200` as `5,200`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
#[path = "app_session_search_flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "app_session_search_tests.rs"]
mod tests;
