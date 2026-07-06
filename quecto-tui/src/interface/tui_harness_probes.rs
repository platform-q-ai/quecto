//! #997 grouped-state probes: drive real App paths, observe the owner groups.

use super::TuiHarness;

impl TuiHarness {
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
}
