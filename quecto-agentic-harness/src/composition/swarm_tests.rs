use serde_json::json;

use super::{board_op_log, build_swarm_board_handles};
use crate::application::swarm::dto::BoardLocation;
use crate::infrastructure::tools::swarm_board_dispatch::call;

/// The production graph: the SQLite file at the location, the wall clock
/// and `uuid4().hex` ids.
#[test]
fn production_handles_bootstrap_and_read_the_board_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let handles = build_swarm_board_handles(
        BoardLocation {
            database: database.clone(),
            checkout: dir.path().to_path_buf(),
        },
        None,
    );
    assert!(handles.telemetry.is_none(), "no event log, no telemetry");
    call(&handles, "parent", "bootstrap_run", json!([1, "s", "/p"])).unwrap();
    assert!(database.exists(), "bootstrap creates the board file");
    let status = call(&handles, "supervisor", "_status", json!([])).unwrap();
    let id = status["id"].as_str().unwrap();
    assert_eq!(id.len(), 32, "{status}");
    assert!(
        id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "{status}"
    );
    assert_eq!(status["status"], json!("setup"));

    // The wall clock: a deadline an hour from now is accepted.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    call(
        &handles,
        "parent",
        "create_run",
        json!(["goal", [], [{"id": "t", "kind": "review", "description": "d"}], 2, now + 3_600.0]),
    )
    .unwrap();
    let status = call(&handles, "supervisor", "_status", json!([])).unwrap();
    assert_eq!(status["status"], json!("running"));
}

/// Owner decision T1 (#2303): the board records in the event log only when
/// `telemetry.event_log.enabled` is on; the production builder then
/// measures and records every call there.
#[test]
fn the_board_records_in_the_event_log_only_when_it_is_on() {
    use crate::infrastructure::persistence::audit_log::AuditLog;
    let base = tempfile::TempDir::new().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:board").unwrap();
    assert!(
        board_op_log(false, &log).is_none(),
        "off: nothing to record in"
    );
    let event_log = board_op_log(true, &log).expect("on: the session's log");
    let dir = tempfile::TempDir::new().unwrap();
    let handles = build_swarm_board_handles(
        BoardLocation {
            database: dir.path().join("swarm.sqlite"),
            checkout: dir.path().to_path_buf(),
        },
        Some(event_log),
    );
    assert!(handles.telemetry.is_some());
    call(&handles, "parent", "bootstrap_run", json!([1, "s", "/p"])).unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:board")).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains(r#""event":"swarm_op""#), "{text}");
}

/// The production graph normalises reserved paths in the board's checkout
/// (#2275): `CheckoutPaths` is bound to `BoardLocation::checkout`.
#[test]
fn production_handles_normalise_paths_in_the_board_checkout() {
    let dir = tempfile::TempDir::new().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(checkout.join("real")).unwrap();
    std::os::unix::fs::symlink(checkout.join("real"), checkout.join("alias")).unwrap();
    let handles = build_swarm_board_handles(
        BoardLocation {
            database: dir.path().join("swarm.sqlite"),
            checkout,
        },
        None,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    call(&handles, "parent", "bootstrap_run", json!([1, "s", "/p"])).unwrap();
    call(
        &handles,
        "parent",
        "create_run",
        json!(["goal", [], [{"id": "t", "kind": "review", "description": "d"}], 2, now + 3_600.0]),
    )
    .unwrap();
    call(&handles, "parent", "task_create", json!(["r", "t", ["ok"]])).unwrap();
    let claim = call(&handles, "parent", "claim", json!([1])).unwrap();
    let reserved = call(
        &handles,
        "parent",
        "reserve",
        json!([1, claim["token"], ["alias/x/../a.rs"]]),
    )
    .unwrap();
    assert_eq!(reserved["paths"], json!(["real/a.rs"]));
    assert_eq!(
        call(
            &handles,
            "parent",
            "reserve",
            json!([1, claim["token"], ["../out"]])
        )
        .unwrap_err()
        .message(),
        "file must resolve inside the shared checkout"
    );
}
