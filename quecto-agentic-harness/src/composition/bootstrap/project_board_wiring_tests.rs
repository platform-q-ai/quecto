//! Review round 2 (#2575): the board composition reads a run's end from
//! the boards the teardown keeps (`kept_swarm_boards`) under the session
//! its front end names (`swarm_session`), and from no other: a front end
//! naming none (the CLI) records nothing reported.
use std::sync::Arc;

use serde_json::{Value, json};

use super::tests::{environment, repository, sh};
use super::*;
use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::hosted_swarm::EndedSwarmBoards;
use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::interface::tools::project_board::run;

/// A session's swarms whose every launch comes up, none seen running.
struct Launches;

impl RunLiveness for Launches {
    fn running_coordinator(&self, _: &Slug, _: u32) -> Option<String> {
        None
    }
}

impl CoordinatorLaunch for Launches {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        Ok(LaunchedCoordinator {
            agent: "7f3a".into(),
            environment_ref: "C1".into(),
        })
    }
}

/// Task `a` started over a real board composed for `session`, its one
/// task item `schema`; the operations to end its run with.
fn started(work: &std::path::Path, session: Option<&str>) -> ProjectBoardHandles {
    let mut writing = environment(work, Access::Write);
    writing.launcher = Some(Arc::new(Launches));
    writing.swarm_session = session.map(str::to_owned);
    let handles = build_project_board_handles(&writing).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    assert_eq!(ops(json!({"op": "bootstrap"}))["ok"], json!(true));
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let proposed = ops(
        json!({"op": "propose_plan", "task": "a", "approach": "Build it.",
                              "items": [{"id": "schema", "title": "Schema"}]}),
    );
    let accepted = ops(json!({"op": "accept_plan", "task": "a", "version": 1,
                              "expected_updated": proposed["plan"]["updated"],
                              "expected_accepted": null}));
    assert_eq!(accepted["ok"], json!(true), "{accepted}");
    let started = ops(json!({"op": "start", "task": "a", "plan_version": 1}));
    assert_eq!(started["ok"], json!(true), "{started}");
    handles
}

/// The teardown's keep of run 1 of `a`'s ended board under `session`: its
/// item `schema` completed by `worker-1`, its evidence at `revision`.
fn kept_at(session: &str, revision: &str) {
    let board = json!({
        "run": {"status": "succeeded", "budget": {"observed_tokens": 52_000}},
        "items": [{"title": "schema: Schema", "state": "completed", "owner": "worker-1",
                   "evidence": [{"artifact": "docs/schema.md", "revision": revision}]}],
        "totals": {"items": 1},
    });
    crate::composition::environments::kept_swarm_boards().keep(
        session,
        "board-a-run-1",
        SwarmBoardSnapshot::object(board).unwrap(),
    );
}

fn kept_for(session: &str) {
    kept_at(session, "abc123");
}

fn succeeded(handles: &ProjectBoardHandles) -> Value {
    let request = json!({"op": "end_run", "task": "a", "run": 1, "outcome": "succeeded"});
    run(&handles.operations, &request.to_string())
}

#[test]
fn a_composed_end_run_records_the_board_kept_under_its_front_ends_session() {
    let (_dir, work) = repository();
    let session = "wiring-composed-session";
    let handles = started(&work, Some(session));
    // #2666: the swarm committed its work in the clone and never pushed it.
    sh(
        &work,
        &["commit", "-q", "--allow-empty", "-m", "the run's work"],
    );
    let head = sh(&work, &["rev-parse", "HEAD"]);
    kept_at(session, &head);
    let refused = succeeded(&handles);
    assert_eq!(
        (
            &refused["error"]["rule"]["code"],
            &refused["error"]["rule"]["revisions"]
        ),
        (&json!("not_pushed"), &json!(1)),
        "{refused}"
    );
    sh(&work, &["push", "-q", "origin", "HEAD:refs/heads/run-a-1"]);
    let ended = succeeded(&handles);
    assert_eq!(ended["ok"], json!(true), "{ended}");
    let item = &ended["run"]["items"][0];
    assert_eq!(
        (&item["id"], &item["state"], &item["owner"]),
        (&json!("schema"), &json!("done"), &json!("worker-1")),
        "{ended}"
    );
    assert_eq!(ended["run"]["cost_tokens"], json!(52_000), "{ended}");
}

#[test]
fn a_front_end_naming_no_session_reads_no_kept_board() {
    let (_dir, work) = repository();
    let handles = started(&work, None);
    kept_for("");
    kept_for("wiring-no-session");
    let refused = succeeded(&handles);
    assert_eq!(
        (
            &refused["error"]["rule"]["code"],
            &refused["error"]["rule"]["report"]
        ),
        (&json!("items_not_done"), &json!("none_kept")),
        "{refused}"
    );
}
