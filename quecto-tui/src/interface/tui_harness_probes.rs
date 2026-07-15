//! #997 grouped-state probes: drive real App paths, observe the owner groups.

use super::TuiHarness;
use crate::interface::components::chat::ChatEntry;

impl TuiHarness {
    /// Full active-session chat transcript (test-harness). Pins to the bottom
    /// first so the no-viewport render emits every entry, letting scroll-back
    /// scenarios assert whole-history content (order / exact-once / gaps)
    /// independent of the current scroll offset.
    pub fn active_chat_text(&mut self, width: usize) -> String {
        use crate::interface::component::Component as _;
        let chat = self.app.active_chat_mut();
        chat.scroll_down(usize::MAX);
        chat.render(width)
            .iter()
            .map(|l| crate::interface::ansi::strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Issue a rewind correlation id through the real idle double-Escape path.
    pub fn issue_rewind_open(&mut self) {
        self.app.handle_idle_escape_for_rewind();
        self.app.handle_idle_escape_for_rewind();
    }

    /// The rewind owner group's monotonic request sequence (#997).
    pub fn rewind_group_request_seq(&self) -> u64 {
        self.app.rewind.request_seq
    }

    /// Request a model-selector open through the real (deferred) `/model` path.
    pub fn request_model_selector_open(&mut self) {
        self.app.open_model_selector();
    }

    /// Deliver a real `list_models` response with `count` synthetic models.
    pub fn deliver_list_models(&mut self, count: usize) {
        let models: Vec<serde_json::Value> = (0..count)
            .map(|i| serde_json::json!({"model": format!("prov/model-{i}"), "provider": "Prov"}))
            .collect();
        self.app
            .handle_list_models(Some(serde_json::json!({ "models": models })));
    }

    /// Model-registry owner group (#997): `(entry count, open pending)`.
    pub fn model_registry_group(&self) -> (usize, bool) {
        (
            self.app.model_registry.entries.len(),
            self.app.model_registry.open_pending,
        )
    }

    /// Tracked sub-agent count held by the sub-agent UI owner group (#997).
    pub fn subagent_group_tracked(&self) -> usize {
        self.app.subagents.tracked.len()
    }

    /// Drain whatever commands are ALREADY queued, without the bounded
    /// polling wait of [`TuiHarness::drain_commands`]. For asserting that an
    /// action sent nothing (#1067): a 400×1ms poll for a command that never
    /// comes stalls the scenario long enough for 3s-lived notifications to
    /// expire under concurrent-scenario scheduling.
    pub fn try_drain_commands(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(line) = self.cmd_rx.try_recv() {
            out.push(line);
        }
        out
    }

    /// The effort selector's visible entries in display order (#1067), or
    /// `None` when the overlay is closed. Lets tests assert the exact
    /// vocabulary instead of substring-matching the rendered frame (where
    /// "high" is a substring of "xhigh" and the footer also names levels).
    pub fn effort_selector_entries(&self) -> Option<Vec<String>> {
        self.app
            .effort_selector
            .as_ref()
            .map(crate::interface::components::effort_selector::EffortSelector::visible_levels)
    }

    /// ANSI-stripped master session footer text (#1085). Lets focus-parity
    /// scenarios assert the master's retained model/effort without switching
    /// away from the focused child (which would hide the child's footer).
    pub fn master_footer_text(&mut self) -> String {
        use crate::interface::component::Component;
        let rendered = self.app.master_session.footer.render(200).join("\n");
        crate::interface::ansi::strip_ansi(&rendered)
    }

    /// The App-level `current_model` tracker (#1085), used with
    /// [`Self::master_footer_text`] to assert late master responses do not
    /// clobber focused-child state.
    pub fn current_model(&self) -> Option<String> {
        self.app.current_model.clone()
    }

    /// Master chat tool entries in transcript order: `(name, result)`.
    pub fn master_tool_entries(&self) -> Vec<(String, Option<String>)> {
        self.app
            .master_session
            .chat
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ChatEntry::ToolExecution {
                    tool_name, result, ..
                } => Some((tool_name.clone(), result.clone())),
                _ => None,
            })
            .collect()
    }
}
