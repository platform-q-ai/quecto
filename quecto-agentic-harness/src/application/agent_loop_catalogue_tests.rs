use std::sync::Arc;

use super::tests::{MockProvider, MockTool, test_config};
use crate::application::agent_loop::AgentLoopImpl;
use crate::infrastructure::tools::registry::{ToolRegistration, ToolRegistryImpl};

#[test]
fn live_agent_exposes_rich_catalogue_entries() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_with_metadata(
        Arc::new(MockTool::new("catalogued", "ok")),
        ToolRegistration::official_native().with_provider_id("quecto:test-provider"),
    ));
    let agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));

    let entries = agent.tool_catalogue_entries();
    let entry = entries
        .iter()
        .find(|entry| entry.name == "catalogued")
        .expect("live agent catalogue should include registered tool");
    assert_eq!(entry.provider_id, "quecto:test-provider");
    assert!(entry.effective_enabled);
}
