use super::*;
use crate::sessions::resume_picker::ResumePicker;

#[derive(Default)]
pub(crate) struct SessionsFlow {
    /// Session resume selector shown after `/resume` lists persisted sessions.
    pub(super) resume_selector: Option<ResumePicker>,
    pub(super) pending_list_id: Option<String>,
    pub(super) scope: crate::protocol::session_payloads::SessionListScope,
    pub(super) eligible_keys: std::collections::BTreeSet<String>,
    /// The home version each listed row was shown at (#2011), by key.
    pub(super) home_versions: std::collections::BTreeMap<String, String>,
    /// The decision dialog of a session that cannot simply be restored (#2011).
    pub(super) resume_decision: Option<crate::sessions::resume_decision::ResumeDecisionDialog>,
    /// The picker selection's key and listed version, consumed by its send.
    pub(super) selected_home_version: Option<(String, String)>,
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
}

impl super::App {
    /// Route a key to the decision dialog (#2011). Cancel and Escape close it
    /// and send nothing; an unavailable action is explained and the dialog
    /// stays; an available one sends identity, action and version.
    fn handle_resume_decision_key(&mut self, key: &Key) {
        use crate::sessions::resume_decision::ResumeDecisionEvent;
        let Some(dialog) = self.ac_mut().sessions.resume_decision.as_mut() else {
            return;
        };
        match dialog.handle_key(key) {
            ResumeDecisionEvent::Pending => {}
            ResumeDecisionEvent::Cancelled => self.ac_mut().sessions.resume_decision = None,
            ResumeDecisionEvent::Unavailable(why) => self.notify(&why, NotifyLevel::Warning),
            ResumeDecisionEvent::Chosen(selection) => {
                self.ac_mut().sessions.resume_decision = None;
                self.send_resume_selection(selection);
            }
        }
    }

    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        use crate::sessions::resume_picker::ResumePickerEvent;
        if self.ac().sessions.resume_decision.is_some() {
            return self.handle_resume_decision_key(key);
        }
        // The shell prefixes the agents pane after rendering this body-local overlay.
        let key = match key {
            Key::MousePress(x, y) => {
                let (panel, divider, _) = self.frame_split();
                let Some(x) = usize::from(*x).checked_sub(panel + divider) else {
                    return;
                };
                Key::MousePress(x as u16, *y)
            }
            other => other.clone(),
        };
        let Some(selector) = self.ac_mut().sessions.resume_selector.as_mut() else {
            return;
        };
        match selector.handle_key(&key) {
            ResumePickerEvent::ScopeChanged(scope) => self.request_session_scope(scope),
            ResumePickerEvent::Selected(choice) => {
                let key = choice
                    .strip_prefix(crate::sessions::resume_rows::SESSION_ROW_PREFIX)
                    .unwrap_or(&choice);
                // Every row is asked of the harness, with the version it was
                // listed at: an eligible one restores, any other is answered
                // with the typed decision. Nothing is decided here.
                let version = self.ac().sessions.home_versions.get(key).cloned();
                self.ac_mut().sessions.selected_home_version =
                    version.map(|version| (key.to_string(), version));
                self.ac_mut().sessions.resume_selector = None;
                self.apply_resume_selection(&choice);
            }
            ResumePickerEvent::Dismissed => {
                self.ac_mut().sessions.resume_selector = None;
                self.ac_mut().sessions.pending_list_id = None;
            }
            ResumePickerEvent::Pending => {}
        }
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
