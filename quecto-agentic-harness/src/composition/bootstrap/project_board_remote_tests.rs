//! The board's operations through a board server (epic #2705 B7): an
//! in-process server over a real SQLite board, reached by the remote
//! adapter composition picks, over its Unix socket and over TCP.
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::super::board_server::{BOARD_FILE, build_board_server};
use super::super::project_board::build_project_board_handles;
use super::super::project_board_operations::fixtures::{QueryListings, assembled};
use super::*;
use crate::application::project_board::dto::NoPullRequestHost;
use crate::application::project_board::dto::{BoardOutcome, ListSummaries, OpNoting, ReadRequest};
use crate::application::project_board::ports::{BoardOperations, ListenAddress, ProjectBoardStore};
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::entities::task_parts::EntryPolicy;
use crate::domain::project_board::value_objects::telemetry::{BoardId, FrontEnd};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::cli::CliContext;
use crate::interface::cli::board_server::BoardServerEnvironment;
use crate::interface::cli::board_server::serve::serve;
use crate::interface::cli::controllers::board::KnownLocation;
use crate::interface::tools::project_board::request::parse;
use crate::interface::tools::project_board::run;

/// Test-side git: no global or system config.
fn sh(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
}

fn vars(home: &Path) -> Vec<(OsString, OsString)> {
    vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), home.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]
}

/// A board server serving a clone's board until dropped.
struct Served {
    dir: tempfile::TempDir,
    work: PathBuf,
    socket: PathBuf,
    tcp: Option<std::net::SocketAddr>,
    board: BoardId,
    /// A token added before the server started, when it listens on TCP.
    token: Option<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

impl Served {
    fn url(&self) -> String {
        format!("unix://{}", self.socket.display())
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("state").join(BOARD_FILE)
    }

    /// The board's operations at `url` for `access`, with `extra` vars.
    fn remote(
        &self,
        url: &str,
        access: Access,
        extra: &[(&str, &str)],
    ) -> Box<dyn BoardOperations> {
        let mut vars = vars(&self.work);
        vars.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into())),
        );
        let environment = ProjectBoardEnvironment {
            repo: self.work.clone(),
            remote: "origin".into(),
            known: KnownLocation {
                board_id: Some(self.board.clone()),
                remote_url: None,
                url: Some(url.into()),
            },
            access,
            vars,
            cancelled: None,
            launcher: None,
            swarm_session: None,
        };
        build_project_board_handles(&environment)
            .map_err(|error| format!("{error:?}"))
            .unwrap()
            .operations
    }

    fn writer(&self) -> Box<dyn BoardOperations> {
        self.remote(&self.url(), Access::Write, &[])
    }

    /// `(name, email, principal)` of each revision, oldest first.
    fn revisions(&self) -> Vec<(String, String, Option<String>)> {
        let db = rusqlite::Connection::open(self.db()).unwrap();
        let mut rows = db
            .prepare(
                "SELECT r.name, r.email, p.principal FROM revisions r \
                 LEFT JOIN revision_principals p ON p.head = r.head ORDER BY r.rowid",
            )
            .unwrap();
        rows.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

/// A clone of a bare remote, served from its own state directory, its
/// board made; on 127.0.0.1 too when `tcp`, with a token `laptop`.
fn served(tcp: bool) -> Served {
    let dir = tempfile::tempdir().unwrap();
    sh(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    sh(dir.path(), &["clone", "-q", "remote.git", "work"]);
    let work = dir.path().join("work");
    sh(&work, &["config", "user.name", "Ada"]);
    sh(&work, &["config", "user.email", "ada@example.com"]);
    let environment = BoardServerEnvironment {
        repo: work.clone(),
        remote: "origin".into(),
        vars: vars(&work),
        config: dir.path().join("base").join("config.json"),
        state: Some(dir.path().join("state")),
        base_dir: dir.path().join("base"),
    };
    let handles = build_board_server(&environment).unwrap();
    let board = handles.serve.board().clone();
    let token = tcp.then(|| handles.access.add("laptop").unwrap().secret);
    let socket = dir.path().join("s").join("board.sock");
    let listen: Vec<ListenAddress> = match tcp {
        true => vec![ListenAddress::parse("127.0.0.1:0").unwrap()],
        false => vec![],
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = std::sync::mpsc::channel();
    let at = socket.clone();
    let thread = std::thread::spawn(move || {
        let stop = move || async move {
            let _ = stopped.await;
        };
        let announce = |started: &crate::interface::cli::board_server::serve::Started| {
            let _ = ready.send(started.tcp.first().copied());
        };
        serve(
            handles,
            &at,
            &listen,
            std::time::Instant::now(),
            stop,
            announce,
        )
        .unwrap();
    });
    let tcp = started.recv().unwrap();
    let served = Served {
        dir,
        work,
        socket,
        tcp,
        board,
        token,
        stop: Some(stop),
        thread: Some(thread),
    };
    let made = run(&*served.writer(), &json!({"op": "bootstrap"}).to_string());
    assert_eq!(made["ok"], json!(true), "{made}");
    served
}

/// A Unix socket in front of `target` that counts its connections (one
/// per request: the client closes each) and drops the reply of the first
/// `drop` of them, after the server made it.
struct Proxy {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    connections: Arc<AtomicUsize>,
}

fn proxy(target: &Path, drop: usize) -> Proxy {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("proxy.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let (target, counted) = (target.to_path_buf(), Arc::clone(&connections));
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(mut client) = client else { return };
            let nth = counted.fetch_add(1, Ordering::SeqCst);
            let target = target.clone();
            std::thread::spawn(move || {
                let mut server = UnixStream::connect(&target).unwrap();
                let mut sending = client.try_clone().unwrap();
                let mut upstream = server.try_clone().unwrap();
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut sending, &mut upstream);
                });
                let mut reply = Vec::new();
                let _ = server.read_to_end(&mut reply);
                if nth >= drop {
                    let _ = client.write_all(&reply);
                }
                let _ = client.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    Proxy {
        _dir: dir,
        socket,
        connections,
    }
}

fn read(text: &Value) -> crate::application::project_board::dto::BoardRequest {
    parse(text, EntryPolicy::SectionedOrLegacy).unwrap()
}

/// `request` through `operations` as the CLI sends it, noted as asked.
fn executed(
    operations: &dyn BoardOperations,
    request: &Value,
    noting: OpNoting,
) -> crate::application::project_board::dto::BoardExecution {
    let text = request.to_string();
    let typed = read(request);
    let sent = ReadRequest {
        request: &typed,
        text: &text,
        front_end: FrontEnd::Cli,
    };
    operations.execute(sent, ListSummaries::Omitted, noting)
}

fn ready(id: &str) -> Value {
    json!({"op": "create", "task": id, "title": id.to_uppercase(), "status": "ready"})
}

/// The `project_board_operations` contract, through the remote adapter:
/// a request is answered as the local adapter answers it, noted as asked,
/// a refusal and a listing pass through, and no run is left.
#[test]
fn the_operations_contract_holds_through_a_board_server() {
    let served = served(false);
    let memory = MemoryBoardStore::default();
    memory.bootstrap().unwrap();
    let listings = QueryListings(QueryBoard::new(Arc::new(memory.client())));
    let local = assembled(
        Arc::new(memory.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(listings),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let remote = served.writer();
    let unnoted = executed(&*remote, &ready("a"), OpNoting::Unnoted);
    let Ok(answered) = &unnoted.result else {
        panic!("created: {unnoted:?}")
    };
    let BoardOutcome::Served(answer) = &answered.outcome else {
        panic!("a server's answer: {answered:?}")
    };
    let answer: Value = serde_json::from_str(&answer.0).unwrap();
    assert_eq!(
        (&answer["ok"], &answer["task"]["id"]),
        (&json!(true), &json!("a"))
    );
    assert_eq!(unnoted.facts, None, "unnoted: nothing noted");
    let noted = executed(&*remote, &ready("b"), OpNoting::Noted);
    assert!(noted.result.is_ok(), "{noted:?}");
    let facts = noted.facts.expect("a noted request gives back its facts");
    assert_eq!((facts.attempts, facts.lost), (Some(1), Some(0)));
    assert!(facts.head.is_some(), "the write's revision");
    // A refusal reads exactly as the local adapter's.
    let missing = json!({"op": "show", "task": "missing"}).to_string();
    let (here, there) = (run(&local, &missing), run(&*remote, &missing));
    assert_eq!(there, here);
    assert_eq!(there["ok"], json!(false));
    let refused = executed(
        &*remote,
        &json!({"op": "show", "task": "missing"}),
        OpNoting::Noted,
    );
    assert!(refused.facts.is_some(), "a refusal is noted too");
    let listed = run(&*remote, &json!({"op": "list"}).to_string());
    let ids: Vec<&Value> = listed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| &t["id"])
        .collect();
    assert_eq!(ids, [&json!("a"), &json!("b")], "{listed}");
    assert_eq!(remote.left_runs(), Ok(vec![]));
}

/// A write whose answer was lost is sent again with its write id and is
/// answered with the first answer: one revision, never two.
#[test]
fn a_dropped_answer_replayed_by_its_write_id_makes_one_revision() {
    let served = served(false);
    let before = served.revisions().len();
    let dropping = proxy(&served.socket, 1);
    let url = format!("unix://{}", dropping.socket.display());
    let remote = served.remote(&url, Access::Write, &[]);
    let created = run(&*remote, &ready("a").to_string());
    assert_eq!(
        created["ok"],
        json!(true),
        "the retry is the first answer: {created}"
    );
    assert_eq!(dropping.connections.load(Ordering::SeqCst), 2, "sent twice");
    assert_eq!(served.revisions().len(), before + 1, "written once");
    // The same write id again, raw: the kept answer, byte for byte.
    let body = json!({"request": ready("b"), "front_end": "cli", "write_id": ID,
                      "committer": {"name": "Ada", "email": "ada@example.com"}})
    .to_string();
    let send = || {
        crate::infrastructure::http::board_service::client::BoardServiceClient::unix(&served.socket)
            .send_read(
                crate::infrastructure::http::board_service::limits::Method::Post,
                &format!("/v1/boards/{}/op", served.board.as_str()),
                body.as_bytes(),
            )
            .unwrap()
    };
    let (first, again) = (send(), send());
    assert_eq!(first.status, 200);
    assert_eq!(again, first, "replayed as first answered");
    assert_eq!(served.revisions().len(), before + 2);
}

const ID: &str = "0b7a1c0d-e000-4000-8000-000000000001";

/// `body` posted to the op route over `client`: status, code, answer.
fn posted(
    client: &crate::infrastructure::http::board_service::client::BoardServiceClient,
    board: &BoardId,
    body: &Value,
) -> (u16, Value) {
    let path = format!("/v1/boards/{}/op", board.as_str());
    let method = crate::infrastructure::http::board_service::limits::Method::Post;
    let reply = client
        .send_read(method, &path, body.to_string().as_bytes())
        .unwrap();
    (
        reply.status,
        serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
    )
}

/// A write id is kept per principal and per request: the same id for
/// another request is 409 `write_id_reused`, and a token cannot replay a
/// local write's id (its write is its own).
#[test]
fn a_write_id_is_kept_per_principal_and_request() {
    use crate::infrastructure::http::board_service::client::{
        BearerToken, BoardServiceClient, Endpoint,
    };
    let served = served(true);
    let local = BoardServiceClient::unix(&served.socket);
    let token = BearerToken::parse(served.token.as_deref().unwrap()).unwrap();
    let device = BoardServiceClient::new(Endpoint::Tcp(served.tcp.unwrap()), Some(token));
    let ada = json!({"name": "Ada", "email": "ada@example.com"});
    let body = |id: &str| {
        json!({"request": ready("a"), "front_end": "cli", "write_id": id,
                                 "committer": ada})
    };
    let (status, first) = posted(&local, &served.board, &body(ID));
    assert_eq!(
        (status, &first["answer"]["ok"]),
        (200, &json!(true)),
        "{first}"
    );
    let mut other = body(ID);
    other["request"] = ready("b");
    let (status, reused) = posted(&local, &served.board, &other);
    assert_eq!((status, &reused["code"]), (409, &json!("write_id_reused")));
    other["request"] = ready("a");
    other["committer"] = json!({"name": "Bob", "email": "bob@example.com"});
    assert_eq!(
        posted(&local, &served.board, &other).0,
        409,
        "another committer"
    );
    let (status, theirs) = posted(&device, &served.board, &body(ID));
    assert_eq!(status, 200);
    assert_eq!(
        theirs["answer"]["error"]["code"],
        json!("exists"),
        "run, not replayed"
    );
    assert_eq!(served.revisions().len(), 2, "made, and one create");
}

/// Over TCP one token holds 4 answers at once: 12 creates at once are
/// answered busy past that, and each is sent again until it lands.
#[test]
fn busy_writes_over_tcp_are_retried_until_they_land() {
    let served = served(true);
    let token = served.token.clone().unwrap();
    let url = format!("http://{}", served.tcp.unwrap());
    let remote: Arc<dyn BoardOperations> =
        Arc::from(served.remote(&url, Access::Write, &[("QUECTO_BOARD_TOKEN", &token)]));
    let threads: Vec<_> = (0..12)
        .map(|n| {
            let remote = Arc::clone(&remote);
            std::thread::spawn(move || run(&*remote, &ready(&format!("t{n}")).to_string()))
        })
        .collect();
    for thread in threads {
        let answer = thread.join().unwrap();
        assert_eq!(answer["ok"], json!(true), "{answer}");
    }
    assert_eq!(served.revisions().len(), 13);
}

/// 8 writers × 50 changes at once: every change lands (none lost to a
/// race the lock cannot lose), and each task ends at its writer's last.
#[test]
fn contention_loses_no_update() {
    const WRITERS: usize = 8;
    const CHANGES: usize = 50;
    let served = served(false);
    let remote: Arc<dyn BoardOperations> = Arc::from(served.writer());
    for writer in 0..WRITERS {
        let made = run(
            &*remote,
            &json!({"op": "create", "task": format!("t{writer}"),
                                          "title": "T", "status": "draft"})
            .to_string(),
        );
        assert_eq!(made["ok"], json!(true), "{made}");
    }
    let began = std::time::Instant::now();
    let threads: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let remote = Arc::clone(&remote);
            std::thread::spawn(move || {
                let (mut attempts, mut lost) = (0, 0);
                for change in 0..CHANGES {
                    let request = json!({"op": "update_draft", "task": format!("t{writer}"),
                                         "title": format!("T {writer} {change}")});
                    let done = executed(&*remote, &request, OpNoting::Noted);
                    let facts = done.facts.unwrap_or_default();
                    let Ok(answered) = done.result else {
                        panic!("{writer}/{change}")
                    };
                    let BoardOutcome::Served(answer) = answered.outcome else {
                        panic!()
                    };
                    let answer: Value = serde_json::from_str(&answer.0).unwrap();
                    assert_eq!(answer["ok"], json!(true), "{answer}");
                    attempts += facts.attempts.unwrap_or(0);
                    lost += facts.lost.unwrap_or(0);
                }
                (attempts, lost)
            })
        })
        .collect();
    let (attempts, lost) = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .fold((0, 0), |(a, l), (attempts, lost)| (a + attempts, l + lost));
    let changes = (WRITERS * CHANGES) as u64;
    eprintln!(
        "contention: {changes} changes, {attempts} attempts, {lost} races lost, {:?}",
        began.elapsed()
    );
    assert!(lost * 50 <= changes, "at most 2% of races lost: {lost}");
    assert_eq!(attempts, changes + lost);
    let listed = run(&*remote, &json!({"op": "list"}).to_string());
    for writer in 0..WRITERS {
        let shown = run(
            &*remote,
            &json!({"op": "show", "task": format!("t{writer}")}).to_string(),
        );
        assert_eq!(
            shown["task"]["title"],
            json!(format!("T {writer} {}", CHANGES - 1)),
            "{listed}"
        );
    }
    assert_eq!(served.revisions().len(), 1 + WRITERS + WRITERS * CHANGES);
}

/// `quecto board <write> --url` is one request to the server.
#[test]
fn a_cli_write_is_one_round_trip() {
    let served = served(false);
    let counting = proxy(&served.socket, 0);
    let ctx = CliContext {
        project_board: Some(build_project_board_handles),
        ..CliContext::default()
    };
    let args: Vec<String> = [
        "create", "--task", "x", "--title", "X", "--status", "ready", "--repo",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .chain([served.work.display().to_string()])
    .chain(["--board-id".into(), served.board.as_str().into()])
    .chain([
        "--url".into(),
        format!("unix://{}", counting.socket.display()),
    ])
    .collect();
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code =
        crate::interface::cli::controllers::board::cmd_board(&ctx, &args, &mut stdout, &mut stderr);
    let answer: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    assert_eq!((code, &answer["ok"]), (0, &json!(true)), "{stdout}{stderr}");
    assert_eq!(
        counting.connections.load(Ordering::SeqCst),
        1,
        "one round trip"
    );
}

/// The committer asserted in `QUECTO_BOARD_COMMITTER` writes each
/// revision, recorded beside who the server let in.
#[test]
fn the_asserted_committer_and_the_principal_are_recorded() {
    let served = served(false);
    let pocket = [(
        "QUECTO_BOARD_COMMITTER",
        "quecto pocket <pocket@quecto.invalid>",
    )];
    let remote = served.remote(&served.url(), Access::Write, &pocket);
    assert_eq!(run(&*remote, &ready("a").to_string())["ok"], json!(true));
    let last = served.revisions().pop().unwrap();
    assert_eq!(
        last,
        (
            "quecto pocket".into(),
            "pocket@quecto.invalid".into(),
            Some("local".into())
        )
    );
}

/// Over TCP an op needs a held token, and the board's admin ops are
/// refused; the token is never in an answer.
#[test]
fn over_tcp_an_op_needs_its_token_and_no_admin_op_is_served() {
    let served = served(true);
    let token = served.token.clone().unwrap();
    let url = format!("http://{}", served.tcp.unwrap());
    let remote = served.remote(&url, Access::Write, &[("QUECTO_BOARD_TOKEN", &token)]);
    let created = run(&*remote, &ready("a").to_string());
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        served.revisions().pop().unwrap().2,
        Some("token:laptop".into())
    );
    let made = run(&*remote, &json!({"op": "bootstrap"}).to_string());
    assert_eq!(made["ok"], json!(false), "{made}");
    assert!(made.to_string().contains("local_only"), "{made}");
    assert!(!made.to_string().contains(&token) && !created.to_string().contains(&token));
    let wrong = served.remote(
        &url,
        Access::Write,
        &[("QUECTO_BOARD_TOKEN", "x-wrong-token-0000")],
    );
    let refused = run(&*wrong, &ready("b").to_string());
    assert!(refused.to_string().contains("401 token"), "{refused}");
    assert!(
        !refused.to_string().contains("x-wrong-token-0000"),
        "{refused}"
    );
}

/// The op route answers each refusal by its typed code.
#[test]
fn the_op_route_refuses_by_typed_codes() {
    use crate::infrastructure::http::board_service::client::BoardServiceClient;
    use crate::infrastructure::http::board_service::limits::Method;
    let served = served(false);
    let client = BoardServiceClient::unix(&served.socket);
    let path = format!("/v1/boards/{}/op", served.board.as_str());
    let code = |method, path: &str, body: &Value| {
        let reply = client
            .send_read(method, path, body.to_string().as_bytes())
            .unwrap();
        let body: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        (
            reply.status,
            body["code"].as_str().unwrap_or_default().to_string(),
        )
    };
    let read = json!({"request": {"op": "list"}, "front_end": "cli"});
    assert_eq!(code(Method::Post, &path, &read).0, 200);
    assert_eq!(
        code(Method::Post, "/v1/boards/0123456789abcdef/op", &read),
        (404, "board".into())
    );
    assert_eq!(code(Method::Get, &path, &read), (405, "method".into()));
    for malformed in [
        json!({"request": {"op": "list"}, "front_end": "pocket"}),
        json!({"request": "list", "front_end": "cli"}),
        json!({"request": {"op": "list"}, "front_end": "cli", "extra": 1}),
        json!({"request": ready("a"), "front_end": "cli", "write_id": "NOT-HEX!",
               "committer": {"name": "Ada", "email": "ada@example.com"}}),
        json!({"request": ready("a"), "front_end": "cli", "write_id": "0123abcd-0000",
               "committer": {"name": "Ada", "email": "ada@example.com"}}),
        json!({"request": ready("a"), "front_end": "cli", "write_id": ID,
               "committer": {"name": "Ada <x>", "email": "ada@example.com"}}),
    ] {
        assert_eq!(
            code(Method::Post, &path, &malformed),
            (400, "request".into()),
            "{malformed}"
        );
    }
    let unwritten = json!({"request": ready("a"), "front_end": "cli"});
    assert_eq!(code(Method::Post, &path, &unwritten), (400, "write".into()));
    // A key named twice anywhere in the body is refused, as the front
    // ends refuse it.
    let twice = br#"{"request": {"op": "list", "op": "list"}, "front_end": "cli"}"#;
    let reply = client.send_read(Method::Post, &path, twice).unwrap();
    assert_eq!(reply.status, 400);
}

/// A request sent within a read's reply time.
trait SendRead {
    fn send_read(
        &self,
        method: crate::infrastructure::http::board_service::limits::Method,
        path: &str,
        body: &[u8],
    ) -> Result<
        crate::infrastructure::http::board_service::client::ClientReply,
        crate::infrastructure::http::board_service::client::ClientError,
    >;
}

impl SendRead for crate::infrastructure::http::board_service::client::BoardServiceClient {
    fn send_read(
        &self,
        method: crate::infrastructure::http::board_service::limits::Method,
        path: &str,
        body: &[u8],
    ) -> Result<
        crate::infrastructure::http::board_service::client::ClientReply,
        crate::infrastructure::http::board_service::client::ClientError,
    > {
        let reply = crate::infrastructure::http::board_service::client::READ_REPLY;
        self.send(method, path, body, reply)
    }
}
