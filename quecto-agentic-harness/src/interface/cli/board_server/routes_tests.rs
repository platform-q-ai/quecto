use super::*;
use crate::application::project_board::ports::{BoardArtifact, BoardHead, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::http::board_service::listener::{ServiceMethod, ServiceRequest};
use crate::infrastructure::persistence::project_board::artifacts::file_of;
use crate::infrastructure::persistence::project_board::snapshot_files::MarkdownSnapshotFiles;
use crate::infrastructure::test_support::board_artifacts::plan;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use serde_json::Value;
use std::sync::Arc;

pub(in crate::interface::cli::board_server) const BOARD: &str = "0123456789abcdef";

pub(in crate::interface::cli::board_server) fn task(id: &str) -> Task {
    let at = Timestamp::parse("2026-10-24T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: "Do it.".into(),
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
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

/// The server's reads of `store`'s board, served as board [`BOARD`].
pub(in crate::interface::cli::board_server) fn serving(store: &MemoryBoardStore) -> ServeBoard {
    ServeBoard::new(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(MarkdownSnapshotFiles),
        BoardId::of_hex(BOARD).unwrap(),
    )
}

/// A board holding a task and a plan of it.
pub(in crate::interface::cli::board_server) fn written() -> (MemoryBoardStore, BoardHead) {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task("alpha")),
        BoardArtifact::Plan(plan("alpha", 1)),
    ];
    let written = store.write(&head, &artifacts, "seed").unwrap();
    let crate::application::project_board::ports::BoardWrite::Written(head) = written else {
        panic!("the seed landed: {written:?}");
    };
    (store, head)
}

fn get(routes: &BoardRoutes, path: &str) -> (u16, Value) {
    answer(routes, ServiceMethod::Get, path)
}

fn answer(routes: &BoardRoutes, method: ServiceMethod, path: &str) -> (u16, Value) {
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: vec![],
    });
    let body = serde_json::from_slice(&reply.body)
        .unwrap_or_else(|error| panic!("{path}: {error}: {:?}", reply.body));
    (reply.status, body)
}

#[test]
fn status_answers_the_role_the_board_and_the_head() {
    let (store, head) = written();
    let routes = BoardRoutes::new(serving(&store));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["role"], "primary");
    assert_eq!(body["board"], BOARD);
    assert_eq!(body["head"], head.0.as_str());
}

#[test]
fn status_of_no_board_answers_no_head() {
    let routes = BoardRoutes::new(serving(&MemoryBoardStore::default()));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["head"], Value::Null);
}

#[test]
fn time_answers_the_servers_clock() {
    let (store, _) = written();
    let (status, body) = get(&BoardRoutes::new(serving(&store)), "/v1/time");
    assert_eq!(status, 200, "{body}");
    let time = body["time"].as_str().unwrap();
    assert!(Timestamp::parse(time).is_ok(), "{time}");
}

#[test]
fn the_snapshot_is_the_stores_files() {
    let (store, head) = written();
    let (status, body) = get(
        &BoardRoutes::new(serving(&store)),
        &format!("/v1/boards/{BOARD}/snapshot"),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["board"], BOARD);
    assert_eq!(body["head"], head.0.as_str());
    let text = |artifact: &BoardArtifact| String::from_utf8(file_of(artifact).unwrap()).unwrap();
    let expected = serde_json::json!([
        { "path": "plans/alpha.1.md", "text": text(&BoardArtifact::Plan(plan("alpha", 1))) },
        { "path": "tasks/alpha.md", "text": text(&BoardArtifact::Task(task("alpha"))) },
    ]);
    assert_eq!(body["files"], expected);
    assert_eq!(body["set_aside"], serde_json::json!([]));
}

#[test]
fn another_boards_snapshot_is_404() {
    let (store, _) = written();
    let routes = BoardRoutes::new(serving(&store));
    for path in [
        "/v1/boards/fedcba9876543210/snapshot",
        "/v1/boards/NOT-AN-ID/snapshot",
        "/v1/boards//snapshot",
    ] {
        assert_eq!(get(&routes, path).0, 404, "{path}");
    }
}

#[test]
fn a_path_outside_the_route_table_is_404() {
    let (store, _) = written();
    let routes = BoardRoutes::new(serving(&store));
    for path in [
        "/",
        "/v1",
        "/v1/status/",
        "/v2/status",
        "/v1/boards",
        &format!("/v1/boards/{BOARD}"),
        &format!("/v1/boards/{BOARD}/snapshot/extra"),
    ] {
        assert_eq!(get(&routes, path).0, 404, "{path}");
    }
}

#[test]
fn a_route_asked_by_a_method_it_does_not_take_is_405() {
    let (store, _) = written();
    let routes = BoardRoutes::new(serving(&store));
    for path in [
        "/v1/status",
        "/v1/time",
        &format!("/v1/boards/{BOARD}/snapshot"),
    ] {
        assert_eq!(answer(&routes, ServiceMethod::Post, path).0, 405, "{path}");
    }
}

#[test]
fn a_store_that_cannot_be_read_is_503() {
    let (store, _) = written();
    store.fail_every_call(
        crate::application::project_board::ports::BoardStoreError::Unavailable(
            "the remote is down".into(),
        ),
    );
    let routes = BoardRoutes::new(serving(&store));
    let (status, body) = get(&routes, "/v1/status");
    assert_eq!(status, 503, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("the remote is down")
    );
}
