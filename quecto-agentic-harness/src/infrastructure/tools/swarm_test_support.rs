//! Explicit fake container composition for deterministic execution contracts.
use super::{
    swarm::{SwarmConfig, SwarmTool},
    swarm_bridge::SwarmContext,
};
use crate::infrastructure::security::sandbox::Sandbox;
use std::{path::PathBuf, sync::Arc};

pub fn tool(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, config: SwarmConfig) -> SwarmTool {
    let context = SwarmContext {
        lifecycle: std::sync::Arc::new(crate::application::ports::SwarmTestLifecycle),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    if !context.database().exists() {
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300;
        context.call("create", serde_json::json!(["test execution", [], [{"id":"tests","kind":"command","description":"pass"}], 10, deadline])).unwrap();
    }
    SwarmTool::new(workspace, sandbox, config).with_context(Some(context))
}
