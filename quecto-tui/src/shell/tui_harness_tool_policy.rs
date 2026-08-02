use super::tui_harness::TuiHarness;
use crate::protocol::client::{ToolCatalogueEntry, ToolPolicyResult};

impl TuiHarness {
    /// Merge tool catalogue entries through the production app event helper.
    pub fn merge_tool_catalogue(&mut self, entries: Vec<ToolCatalogueEntry>) -> &mut Self {
        self.app.merge_tool_catalogue_event(entries);
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
