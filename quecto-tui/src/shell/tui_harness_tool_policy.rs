use super::tui_harness::TuiHarness;
use crate::protocol::client::{ToolCatalogueEntry, ToolPolicyResult};

impl TuiHarness {
    /// Merge tool catalogue entries through the production app event helper.
    pub fn merge_tool_catalogue(&mut self, entries: Vec<ToolCatalogueEntry>) -> &mut Self {
        let request_id = self.app.tool_policy_modal_pending_catalogue_id.clone();
        self.app.merge_tool_catalogue_event(entries.clone());
        if let Some(id) = request_id {
            self.app.handle_response(
                Some(id),
                "get_tool_catalogue".into(),
                true,
                Some(serde_json::json!({ "tools": entries })),
                None,
            );
        }
        self.capture();
        self
    }

    /// Merge tool policy results through the production app event helper.
    pub fn merge_tool_policy_results(&mut self, results: Vec<ToolPolicyResult>) -> &mut Self {
        self.app.merge_tool_policy_results(results);
        self.capture();
        self
    }
}
