//! A listing's `summary` per task, opt-in on the shared path (the CLI's
//! `--summary`): where the task's plan and latest run stand, in the words
//! and times `show` gives them. Without it, and always on the agent tool,
//! a listing answers as it did before; `show` is unchanged; a listing
//! whose summaries cannot be read is answered without them, saying why.
use serde_json::{Value, json};
use std::sync::Arc;

use super::tests::{site, tool};
use crate::application::project_board::operations::ListSummaries;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardWrite, ProjectBoardStore,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
use crate::composition::bootstrap::project_board_operations::fixtures::{QueryListings, assembled};
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::run::{Run, RunFields, RunItem};
use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, PlanState, RunOutcome,
};
use crate::infrastructure::test_support::board_artifacts::{plan, run};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::BoardCall;
use crate::interface::tools::project_board::run_recorded_with;
use crate::interface::tools::project_board::site::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

/// Version `version` of `task`'s plan in `state`, of items `build` and
/// `test`, its team's deadline `deadline` (none: a team naming none).
fn plan_in(task: &str, version: u32, state: PlanState, deadline: Option<u32>) -> Plan {
    let base = plan(task, version).into_fields();
    let accepted = matches!(state, PlanState::Accepted | PlanState::Superseded)
        .then(|| (base.proposed_by.clone(), base.created.clone()));
    let item = |id: &str| PlanItem {
        id: slug(id),
        title: format!("Item {id}"),
        criteria: vec![format!("{id} proved")],
        depends_on: vec![],
    };
    let team = Team {
        roles: vec![Role {
            name: slug("worker"),
            model: "openai/gpt-5.5".into(),
            effort: Effort::High,
            limits: RoleLimits {
                members: 2,
                turns: None,
            },
        }],
        deadline_seconds: deadline,
        image: None,
    };
    Plan::new(PlanFields {
        state,
        accepted,
        items: vec![item("build"), item("test")],
        team: Some(team),
        ..base
    })
    .unwrap()
}

/// Run `number` of `task`'s plan 1 (started 09:00), ended at 09:30 as
/// `outcome` with `items`, or running.
fn run_of(task: &str, number: u32, ended: Option<(RunOutcome, &[(&str, ItemState)])>) -> Run {
    let base = run(task, number).into_fields();
    let fields = match ended {
        None => base,
        Some((outcome, items)) => RunFields {
            ended: Some(at("09:30:00")),
            outcome: Some(outcome),
            cost: Some(1_000),
            items: items
                .iter()
                .map(|(id, state)| RunItem {
                    id: slug(id),
                    state: *state,
                    owner: None,
                    evidence: vec![],
                })
                .collect(),
            ..base
        },
    };
    Run::new(fields).unwrap()
}

/// A board of three tasks: `a`, whose accepted plan's team gives an hour
/// and whose latest run failed with its items reported; `b`, whose
/// newest plan is a draft over the accepted one, with a run going on a
/// team naming no deadline; and `c`, with nothing but a run file set aside.
fn summarised_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    for id in ["a", "b", "c"] {
        let request = json!({"op": "create", "task": id, "title": id, "status": "ready"});
        let created =
            crate::interface::tools::project_board::run(&operations, &request.to_string());
        assert_eq!(created["ok"], json!(true), "{created}");
    }
    let reported: &[(&str, ItemState)] =
        &[("build", ItemState::Done), ("test", ItemState::Blocked)];
    let artifacts = vec![
        BoardArtifact::Plan(plan_in("a", 1, PlanState::Accepted, Some(3_600))),
        BoardArtifact::Run(run_of("a", 1, Some((RunOutcome::Stopped, &[])))),
        BoardArtifact::Run(run_of("a", 2, Some((RunOutcome::Failed, reported)))),
        BoardArtifact::Plan(plan_in("b", 1, PlanState::Accepted, None)),
        BoardArtifact::Plan(plan_in("b", 2, PlanState::Draft, Some(600))),
        BoardArtifact::Run(run_of("b", 1, None)),
    ];
    let head = store.read().unwrap().head;
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store.quarantine_key(ArtifactKey::Run(slug("c"), 1), "runs/c.1.md", "malformed");
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

fn environment() -> ProjectBoardEnvironment {
    ProjectBoardEnvironment {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        known: Default::default(),
        access: crate::interface::tools::project_board::site::Access::Read,
        vars: vec![],
        cancelled: None,
        launcher: None,
        swarm_session: None,
    }
}

/// The shared path's answer to `request`, summaries asked for as
/// `summaries` says, as the CLI gives it.
fn shared_with(request: Value, summaries: ListSummaries) -> Value {
    let handles = summarised_board(&environment()).unwrap();
    let mut call = BoardCall::unrecorded();
    run_recorded_with(
        &*handles.operations,
        &request.to_string(),
        summaries,
        &mut call,
    )
}

/// [`shared_with`] summaries.
fn shared(request: Value) -> Value {
    shared_with(request, ListSummaries::Wanted)
}

/// [`shared_with`] no summaries: what every listing answered before.
fn plain(request: Value) -> Value {
    shared_with(request, ListSummaries::Omitted)
}

fn summaries(answer: &Value) -> Vec<(String, Value)> {
    answer["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            (
                task["id"].as_str().unwrap().to_string(),
                task["summary"].clone(),
            )
        })
        .collect()
}

#[test]
fn a_listing_carries_each_tasks_summary() {
    let listed = shared(json!({"op": "list"}));
    assert_eq!(listed["ok"], json!(true), "{listed}");
    let items = |done: u32, blocked: u32| json!({"total": done + blocked, "todo": 0, "doing": 0, "review": 0, "done": done, "blocked": blocked});
    assert_eq!(
        summaries(&listed),
        [
            (
                "a".to_string(),
                json!({
                    "subtitle": null,
                    "plan": {"version": 1, "state": "accepted"},
                    "run": {
                        "number": 2,
                        "plan_version": 1,
                        "state": "failed",
                        "started": "2026-10-10T09:00:00Z",
                        "ended": "2026-10-10T09:30:00Z",
                        "deadline": "2026-10-10T10:00:00Z",
                        "items": items(1, 1),
                    },
                    "may_be_incomplete": false,
                })
            ),
            (
                "b".to_string(),
                json!({
                    "subtitle": null,
                    "plan": {"version": 2, "state": "draft"},
                    "run": {
                        "number": 1,
                        "plan_version": 1,
                        "state": "running",
                        "started": "2026-10-10T09:00:00Z",
                        "ended": null,
                        "deadline": "2026-10-11T09:00:00Z",
                        "items": items(0, 0),
                    },
                    "may_be_incomplete": false,
                })
            ),
            (
                "c".to_string(),
                json!({"subtitle": null, "plan": null, "run": null, "may_be_incomplete": true})
            ),
        ]
    );
    assert_eq!(listed.get("summary_skipped"), None, "{listed}");
    // Every listing op carries it.
    for request in [
        json!({"op": "list_ready"}),
        json!({"op": "list_takeable"}),
        json!({"op": "list", "status": "ready"}),
    ] {
        let request = match request["op"].as_str() {
            // No claim has expired: a takeable listing lists none.
            Some("list_takeable") => {
                let listed = shared(request);
                assert_eq!(listed["ok"], json!(true), "{listed}");
                assert_eq!(listed["tasks"], json!([]), "{listed}");
                continue;
            }
            Some(_) | None => request,
        };
        assert_eq!(
            summaries(&shared(request.clone())),
            summaries(&listed),
            "{request}"
        );
    }
}

/// A summary's run is `show`'s latest run in the same words and times,
/// and its `may_be_incomplete` is `show`'s; `show` carries no summary.
#[test]
fn a_summary_agrees_with_show_and_show_is_unchanged() {
    let listed = shared(json!({"op": "list"}));
    for (id, summary) in summaries(&listed) {
        let shown = plain(json!({"op": "show", "task": id}));
        assert_eq!(shown["ok"], json!(true), "{shown}");
        let mut keys: Vec<&str> = shown
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "head",
                "may_be_incomplete",
                "ok",
                "op",
                "plan",
                "plans",
                "quarantined",
                "runs",
                "task"
            ]
        );
        assert_eq!(
            shown["task"].get("summary"),
            None,
            "show's task is the task alone"
        );
        assert_eq!(shown["may_be_incomplete"], summary["may_be_incomplete"]);
        let latest = shown["runs"].as_array().unwrap().last().cloned();
        match latest {
            None => assert_eq!(summary["run"], Value::Null),
            Some(latest) => {
                for field in ["number", "plan_version", "state", "started", "ended"] {
                    assert_eq!(summary["run"][field], latest[field], "{id} {field}");
                }
            }
        }
    }
}

/// Without `--summary` a listing answers exactly as before: no task
/// carries a summary and nothing is said of one.
#[test]
fn a_listing_without_summaries_is_as_before() {
    for request in [
        json!({"op": "list"}),
        json!({"op": "list_ready"}),
        json!({"op": "list_takeable"}),
    ] {
        // One board for both: its times are the clock's.
        let handles = summarised_board(&environment()).unwrap();
        let text = request.to_string();
        let mut call = BoardCall::unrecorded();
        let listed = run_recorded_with(
            &*handles.operations,
            &text,
            ListSummaries::Omitted,
            &mut call,
        );
        assert_eq!(listed["ok"], json!(true), "{listed}");
        // A takeable listing's clock note is its own, as ever.
        let mut keys: Vec<&str> = listed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .filter(|key| *key != "clock")
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["head", "ok", "op", "quarantined", "tasks"],
            "{request}"
        );
        for task in listed["tasks"].as_array().unwrap() {
            assert_eq!(task.get("summary"), None, "{task}");
        }
        // `run` (every front end's way in before) is the same answer.
        let ran = crate::interface::tools::project_board::run(&*handles.operations, &text);
        assert_eq!(ran, listed, "{request}");
    }
}

/// Summaries are asked only of a listing: of any other op, the request
/// is refused as the request's, and nothing is read.
#[test]
fn a_summary_of_another_op_is_refused() {
    let refused = shared(json!({"op": "show", "task": "a"}));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["error"]["code"], json!("invalid_request"));
    assert_eq!(refused["error"]["field"], json!("summary"));
    assert_eq!(
        refused["error"]["problem"],
        json!("is taken only by list, list_ready and list_takeable")
    );
}

/// The tasks `a` and `b`, as a listing at head `h` holds them.
fn listing_of_two() -> crate::application::project_board::dto::BoardListing {
    use crate::application::project_board::ports::BoardHead;
    let tasks = QueryBoard::new(Arc::new(seeded_store()))
        .all()
        .unwrap()
        .tasks;
    assert_eq!(tasks.len(), 2);
    crate::application::project_board::dto::BoardListing {
        head: BoardHead("h".into()),
        tasks,
        quarantined: vec![],
    }
}

fn answered(listed: crate::application::project_board::task_summary::Listed) -> Value {
    let outcome = crate::application::project_board::operations::BoardOutcome::Listing(listed);
    crate::interface::tools::project_board::answer::outcome("list", &outcome, None, None)
}

/// A listing whose summaries could not be read is answered all the same,
/// no task carrying one, with a top-level `summary_skipped`: the code of
/// why.
#[test]
fn a_skipped_summary_answers_the_listing_with_why() {
    use crate::application::project_board::dto::ProjectBoardError;
    use crate::application::project_board::ports::BoardStoreError;
    use crate::application::project_board::task_summary::{Listed, SkipReason};
    for (why, code) in [
        (SkipReason::Contended, "contended"),
        (SkipReason::IndexDisagrees, "index_disagrees"),
        (
            SkipReason::Failed(Box::new(ProjectBoardError::Store(
                BoardStoreError::Unavailable("gone".into()),
            ))),
            "store_unavailable",
        ),
    ] {
        let answer = answered(Listed::skipped(listing_of_two(), why));
        assert_eq!(answer["ok"], json!(true), "{answer}");
        assert_eq!(answer["summary_skipped"], json!(code), "{answer}");
        for task in answer["tasks"].as_array().unwrap() {
            assert_eq!(task.get("summary"), None, "{answer}");
            assert_eq!(task.get("summary_skipped"), None, "{answer}");
        }
    }
}

/// A task whose plans or runs the board holds twice goes without its own
/// summary (`summary: null` and its `summary_skipped`); the others keep
/// theirs, and the listing says nothing of a skip at its top.
#[test]
fn a_duplicate_artifact_skips_its_own_rows_summary() {
    use crate::application::project_board::dto::ProjectBoardError;
    use crate::application::project_board::task_summary::{Listed, RowSummary, TaskSummary};
    let rows = vec![
        RowSummary::Skipped(Box::new(ProjectBoardError::Duplicate {
            key: ArtifactKey::Plan(slug("a"), 1),
        })),
        RowSummary::Summary(TaskSummary {
            subtitle: None,
            plan: None,
            run: None,
            may_be_incomplete: false,
        }),
    ];
    let answer = answered(Listed::summarized(listing_of_two(), rows));
    assert_eq!(answer.get("summary_skipped"), None, "{answer}");
    let tasks = answer["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["id"], json!("a"));
    assert_eq!(tasks[0]["summary"], Value::Null, "{answer}");
    assert_eq!(tasks[0]["summary_skipped"], json!("duplicate_artifact"));
    assert_eq!(tasks[1]["id"], json!("b"));
    assert_eq!(
        tasks[1]["summary"],
        json!({"subtitle": null, "plan": null, "run": null, "may_be_incomplete": false})
    );
    assert_eq!(tasks[1].get("summary_skipped"), None, "{answer}");
}

/// A board of two ready tasks, `a` and `b`.
fn seeded_store() -> MemoryBoardStore {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    use crate::domain::project_board::entities::task::{Task, TaskFields};
    use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
    let task = |id: &str| {
        let task = Task::new(TaskFields {
            id: slug(id),
            title: id.to_uppercase(),
            kind: TaskKind::Task,
            description: String::new(),
            status: TaskStatus::Ready,
            parent: None,
            depends_on: vec![],
            accepted_plan: None,
            claim: None,
            prs: vec![],
            github_issue: None,
            resolution: None,
            superseded_by: vec![],
            references: vec![],
            created: at("09:00:00"),
            updated: at("09:00:00"),
            description_form: Default::default(),
        });
        BoardArtifact::Task(task.unwrap())
    };
    let head = store.read().unwrap().head;
    let written = store.write(&head, &[task("a"), task("b")], "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

/// The agent tool never asks for summaries: its listing and its page are
/// as before, and its schema names no `summary`.
#[tokio::test(flavor = "multi_thread")]
async fn the_tools_listing_carries_no_summary() {
    let tool = tool(site(Some(summarised_board)));
    let listed = tool.execute(r#"{"op":"list"}"#).await.unwrap();
    assert!(!listed.is_error, "{}", listed.content);
    let answer: Value = serde_json::from_str(&listed.content).unwrap();
    assert_eq!(answer.get("summary_skipped"), None, "{answer}");
    for task in answer["tasks"].as_array().unwrap() {
        assert_eq!(task.get("summary"), None, "{task}");
    }
    let definition = tool.definition();
    let schema: Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    assert_eq!(schema["properties"].get("summary"), None, "{schema}");
    let refused = tool
        .execute(r#"{"op":"list","summary":true}"#)
        .await
        .unwrap();
    assert!(refused.is_error, "{}", refused.content);
}
