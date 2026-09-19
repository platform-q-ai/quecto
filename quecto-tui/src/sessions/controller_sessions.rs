use super::*;
use crate::sessions::resume_picker::ResumePicker;

#[derive(Default)]
pub(crate) struct SessionsFlow {
    /// Session resume selector shown after `/resume` lists persisted sessions.
    pub(super) resume_selector: Option<ResumePicker>,
    pub(super) pending_list_id: Option<String>,
    pub(super) scope: crate::protocol::session_payloads::SessionListScope,
    /// The home version each listed row was shown at (#2011), by key.
    pub(super) home_versions: std::collections::BTreeMap<String, String>,
    /// The title each listed row was shown with, by key: a decision dialog
    /// names the session the way the picker did.
    pub(super) listed_titles: std::collections::BTreeMap<String, String>,
    /// Legacy storage retained for protocol/test compatibility; refusals now notify plainly.
    pub(super) resume_decision: Option<crate::sessions::resume_decision::ResumeDecisionDialog>,
    /// The picker selection's key and listed version, consumed by its send.
    pub(super) selected_home_version: Option<(String, String)>,
    /// Which metadata search is in flight and which answer may be shown (#2010).
    pub(super) search: crate::sessions::session_search::SearchFlight,
    /// The rows of the last listing, newest first: what the search box
    /// filters here when the harness cannot search (R1-T5).
    pub(super) listed: Vec<crate::protocol::session_payloads::ResumeSessionSummary>,
    /// The scope `listed` was listed in; `None` when no listing is held. A
    /// closed picker holds none, so a new one inherits no rows (R3-T2).
    pub(super) listed_scope: Option<crate::protocol::session_payloads::SessionListScope>,
    /// This connection's harness rejected `search_session_metadata`: told
    /// once, never asked again until the next connection (R1-T5).
    pub(super) search_unsupported: bool,
    /// Discovery diagnostics already toasted in this process (#2018).
    pub(super) shown_diagnostics: std::collections::BTreeSet<String>,
    /// Session stats fallback to learn real context window for current session/model.
    pub(super) context_stats_requested: bool,
}

impl SessionsFlow {
    /// A sessions modal owns the keyboard: the decision dialog or the picker.
    pub(super) fn has_modal(&self) -> bool {
        self.resume_decision.is_some() || self.resume_selector.is_some()
    }

    /// The ONE way the picker closes — Escape, a selection, a failed listing,
    /// a tab switch (R1-T7): whatever it asked is nobody's any more, so a
    /// late answer finds no flight and no awaited listing.
    pub(crate) fn close_picker(&mut self) {
        self.resume_selector = None;
        self.pending_list_id = None;
        self.listed.clear();
        self.listed_scope = None;
        self.search.abandon();
    }
}

impl super::App {
    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        use crate::sessions::resume_picker::ResumePickerEvent;
        // The shell prefixes the agents pane after rendering this body-local overlay.
        let key = match key {
            Key::MousePress(x, y) => {
                let (panel, divider, _) = self.frame_split();
                let Some(x) = usize::from(*x).checked_sub(panel + divider) else {
                    // A click outside the picker still withdraws an owed Enter.
                    let picker = self.ac_mut().sessions.resume_selector.as_mut();
                    return picker.map_or((), |picker| picker.withdraw_enter());
                };
                Key::MousePress(x as u16, *y)
            }
            other => other.clone(),
        };
        let Some(selector) = self.ac_mut().sessions.resume_selector.as_mut() else {
            return;
        };
        match selector.handle_key(&key) {
            // A scope or search-text edit asks the harness again (#2010): the
            // picker filters nothing and the TUI decides no scope or match.
            ResumePickerEvent::ScopeChanged(scope) => self.request_session_discovery(scope),
            ResumePickerEvent::QueryChanged(_) => {
                let scope = self.ac().sessions.scope;
                self.request_session_discovery(scope);
            }
            ResumePickerEvent::Selected(choice) => self.resume_picker_selected(&choice),
            ResumePickerEvent::Dismissed => self.ac_mut().sessions.close_picker(),
            ResumePickerEvent::Pending => {}
        }
    }

    /// A row was chosen — by Enter or a click on settled rows, or by the
    /// settled answer an Enter was typed ahead of (R1-T1). Every row is asked
    /// of the harness, with the version it was listed at: an eligible one
    /// restores, any other is answered with the typed decision. Nothing is
    /// decided here.
    pub(super) fn resume_picker_selected(&mut self, choice: &str) {
        let key = choice
            .strip_prefix(crate::sessions::resume_rows::SESSION_ROW_PREFIX)
            .unwrap_or(choice);
        let version = self.ac().sessions.home_versions.get(key).cloned();
        self.ac_mut().sessions.selected_home_version =
            version.map(|version| (key.to_string(), version));
        self.ac_mut().sessions.close_picker();
        self.apply_resume_selection(choice);
    }

    /// Route a successful `get_session_stats` response (#1472 r2): the own
    /// quiet footer refresh updates gauges silently, a PEER's quiet refresh
    /// (any tab namespace or legacy bare literal) is dropped silently, and
    /// only a user-solicited /session shows the chat Status line.
    pub(super) fn handle_session_stats_response(
        &mut self,
        id: Option<&str>,
        data: Option<serde_json::Value>,
    ) {
        let Some(data) = data else { return };
        if id == Some(self.ac().namespaced_id("stats-footer").as_str()) {
            self.update_footer_stats(&data);
        } else if id.is_some_and(|i| super::app_response::strip_tab_namespace(i) == "stats-footer")
        {
            // Peer quiet refresh: stay quiet.
        } else {
            self.show_session_stats(&data);
        }
    }
}
