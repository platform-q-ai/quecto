//! The coordinator launch port's contract (S4c), run against its
//! spawn-backed adapter over a spawn that answers as told: it launches
//! only while the session's policy allows spawn, asks spawn for a
//! coordinator in a new container, and answers what spawn says runs.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use quecto::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::tool::{ToolDefinition, ToolResult};
use quecto::infrastructure::tools::coordinator_launch::SpawnCoordinatorLauncher;
use quecto::infrastructure::tools::coordinator_launch::SpawnPermit;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;

/// A `spawn` that answers `content` (an error when `failing`) and keeps
/// the arguments of each call.
struct Spawn {
    content: String,
    failing: bool,
    calls: Mutex<Vec<String>>,
}

impl Tool for Spawn {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "a test spawn".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        self.calls.lock().unwrap().push(arguments.to_owned());
        let result = ToolResult {
            content: self.content.clone(),
            is_error: self.failing,
            image_blocks: vec![],
            delivery_metadata: None,
        };
        Box::pin(async move { Ok(result) })
    }
}

fn spawn(content: &str, failing: bool) -> Arc<Spawn> {
    Arc::new(Spawn {
        content: content.into(),
        failing,
        calls: Mutex::default(),
    })
}

/// A launcher over `spawn`, its permit watched by a registry holding it
/// and judged by its policy (`allowed`: left enabled, or disabled).
fn launcher(spawn: Arc<Spawn>, allowed: bool) -> (SpawnCoordinatorLauncher, ToolRegistryImpl) {
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(spawn.clone());
    registry.watch_spawn_permit(permit.clone());
    if !allowed {
        registry.disable_tool("spawn");
    }
    (SpawnCoordinatorLauncher::new(spawn, permit), registry)
}

fn request() -> CoordinatorRequest {
    CoordinatorRequest {
        task: Slug::parse("demo-hello").unwrap(),
        run: 1,
        plan_version: 1,
        brief: "Create this swarm run".into(),
        model: Some("anthropic/claude-opus".into()),
        effort: None,
    }
}

/// `launcher` asked to launch, from a blocking thread of a runtime, as a
/// board call runs.
async fn launched(
    launcher: Arc<SpawnCoordinatorLauncher>,
) -> Result<LaunchedCoordinator, LaunchRefused> {
    tokio::task::spawn_blocking(move || launcher.launch(&request()))
        .await
        .unwrap()
}

const RUNNING: &str = "Subagent 'board-demo-hello-run-1' is running (uuid=0b5e) \
     environment_ref=C4 container_config=standard workspace=/w. Use agent_cmd to interact.";

#[tokio::test(flavor = "multi_thread")]
async fn a_coordinator_launches_through_spawn_while_the_policy_allows_it() {
    let spawned = spawn(RUNNING, false);
    let (launching, _registry) = launcher(spawned.clone(), true);
    assert!(launching.may_launch());
    assert_eq!(
        launched(Arc::new(launching)).await,
        Ok(LaunchedCoordinator {
            agent: "0b5e".into(),
            environment_ref: "C4".into(),
        })
    );
    let calls = spawned.calls.lock().unwrap().clone();
    let call: serde_json::Value = serde_json::from_str(&calls[0]).unwrap();
    assert_eq!(
        (
            &call["coordinator"],
            &call["container"],
            &call["task"],
            &call["model"]
        ),
        (
            &serde_json::json!(true),
            &serde_json::json!(true),
            &serde_json::json!("Create this swarm run"),
            &serde_json::json!("anthropic/claude-opus")
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_policy_that_denies_spawn_launches_nothing() {
    let spawned = spawn(RUNNING, false);
    let (launching, _registry) = launcher(spawned.clone(), false);
    assert!(!launching.may_launch());
    let refused = launched(Arc::new(launching)).await;
    assert!(matches!(&refused, Err(LaunchRefused(why)) if why.contains("does not allow spawn")));
    assert!(
        spawned.calls.lock().unwrap().is_empty(),
        "spawn never asked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn spawns_refusal_or_an_answer_naming_no_container_is_a_refusal() {
    let failing = spawn("no container runtime is available", true);
    let (launching, _registry) = launcher(failing, true);
    assert_eq!(
        launched(Arc::new(launching)).await,
        Err(LaunchRefused("no container runtime is available".into()))
    );
    let local = spawn(
        "Subagent 'x' is running (uuid=0b5e). Use agent_cmd to interact.",
        false,
    );
    let (launching, _registry) = launcher(local, true);
    let refused = launched(Arc::new(launching)).await;
    assert!(
        matches!(&refused, Err(LaunchRefused(why)) if why.starts_with("spawn answered no running coordinator")),
        "{refused:?}"
    );
}
