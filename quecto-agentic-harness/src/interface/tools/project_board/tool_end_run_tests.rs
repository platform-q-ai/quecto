//! Review round 1 (L5, #2575): the whole path a board run's swarm takes
//! to the board. The swarm seeds its items from the brief's titles and
//! completes them; its coordinator is killed, and the real final-member
//! teardown keeps its board under the session and the coordinator's label
//! before the box goes; then the agent tool's `end_run` records the run
//! from it, by the run's label, with no memory of the launch.
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::super::site::{BuildError, ProjectBoardEnvironment, ProjectBoardHandles};
use super::BoardSite;
use super::tests::tool;
use crate::application::environments::dto::{
    EnvironmentLiveness, HostedBoardRevision, SwarmBoardSnapshot,
};
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmBoard, HostedSwarmRunObservation, PortFuture,
};
use crate::application::environments::use_cases::FinalizeEnvironmentMember;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{
    QueryListings, Swarms, assembled_reporting,
};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::swarm_brief::brief_of;
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environments::services::environment_retention::{
    CoordinatorLoss, HostedSwarmRun, MemberFinalizeMode, SwarmRunObservation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::starts::{Started, start};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};
use crate::domain::shared::error::DomainError;
use crate::domain::swarm::value_objects::lifecycle::RunStatus;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::infrastructure::tools::agents::coordinator_launch::{
    KeptRunReports, KeptSwarmBoards, SpawnCoordinatorLauncher, SpawnPermit,
};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Task `a`, ready, with version 1 of its plan accepted: items `schema`
/// and `api`.
fn ready_with_plan() -> (Task, Plan) {
    let at = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    let item = |id: &str, title: &str| PlanItem {
        id: slug(id),
        title: title.into(),
        criteria: vec![format!("{id} proved")],
        depends_on: vec![],
    };
    let plan = Plan::new(PlanFields {
        task: slug("a"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Build it.".into(),
        items: vec![
            item("schema", "Write the schema"),
            item("api", "Serve the api"),
        ],
        team: None,
        proposed_by: ada(),
        edited_by: None,
        accepted: Some((ada(), at.clone())),
        created: at.clone(),
        updated: at.clone(),
    })
    .unwrap();
    let task = Task::new(TaskFields {
        id: slug("a"),
        title: "A".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap();
    (task, plan)
}

/// Task `a` started by Ada now: run 1 going on its accepted plan.
fn started_now() -> Started {
    let (task, plan) = ready_with_plan();
    let now = SystemBoardClock::default().now().unwrap();
    start(&task, &[], Some(&plan), &ada(), &now).unwrap()
}

/// The boards this test's teardown keeps, as composition keeps one store
/// for the teardown and the board's reports.
fn kept() -> Arc<KeptSwarmBoards> {
    static KEPT: std::sync::OnceLock<Arc<KeptSwarmBoards>> = std::sync::OnceLock::new();
    Arc::clone(KEPT.get_or_init(Arc::default))
}

/// The board as `start` left it (run 1 going, held by Ada), its ends read
/// from [`kept`] under the front end's session, as composition wires it.
fn started_board(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let plan = ready_with_plan().1;
    let (task, run) = started_now().into_writes();
    let head = store.read().unwrap().head;
    let seed = [
        BoardArtifact::Task(task),
        BoardArtifact::Plan(plan),
        BoardArtifact::Run(run),
    ];
    store.write(&head, &seed, "board: a started").unwrap();
    let launcher = environment.launcher.clone();
    let reports = environment.swarm_session.as_deref().map(|session| {
        Arc::new(KeptRunReports::new(kept(), session))
            as Arc<dyn crate::application::project_board::ports::SwarmRunReports>
    });
    let operations = assembled_reporting(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
        Swarms {
            launcher: launcher.clone().map(|swarms| swarms as _),
            liveness: launcher.map(|swarms| swarms as _),
            reports,
        },
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

/// The coordinator's box: it hosts a run closed as succeeded, whose board
/// is `board`, and records the scripts the teardown runs.
struct ClosedBox {
    board: Value,
    scripts: Mutex<Vec<&'static str>>,
}

impl HostedSwarmRunObservation for ClosedBox {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        Box::pin(async {
            SwarmRunObservation::Run(HostedSwarmRun {
                id: "run-1".into(),
                status: RunStatus::Succeeded,
                outcome: None,
                coordinator: "member-1".into(),
                deadline: 4_102_444_800.0,
            })
        })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
        _: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
        Box::pin(async { panic!("an owner's kill records no loss") })
    }
}

impl HostedSwarmBoard for ClosedBox {
    fn board<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let board = SwarmBoardSnapshot::object(self.board.clone());
        Box::pin(async move { Ok(board) })
    }

    fn revision<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async { panic!("a teardown never polls a revision") })
    }
}

impl EnvironmentProcessCommands for ClosedBox {
    fn run_retained_inspect<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        self.scripts.lock().unwrap().push("kill");
        Box::pin(async { Ok(()) })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        self.scripts.lock().unwrap().push("cleanup");
        Box::pin(async { Ok(()) })
    }

    fn observe_liveness<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness> {
        Box::pin(async { panic!("a teardown never asks liveness") })
    }
}

/// `registry` holding the coordinator's running box, `C1`.
fn coordinator_box(registry: &EnvironmentRegistry) -> String {
    let env_ref = registry.mint_ref().unwrap();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-1".into(),
        environment_uuid: "u1".into(),
        name: None,
        workspace_path: "/ws/C1".into(),
        repository: String::new(),
        script_name: "standard".into(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into()],
        retained_cleanup_argv: vec!["cleanup.sh".into()],
        retained_inspect_argv: vec![],
        members: vec!["coordinator-uuid".into()],
        status: EnvironmentStatus::Running,
        metadata: json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    env_ref
}

/// A tool never called here.
struct Unused;

impl Tool for Unused {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "unused".into(),
            description: "never called".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        Box::pin(async { Err(DomainError::Tool("never called".into())) })
    }
}

/// The agent tool over the started board, as the tool runtime builds it
/// for a session spawning into `environments`: its launcher built afresh
/// (as after a restart: it never launched run 1).
fn agent_tool(environments: &EnvironmentRegistry) -> super::BoardTool {
    let launcher = SpawnCoordinatorLauncher::new(
        Arc::new(Unused),
        Arc::new(Unused),
        SpawnPermit::default(),
        crate::infrastructure::tools::agents::subagent_registry::new_registry(),
    );
    tool(BoardSite::agent(
        ".".into(),
        started_board,
        Arc::new(launcher),
        environments,
    ))
}

/// `end_run(a, 1, succeeded)` through `tool`: its answer, and whether it
/// was an error.
async fn succeeded(tool: &super::BoardTool) -> (Value, bool) {
    let request = json!({"op": "end_run", "task": "a", "run": 1, "outcome": "succeeded"});
    let result = tool.execute(&request.to_string()).await.unwrap();
    (
        serde_json::from_str(&result.content).unwrap(),
        result.is_error,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_swarm_that_did_its_items_is_recorded_on_the_board_through_the_real_teardown() {
    // The swarm seeds its items with the brief's titles, and completes them.
    let brief = brief_of(&started_now()).unwrap();
    let items: Vec<Value> = brief
        .items
        .iter()
        .map(|item| {
            json!({"title": item.title, "state": "completed", "owner": "worker-1",
                   "evidence": [{"artifact": format!("docs/{}.md", item.id.as_str()),
                                 "revision": "abc123"}]})
        })
        .collect();
    assert_eq!(items.len(), 2, "one swarm item per plan item");
    let board = json!({
        "run": {"status": "succeeded", "budget": {"observed_tokens": 52_000}},
        "totals": {"items": items.len()},
        "items": items,
        "truncated": false,
    });
    let registry = EnvironmentRegistry::new();
    let session = registry.session().to_owned();
    let kept = kept();
    let tool = agent_tool(&registry);

    // Before the teardown, nothing of the swarm reached the board: a
    // success is refused, and nothing is written.
    let (refused, is_error) = succeeded(&tool).await;
    assert!(is_error, "{refused}");
    assert_eq!(
        (
            &refused["error"]["rule"]["code"],
            &refused["error"]["rule"]["report"]
        ),
        (&json!("items_not_done"), &json!("none_kept")),
        "{refused}"
    );

    // The owner kills the closed run's coordinator: the real teardown keeps
    // its board under the session and the label, then tears the box down.
    let closed = Arc::new(ClosedBox {
        board,
        scripts: Mutex::default(),
    });
    let env_ref = coordinator_box(&registry);
    let teardown = FinalizeEnvironmentMember::new(registry.clone(), closed.clone(), closed.clone())
        .keeping_ended_boards(closed.clone(), Arc::clone(&kept) as _);
    teardown
        .finalize_labelled_member(
            &env_ref,
            "coordinator-uuid",
            Some("board-a-run-1"),
            None,
            MemberFinalizeMode::OwnerEnd,
        )
        .await;
    assert_eq!(
        closed.scripts.lock().unwrap().as_slice(),
        ["kill"],
        "the box went"
    );

    let (answer, is_error) = succeeded(&tool).await;
    assert!(!is_error, "{answer}");
    let run = &answer["run"];
    assert_eq!(run["state"], json!("succeeded"), "{answer}");
    assert_eq!(run["cost_tokens"], json!(52_000), "{answer}");
    let items = run["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{answer}");
    for (item, id) in items.iter().zip(["schema", "api"]) {
        assert_eq!(
            (&item["id"], &item["state"], &item["owner"]),
            (&json!(id), &json!("done"), &json!("worker-1")),
            "{answer}"
        );
        assert_eq!(
            item["evidence"],
            json!([format!("docs/{id}.md at abc123")]),
            "{answer}"
        );
    }
    assert_eq!(answer.get("items_recorded"), None, "no todo line: {answer}");
    assert_eq!(answer["liveness"], json!("checked"));
    assert_eq!(
        kept.view(&session, "board-a-run-1"),
        None,
        "dropped once recorded"
    );
}
