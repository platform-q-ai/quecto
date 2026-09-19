//! The `/resume` picker's metadata search (#2010): what a scope or search-text
//! edit sends and which answer is shown. The harness owns scope and matching;
//! this shell edge owns only the request id, the flight control and the toast.
//! Child module of `app_session_resume`.
use super::super::super::*;
use crate::protocol::session_search_payloads::{SessionSearchRequest, parse_session_search};
use crate::sessions::session_search::Settled;

impl App {
    /// The picker's scope or search text changed: visible text is searched by
    /// the harness in the scope on screen, an empty box lists that scope.
    pub(in crate::shell) fn request_session_discovery(
        &mut self,
        scope: crate::protocol::session_payloads::SessionListScope,
    ) {
        let searching = (self.ac().sessions.resume_selector.as_ref())
            .is_some_and(|picker| !picker.query().trim().is_empty());
        if !searching {
            return self.request_session_scope(scope);
        }
        self.ac_mut().sessions.scope = scope;
        // A listing still in flight would overwrite the rows the search shows.
        self.ac_mut().sessions.pending_list_id = None;
        if let Some(generation) = self.ac_mut().sessions.search.edited() {
            self.send_session_search(generation);
        }
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
        self.ac_mut().sessions.search.sent(id.clone(), generation);
        if !self.send_command(Command::SearchSessionMetadata {
            id: Some(id),
            search,
        }) {
            self.ac_mut().sessions.search.unsent();
        }
    }

    /// Show an answer only when it is this tab's search in flight AND still
    /// the latest edit; an overtaken one is dropped and the latest edit sent.
    pub(in crate::shell) fn handle_session_search_response(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        let answer = data.as_ref().map(parse_session_search);
        let flight = &mut self.ac_mut().sessions.search;
        // A failure echoes nothing: it stands for the generation it was sent with.
        let generation = match (&answer, success) {
            (Some(answer), true) => answer.generation,
            _ => flight.sent_generation(id),
        };
        match flight.settle(id, generation) {
            Settled::Foreign | Settled::Stale { resend: false } => {}
            Settled::Stale { resend: true } => {
                let latest = self.ac().sessions.search.latest();
                self.send_session_search(latest);
            }
            Settled::Fresh => match answer.filter(|_| success) {
                Some(answer) => self.show_session_search(answer, data.as_ref()),
                None => self.notify_response_error("Could not search sessions", error),
            },
        }
    }

    fn show_session_search(
        &mut self,
        answer: crate::protocol::session_search_payloads::SessionSearchAnswer,
        data: Option<&serde_json::Value>,
    ) {
        if let Some(line) = data.and_then(|data| {
            crate::sessions::discovery_diagnostics::unseen_diagnostics_toast(
                &mut self.ac_mut().sessions.shown_diagnostics,
                &crate::protocol::session_payloads::session_discovery_diagnostics(data),
            )
        }) {
            self.notify(&line, NotifyLevel::Warning);
        }
        if let Some(refused) = &answer.refused {
            let refused = crate::components::ansi::sanitize_untrusted_label(refused, 120);
            self.notify(&format!("Search refused: {refused}"), NotifyLevel::Warning);
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
        }
    }
}

#[cfg(test)]
#[path = "app_session_search_tests.rs"]
mod tests;
