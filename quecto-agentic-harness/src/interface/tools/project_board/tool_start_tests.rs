//! S4c: the tool starts a task with its session's launcher, moved beside
//! `tool_tests.rs` as it grew past its room.
use std::sync::Arc;

use serde_json::{Value, json};

use super::super::site::{BuildError, ProjectBoardEnvironment, ProjectBoardHandles};
use super::BoardSite;
use super::tests::{recorded_tool, site, tool};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::QueryListings;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::answer::start_steps as steps;
use crate::interface::tools::project_board::recording::tests::Records;

/// S4c: the board a test's launcher starts tasks on: task `a` ready with
/// version 1 of its plan accepted (a coordinator and a builder, one item).
fn launching_board(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    use crate::application::project_board::operations::query_listings::assembled_launching;
    use crate::application::project_board::ports::BoardArtifact;
    use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
    use crate::domain::project_board::entities::task::{Task, TaskFields};
    use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
    use crate::domain::project_board::value_objects::slug::Slug;
    use crate::domain::project_board::value_objects::timestamp::Timestamp;
    use crate::domain::project_board::value_objects::vocabulary::{Effort, PlanState};
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let slug = |text: &str| Slug::parse(text).unwrap();
    let at = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    let ada = crate::domain::project_board::entities::claim::Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let role = |name: &str, members| Role {
        name: slug(name),
        // The coordinator's model names its provider, as spawn takes it.
        model: match name {
            "coordinator" => "openai-oauth/gpt-5.5".into(),
            _ => "gpt-5.5".into(),
        },
        effort: Effort::High,
        limits: RoleLimits {
            members,
            turns: None,
        },
    };
    let plan = Plan::new(PlanFields {
        task: slug("a"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Build it.".into(),
        items: vec![PlanItem {
            id: slug("schema"),
            title: "Schema".into(),
            criteria: vec!["migrates".into()],
            depends_on: vec![],
        }],
        team: Some(Team {
            roles: vec![role("coordinator", 1), role("builder", 2)],
            deadline_seconds: None,
            image: None,
        }),
        proposed_by: ada.clone(),
        edited_by: None,
        accepted: Some((ada, at.clone())),
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
    let head = store.read().unwrap().head;
    let seed = [BoardArtifact::Task(task), BoardArtifact::Plan(plan)];
    store.write(&head, &seed, "seed").unwrap();
    let operations = assembled_launching(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
        environment.launcher.clone().map(|swarms| {
            swarms as Arc<dyn crate::application::project_board::ports::CoordinatorLaunch>
        }),
        environment
            .launcher
            .clone()
            .map(|swarms| swarms as Arc<dyn crate::application::project_board::ports::RunLiveness>),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

/// A launcher that launches every coordinator it is asked to.
struct Launching;

impl crate::application::project_board::ports::RunLiveness for Launching {
    fn running_coordinator(
        &self,
        _: &crate::domain::project_board::value_objects::slug::Slug,
        _: u32,
    ) -> Option<String> {
        None
    }
}

impl crate::application::project_board::ports::CoordinatorLaunch for Launching {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        request: &crate::application::project_board::ports::CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<
        crate::application::project_board::ports::LaunchedCoordinator,
        crate::application::project_board::ports::LaunchRefused,
    > {
        Ok(
            crate::application::project_board::ports::LaunchedCoordinator {
                agent: format!("agent-{}", request.task.as_str()),
                environment_ref: "C3".into(),
            },
        )
    }
}

/// S4c: the tool starts a task with its session's launcher, answers the
/// run and the coordinator, and records the start as a `board_op` and a
/// `board_start` of counts and kinds; with no launcher (a session that may
/// not spawn) it starts nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_starts_a_task_with_its_sessions_launcher() {
    let records = Arc::new(Records::default());
    let launching = BoardSite {
        launcher: Some(Arc::new(Launching)),
        ..site(Some(launching_board))
    };
    let tool = recorded_tool(launching, &records);
    let request = json!({"op": "start", "task": "a", "plan_version": 1});
    let started = tool.execute(&request.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&started.content).unwrap();
    assert!(!started.is_error, "{answer}");
    assert_eq!(
        (
            &answer["task"]["status"],
            &answer["run"]["number"],
            &answer["run"]["state"],
            &answer["coordinator"],
            &answer["claim"]
        ),
        (
            &json!("in_progress"),
            &json!(1),
            &json!("running"),
            &json!({"agent": "agent-a", "environment_ref": "C3"}),
            &json!("fresh")
        ),
        "{answer}"
    );
    assert_eq!(
        answer["not_applied"],
        json!([{"role": "builder", "model": "gpt-5.5", "effort": "high"}])
    );
    // #2656: when to renew the claim: halfway through its two hours.
    let since = answer["task"]["claim"]["since"]
        .as_str()
        .unwrap()
        .to_string();
    let halfway = crate::domain::project_board::value_objects::timestamp::Timestamp::parse(&since)
        .unwrap()
        .plus_seconds(60 * 60)
        .unwrap();
    assert_eq!(answer["renew_after"], json!(halfway.as_str()), "{answer}");
    // The steps for running it, the start prompt's words for its run.
    assert_eq!(answer["steps"], json!(steps::running(&slug_of("a"), 1)));
    let ops = records.taken();
    assert_eq!(ops.len(), 1);
    assert_eq!(
        (ops[0].op.as_str(), ops[0].plan_version),
        ("start", Some(1))
    );
    let starts = records.starts();
    assert_eq!(starts.len(), 1);
    let line = serde_json::to_value(crate::domain::audit::AuditEvent::BoardStart(Box::new(
        starts[0].clone(),
    )))
    .unwrap();
    assert_eq!(
        [
            &line["task"],
            &line["run"],
            &line["plan_version"],
            &line["members"],
            &line["items"],
            &line["claim"]
        ],
        [
            &json!("a"),
            &json!(1),
            &json!(1),
            &json!(3),
            &json!(1),
            &json!("fresh")
        ],
        "{line}"
    );
    assert!(line.get("launch_us").is_some(), "{line}");
    // Who the call is for: the agent itself, unless the relay names
    // itself in the harness's environment.
    let client = crate::interface::tools::project_board::recording::named_client(
        std::env::var_os(crate::interface::tools::project_board::recording::CLIENT_VARIABLE)
            .as_deref(),
        crate::domain::project_board::value_objects::telemetry::Client::Agent,
    );
    assert_eq!((ops[0].client, starts[0].client), (client, client));
    let unlaunched = tool_without_launcher()
        .execute(&request.to_string())
        .await
        .unwrap();
    let answer: Value = serde_json::from_str(&unlaunched.content).unwrap();
    assert_eq!(answer["error"]["code"], json!("no_launcher"), "{answer}");
    assert_eq!(answer["error"]["steps"], json!(steps::REFUSED), "{answer}");
}

fn slug_of(text: &str) -> crate::domain::project_board::value_objects::slug::Slug {
    crate::domain::project_board::value_objects::slug::Slug::parse(text).unwrap()
}

/// A start refused before the board answers carries the stop too; no
/// other op's refusal carries steps.
#[tokio::test(flavor = "multi_thread")]
async fn every_refused_start_says_to_stop_and_no_other_op_does() {
    let launching = BoardSite {
        launcher: Some(Arc::new(Launching)),
        ..site(Some(launching_board))
    };
    let board = tool(launching);
    let unread = json!({"op": "start", "task": "a"});
    let refused = board.execute(&unread.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["steps"]),
        (&json!("invalid_request"), &json!(steps::REFUSED)),
        "{answer}"
    );
    let unknown = json!({"op": "start", "task": "missing", "plan_version": 1});
    let refused = board.execute(&unknown.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["steps"]),
        (&json!("not_found"), &json!(steps::REFUSED)),
        "{answer}"
    );
    let uncomposed = tool(site(None));
    let refused = uncomposed.execute(&unknown.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["steps"]),
        (&json!("store_unavailable"), &json!(steps::REFUSED)),
        "{answer}"
    );
    for other in [
        json!({"op": "end_run", "task": "a", "run": 9, "outcome": "failed"}),
        json!({"op": "end_run", "task": "a"}),
        json!({"op": "show", "task": "missing"}),
    ] {
        let refused = board.execute(&other.to_string()).await.unwrap();
        let answer: Value = serde_json::from_str(&refused.content).unwrap();
        assert_eq!(answer["ok"], json!(false), "{answer}");
        assert!(answer["error"].get("steps").is_none(), "{answer}");
    }
    let listed = board
        .execute(&json!({"op": "list"}).to_string())
        .await
        .unwrap();
    let answer: Value = serde_json::from_str(&listed.content).unwrap();
    assert!(answer.get("steps").is_none(), "{answer}");
}

fn tool_without_launcher() -> super::BoardTool {
    tool(site(Some(launching_board)))
}

/// A launcher whose every launch fails.
struct Failing;

impl crate::application::project_board::ports::RunLiveness for Failing {
    fn running_coordinator(
        &self,
        _: &crate::domain::project_board::value_objects::slug::Slug,
        _: u32,
    ) -> Option<String> {
        None
    }
}

impl crate::application::project_board::ports::CoordinatorLaunch for Failing {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &crate::application::project_board::ports::CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<
        crate::application::project_board::ports::LaunchedCoordinator,
        crate::application::project_board::ports::LaunchRefused,
    > {
        Err(
            crate::application::project_board::ports::LaunchRefused::because(
                "no container runtime",
            ),
        )
    }
}

/// S4c review round 1: a start whose launch failed answers where its undo
/// left the run, and records the undo: `folded` and `released` on its
/// `board_start`, and the fold-back's own `board_fold_back`.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_launch_records_its_undo() {
    let records = Arc::new(Records::default());
    let failing = BoardSite {
        launcher: Some(Arc::new(Failing)),
        ..site(Some(launching_board))
    };
    let tool = recorded_tool(failing, &records);
    let request = json!({"op": "start", "task": "a", "plan_version": 1});
    let refused = tool.execute(&request.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert!(refused.is_error, "{answer}");
    assert_eq!(
        (
            &answer["error"]["code"],
            &answer["error"]["run_state"],
            &answer["error"]["released"]
        ),
        (&json!("launch_failed"), &json!("folded"), &json!(true)),
        "{answer}"
    );
    assert_eq!(
        answer["error"]["steps"],
        json!(steps::undo(&slug_of("a"))),
        "the undo steps: {answer}"
    );
    let starts = records.starts();
    assert_eq!(
        (
            starts.len(),
            starts[0].run_state.map(|kind| kind.as_str()),
            starts[0].released
        ),
        (1, Some("folded"), Some(true))
    );
    let folds = records.fold_backs();
    assert_eq!(folds.len(), 1, "the undo's fold-back is recorded");
    let line = serde_json::to_value(crate::domain::audit::AuditEvent::BoardFoldBack(Box::new(
        folds[0].clone(),
    )))
    .unwrap();
    assert_eq!(
        (
            &line["task"],
            &line["run"],
            &line["outcome"],
            &line["run_outcome"]
        ),
        (&json!("a"), &json!(1), &json!("ok"), &json!("failed")),
        "{line}"
    );
}

/// A launcher whose session still runs run 1's coordinator of task `a`.
struct StillRunning;

impl crate::application::project_board::ports::RunLiveness for StillRunning {
    fn running_coordinator(
        &self,
        task: &crate::domain::project_board::value_objects::slug::Slug,
        run: u32,
    ) -> Option<String> {
        ((task.as_str(), run) == ("a", 1)).then(|| "7f3a".into())
    }
}

impl crate::application::project_board::ports::CoordinatorLaunch for StillRunning {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &crate::application::project_board::ports::CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<
        crate::application::project_board::ports::LaunchedCoordinator,
        crate::application::project_board::ports::LaunchRefused,
    > {
        Err(crate::application::project_board::ports::LaunchRefused::because("not in this test"))
    }
}

/// Review rounds 2-3 (M-A, M2): the tool's end_run of a run whose swarm
/// this session still runs is refused by the use case (`run_live`), and
/// recorded; another run reaches the board's rules.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_refuses_to_end_a_run_its_session_still_runs() {
    let records = Arc::new(Records::default());
    let running = BoardSite {
        launcher: Some(Arc::new(StillRunning)),
        ..site(Some(launching_board))
    };
    let tool = recorded_tool(running, &records);
    let request = json!({"op": "end_run", "task": "a", "run": 1, "outcome": "failed"});
    let refused = tool.execute(&request.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&refused.content).unwrap();
    assert!(refused.is_error, "{answer}");
    assert_eq!(answer["error"]["code"], json!("run_live"), "{answer}");
    assert_eq!(
        (
            &answer["error"]["task"],
            &answer["error"]["run"],
            &answer["error"]["agent"]
        ),
        (&json!("a"), &json!(1), &json!("7f3a")),
        "{answer}"
    );
    assert_eq!(
        answer["error"]["message"],
        json!(
            "run 1 of a still has its swarm running in this session: stop its coordinator \
             first (agent_cmd kill 7f3a), then end the run"
        )
    );
    let ops = records.taken();
    let line = serde_json::to_value(&ops[0]).unwrap();
    assert_eq!(
        (&line["op"], &line["code"]),
        (&json!("end_run"), &json!("run_live"))
    );
    let other = json!({"op": "end_run", "task": "a", "run": 2, "outcome": "failed"});
    let answered = tool.execute(&other.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&answered.content).unwrap();
    assert_eq!(
        answer["error"]["code"],
        json!("rule"),
        "the board's: {answer}"
    );
}

/// Review round 2 (#2567): a start the strict read refuses (a key named
/// twice, a request past its bytes) is still a start: its refusal names
/// the op and carries the stop. When the op itself cannot be known (named
/// twice), the refusal names none and carries no steps.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_the_strict_read_refuses_still_says_to_stop() {
    let board = tool(site(Some(launching_board)));
    let pad = "x".repeat(super::super::MAX_REQUEST_BYTES);
    for text in [
        r#"{"op": "start", "task": "a", "task": "b", "plan_version": 1}"#.to_owned(),
        format!(r#"{{"op": "start", "task": "a", "plan_version": 1, "pad": "{pad}"}}"#),
    ] {
        let refused = board.execute(&text).await.unwrap();
        let answer: Value = serde_json::from_str(&refused.content).unwrap();
        assert_eq!(
            (
                &answer["op"],
                &answer["error"]["code"],
                &answer["error"]["steps"]
            ),
            (
                &json!("start"),
                &json!("invalid_request"),
                &json!(steps::REFUSED)
            ),
            "{}",
            &text[..60]
        );
    }
    // Review round 3: an `op` named twice, an array (never a request,
    // however it pads), and another op's strict refusal name no op and
    // carry no steps, through the tool and through the site path.
    let array = format!(
        r#"["start"{}]"#,
        " ".repeat(super::super::MAX_REQUEST_BYTES)
    );
    for text in [
        r#"{"op": "start", "op": "start", "task": "a", "plan_version": 1}"#.to_owned(),
        array,
        r#"{"op": "move", "task": "a", "to": "ready", "to": "done"}"#.to_owned(),
    ] {
        let refused = board.execute(&text).await.unwrap();
        let answer: Value = serde_json::from_str(&refused.content).unwrap();
        assert_eq!(answer["op"], json!(null), "{}", &text[..40]);
        assert!(answer["error"].get("steps").is_none(), "{}", &text[..40]);
        let mut call = super::super::recording::BoardCall::unrecorded();
        let answer =
            super::super::site::answer(&site(Some(launching_board)), &text, None, &mut call);
        assert_eq!(answer["op"], json!(null), "site: {}", &text[..40]);
        assert!(
            answer["error"].get("steps").is_none(),
            "site: {}",
            &text[..40]
        );
    }
}
