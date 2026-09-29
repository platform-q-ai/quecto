//! The harness's Rust board and members' structured board ops on one board
//! file (#2278, #2281): the calls the harness makes through its
//! `SwarmContext`s and the calls a member's structured `swarm` ops make
//! leave the file the pure-Python board leaves for the same calls. Both
//! sides run on one fixed instant and draw ids from one counter (review
//! L3), so the two files are compared unmasked: every time, id and float
//! as stored.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardOpLog, Clock, IdSource};
use quecto::composition::swarm::{SwarmBoardHandles, build_swarm_board_handles_with};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::call_work::off_the_runtime;
use quecto::infrastructure::tools::swarm_bridge::{SwarmBoard, SwarmContext};
use quecto::infrastructure::workspace::checkout_paths::ResolvedCheckout;

use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::dump::{first_difference, logical_dump};
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;

/// The instant every call reads, the harness's and the members' ops alike:
/// fixed when the test starts (the tool checks the run's deadline against
/// the wall clock, so it is the start's own time), as `f64` bits.
static INSTANT: AtomicU64 = AtomicU64::new(0);

fn instant() -> f64 {
    f64::from_bits(INSTANT.load(Ordering::SeqCst))
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_seconds(&self) -> f64 {
        instant()
    }
}

/// How many ids the harness's boards have drawn: every board this test
/// builds draws from it, `format(n, '032x')` as the Python driver's
/// `uuid4` draws.
static DRAWN: AtomicU64 = AtomicU64::new(0);

struct CounterIds;

impl IdSource for CounterIds {
    fn hex32(&self) -> String {
        format!("{:032x}", DRAWN.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

/// Composition's handles over the fixed clock and the shared counter.
fn fixed_handles(location: BoardLocation, _log: Option<Arc<dyn BoardOpLog>>) -> SwarmBoardHandles {
    build_swarm_board_handles_with(
        Arc::new(SqliteBoardRepository::new(&location)),
        Arc::new(FixedClock),
        Arc::new(CounterIds),
        Arc::new(ResolvedCheckout::new(location.checkout)),
    )
}

/// A member's structured board op on the real `SwarmTool`, as `context`'s
/// member (#2281), on the harness's board (its fixed instant and id
/// counter): it must succeed.
async fn board_op(context: &SwarmContext, request: Value) {
    use quecto::application::tools::ports::Tool;
    use quecto::infrastructure::security::sandbox::Sandbox;
    use quecto::infrastructure::tools::swarm::{SwarmConfig, SwarmTool};
    let workspace = Arc::new(context.checkout.clone());
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context.clone()));
    let result = tool
        .execute(&request.to_string())
        .await
        .expect("the op ran");
    assert!(!result.is_error, "{request}: {}", result.content);
}

#[tokio::test]
async fn member_board_ops_and_rust_harness_share_one_board() {
    use quecto::application::swarm::ports::CoordinationPort;
    use quecto::domain::swarm::{MemberExit, ProcessIdentity};
    use quecto::infrastructure::tools::swarm_bridge::process_start;
    // The start's own time, with a fraction: the stored times' float text
    // is compared too, not masked (review L3).
    let now = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0)
        .floor()
        / 1000.0
        + 0.0625;
    INSTANT.store(now.to_bits(), Ordering::SeqCst);
    let root = tempfile::tempdir().expect("a directory for the boards");
    let checkout = root
        .path()
        .join(".quecto/container-environments/environment/workspace/repo");
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let board = SwarmBoard::new(fixed_handles, quecto::composition::swarm::board_wire());
    let context = |member: &str| SwarmContext {
        checkout: checkout.clone(),
        member: member.into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
        board: board.clone(),
    };
    let (coordinator, worker) = (context("parent"), context("worker"));
    let identity = ProcessIdentity {
        pid: std::process::id(),
        started: process_start(std::process::id()).unwrap(),
    };
    let deadline = (now + 3_600.0).floor();
    let contract = json!({"goal": "mixed", "constraints": [], "criteria": [{"id": "tests", "kind": "command", "description": "pass"}], "member_limit": 3, "deadline": deadline});
    // The harness (Rust) creates the run and admits the worker.
    off_the_runtime(|| {
        coordinator.create_run(&contract, &identity, None)?;
        coordinator.reserve_member("worker", "res-w")?;
        worker.join(&identity, None, Some("res-w"))
    })
    .unwrap();
    // 1. A member creates and claims a task through structured ops.
    board_op(
        &worker,
        json!({"op": "task_create", "request": "r1", "title": "first", "acceptance": ["tests pass"]}),
    )
    .await;
    board_op(&worker, json!({"op": "claim", "task_id": 1})).await;
    // 2. The harness's Rust summary sees it.
    let summary = off_the_runtime(|| coordinator.summary()).unwrap();
    assert_eq!(summary["tasks"][0]["status"], "claimed", "{summary}");
    assert_eq!(summary["tasks"][0]["owner"], "worker");
    // 3. The harness's Rust `_confirmed_dead` blocks it.
    off_the_runtime(|| coordinator.confirm_dead("worker", MemberExit::Orderly)).unwrap();
    // 4. The coordinator's `recover` op reopens it.
    board_op(&coordinator, json!({"op": "recover", "task_id": 1})).await;
    let summary = off_the_runtime(|| coordinator.summary()).unwrap();
    assert_eq!(summary["tasks"][0]["status"], "ready", "{summary}");

    // 5. The same sequence on the pure-Python board, on the same instant
    // and id sequence: every call the harness and the tool made above, in
    // order.
    let replay = root.path().join("replay");
    std::fs::create_dir_all(&replay).unwrap();
    let database = replay.join("swarm.sqlite");
    let mut python = PyBoard::start(&database, &replay, &replay);
    let created = python.call(
        "parent",
        "create",
        &json!(["mixed", [], contract["criteria"], 3, deadline]).to_string(),
        now,
    );
    let Outcome::Ok(created) = created else {
        panic!("create: {created:?}")
    };
    let reservation = created["members"]
        .as_array()
        .and_then(|members| members.iter().find(|m| m["id"] == "parent"))
        .and_then(|m| m["reservation"].as_str())
        .unwrap()
        .to_owned();
    let pid = identity.pid;
    let started = identity.started.clone();
    let replayed: Vec<(&str, &str, Value)> = vec![
        (
            "parent",
            "_activate",
            json!(["parent", reservation, pid, started, null]),
        ),
        ("parent", "_snapshot", json!([])),
        ("parent", "_admit", json!(["worker", "res-w"])),
        ("worker", "_bootstrap", json!([pid, started, null, "res-w"])),
        // Each of the worker's structured ops: the running gate's
        // `_status`, the op, and, the event cursor having moved (its reads,
        // `_event_cursor`, write nothing and are Rust's alone), the
        // lifecycle's summary and notifications.
        ("worker", "_status", json!([])),
        (
            "worker",
            "task_create",
            json!(["r1", "first", ["tests pass"]]),
        ),
        ("worker", "summary", json!([null])),
        ("worker", "_notifications", json!([true])),
        ("worker", "_status", json!([])),
        ("worker", "claim", json!([1])),
        ("worker", "summary", json!([null])),
        ("worker", "_notifications", json!([true])),
        ("parent", "summary", json!([null])),
        ("parent", "_confirmed_dead", json!(["worker", "orderly"])),
        ("parent", "_status", json!([])),
        ("parent", "recover", json!([1])),
        ("parent", "summary", json!([null])),
        ("parent", "_notifications", json!([true])),
        ("parent", "summary", json!([null])),
    ];
    for (member, method, args) in replayed {
        let outcome = python.call(member, method, &args.to_string(), now);
        assert!(
            matches!(outcome, Outcome::Ok(_)),
            "{member} {method}: {outcome:?}"
        );
    }
    let harness = logical_dump(&coordinator.database());
    let pure = logical_dump(&database);
    assert_eq!(
        first_difference(&pure, &harness),
        None,
        "the harness's Rust calls and the members' structured ops leave the pure-Python board, byte for byte"
    );
}
