//! #997 grouped-state probes for the headless render harness.
//!
//! Drive real App paths, then observe the value through the OWNER GROUP
//! (rewind flow / sub-agent UI / model registry). The observers are RED
//! scaffolds until the App fields move into their owner structs.

use super::TuiHarness;

impl TuiHarness {
    // Drive real App paths, then observe the value through the OWNER GROUP
    // (rewind flow / sub-agent UI / model registry). The observers are RED
    // scaffolds until the App fields move into their owner structs.

    /// Issue a rewind correlation id through the real idle double-Escape path.
    pub fn issue_rewind_open(&mut self) {
        self.app.handle_idle_escape_for_rewind();
        self.app.handle_idle_escape_for_rewind();
    }

    /// The rewind owner group's monotonic request sequence (#997).
    pub fn rewind_group_request_seq(&self) -> u64 {
        unimplemented!("issue #997: rewind state is not yet grouped into an owner struct")
    }

    /// Deliver a `list_models` response with `count` synthetic models through
    /// the real response handler.
    pub fn deliver_list_models(&mut self, count: usize) {
        let models: Vec<serde_json::Value> = (0..count)
            .map(|i| serde_json::json!({"model": format!("prov/model-{i}"), "provider": "Prov"}))
            .collect();
        self.app
            .handle_list_models(Some(serde_json::json!({ "models": models })));
    }

    /// Entry count held by the model-registry owner group (#997).
    pub fn model_registry_group_entries(&self) -> usize {
        unimplemented!("issue #997: model registry is not yet a named owner struct")
    }

    /// Whether the model-registry owner group has a pending selector open (#997).
    pub fn model_registry_group_pending(&self) -> bool {
        unimplemented!("issue #997: model registry is not yet a named owner struct")
    }

    /// Tracked sub-agent count held by the sub-agent UI owner group (#997).
    pub fn subagent_group_tracked(&self) -> usize {
        unimplemented!("issue #997: sub-agent UI state is not yet grouped into an owner struct")
    }
}
