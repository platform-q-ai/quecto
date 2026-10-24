use super::*;
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardFile, BoardHead, BoardSnapshot, ProjectBoardStore,
    SnapshotFiles, SnapshotFilesError,
};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::http::board_service::listener::{ServiceMethod, ServiceRequest};
use crate::infrastructure::persistence::project_board::artifacts::file_of;
use crate::infrastructure::persistence::project_board::board_tree::{folder_file, task_file};
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

/// The memory store's stored files: it keeps artifacts by value, so its
/// bytes are their codec's; a file set aside keeps none.
struct MemoryFiles;

impl SnapshotFiles for MemoryFiles {
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError> {
        let artifacts = snapshot
            .tasks
            .iter()
            .cloned()
            .map(BoardArtifact::Task)
            .chain(snapshot.plans.iter().cloned().map(BoardArtifact::Plan));
        let mut files: Vec<BoardFile> = artifacts
            .map(|artifact| {
                let path = match artifact.key() {
                    ArtifactKey::Task(id) => format!("tasks/{}", task_file(&id)),
                    key => {
                        let file = folder_file(&key, Vec::new()).unwrap();
                        format!("{}/{}", file.folder, file.name)
                    }
                };
                BoardFile {
                    path,
                    bytes: Some(file_of(&artifact).unwrap()),
                    set_aside: None,
                }
            })
            .chain(snapshot.quarantined.iter().map(|file| BoardFile {
                path: file.path.clone(),
                bytes: None,
                set_aside: Some(file.reason.clone()),
            }))
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

/// A files port that answers as told, whatever the snapshot.
struct Told(Result<Vec<BoardFile>, SnapshotFilesError>);

impl SnapshotFiles for Told {
    fn files(&self, _: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError> {
        self.0.clone()
    }
}

/// The server's reads of `store`'s board, served as board [`BOARD`].
pub(in crate::interface::cli::board_server) fn serving(store: &MemoryBoardStore) -> ServeBoard {
    serving_with(store, Arc::new(MemoryFiles))
}

fn serving_with(store: &MemoryBoardStore, files: Arc<dyn SnapshotFiles>) -> ServeBoard {
    ServeBoard::new(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        files,
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

fn snapshot_path() -> String {
    format!("/v1/boards/{BOARD}/snapshot")
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
fn the_snapshot_is_the_stores_files_those_set_aside_included() {
    let (store, _) = written();
    store.quarantine("broken", "no marker opens it");
    let head = store.read().unwrap().head;
    let (status, body) = get(&BoardRoutes::new(serving(&store)), &snapshot_path());
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["board"], BOARD);
    assert_eq!(body["head"], head.0.as_str());
    let text = |artifact: &BoardArtifact| String::from_utf8(file_of(artifact).unwrap()).unwrap();
    let expected = serde_json::json!([
        { "path": "plans/alpha.1.md", "encoding": "utf-8",
          "content": text(&BoardArtifact::Plan(plan("alpha", 1))), "set_aside": null },
        { "path": "tasks/alpha.md", "encoding": "utf-8",
          "content": text(&BoardArtifact::Task(task("alpha"))), "set_aside": null },
        { "path": "tasks/broken.md", "encoding": "none", "content": null,
          "set_aside": "no marker opens it" },
    ]);
    assert_eq!(body["files"], expected);
}

#[test]
fn bytes_that_are_not_plain_text_are_sent_as_base64() {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    store.quarantine("odd", "binary");
    let odd = |bytes: &[u8]| {
        Told(Ok(vec![BoardFile {
            path: "tasks/odd.md".into(),
            bytes: Some(bytes.to_vec()),
            set_aside: Some("binary".into()),
        }]))
    };
    for (bytes, encoding, content) in [
        (&[0xff, 0x00, 0x41][..], "base64", "/wBB"),
        (b"bell\x07", "base64", "YmVsbAc="),
        (b"plain\n\ttext \"q\" \\", "utf-8", "plain\n\ttext \"q\" \\"),
    ] {
        let routes = BoardRoutes::new(serving_with(&store, Arc::new(odd(bytes))));
        let (status, body) = get(&routes, &snapshot_path());
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["files"][0]["encoding"], encoding);
        assert_eq!(body["files"][0]["content"], content);
    }
}

#[test]
fn files_the_port_cannot_give_whole_are_500_never_a_panic() {
    let (store, _) = written();
    for (files, says) in [
        // One file for a board of two artifacts: the port's contract broken.
        (
            Told(Ok(vec![BoardFile {
                path: "tasks/alpha.md".into(),
                bytes: Some(b"x".to_vec()),
                set_aside: None,
            }])),
            "1 files for 2 artifacts",
        ),
        (
            Told(Err(SnapshotFilesError::TooLarge("past 64 MiB".into()))),
            "past 64 MiB",
        ),
        (
            Told(Err(SnapshotFilesError::Unreadable("no blob".into()))),
            "no blob",
        ),
    ] {
        let routes = BoardRoutes::new(serving_with(&store, Arc::new(files)));
        let (status, body) = get(&routes, &snapshot_path());
        assert_eq!(status, 500, "{body}");
        assert!(body["error"].as_str().unwrap().contains(says), "{body}");
    }
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
