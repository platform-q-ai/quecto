use super::*;
use crate::application::project_board::use_cases::BoardAccess;
use crate::infrastructure::http::board_service::listener::{
    PeerCredentials, SocketPeerCredentials, this_uid,
};
use crate::infrastructure::http::board_service::test_server::raw;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_board_tokens::MemoryBoardTokens;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::cli::board_server::routes::tests::{BOARD, operating, serving, written};
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
    let (dir, socket, _, stop, thread) = served_on(handles, &[]);
    (dir, socket, stop, thread)
}

/// A served server: its temp dir, socket, TCP addresses, stop and thread.
type ServedOn = (
    tempfile::TempDir,
    std::path::PathBuf,
    Vec<std::net::SocketAddr>,
    tokio::sync::oneshot::Sender<()>,
    std::thread::JoinHandle<Result<(), String>>,
);

/// Serves `handles` on a temp socket and `tcp`; gives the TCP addresses
/// bound too.
fn served_on(handles: BoardServerHandles, tcp: &[ListenAddress]) -> ServedOn {
    let tcp = tcp.to_vec();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = std::sync::mpsc::channel();
    let at = socket.clone();
    let thread = std::thread::spawn(move || {
        serve(
            handles,
            &at,
            &tcp,
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
    (dir, socket, started.tcp, stop, thread)
}

/// `GET path` over `socket`, as curl sends it: the status and body.
fn get(socket: &std::path::Path, path: &str) -> (u16, String) {
    let request = format!("GET {path} HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\r\n");
    raw(socket, request.as_bytes())
}

fn handles(serve: crate::application::project_board::use_cases::ServeBoard) -> BoardServerHandles {
    let tokens = Arc::new(MemoryBoardTokens::default());
    BoardServerHandles {
        serve,
        operations: operating(&MemoryBoardStore::default()),
        feed: None,
        replication: None,
        follower: None,
        access: BoardAccess::new(tokens, Arc::new(SystemBoardClock::default())),
        peers: Arc::new(SocketPeerCredentials) as Arc<dyn PeerCredentials>,
        uid: this_uid(),
        listen: vec![],
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
        &[],
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

fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

/// `--listen` is repeatable and `--state` taken once; each address is
/// judged as it is parsed.
#[test]
fn serve_takes_listen_addresses_and_a_state_directory() {
    let parsed = parse(&args(
        "serve --socket /s --repo /r --listen 127.0.0.1:7979 --listen [::1]:7979 \
         --listen 100.101.102.103:7979 --state /var/lib/board",
    ))
    .unwrap();
    let listen: Vec<String> = parsed
        .listen
        .iter()
        .map(|address| address.socket_addr().to_string())
        .collect();
    assert_eq!(
        listen,
        ["127.0.0.1:7979", "[::1]:7979", "100.101.102.103:7979"]
    );
    assert_eq!(
        parsed.state,
        Some(std::path::PathBuf::from("/var/lib/board"))
    );
    let bare = parse(&args("serve --socket /s --repo /r")).unwrap();
    assert!(bare.listen.is_empty() && bare.state.is_none());
    for refused in [
        "serve --socket /s --repo /r --state /a --state /b",
        "serve --socket /s --repo /r --listen",
        "serve --socket /s --repo /r --listen localhost:7979",
    ] {
        assert!(parse(&args(refused)).is_err(), "{refused}");
    }
}

/// B11: `--follow` names the primary; over TCP its token comes from
/// `--token-file`, never the command line; a socket's primary needs none.
#[test]
fn serve_follows_a_primary_with_its_token_read_from_a_file() {
    let tcp = parse(&args(
        "serve --socket /s --repo /r --follow http://100.64.0.1:7979 --token-file /etc/t",
    ))
    .unwrap();
    assert_eq!(
        tcp.follow,
        Some(FollowPrimary {
            url: "http://100.64.0.1:7979".into(),
            token_file: Some("/etc/t".into()),
            replace_tokens: false,
        })
    );
    let replacing = parse(&args(
        "serve --socket /s --repo /r --follow unix:///p --replace-tokens",
    ))
    .unwrap();
    assert!(replacing.follow.unwrap().replace_tokens);
    let socket = parse(&args(
        "serve --socket /s --repo /r --follow unix:///run/p.sock",
    ))
    .unwrap();
    assert_eq!(socket.follow.unwrap().token_file, None);
    assert_eq!(
        parse(&args("serve --socket /s --repo /r")).unwrap().follow,
        None
    );
    for (refused, why) in [
        (
            "serve --socket /s --repo /r --follow http://100.64.0.1:7979",
            "needs --token-file",
        ),
        (
            "serve --socket /s --repo /r --token-file /t",
            "needs --follow",
        ),
        (
            "serve --socket /s --repo /r --replace-tokens",
            "needs --follow",
        ),
        (
            "serve --socket /s --repo /r --follow unix:///p --replace-tokens --replace-tokens",
            "given twice",
        ),
        (
            "serve --socket /s --repo /r --follow unix:///p --token-file /t",
            "is for a primary over TCP",
        ),
        (
            "serve --socket /s --repo /r --follow ftp://p",
            "--follow takes",
        ),
        (
            "serve --socket /s --repo /r --follow unix:///a --follow unix:///b",
            "given twice",
        ),
    ] {
        let error = parse(&args(refused)).unwrap_err();
        assert!(error.contains(why), "{refused}: {error}");
    }
}

/// B5 red: binding to `0.0.0.0` (or `::`, or any address off the
/// allowlist) is refused at start, before anything is built or bound.
#[test]
fn binding_to_every_interface_is_refused_at_start() {
    for address in ["0.0.0.0:7979", "[::]:7979", "192.168.1.20:7979"] {
        let error = parse(&args(&format!(
            "serve --socket /s --repo /r --listen {address}"
        )))
        .unwrap_err();
        assert!(
            error.contains(&format!("--listen {address} is refused")),
            "{error}"
        );
        assert!(error.contains("100.64.0.0/10"), "{error}");
    }
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let ctx = crate::interface::cli::CliContext::default();
    let (mut out, mut err) = (String::new(), String::new());
    let command = format!(
        "serve --socket {} --repo /r --listen 0.0.0.0:7979",
        socket.display()
    );
    let code = super::super::cmd_board_server(&ctx, &args(&command), &mut out, &mut err);
    assert_eq!(code, 1);
    assert!(err.contains("--listen 0.0.0.0:7979 is refused"), "{err}");
    assert!(
        !err.contains("not composed"),
        "refused before composition: {err}"
    );
    assert!(std::fs::symlink_metadata(&socket).is_err(), "nothing bound");
}

/// `GET path` over TCP with `authorization`: the status and body.
fn tcp_get(
    address: std::net::SocketAddr,
    path: &str,
    authorization: Option<&str>,
) -> (u16, String) {
    use std::io::{Read, Write};
    let header = authorization
        .map(|value| format!("Authorization: {value}\r\n"))
        .unwrap_or_default();
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: board\r\n{header}Connection: close\r\n\r\n");
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    let status = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {answer:?}"));
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string());
    (status, body.unwrap_or_default())
}

/// `quecto board-server token <rest> --socket <socket>`: code, out, err.
fn token(socket: &std::path::Path, rest: &str) -> (i32, String, String) {
    let ctx = crate::interface::cli::CliContext::default();
    let (mut out, mut err) = (String::new(), String::new());
    let mut command = args(&format!("token {rest} --socket"));
    command.push(socket.display().to_string());
    let code = super::super::cmd_board_server(&ctx, &command, &mut out, &mut err);
    (code, out, err)
}

/// B5 red, end to end on 127.0.0.1: over TCP no token is 401, a token
/// added over the socket reads (200), admin over TCP is 403 even with it,
/// and once revoked it is 401. `token add` prints the token once, alone on
/// stdout; `token list` gives its name and date, never the token.
#[test]
fn tcp_reads_with_a_token_added_over_the_socket() {
    let (store, _) = written();
    let loopback = ListenAddress::parse("127.0.0.1:0").unwrap();
    let (_dir, socket, tcp, stop, thread) = served_on(handles(serving(&store)), &[loopback]);
    let [address] = tcp.as_slice() else {
        panic!("one TCP address: {tcp:?}")
    };
    assert!(address.ip().is_loopback() && address.port() != 0);
    let (status, body) = tcp_get(*address, "/v1/status", None);
    assert_eq!(status, 401, "{body}");
    assert!(body.contains(r#""code":"token""#), "{body}");

    let (code, out, err) = token(&socket, "add laptop");
    assert_eq!(code, 0, "{err}");
    let secret = out.trim().to_string();
    assert_eq!(out, format!("{secret}\n"), "the token alone on stdout");
    assert!(!secret.is_empty());
    assert_eq!(
        (out.clone() + &err).matches(&secret).count(),
        1,
        "shown once"
    );
    assert!(
        err.contains("token 'laptop' added; it is shown only this once"),
        "{err}"
    );

    let bearer = format!("Bearer {secret}");
    let (status, body) = tcp_get(*address, "/v1/status", Some(&bearer));
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(BOARD), "{body}");
    let (status, body) = tcp_get(*address, "/v1/admin/tokens", Some(&bearer));
    assert_eq!(status, 403, "{body}");
    assert!(body.contains(r#""code":"local_only""#), "{body}");

    let (code, out, err) = token(&socket, "list");
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("laptop\t"), "{out}");
    assert!(!(out.clone() + &err).contains(&secret), "{out}{err}");
    assert_eq!(token(&socket, "add laptop").0, 1, "a taken name");

    let (code, out, err) = token(&socket, "revoke laptop");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "board-server: token 'laptop' revoked\n");
    assert_eq!(tcp_get(*address, "/v1/status", Some(&bearer)).0, 401);
    let (code, _, err) = token(&socket, "revoke laptop");
    assert_eq!(code, 1);
    assert!(
        err.contains("no token is named 'laptop' (404 no_token)"),
        "{err}"
    );
    let (code, out, err) = token(&socket, "list");
    assert_eq!((code, out.as_str()), (0, ""));
    assert!(err.contains("no tokens"), "{err}");
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
}

#[test]
fn token_commands_need_a_server_and_their_options() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = token(&dir.path().join("none.sock"), "list");
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(err.contains("no answer from a server on"), "{err}");
    let ctx = crate::interface::cli::CliContext::default();
    for refused in [
        "token",
        "token add",
        "token add laptop",
        "token add --socket /s",
        "token remove laptop --socket /s",
        "token list --socket /s extra",
    ] {
        let (mut out, mut err) = (String::new(), String::new());
        let code = super::super::cmd_board_server(&ctx, &args(refused), &mut out, &mut err);
        assert_eq!(code, 1, "{refused}");
        assert!(err.contains(USAGE), "{refused}: {err}");
    }
}

/// An address that cannot be bound stops the start, and the socket bound
/// first is removed.
#[test]
fn an_address_in_use_is_refused_and_the_socket_removed() {
    let (store, _) = written();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = ListenAddress::parse(&taken.local_addr().unwrap().to_string()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let error = serve(
        handles(serving(&store)),
        &socket,
        &[address],
        std::time::Instant::now(),
        || async {},
        |_: &Started| panic!("never ready"),
    )
    .unwrap_err();
    assert!(error.starts_with("board-server: 127.0.0.1:"), "{error}");
    assert!(std::fs::symlink_metadata(&socket).is_err());
}

/// A clock whose `now` stalls until released (or 20 s pass), as a store
/// reading a black-hole remote does.
#[derive(Default)]
struct Stalled {
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}

impl Stalled {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

impl crate::application::project_board::ports::BoardClock for Stalled {
    fn now(
        &self,
    ) -> Result<
        crate::domain::project_board::value_objects::timestamp::Timestamp,
        crate::application::project_board::ports::BoardClockError,
    > {
        let released = self.released.lock().unwrap();
        let wait = std::time::Duration::from_secs(20);
        drop(
            self.wake
                .wait_timeout_while(released, wait, |released| !*released),
        );
        crate::application::project_board::ports::BoardClock::now(&SystemBoardClock::default())
    }
    fn reference(
        &self,
    ) -> Option<crate::domain::project_board::value_objects::timestamp::Timestamp> {
        None
    }
    fn reference_status(&self) -> crate::application::project_board::ports::ReferenceStatus {
        crate::application::project_board::ports::ReferenceStatus::NoReference("stalled".into())
    }
    fn pause(&self, _attempt: usize) {}
    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

/// Review round 4: answers stalled on a slow store (here, a clock) never
/// wedge the server. The TCP gate runs inline; one caller holds at most 4
/// blocking answers (the rest are 503 `busy`); and the Unix socket keeps
/// its own threads, so `token list` still answers, and a stolen token can
/// still be revoked.
#[test]
fn stalled_answers_never_wedge_authentication_or_the_socket() {
    use crate::application::project_board::ports::BoardTokens;
    let (store, _) = written();
    let stalled = Arc::new(Stalled::default());
    let tokens = Arc::new(MemoryBoardTokens::default());
    let at = crate::domain::project_board::value_objects::timestamp::Timestamp::parse(
        "2026-10-24T12:00:00Z",
    )
    .unwrap();
    let laptop = crate::domain::project_board::value_objects::slug::Slug::parse("laptop");
    tokens.add(&laptop.unwrap(), &at).unwrap();
    let serve = crate::application::project_board::use_cases::ServeBoard::new(
        Arc::new(store.client()),
        Arc::clone(&stalled) as Arc<dyn crate::application::project_board::ports::BoardClock>,
        crate::domain::project_board::value_objects::telemetry::BoardId::of_hex(BOARD).unwrap(),
    );
    let handles = BoardServerHandles {
        access: BoardAccess::new(tokens, Arc::new(SystemBoardClock::default())),
        ..handles(serve)
    };
    let loopback = ListenAddress::parse("127.0.0.1:0").unwrap();
    let (_dir, socket, tcp, stop, thread) = served_on(handles, &[loopback]);
    let address = tcp[0];
    let bearer = "Bearer token-laptop";
    let stalls: Vec<_> = (0..6)
        .map(|_| std::thread::spawn(move || tcp_get(address, "/v1/time", Some(bearer)).0))
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let began = std::time::Instant::now();
    assert_eq!(
        tcp_get(address, "/v1/time", None).0,
        401,
        "the gate answers"
    );
    let (status, body) = tcp_get(address, "/v1/time", Some(bearer));
    assert_eq!(status, 503, "{body}");
    assert!(body.contains(r#""code":"busy""#), "{body}");
    let (code, out, err) = token(&socket, "list");
    assert_eq!(
        (code, out.as_str()),
        (0, "laptop\t2026-10-24T12:00:00Z\tdevice\n"),
        "{err}"
    );
    assert_eq!(token(&socket, "revoke laptop").0, 0);
    assert!(
        began.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    stalled.release();
    let mut answered: Vec<u16> = stalls.into_iter().map(|s| s.join().unwrap()).collect();
    answered.sort_unstable();
    assert_eq!(answered, [200, 200, 200, 200, 503, 503]);
    stop.send(()).unwrap();
    thread.join().unwrap().unwrap();
}
