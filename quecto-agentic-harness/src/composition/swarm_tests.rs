use serde_json::json;

use super::build_swarm_board_handles;
use crate::application::swarm::dto::BoardLocation;
use crate::infrastructure::tools::swarm_board_dispatch::call;

/// The production graph: the SQLite file at the location, the wall clock
/// and `uuid4().hex` ids.
#[test]
fn production_handles_bootstrap_and_read_the_board_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let handles = build_swarm_board_handles(BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
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
