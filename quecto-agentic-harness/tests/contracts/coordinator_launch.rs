//! The coordinator launch port's contract (S4c), run against its
//! spawn-backed adapter over a spawn that answers as told: it launches
//! only while the session's policy allows spawn, asks spawn for a
//! coordinator in a new container, and answers what spawn's structured
//! answer says runs, stopping a coordinator that came up outside one.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use quecto::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use quecto::infrastructure::tools::agents::coordinator_launch::SpawnCoordinatorLauncher;
use quecto::infrastructure::tools::agents::coordinator_launch::SpawnPermit;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;

/// A `spawn` (or, named so, `agent_cmd`) that answers `content` (an
/// error when `failing`) with `metadata`, and keeps the arguments of each
/// call.
struct Spawn {
    name: &'static str,
    content: String,
    failing: bool,
    metadata: Option<String>,
    calls: Mutex<Vec<String>>,
}

impl Tool for Spawn {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
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
            delivery_metadata: self.metadata.clone(),
        };
        Box::pin(async move { Ok(result) })
    }
}

fn spawn(content: &str, failing: bool) -> Arc<Spawn> {
    spawned_with(content, failing, None)
}

fn spawned_with(content: &str, failing: bool, metadata: Option<&str>) -> Arc<Spawn> {
    Arc::new(Spawn {
        name: "spawn",
        content: content.into(),
        failing,
        metadata: metadata.map(str::to_owned),
        calls: Mutex::default(),
    })
}

/// What spawn's structured answer says of a child it launched.
fn launched_child(environment_ref: Option<&str>) -> String {
    serde_json::json!({"spawned": {"uuid": "0b5e", "environment_ref": environment_ref}}).to_string()
}

fn agent_cmd() -> Arc<Spawn> {
    Arc::new(Spawn {
        name: "agent_cmd",
        content: "killed".into(),
        failing: false,
        metadata: None,
        calls: Mutex::default(),
    })
}

/// A launcher over `spawn`, its permit watched by a registry holding it
/// and judged by its policy (`allowed`: left enabled, or disabled).
fn launcher(spawn: Arc<Spawn>, allowed: bool) -> (SpawnCoordinatorLauncher, ToolRegistryImpl) {
    launcher_stopping_with(spawn, agent_cmd(), allowed)
}

/// A launcher over any `spawn`, its policy allowing it, stopping with
/// `agent_cmd`.
fn launcher_over(
    spawn: Arc<dyn Tool>,
    agent_cmd: Arc<Spawn>,
) -> (SpawnCoordinatorLauncher, ToolRegistryImpl) {
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::clone(&spawn));
    registry.observe_policy(Arc::new(permit.clone()));
    let subagents = quecto::infrastructure::tools::agents::subagent_registry::new_registry();
    (
        SpawnCoordinatorLauncher::new(spawn, agent_cmd, permit, subagents),
        registry,
    )
}

fn launcher_stopping_with(
    spawn: Arc<Spawn>,
    agent_cmd: Arc<Spawn>,
    allowed: bool,
) -> (SpawnCoordinatorLauncher, ToolRegistryImpl) {
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(spawn.clone());
    registry.observe_policy(std::sync::Arc::new(permit.clone()));
    if !allowed {
        registry.disable_tool("spawn");
    }
    let subagents = quecto::infrastructure::tools::agents::subagent_registry::new_registry();
    (
        SpawnCoordinatorLauncher::new(spawn, agent_cmd, permit, subagents),
        registry,
    )
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
    tokio::task::spawn_blocking(move || launcher.launch(&request(), &AtomicBool::new(false)))
        .await
        .unwrap()
}

const RUNNING: &str = "Subagent 'board-demo-hello-run-1' is running (uuid=0b5e) \
     environment_ref=C4 container_config=standard workspace=/w. Use agent_cmd to interact.";

#[tokio::test(flavor = "multi_thread")]
async fn a_coordinator_launches_through_spawn_while_the_policy_allows_it() {
    let spawned = spawned_with(RUNNING, false, Some(&launched_child(Some("C4"))));
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
    assert!(
        matches!(&refused, Err(LaunchRefused { reason: why, .. }) if why.contains("does not allow spawn"))
    );
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
        Err(LaunchRefused::because("no container runtime is available"))
    );
    // Words alone launch nothing: only spawn's structured answer names a
    // coordinator; a uuid its words name is stopped.
    let named = agent_cmd();
    let (launching, _registry) = launcher_stopping_with(spawn(RUNNING, false), named.clone(), true);
    assert_eq!(
        launched(Arc::new(launching)).await,
        Err(LaunchRefused::because(
            "spawn named no container for the coordinator, so it was stopped"
        ))
    );
    assert_eq!(named.calls.lock().unwrap().len(), 1);
    let (launching, _registry) = launcher(spawn("odd", false), true);
    let refused = launched(Arc::new(launching)).await;
    assert!(
        matches!(&refused, Err(LaunchRefused { reason: why, .. }) if why.starts_with("spawn answered no running coordinator")),
        "{refused:?}"
    );
    // A coordinator outside a container is stopped, then refused.
    let local = spawned_with("running", false, Some(&launched_child(None)));
    let stopper = agent_cmd();
    let (launching, _registry) = launcher_stopping_with(local, stopper.clone(), true);
    let refused = launched(Arc::new(launching)).await;
    assert_eq!(
        refused,
        Err(LaunchRefused::because(
            "the coordinator came up outside a container, so it was stopped"
        ))
    );
    let kill: serde_json::Value = serde_json::from_str(&stopper.calls.lock().unwrap()[0]).unwrap();
    assert_eq!(
        kill,
        serde_json::json!({"agent_id": "0b5e", "command": "kill"})
    );
}

#[path = "coordinator_launch_cancel.rs"]
mod cancel;
