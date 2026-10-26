//! A follower end to end (epic #2705 B11): a real primary on a temp socket
//! and loopback TCP, its follower built by composition from `--follow
//! http://..` and a token file, caught up over the wire: the copy's head is
//! the primary's, a write to it is 409 `standby`, its status shows its lag,
//! and a token revoked on the primary is revoked on it.
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::tests::{Synced, environment, seeded};
use super::*;
use crate::application::project_board::ports::ListenAddress;
use crate::infrastructure::http::board_service::client::{BoardServiceClient, READ_REPLY};
use crate::infrastructure::http::board_service::limits::Method;
use crate::infrastructure::http::board_service::listener::{
    ConnectionLimits, Listener, ServiceHandler, ServiceMethod, ServiceParts, ServiceRequest,
    Transport, bind, bind_tcp, serve,
};
use crate::interface::cli::board_server::routes::BoardRoutes;

/// A primary serving on a temp socket and loopback TCP until dropped.
struct Primary {
    client: BoardServiceClient,
    board: BoardId,
    tcp: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Primary {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

fn primary(handles: BoardServerHandles, socket: &Path) -> Primary {
    let board = handles.serve.board().clone();
    let loopback = ListenAddress::parse("127.0.0.1:0").unwrap();
    let (tcp_listener, tcp) = bind_tcp(loopback.socket_addr()).unwrap();
    let listeners = vec![
        Listener::Unix(bind(socket).unwrap()),
        Listener::Tcp(tcp_listener),
    ];
    let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations)
        .with_feed(handles.feed)
        .with_replication(handles.replication);
    let parts = ServiceParts {
        handler: Arc::new(routes),
        peers: handles.peers,
        uid: handles.uid,
        limits: ConnectionLimits::PRODUCTION,
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let stop = async {
            let _ = stopped.await;
        };
        runtime.block_on(serve(listeners, parts, stop)).unwrap();
        runtime.shutdown_timeout(Duration::from_secs(1));
    });
    Primary {
        client: BoardServiceClient::unix(socket),
        board,
        tcp,
        stop: Some(stop),
        thread: Some(thread),
    }
}

/// Runs `request` on the primary as a write.
fn write(primary: &Primary, request: Value) {
    let body = json!({"request": request, "front_end": "cli",
        "write_id": uuid::Uuid::new_v4().to_string(),
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let path = format!("/v1/boards/{}/op", primary.board.as_str());
    let reply = (primary.client)
        .send(Method::Post, &path, body.to_string().as_bytes(), READ_REPLY)
        .unwrap();
    let answer: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(
        (reply.status, &answer["answer"]["ok"]),
        (200, &json!(true)),
        "{answer}"
    );
}

/// A token file holding `token`, of mode `mode`.
fn token_file(dir: &Path, token: &str, mode: u32) -> std::path::PathBuf {
    let path = dir.join("primary.token");
    std::fs::write(&path, format!("{token}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

fn answered(routes: &BoardRoutes, method: ServiceMethod, path: &str, body: Value) -> (u16, Value) {
    let reply = routes.answer(&ServiceRequest {
        method,
        path: path.into(),
        body: body.to_string().into_bytes(),
        transport: Transport::Local,
        principal: None,
        query: None,
        last_event_id: None,
    });
    (reply.status, serde_json::from_slice(&reply.body).unwrap())
}

#[test]
fn a_follower_over_tcp_catches_up_refuses_writes_and_copies_a_revoke() {
    let (dir, work) = seeded();
    let built = build_board_server_with(&environment(&dir, &work, "origin"), Arc::new(Synced));
    let handles = built.unwrap();
    let replica = crate::application::project_board::ports::TokenKind::Replica;
    let eccup = handles.access.add_kind("eccup", replica).unwrap();
    let phone = handles.access.add("phone").unwrap();
    let socket = dir.path().join("p").join("board.sock");
    let primary = primary(handles, &socket);
    write(&primary, json!({"op": "bootstrap"}));
    for at in 0..5 {
        let task = format!("t{at}");
        write(
            &primary,
            json!({"op": "create", "task": task, "title": "T", "status": "ready"}),
        );
    }
    let token = token_file(dir.path(), &eccup.secret, 0o600);
    let follow = FollowPrimary {
        url: format!("http://{}", primary.tcp),
        token_file: Some(token),
    };
    let mut following = environment(&dir, &work, "origin");
    following.state = Some(dir.path().join("follower"));
    following.follow = Some(follow);
    let handles = build_board_server_with(&following, Arc::new(Synced)).unwrap();
    let (replicate, named) = handles.follower.clone().expect("a follower");
    assert_eq!(named, format!("http://{}", primary.tcp));
    let caught = replicate.catch_up().expect("caught up over TCP");
    assert_eq!(caught.applied, 6);
    let routes = BoardRoutes::new(handles.serve, handles.access, handles.operations)
        .with_replication(handles.replication)
        .standby(named.clone());
    let (status, body) = answered(&routes, ServiceMethod::Get, "/v1/status", Value::Null);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["role"], "follower");
    assert_eq!(body["head"], "1:6");
    assert_eq!(body["replication"]["applied"], 6);
    assert_eq!(body["replication"]["primary_head"], 6);
    let op = format!("/v1/boards/{}/op", primary.board.as_str());
    let create = json!({"request": {"op": "create", "task": "x", "title": "X", "status": "ready"},
        "front_end": "cli", "write_id": uuid::Uuid::new_v4().to_string(),
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let (status, body) = answered(&routes, ServiceMethod::Post, &op, create);
    assert_eq!((status, &body["code"]), (409, &json!("standby")), "{body}");
    assert_eq!(body["primary"], named.as_str());
    let list = json!({"request": {"op": "list"}, "front_end": "cli"});
    let (status, body) = answered(&routes, ServiceMethod::Post, &op, list);
    assert_eq!(
        (status, &body["answer"]["ok"]),
        (200, &json!(true)),
        "{body}"
    );
    // A token revoked on the primary is revoked here at the next batch.
    let revoke = json!({"name": "phone"}).to_string();
    let reply = (primary.client)
        .send(
            Method::Post,
            "/v1/admin/tokens/revoke",
            revoke.as_bytes(),
            READ_REPLY,
        )
        .unwrap();
    assert_eq!(reply.status, 200);
    let gate = |token: &str| routes.authenticate_remote(Some(&format!("Bearer {token}")));
    assert!(gate(&phone.secret).is_ok(), "copied while held");
    replicate.catch_up().unwrap();
    assert!(gate(&phone.secret).is_err(), "revoked here too");
    assert_eq!(gate(&eccup.secret).ok().as_deref(), Some("replica:eccup"));
    // Running, it follows the primary's feed: a write there lands here.
    let running = Arc::new(AtomicBool::new(true));
    let runner = {
        let (replicate, running) = (Arc::clone(&replicate), Arc::clone(&running));
        std::thread::spawn(move || replicate.run(&|| running.load(Ordering::SeqCst)))
    };
    write(
        &primary,
        json!({"op": "create", "task": "late", "title": "L", "status": "ready"}),
    );
    let began = std::time::Instant::now();
    while replicate.status().applied < 7 {
        assert!(began.elapsed() < Duration::from_secs(20), "never followed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(replicate.status().state.as_str(), "following");
    // Stopped, and its primary gone, it ends.
    running.store(false, Ordering::SeqCst);
    drop(primary);
    runner.join().unwrap();
}

#[test]
fn a_token_file_another_user_may_read_is_refused_unquoted() {
    let (dir, work) = seeded();
    let secret = "A-SECRET-TOKEN-OF-SIXTEEN-OR-MORE";
    let mut following = environment(&dir, &work, "origin");
    following.follow = Some(FollowPrimary {
        url: "http://127.0.0.1:9".into(),
        token_file: Some(token_file(dir.path(), secret, 0o644)),
    });
    let Err(refused) = build_board_server_with(&following, Arc::new(Synced)) else {
        panic!("a readable token file is refused");
    };
    assert!(refused.contains("no one else may read"), "{refused}");
    assert!(!refused.contains(secret), "{refused}");
}
