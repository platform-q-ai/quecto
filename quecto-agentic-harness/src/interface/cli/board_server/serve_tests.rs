use super::*;
use crate::infrastructure::http::board_service::listener::{
    PeerCredentials, SocketPeerCredentials, this_uid,
};
use crate::infrastructure::http::board_service::test_server::raw;
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

/// `GET path` over `socket`, as curl sends it: the status and body.
fn get(socket: &std::path::Path, path: &str) -> (u16, String) {
    let request = format!("GET {path} HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\r\n");
    raw(socket, request.as_bytes())
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
    let (status, reply) = get(&socket, "/v1/status");
    assert_eq!(status, 200, "{reply}");
    let body: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(body["role"], "primary");
    assert_eq!(body["head"], head.0.as_str());
    assert_eq!(body["board"], BOARD);
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
    // The server removes its socket as it stops.
    assert!(std::fs::symlink_metadata(&socket).is_err());
}

/// Review round 2: a server that stops removes only the socket it bound,
/// never a newer server's at the same path.
#[test]
fn stopping_leaves_a_newer_servers_socket() {
    let (store, _) = written();
    let (_dir, socket, stop, thread) = served(handles(serving(&store)));
    std::fs::remove_file(&socket).unwrap();
    let newer = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
    assert!(
        std::fs::symlink_metadata(&socket).is_ok(),
        "the newer socket stays"
    );
    drop(newer);
}

#[test]
fn an_unknown_path_over_the_socket_is_404() {
    let (store, _) = written();
    let (_dir, socket, stop, thread) = served(handles(serving(&store)));
    assert_eq!(get(&socket, "/v1/nothing").0, 404);
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

#[test]
fn the_command_refuses_bad_options_and_an_uncomposed_server() {
    let ctx = crate::interface::cli::CliContext::default();
    let (mut out, mut err) = (String::new(), String::new());
    let code = super::super::cmd_board_server(&ctx, &["serve".into()], &mut out, &mut err);
    assert_eq!(code, 1);
    assert!(err.contains("--socket <path> is required"), "{err}");
    assert!(err.contains(USAGE), "{err}");
    err.clear();
    let args = ["serve", "--socket", "/s", "--repo", "/r"].map(String::from);
    assert_eq!(
        super::super::cmd_board_server(&ctx, &args, &mut out, &mut err),
        1
    );
    assert!(err.contains("not composed"), "{err}");
    assert!(out.is_empty());
}

/// A writer that always fails, as a closed stdout does.
struct Broken;

impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

/// Review round 4: the start-up line on a broken stdout neither panics nor
/// fails the server; on a working one it is written whole.
#[test]
fn the_start_up_line_survives_a_broken_stdout() {
    announce(&mut Broken, "board-server: serving\n");
    let mut written = Vec::new();
    announce(&mut written, "board-server: serving\n");
    assert_eq!(written, b"board-server: serving\n");
}
