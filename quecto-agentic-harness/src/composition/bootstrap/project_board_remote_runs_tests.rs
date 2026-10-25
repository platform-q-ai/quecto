//! Epic #2705 B8: `start` and `end_run` across the wire. A start is begun
//! on the board server and launched by its client; a launch that fails is
//! undone on the server, answered with the fields a local start's undo
//! answers (`start_board_task_undo_tests`, run remote); an end carries
//! what the client's session saw of its run.
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::application::project_board::dto::{RunEnd, RunEndOutcome};
use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator, RunLiveness,
    StartedRuns,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// How a test's launch goes.
enum Launch {
    /// The coordinator comes up as `c-1`.
    Up,
    /// No container runtime.
    Refused,
    /// It came up wrong and could not be stopped: `7f3a` may still run.
    LeftRunning,
    /// The call is cancelled (`cancel` set) as spawn begins.
    CancelledAsItCameUp(Arc<AtomicBool>),
    /// Another writer ends the run as stopped, then the launch fails.
    EndedMeanwhile(Box<dyn BoardOperations>),
}

/// This session's swarms as a test drives them.
struct Swarms {
    launch: Launch,
    running: Mutex<Option<String>>,
    started: Mutex<Vec<(Slug, u32)>>,
    launched: Mutex<Vec<CoordinatorRequest>>,
}

impl Swarms {
    fn new(launch: Launch) -> Arc<Self> {
        Arc::new(Self {
            launch,
            running: Mutex::default(),
            started: Mutex::default(),
            launched: Mutex::default(),
        })
    }

    fn seeing(&self, agent: Option<&str>) {
        *self.running.lock().unwrap() = agent.map(str::to_string);
    }
}

impl CoordinatorLaunch for Swarms {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        request: &CoordinatorRequest,
        _: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        self.launched.lock().unwrap().push(request.clone());
        self.started
            .lock()
            .unwrap()
            .push((request.task.clone(), request.run));
        match &self.launch {
            Launch::Up => Ok(LaunchedCoordinator {
                agent: "c-1".into(),
                environment_ref: "env-1".into(),
            }),
            Launch::Refused => Err(LaunchRefused::because("no container runtime")),
            Launch::LeftRunning => Err(LaunchRefused::leaving(
                "the coordinator came up outside a container, so stopping it timed out",
                "7f3a",
            )),
            Launch::CancelledAsItCameUp(cancel) => {
                cancel.store(true, Ordering::SeqCst);
                Err(LaunchRefused::because(
                    "the call was cancelled while the coordinator launched: the launch was \
                     dropped",
                ))
            }
            Launch::EndedMeanwhile(other) => {
                let end = json!({"op": "end_run", "task": "a", "run": request.run,
                                 "outcome": "stopped"});
                let ended = run(&**other, &end.to_string());
                assert_eq!(ended["ok"], json!(true), "{ended}");
                Err(LaunchRefused::because("no container runtime"))
            }
        }
    }
}

impl RunLiveness for Swarms {
    fn running_coordinator(&self, _: &Slug, _: u32) -> Option<String> {
        self.running.lock().unwrap().clone()
    }
}

impl StartedRuns for Swarms {
    fn started(&self) -> Vec<(Slug, u32)> {
        self.started.lock().unwrap().clone()
    }
}

/// The board's operations at `served`'s socket for a session whose
/// swarms are `swarms`, its calls cancelled by `cancelled`.
fn launching(
    served: &Served,
    swarms: &Arc<Swarms>,
    cancelled: Option<Arc<AtomicBool>>,
) -> Box<dyn BoardOperations> {
    let environment = ProjectBoardEnvironment {
        repo: served.work.clone(),
        remote: "origin".into(),
        known: KnownLocation {
            board_id: Some(served.board.clone()),
            remote_url: None,
            url: Some(served.url()),
        },
        access: Access::Write,
        vars: vars(&served.work),
        cancelled,
        launcher: Some(Arc::clone(swarms) as _),
        swarm_session: None,
    };
    let built = build_project_board_handles(&environment);
    built
        .map_err(|error| format!("{error:?}"))
        .unwrap()
        .operations
}

/// A server whose task `a` is ready with version 1 of its plan accepted.
fn startable() -> Served {
    let served = served(false);
    let writer = served.writer();
    let ops = |request: Value| run(&*writer, &request.to_string());
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    let proposed = ops(
        json!({"op": "propose_plan", "task": "a", "approach": "Build it.",
                              "items": [{"id": "schema", "title": "Schema"}]}),
    );
    let updated = proposed["plan"]["updated"].clone();
    let accepted = ops(json!({"op": "accept_plan", "task": "a", "version": 1,
                              "expected_updated": updated, "expected_accepted": null}));
    assert_eq!(accepted["ok"], json!(true), "{accepted}");
    served
}

fn start(operations: &dyn BoardOperations) -> Value {
    run(
        operations,
        &json!({"op": "start", "task": "a", "plan_version": 1}).to_string(),
    )
}

/// Task `a` on the server: its status, and its run 1's state.
fn on_server(served: &Served) -> (Value, Value) {
    let shown = run(
        &*served.writer(),
        &json!({"op": "show", "task": "a"}).to_string(),
    );
    (
        shown["task"]["status"].clone(),
        shown["runs"][0]["state"].clone(),
    )
}

#[test]
fn a_remote_start_is_launched_here_and_answered_as_a_local_one() {
    let served = startable();
    let swarms = Swarms::new(Launch::Up);
    let started = start(&*launching(&served, &swarms, None));
    assert_eq!(started["ok"], json!(true), "{started}");
    assert_eq!(
        (
            &started["coordinator"],
            &started["run"]["number"],
            &started["claim"]
        ),
        (
            &json!({"agent": "c-1", "environment_ref": "env-1"}),
            &json!(1),
            &json!("fresh")
        ),
        "{started}"
    );
    assert_eq!((started.get("launch"), started.get("from")), (None, None));
    assert!(
        started["steps"].as_str().is_some(),
        "the tool's steps: {started}"
    );
    let launched = swarms.launched.lock().unwrap().clone();
    assert_eq!(launched.len(), 1);
    assert_eq!(
        (
            launched[0].task.as_str(),
            launched[0].run,
            launched[0].plan_version
        ),
        ("a", 1, 1)
    );
    assert!(launched[0].brief.contains("schema"), "the plan's brief");
    assert_eq!(on_server(&served), (json!("in_progress"), json!("running")));
}

/// A failed launch, the coordinator seen running, the refusal's fields, and
/// the task's status and run 1's state on the server after.
type UndoCase = (
    fn(&Served) -> Launch,
    Option<&'static str>,
    Value,
    (Value, Value),
);

/// The local undo tests' answers, run remote: refused, left running, seen
/// running after a cancel, and ended meanwhile by another writer.
#[test]
fn a_remote_start_whose_launch_fails_is_undone_as_a_local_one() {
    let cases: [UndoCase; 4] = [
        (
            |_| Launch::Refused,
            None,
            json!({"reason": "no container runtime", "run": 1, "run_state": "folded",
                   "released": true, "restored": null, "still_running": null}),
            (json!("ready"), json!("failed")),
        ),
        (
            |_| Launch::LeftRunning,
            None,
            json!({"reason": "the coordinator came up outside a container, so stopping it \
                             timed out",
                   "run": 1, "run_state": "open", "released": false, "restored": null,
                   "still_running": "7f3a"}),
            (json!("in_progress"), json!("running")),
        ),
        (
            |_| Launch::CancelledAsItCameUp(Arc::new(AtomicBool::new(false))),
            Some("7f3a"),
            json!({"reason": "the call was cancelled while the coordinator launched: the \
                             launch was dropped",
                   "run": 1, "run_state": "open", "released": false, "restored": null,
                   "still_running": "7f3a"}),
            (json!("in_progress"), json!("running")),
        ),
        (
            |served| Launch::EndedMeanwhile(served.writer()),
            None,
            json!({"reason": "no container runtime", "run": 1, "run_state": "ended_already",
                   "released": null, "restored": null, "still_running": null}),
            (json!("ready"), json!("stopped")),
        ),
    ];
    for (launch, running, fields, left) in cases {
        let served = startable();
        let swarms = Swarms::new(launch(&served));
        swarms.seeing(running);
        let refused = start(&*launching(&served, &swarms, None));
        let error = &refused["error"];
        assert_eq!(error["code"], json!("launch_failed"), "{refused}");
        for (key, value) in fields.as_object().unwrap() {
            assert_eq!(&error[key], value, "{key}: {refused}");
        }
        assert_eq!(on_server(&served), left, "{refused}");
    }
}

/// A call cancelled during the launch launches nothing that stays: the
/// start is undone on the server, past the cancel. One cancelled before it
/// writes nothing at all, as a local start's cancelled store does (review
/// round 1).
#[test]
fn a_remote_start_cancelled_during_its_launch_is_undone() {
    let served = startable();
    let cancel = Arc::new(AtomicBool::new(false));
    let swarms = Swarms::new(Launch::CancelledAsItCameUp(Arc::clone(&cancel)));
    let refused = start(&*launching(&served, &swarms, Some(Arc::clone(&cancel))));
    let error = &refused["error"];
    assert_eq!(
        (&error["code"], &error["run_state"], &error["released"]),
        (&json!("launch_failed"), &json!("folded"), &json!(true)),
        "{refused}"
    );
    assert_eq!(on_server(&served), (json!("ready"), json!("failed")));
    let served = startable();
    let swarms = Swarms::new(Launch::Up);
    let cancelled = Some(Arc::new(AtomicBool::new(true)));
    let refused = start(&*launching(&served, &swarms, cancelled));
    assert_eq!(refused["error"]["code"], json!("refused"), "{refused}");
    assert!(
        swarms.launched.lock().unwrap().is_empty(),
        "nothing launched"
    );
    assert_eq!(
        on_server(&served),
        (json!("ready"), Value::Null),
        "nothing written"
    );
}

/// Review round 1: an end's facts or a start's undo are part of its write:
/// the same write id with other facts is `write_id_reused`.
#[test]
fn a_write_id_is_bound_to_its_facts() {
    let served = startable();
    let client =
        crate::infrastructure::http::board_remote::endpoint_of(&served.url(), None).unwrap();
    let body = |facts: Value| {
        json!({"request": {"op": "end_run", "task": "a", "run": 1, "outcome": "failed"},
               "front_end": "cli", "write_id": ID,
               "committer": {"name": "Ada", "email": "ada@example.com"}, "facts": facts})
    };
    let unseen = json!({"liveness": "unseen", "reports": null});
    let first = posted(&client, &served.board, &body(unseen));
    assert_eq!(first.0, 200, "{first:?}");
    let seen = json!({"liveness": {"seen": null}, "reports": null});
    let again = posted(&client, &served.board, &body(seen));
    assert_eq!(
        (again.0, &again.1["code"]),
        (409, &json!("write_id_reused"))
    );
}

/// An end is judged on the server by what this session saw: its
/// coordinator running (`run_live`), then stopped.
#[test]
fn a_remote_end_is_refused_while_this_session_sees_its_swarm_running() {
    let served = startable();
    let swarms = Swarms::new(Launch::Up);
    let operations = launching(&served, &swarms, None);
    assert_eq!(start(&*operations)["ok"], json!(true));
    let end = json!({"op": "end_run", "task": "a", "run": 1, "outcome": "failed"}).to_string();
    swarms.seeing(Some("c-1"));
    let live = run(&*operations, &end);
    assert_eq!(
        (&live["error"]["code"], &live["error"]["agent"]),
        (&json!("run_live"), &json!("c-1")),
        "{live}"
    );
    assert_eq!(on_server(&served), (json!("in_progress"), json!("running")));
    swarms.seeing(None);
    let ended = run(&*operations, &end);
    assert_eq!(
        (&ended["ok"], &ended["status"], &ended["liveness"]),
        (&json!(true), &json!("ready"), &json!("checked")),
        "{ended}"
    );
}

/// A session's starts on a board server leave their runs owed as the
/// session ends, until each is ended.
#[test]
fn a_remote_session_owes_the_runs_its_starts_left_running() {
    let served = startable();
    let swarms = Swarms::new(Launch::Up);
    let operations = launching(&served, &swarms, None);
    assert_eq!(start(&*operations)["ok"], json!(true));
    let owed = RunEnd {
        task: Slug::parse("a").unwrap(),
        run: 1,
        outcome: RunEndOutcome::Stopped,
        expected_updated: None,
        stop: None,
    };
    assert_eq!(operations.left_runs(), Ok(vec![owed]));
    let end = json!({"op": "end_run", "task": "a", "run": 1, "outcome": "stopped"});
    assert_eq!(run(&*operations, &end.to_string())["ok"], json!(true));
    assert_eq!(operations.left_runs(), Ok(vec![]));
}
