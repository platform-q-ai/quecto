use crate::components::select_list::SelectList;

#[derive(Default)]
pub(crate) struct SessionsFlow {
    /// Session resume selector shown after `/resume` lists persisted sessions.
    pub(super) resume_selector: Option<SelectList>,
    /// Session stats fallback to learn real context window for current session/model.
    pub(super) context_stats_requested: bool,
}

impl super::App {
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
