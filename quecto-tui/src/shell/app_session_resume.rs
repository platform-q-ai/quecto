//! `/resume` request and response handling: the request id owned by this tab,
//! the Coordinator clock reset on a session identity change, and the view
//! refresh every resume answer triggers (#1726). Child module of
//! `app_response`.
use super::super::*;
use crate::protocol::resume_decision_payloads::{
    ResumeAnswer, ResumeSelection, parse_resume_answer,
};
use crate::sessions::resume_decision::ResumeDecisionDialog;
use crate::sessions::resume_picker::RowsState;

/// The toast of a `stale_home_version` refusal.
const STALE_LIST: &str = "List out of date — reopen /resume and pick again";

impl App {
    pub(in crate::shell) fn send_list_sessions(&mut self) {
        self.ac_mut().sessions.search.abandon();
        self.request_session_scope(crate::protocol::session_payloads::SessionListScope::Local);
    }

    pub(in crate::shell) fn request_session_scope(
        &mut self,
        scope: crate::protocol::session_payloads::SessionListScope,
    ) {
        // A listing replaces whatever a search in flight would have shown.
        self.ac_mut().sessions.search.superseded();
        let id = self.ac().namespaced_id(&format!(
            "resume-list-{}",
            super::super::app_events::uuid_like()
        ));
        self.ac_mut().sessions.pending_list_id = Some(id.clone());
        self.ac_mut().sessions.scope = scope;
        // The rows on screen are about to be replaced: their versions with them.
        self.ac_mut().sessions.home_versions.clear();
        self.ac_mut().sessions.listed_titles.clear();
        self.ac_mut().sessions.selected_home_version = None;
        if self.ac().sessions.resume_selector.is_none() {
            self.ac_mut().sessions.resume_selector = Some(
                crate::sessions::resume_picker::ResumePicker::new(Vec::new(), scope),
            );
        }
        if !self.send_command(Command::ListSessions {
            id: Some(id),
            scope,
        }) {
            // Never sent: nothing to await — the picker says so (R1-T4).
            self.ac_mut().sessions.pending_list_id = None;
            let picker = self.ac_mut().sessions.resume_selector.as_mut();
            picker.map(|picker| picker.set_rows_state(RowsState::Stalled));
            return;
        }
        self.sync_picker_rows_state();
    }

    pub(in crate::shell) fn handle_session_list_response(
        &mut self,
        id: Option<&str>,
        data: Option<serde_json::Value>,
    ) {
        if self
            .ac()
            .sessions
            .pending_list_id
            .as_deref()
            .is_some_and(|pending| Some(pending) == id)
        {
            self.ac_mut().sessions.pending_list_id = None;
            if let Some(data) = data {
                let diagnostics =
                    crate::protocol::session_payloads::session_discovery_diagnostics(&data);
                if let Some(line) = crate::sessions::discovery_diagnostics::unseen_diagnostics_toast(
                    &mut self.ac_mut().sessions.shown_diagnostics,
                    &diagnostics,
                ) {
                    self.notify(&line, NotifyLevel::Warning);
                }
                self.open_resume_selector(&data);
            }
        }
    }

    /// The picker opened for this answer; with no rows to show it closes, so
    /// an empty overlay never lingers.
    pub(in crate::shell) fn handle_session_list_failure(
        &mut self,
        id: Option<&str>,
        error: Option<String>,
    ) {
        let pending = self.ac().sessions.pending_list_id.as_deref();
        if pending.is_some_and(|pending| Some(pending) == id) {
            self.ac_mut().sessions.close_picker();
            self.notify_response_error("Could not list sessions", error);
        }
    }

    pub(in crate::shell) fn send_resume_session(&mut self, session: &str) {
        if session.trim().is_empty() {
            self.send_list_sessions();
            return;
        }
        // A picker selection carries the version its row was listed at; a
        // typed key or a latched resume of another key carries none.
        let listed = self.ac_mut().sessions.selected_home_version.take();
        let mut selection = ResumeSelection::exact(session.trim());
        selection.expected_home_version = listed
            .filter(|(key, _)| key == &selection.session)
            .map(|(_, version)| version);
        self.send_resume_selection(selection);
    }

    /// The one `resume_session` send: stable identity, optional explicit
    /// action and the home version the user was shown (#2011).
    pub(in crate::shell) fn send_resume_selection(&mut self, resume: ResumeSelection) {
        let id = self
            .ac()
            .namespaced_id(&format!("resume-{}", super::super::app_events::uuid_like()));
        self.ac_mut().pending_session_resume_id = Some(id.clone());
        self.ac_mut().pending_session_resume_acts = resume.action.is_some();
        let sent = self.send_command(Command::ResumeSession {
            id: Some(id),
            resume,
        });
        if !sent {
            self.ac_mut().pending_session_resume_id = None;
        }
    }

    /// A restore refreshes the view whoever asked (the agent's session changed
    /// for all its clients); only this tab's own answer settles the resume
    /// latches, so a foreign answer cannot cancel an in-flight resume. A
    /// decision, a refusal and an unreadable answer are told only to the tab
    /// that asked: another tab's id is a peer's (an answer with no id at all
    /// is nobody's in particular, and its failure is still toasted).
    /// Each tab has its own harness connection and an answer is applied to
    /// the tab it was routed to, so a decision that arrives after the user
    /// switched away is parked on the asking tab and shown when they return; a
    /// second send overwrites the id in flight, and the first answer then
    /// counts as a peer's (a restore still refreshes, a decision is dropped).
    pub(in crate::shell) fn handle_resume_response(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        let owned = self.is_owned_resume_response(id);
        let peers = id.is_some() && !owned;
        if owned {
            self.ac_mut().pending_session_resume_id = None;
            self.ac_mut().pending_session_resume = None;
        }
        match parse_resume_answer(success, data.as_ref()) {
            ResumeAnswer::Resumed(ack) => {
                self.clear_message_recovery();
                self.handle_resume_success(data.is_some().then_some(ack));
            }
            // Nothing changed for anyone: no refresh, no toast.
            ResumeAnswer::Cancelled => {}
            // A peer's decision, refusal or unreadable answer is not this tab's.
            _ if peers => {}
            ResumeAnswer::Decision(decision) if owned => {
                let sessions = &self.ac().sessions;
                let title = sessions.listed_titles.get(&decision.session_key).cloned();
                self.ac_mut().sessions.resume_decision =
                    Some(ResumeDecisionDialog::new(decision, title.as_deref()));
            }
            ResumeAnswer::Decision(_) => {}
            // One truncated line: the instruction first, and no "home".
            ResumeAnswer::Refused(Some(code)) if code == "stale_home_version" => {
                self.notify(STALE_LIST, NotifyLevel::Error);
            }
            ResumeAnswer::Refused(_) => self.notify_response_error("Resume failed", error),
            // A success this TUI cannot read as a restore changes nothing here:
            // no key adopted, no clock reset, no manifest write, no "Resumed".
            ResumeAnswer::Unrecognized(outcome) => {
                let outcome = outcome.unwrap_or_else(|| "an unreadable answer".to_string());
                let outcome = crate::components::ansi::sanitize_untrusted_label(&outcome, 40);
                self.notify(
                    &format!("Nothing changed: update quecto-tui (unknown answer \u{201c}{outcome}\u{201d})"),
                    NotifyLevel::Warning,
                );
            }
        }
    }

    /// The harness rejected a line it could not decode as `resume_session` —
    /// an action newer than the harness. A `parse_error` carries no `id`, so
    /// the request in flight would never be answered: settle it here, change
    /// nothing, and say so. `parse_error` is broadcast, so it is this tab's
    /// only when the request in flight CARRIED an action (a plain restore can
    /// never cause "unknown resume action" — that one is a peer's) and, should
    /// an id come with it, the id is the one in flight.
    pub(in crate::shell) fn handle_resume_parse_error(
        &mut self,
        id: Option<&str>,
        error: Option<String>,
    ) {
        let about_an_action = error
            .as_deref()
            .is_some_and(|e| e.contains("resume action"));
        let ours = about_an_action
            && self.ac().pending_session_resume_acts
            && (id.is_none() || self.is_owned_resume_response(id));
        if ours && self.ac_mut().pending_session_resume_id.take().is_some() {
            self.ac_mut().pending_session_resume = None;
            self.notify_response_error("Resume failed", error);
        }
    }

    fn is_owned_resume_response(&self, id: Option<&str>) -> bool {
        self.ac()
            .pending_session_resume_id
            .as_deref()
            .is_some_and(|pending| id == Some(pending))
    }

    fn handle_resume_success(
        &mut self,
        ack: Option<crate::protocol::state_payloads::ResumeSessionAck>,
    ) {
        // A resume into a session other than the one this tab is showing
        // (including one it has not learned yet) is a session boundary for
        // the Coordinator clock. An answer without an identity is not.
        if let Some(key) = ack.as_ref().and_then(|ack| ack.session_key.as_deref()) {
            let changed = self.ac().session_key.as_deref() != Some(key);
            if changed {
                self.ac_mut()
                    .reset_coordinator_clock(tokio::time::Instant::now());
            }
            self.ac_mut().session_key = Some(key.to_owned());
            // Learning the key here pre-empts the get_state snapshot path, so
            // the durable registry/manifest must be written here too.
            if changed {
                self.persist_default_durability();
            }
        }
        let session = ack.map_or_else(|| "session".to_string(), |ack| ack.name);
        self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
        self.request_resumed_transcript();
        self.send_session_stats();
        // The agent resets session-scoped state (e.g. the effort override,
        // #1067) on resume_session; re-fetch so the footer tracks it.
        self.send_state_resync();
    }
}

#[path = "app_session_search.rs"]
mod app_session_search;

#[cfg(test)]
#[path = "app_resume_decision_tests.rs"]
mod resume_decision_tests;
