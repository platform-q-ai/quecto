use super::*;
use crate::sessions::resume_picker::ResumePicker;

#[derive(Default)]
pub(crate) struct SessionsFlow {
    /// Session resume selector shown after `/resume` lists persisted sessions.
    pub(super) resume_selector: Option<ResumePicker>,
    pub(super) pending_list_id: Option<String>,
    pub(super) scope: crate::protocol::session_payloads::SessionListScope,
    pub(super) eligible_keys: std::collections::BTreeSet<String>,
    /// Discovery diagnostics already toasted in this process (#2018).
    pub(super) shown_diagnostics: std::collections::BTreeSet<String>,
    /// Session stats fallback to learn real context window for current session/model.
    pub(super) context_stats_requested: bool,
}

impl super::App {
    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        use crate::sessions::resume_picker::ResumePickerEvent;
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
                if self.ac().sessions.eligible_keys.contains(key) {
                    self.ac_mut().sessions.resume_selector = None;
                    self.apply_resume_selection(&choice);
                } else {
                    self.notify("Resume unavailable. Open original / Fork / Locate unavailable; Cancel with Escape.", NotifyLevel::Warning);
                }
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
