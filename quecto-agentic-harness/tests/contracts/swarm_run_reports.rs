//! The swarm run reports port's contract (#2481 S4c follow-up), run
//! against its spawn-backed adapter: a run's swarm is reported from the
//! board kept under the coordinator the launcher launched for that run,
//! and only when that board shows the run ended; a run it did not launch,
//! a launch refused, a board not kept or a run not ended reports nothing.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::EndedSwarmBoards;
use quecto::application::project_board::fold_back::{SwarmEnd, SwarmItemState};
use quecto::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, SwarmRunReports,
};
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::tool::{ToolDefinition, ToolResult};
use quecto::infrastructure::tools::coordinator_launch::{
    KeptSwarmBoards, SpawnCoordinatorLauncher, SpawnPermit,
};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use serde_json::{Value, json};

/// A `spawn` that launches coordinator `0b5e` in `C4`, or refuses.
struct Spawn {
    launches: bool,
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
        _: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let metadata = self
            .launches
            .then(|| json!({"spawned": {"uuid": "0b5e", "environment_ref": "C4"}}).to_string());
        let result = ToolResult {
            content: "spawned".into(),
            is_error: !self.launches,
            image_blocks: vec![],
            delivery_metadata: metadata,
        };
        Box::pin(async move { Ok(result) })
    }
}

/// A launcher over a spawn that `launches` or not, reading `kept`.
fn launcher(
    launches: bool,
    kept: &Arc<KeptSwarmBoards>,
) -> (Arc<SpawnCoordinatorLauncher>, ToolRegistryImpl) {
    let spawn: Arc<dyn Tool> = Arc::new(Spawn { launches });
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::clone(&spawn));
    registry.observe_policy(Arc::new(permit.clone()));
    let subagents = quecto::infrastructure::tools::subagent_registry::new_registry();
    let launcher =
        SpawnCoordinatorLauncher::new(spawn, Arc::new(Spawn { launches: true }), permit, subagents)
            .keeping(Arc::clone(kept));
    (Arc::new(launcher), registry)
}

fn task() -> Slug {
    Slug::parse("light-well-tanking").unwrap()
}

/// Run 1 of the task launched, from a blocking thread, as a board call runs.
async fn launch(launcher: &Arc<SpawnCoordinatorLauncher>) -> bool {
    let launcher = Arc::clone(launcher);
    tokio::task::spawn_blocking(move || {
        let request = CoordinatorRequest {
            task: task(),
            run: 1,
            plan_version: 1,
            brief: "the brief".into(),
            model: None,
            effort: None,
        };
        launcher.launch(&request, &AtomicBool::new(false)).is_ok()
    })
    .await
    .unwrap()
}

/// A board whose run is `status`, with one completed item.
fn board(status: &str) -> SwarmBoardSnapshot {
    let board: Value = json!({
        "run": {"status": status, "budget": {"observed_tokens": 52_000}},
        "items": [{"title": "assess-failure: Assess", "state": "completed", "owner": "w1",
                   "evidence": [{"artifact": "docs/a.md", "revision": "abc"}]}],
    });
    SwarmBoardSnapshot::object(board).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launched_runs_swarm_is_reported_from_its_coordinators_kept_board() {
    let kept = Arc::new(KeptSwarmBoards::default());
    let (launching, _registry) = launcher(true, &kept);
    assert_eq!(launching.ended_run(&task(), 1), None, "nothing launched");
    assert!(launch(&launching).await);
    assert_eq!(launching.ended_run(&task(), 1), None, "no board kept");
    kept.keep("0b5e", board("running"));
    assert_eq!(launching.ended_run(&task(), 1), None, "not ended");
    kept.keep("0b5e", board("succeeded"));
    let view = launching.ended_run(&task(), 1).expect("an ended run");
    assert_eq!(
        (view.end, view.observed_tokens, view.items[0].state),
        (SwarmEnd::Succeeded, 52_000, SwarmItemState::Completed)
    );
    assert_eq!(launching.ended_run(&task(), 2), None, "another run");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_launch_reports_nothing() {
    let kept = Arc::new(KeptSwarmBoards::default());
    let (launching, _registry) = launcher(false, &kept);
    assert!(!launch(&launching).await);
    kept.keep("0b5e", board("succeeded"));
    assert_eq!(launching.ended_run(&task(), 1), None);
}
