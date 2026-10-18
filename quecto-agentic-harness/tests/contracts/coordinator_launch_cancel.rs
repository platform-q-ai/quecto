//! S4c review round 2 (H-A): a cancel during the launch, through the real
//! launcher. A spawn that never answers is dropped once the call is
//! cancelled, and the start is undone. A spawn that came up as the call was
//! cancelled is stopped.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{agent_cmd, launched_child, launcher_over, request};
use quecto::application::project_board::dto::{
    NewBoardTask, NewBoardTaskStatus, PlanAcceptance, PlanProposal, ProjectBoardError,
    RunAfterUndo, TaskStart,
};
use quecto::application::project_board::ports::{
    CoordinatorLaunch, LaunchRefused, ProjectBoardStore,
};
use quecto::application::project_board::use_cases::{
    AcceptPlan, CreateBoardTask, ProposePlan, StartBoardTask,
};
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::entities::plan::PlanItem;
use quecto::domain::project_board::entities::team::{Role, RoleLimits, Team};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::vocabulary::{
    Effort, RunOutcome, TaskKind, TaskStatus,
};
use quecto::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use quecto::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use quecto::infrastructure::time::board_clock::SystemBoardClock;

/// Sets its flag when dropped: the spawn's work was abandoned.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A `spawn` whose work never answers, noting when it is dropped.
struct Stalling(Arc<AtomicBool>);

impl Tool for Stalling {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "a spawn that never answers".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let dropped = Dropped(Arc::clone(&self.0));
        Box::pin(async move {
            let _held = dropped;
            std::future::pending::<Result<ToolResult, DomainError>>().await
        })
    }
}

const CANCELLED: &str =
    "the call was cancelled while the coordinator launched: the launch was dropped";

/// `cancelled` set after `after`, from another thread.
fn cancel_after(cancelled: &Arc<AtomicBool>, after: Duration) {
    let cancelled = Arc::clone(cancelled);
    std::thread::spawn(move || {
        std::thread::sleep(after);
        cancelled.store(true, Ordering::SeqCst);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_the_launch_drops_spawns_work() {
    let dropped = Arc::new(AtomicBool::new(false));
    let spawn = Arc::new(Stalling(Arc::clone(&dropped)));
    let (launching, _registry) = launcher_over(spawn, agent_cmd());
    let cancelled = Arc::new(AtomicBool::new(false));
    cancel_after(&cancelled, Duration::from_millis(300));
    let began = Instant::now();
    let flag = Arc::clone(&cancelled);
    let refused = tokio::task::spawn_blocking(move || launching.launch(&request(), &flag))
        .await
        .unwrap();
    assert_eq!(refused, Err(LaunchRefused::because(CANCELLED)));
    assert!(dropped.load(Ordering::SeqCst), "spawn's work was dropped");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "given up within a poll"
    );
}

/// A spawn that answers its coordinator running as the call is cancelled.
struct CameUpAsCancelled(Arc<AtomicBool>);

impl Tool for CameUpAsCancelled {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "a spawn racing a cancel".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        self.0.store(true, Ordering::SeqCst);
        let result = ToolResult {
            content: "running".into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: Some(launched_child(Some("C4"))),
        };
        Box::pin(async move { Ok(result) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_coordinator_that_came_up_as_the_call_was_cancelled_is_stopped() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let spawn = Arc::new(CameUpAsCancelled(Arc::clone(&cancelled)));
    let stopper = agent_cmd();
    let (launching, _registry) = launcher_over(spawn, stopper.clone());
    let flag = Arc::clone(&cancelled);
    let refused = tokio::task::spawn_blocking(move || launching.launch(&request(), &flag))
        .await
        .unwrap();
    assert_eq!(
        refused,
        Err(LaunchRefused::because(
            "the call was cancelled as the coordinator came up, so it was stopped"
        ))
    );
    let kill: serde_json::Value = serde_json::from_str(&stopper.calls.lock().unwrap()[0]).unwrap();
    assert_eq!(
        kill,
        serde_json::json!({"agent_id": "0b5e", "command": "kill"})
    );
}

/// A board holding task `a`, ready, its plan's version 1 accepted.
fn ready_board(store: &MemoryBoardStore) {
    let clock = Arc::new(SystemBoardClock::default());
    let ada: Arc<dyn ProjectBoardStore> = Arc::new(store.client());
    store.bootstrap().unwrap();
    let id = Slug::parse("a").unwrap();
    CreateBoardTask::new(Arc::clone(&ada), clock.clone())
        .unwrap()
        .execute(&NewBoardTask {
            id: id.clone(),
            title: "A".into(),
            kind: TaskKind::Task,
            description: String::new(),
            status: NewBoardTaskStatus::Ready,
            parent: None,
            depends_on: vec![],
        })
        .unwrap();
    let role = |name: &str, model: &str| Role {
        name: Slug::parse(name).unwrap(),
        model: model.into(),
        effort: Effort::High,
        limits: RoleLimits {
            members: 1,
            turns: None,
        },
    };
    let plan = ProposePlan::new(Arc::clone(&ada), clock.clone())
        .unwrap()
        .execute(&PlanProposal {
            task: id.clone(),
            approach: "Build it.".into(),
            items: vec![PlanItem {
                id: Slug::parse("schema").unwrap(),
                title: "Schema".into(),
                criteria: vec!["migrates".into()],
                depends_on: vec![],
            }],
            team: Some(Team {
                roles: vec![
                    role("coordinator", "anthropic/claude-opus"),
                    role("builder", "gpt-5.5"),
                ],
                deadline_seconds: None,
                image: None,
            }),
        })
        .unwrap();
    AcceptPlan::new(ada, clock)
        .unwrap()
        .execute(&PlanAcceptance {
            task: id,
            version: 1,
            expected_updated: plan.fields().updated.clone(),
            expected_accepted: None,
        })
        .unwrap();
}

/// The start a cancel reaches during the launch, through the real
/// launcher: spawn's work is dropped, and the start undone past the
/// cancel, run 1 folded back and the claim released.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_cancelled_during_its_launch_drops_it_and_is_undone() {
    let store = MemoryBoardStore::default();
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    ready_board(&store.client_as(ada.clone()));
    let dropped = Arc::new(AtomicBool::new(false));
    let spawn = Arc::new(Stalling(Arc::clone(&dropped)));
    let (launching, _registry) = launcher_over(spawn, agent_cmd());
    let launching: Arc<dyn CoordinatorLaunch> = Arc::new(launching);
    let cancelled = Arc::new(AtomicBool::new(false));
    let starting = StartBoardTask::new(
        Arc::new(store.client_as(ada.clone())),
        Arc::new(store.client_as(ada)),
        Arc::new(SystemBoardClock::default()),
        Arc::new(MarkdownTextCells),
        Some(launching),
        Some(Arc::clone(&cancelled)),
    )
    .unwrap();
    cancel_after(&cancelled, Duration::from_millis(300));
    let start = TaskStart {
        task: Slug::parse("a").unwrap(),
        plan_version: 1,
    };
    let refused = tokio::task::spawn_blocking(move || starting.execute(&start))
        .await
        .unwrap();
    assert_eq!(
        refused.map(|_| ()),
        Err(ProjectBoardError::LaunchFailed {
            reason: CANCELLED.into(),
            run: 1,
            run_state: RunAfterUndo::Folded,
            released: Some(true),
            restored: None,
            still_running: None,
        })
    );
    assert!(dropped.load(Ordering::SeqCst), "spawn's work was dropped");
    let board = store.read().unwrap();
    assert_eq!(
        (
            board.tasks[0].fields().status,
            &board.tasks[0].fields().claim
        ),
        (TaskStatus::Ready, &None)
    );
    assert_eq!(board.runs[0].fields().outcome, Some(RunOutcome::Failed));
}
