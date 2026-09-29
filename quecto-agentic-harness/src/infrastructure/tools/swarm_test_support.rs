//! Explicit fake container composition for deterministic execution contracts.
use super::{
    swarm::{SwarmConfig, SwarmTool},
    swarm_bridge::SwarmContext,
};
use crate::infrastructure::security::sandbox::Sandbox;
use std::{path::PathBuf, sync::Arc};

/// The tool over a fake container whose board is `board` (composition's,
/// `composition::swarm::swarm_board`, #2278).
pub fn tool(
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    config: SwarmConfig,
    board: super::swarm_bridge::SwarmBoard,
) -> SwarmTool {
    let context = SwarmContext {
        board,
        lifecycle: std::sync::Arc::new(crate::application::ports::SwarmTestLifecycle),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    std::fs::create_dir_all(context.database().parent().unwrap()).unwrap();
    if !context.database().exists() {
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300;
        // Test setup: the board call made off any async worker (#2278).
        super::call_work::off_the_runtime(|| {
            context.call("create", serde_json::json!(["test execution", [], [{"id":"tests","kind":"command","description":"pass"}], 10, deadline]))
        })
        .unwrap();
    }
    SwarmTool::new(workspace, sandbox, config).with_context(Some(context))
}
