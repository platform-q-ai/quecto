use crate::{DebugSwarm, QuectoWorld};
use cucumber::{given, then};
use quecto::application::tools::ports::Tool;
use quecto::domain::tool::ToolResult;
use quecto::infrastructure::tools::swarm::SwarmTool;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// One per-process runtime drives every swarm step, so work a step's call
/// leaves behind (a wake hint, the settlement watcher) outlives the step.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build swarm test runtime")
    })
}

fn ensure_workspace(world: &mut QuectoWorld) -> PathBuf {
    if world.swarm_workspace.is_none() {
        let tmp = TempDir::new().expect("failed to create swarm temp dir");
        let path = tmp.path().to_path_buf();
        world._swarm_temp_dir = Some(tmp);
        world.swarm_workspace = Some(path);
    }
    world.swarm_workspace.clone().unwrap()
}

/// One tool instance per scenario, shared by its steps.
pub(crate) fn tool(world: &mut QuectoWorld) -> Arc<SwarmTool> {
    let ws = ensure_workspace(world);
    if world.swarm_tool.is_none() {
        let tool = quecto::infrastructure::tools::swarm_test_support::tool(
            Arc::new(ws),
            quecto::composition::swarm::swarm_board(),
        );
        world.swarm_tool = Some(DebugSwarm(Arc::new(tool)));
    }
    world.swarm_tool.as_ref().unwrap().0.clone()
}

fn run(world: &mut QuectoWorld, args: serde_json::Value) {
    let tool = tool(world);
    let result = runtime()
        .block_on(async { tool.execute(&args.to_string()).await })
        .unwrap_or_else(|e| ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        });
    world.swarm_result = Some(result);
}

fn result(world: &QuectoWorld) -> &ToolResult {
    world
        .swarm_result
        .as_ref()
        .expect("expected a swarm result")
}

fn result_json(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&result(world).content).unwrap_or_else(|e| {
        panic!(
            "swarm result should be JSON ({e}): {}",
            result(world).content
        )
    })
}

// ---------------------------------------------------------------------------
// Steps shared by the swarm features
// ---------------------------------------------------------------------------

#[given("a swarm workspace")]
fn given_workspace(world: &mut QuectoWorld) {
    ensure_workspace(world);
}

#[then(regex = r#"^the swarm result should contain "(.+)"$"#)]
fn then_contains(world: &mut QuectoWorld, needle: String) {
    let content = &result(world).content;
    assert!(
        content.contains(&needle),
        "expected swarm result to contain {needle:?}, got: {content}"
    );
}

#[then("the swarm result should be an error")]
fn then_is_error(world: &mut QuectoWorld) {
    assert!(
        result(world).is_error,
        "expected swarm result to be an error: {}",
        result(world).content
    );
}

#[then("the swarm result should not be an error")]
fn then_not_error(world: &mut QuectoWorld) {
    assert!(
        !result(world).is_error,
        "expected swarm result not to be an error: {}",
        result(world).content
    );
}

#[path = "swarm_coordination_steps.rs"]
mod coordination;
#[path = "swarm_coordination_ops_steps.rs"]
mod coordination_ops;
#[path = "swarm_structured_ops_steps.rs"]
mod structured_ops;

#[path = "swarm_recovery_steps.rs"]
mod recovery_steps;
#[path = "swarm_supervision_steps.rs"]
mod supervision_steps;
