//! A listing's `summary` per task, as the CLI (the shared path) and the
//! agent tool answer it: where the task's plan and latest run stand, in
//! the words and times `show` gives them; `show` itself unchanged.
use serde_json::{Value, json};
use std::sync::Arc;

use super::tests::{site, tool};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardWrite, ProjectBoardStore,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
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
    Ok(ProjectBoardHandles { operations })
}

fn environment() -> ProjectBoardEnvironment {
    ProjectBoardEnvironment {
        repo: std::path::PathBuf::from("."),
        remote: "origin".into(),
        access: crate::interface::tools::project_board::site::Access::Read,
        vars: vec![],
        cancelled: None,
        launcher: None,
        swarm_session: None,
    }
}

/// The shared path's answer to `request`, as the CLI gives it.
fn shared(request: Value) -> Value {
    let handles = summarised_board(&environment()).unwrap();
    crate::interface::tools::project_board::run(&handles.operations, &request.to_string())
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
                json!({"plan": null, "run": null, "may_be_incomplete": true})
            ),
        ]
    );
    // Every listing op carries it.
    for request in [
        json!({"op": "list_ready"}),
        json!({"op": "list", "status": "ready"}),
    ] {
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
        let shown = shared(json!({"op": "show", "task": id}));
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

/// The agent tool's listing keeps each task's summary whole.
#[tokio::test(flavor = "multi_thread")]
async fn the_tools_listing_keeps_each_summary() {
    let tool = tool(site(Some(summarised_board)));
    let listed = tool.execute(r#"{"op":"list"}"#).await.unwrap();
    assert!(!listed.is_error, "{}", listed.content);
    let answer: Value = serde_json::from_str(&listed.content).unwrap();
    assert_eq!(
        summaries(&answer),
        summaries(&shared(json!({"op": "list"})))
    );
}
