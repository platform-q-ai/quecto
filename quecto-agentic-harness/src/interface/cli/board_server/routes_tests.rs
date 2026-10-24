use super::*;
use crate::application::project_board::ports::{BoardArtifact, BoardHead, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::http::board_service::listener::{ServiceMethod, ServiceRequest};
use crate::infrastructure::test_support::board_artifacts::plan;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use serde_json::Value;
use std::sync::Arc;

pub(in crate::interface::cli::board_server) const BOARD: &str = "0123456789abcdef";

fn task(id: &str) -> Task {
    let at = Timestamp::parse("2026-10-24T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse(id).unwrap(),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: "Do it.".into(),
        description_form: Default::default(),
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

/// A route's 405 names the methods it takes, with its code.
#[test]
fn a_routes_405_names_its_methods() {
    let (store, _) = written();
    let routes = BoardRoutes::new(serving(&store));
    for path in ["/v1/status", "/v1/time"] {
        let reply = routes.answer(&ServiceRequest {
            method: ServiceMethod::Post,
            path: path.into(),
            body: vec![],
        });
        assert_eq!((reply.status, reply.allow), (405, Some("GET")), "{path}");
        assert_eq!(
            answer(&routes, ServiceMethod::Post, path).1["code"],
            "method"
        );
    }
    assert_eq!(routes.allowed("/v1/time"), Some("GET"));
    assert_eq!(routes.allowed("/v1/nothing"), None);
}

/// The snapshot route moved to B11: its path is no route here.
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
        &format!("/v1/boards/{BOARD}/snapshot"),
    ] {
        let (status, body) = get(&routes, path);
        assert_eq!(
            (status, &body["code"]),
            (404, &Value::from("route")),
            "{path}"
        );
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
    assert_eq!((status, &body["code"]), (503, &Value::from("unavailable")));
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("the remote is down")
    );
}
