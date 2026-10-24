use super::*;
use crate::infrastructure::http::board_service::client::BoardServiceClient;
use crate::infrastructure::http::board_service::listener::{
    PeerCredentials, SocketPeerCredentials, this_uid,
};
use crate::interface::cli::board_server::routes::tests::{BOARD, serving, written};
use std::sync::Arc;

/// Serves `handles` on a temp socket on its own thread; gives the socket
/// once it is ready, and a sender that stops it.
fn served(
    handles: BoardServerHandles,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    tokio::sync::oneshot::Sender<()>,
    std::thread::JoinHandle<Result<(), String>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = std::sync::mpsc::channel();
    let at = socket.clone();
    let thread = std::thread::spawn(move || {
        serve(
            handles,
            &at,
            std::time::Instant::now(),
            || async {
                let _ = stopped.await;
            },
            |started: &Started| ready.send(started.clone()).unwrap(),
        )
    });
    let started = started
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the server starts");
    assert_eq!(started.socket, socket);
    assert_eq!(started.board.as_str(), BOARD);
    (dir, socket, stop, thread)
}

fn handles(serve: crate::application::project_board::use_cases::ServeBoard) -> BoardServerHandles {
    BoardServerHandles {
        serve,
        peers: Arc::new(SocketPeerCredentials) as Arc<dyn PeerCredentials>,
        uid: this_uid(),
    }
}

#[test]
fn status_over_a_temp_socket_answers_the_role_and_head() {
    let (store, head) = written();
    let (_dir, socket, stop, thread) = served(handles(serving(&store)));
    let reply = BoardServiceClient::unix(&socket).get("/v1/status").unwrap();
    assert_eq!(reply.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(body["role"], "primary");
    assert_eq!(body["head"], head.0.as_str());
    assert_eq!(body["board"], BOARD);
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
    // The server removes its socket as it stops.
    assert!(std::fs::symlink_metadata(&socket).is_err());
}

#[test]
fn an_unknown_path_over_the_socket_is_404() {
    let (store, _) = written();
    let (_dir, socket, stop, thread) = served(handles(serving(&store)));
    let reply = BoardServiceClient::unix(&socket)
        .get("/v1/nothing")
        .unwrap();
    assert_eq!(reply.status, 404);
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
}

#[test]
fn a_socket_that_cannot_be_bound_is_refused_before_serving() {
    let (store, _) = written();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("taken");
    std::fs::write(&file, "x").unwrap();
    let error = serve(
        handles(serving(&store)),
        &file.join("board.sock"),
        std::time::Instant::now(),
        || async {},
        |_: &Started| panic!("never ready"),
    )
    .unwrap_err();
    assert!(error.starts_with("board-server:"), "{error}");
}

#[test]
fn serve_takes_exactly_its_options() {
    let parsed = parse(&[
        "serve".into(),
        "--socket".into(),
        "/run/b.sock".into(),
        "--repo".into(),
        "/src/r".into(),
    ])
    .unwrap();
    assert_eq!(parsed.socket, std::path::PathBuf::from("/run/b.sock"));
    assert_eq!(parsed.repo, std::path::PathBuf::from("/src/r"));
    assert_eq!(parsed.remote, "origin");
    let remote = parse(&[
        "serve".into(),
        "--socket".into(),
        "/s".into(),
        "--repo".into(),
        "/r".into(),
        "--remote".into(),
        "board".into(),
    ])
    .unwrap();
    assert_eq!(remote.remote, "board");
    for refused in [
        vec![],
        vec!["run".to_string()],
        vec!["serve".into(), "--socket".into(), "/s".into()],
        vec!["serve".into(), "--repo".into(), "/r".into()],
        vec![
            "serve".into(),
            "--socket".into(),
            "/s".into(),
            "--repo".into(),
            "/r".into(),
            "--tcp".into(),
        ],
        vec!["serve".into(), "--socket".into()],
    ] {
        assert!(parse(&refused).is_err(), "{refused:?}");
    }
}
