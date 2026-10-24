//! Contract for [`BoardOperations`] (epic #2705 B6, B7): what the board's
//! front ends are handed for a request, as every adapter must answer it.
//! A front end answers what it is handed through one path (an answer, or
//! the board's refusal, rendered by its call), so the checks judge that
//! answer: the local adapter hands typed outcomes over, a board server's
//! (B7) the answer its server rendered, and both must read alike. The
//! checks take the operations of one fresh board not bootstrapped yet
//! ([`Fixture`]): the local adapter as composition builds it for `quecto
//! board` over a clone of a bare git remote, and the remote adapter
//! composition picks for `--url`, against an in-process board server on
//! its Unix socket.
use std::path::Path;

use quecto::application::project_board::dto::{
    BoardExecution, ListSummaries, OpNoting, ReadRequest, SUMMARIES_LISTINGS_ONLY,
};
use quecto::application::project_board::ports::BoardOperations;
use quecto::application::project_board::telemetry::OpFacts;
use quecto::composition::bootstrap::board_server::build_board_server;
use quecto::composition::bootstrap::project_board::build_project_board_handles;
use quecto::domain::project_board::entities::task_parts::EntryPolicy;
use quecto::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
use quecto::infrastructure::time::remote_time::{ReferenceSource, reference_source};
use quecto::interface::cli::board_server::BoardServerEnvironment;
use quecto::interface::cli::board_server::serve::{Started, serve};
use quecto::interface::cli::controllers::board::{Access, KnownLocation, ProjectBoardEnvironment};
use quecto::interface::tools::project_board::recording::BoardCall;
use quecto::interface::tools::project_board::request::parse;
use serde_json::{Value, json};

use super::project_board_git_store::{Remote, sh};

/// A fresh board's operations, and the clock note its fixture's clock
/// gives a request that reads it (`None`: the clock has an outside
/// reference); and, for a board server, the server, stopped when dropped.
pub struct Fixture {
    pub operations: Box<dyn BoardOperations>,
    pub clock_note: Option<String>,
    pub _server: Option<Server>,
}

/// `value` handed to `operations` as the CLI reads it, and its answer as
/// the CLI renders it, with the facts noted when `noting` asks.
fn answer(
    operations: &dyn BoardOperations,
    value: Value,
    summaries: ListSummaries,
    noting: OpNoting,
) -> (Value, Option<OpFacts>) {
    let text = value.to_string();
    let request = parse(&value, EntryPolicy::SectionedOrLegacy).expect("a well-formed request");
    let op = value["op"].as_str().expect("an op");
    let read = ReadRequest {
        request: &request,
        text: &text,
        front_end: FrontEnd::Cli,
    };
    let BoardExecution { result, facts } = operations.execute(read, summaries, noting);
    let mut call = BoardCall::start(None, FrontEnd::Cli, Client::Cli);
    let answer = match result {
        Ok(answered) => call.answer(op, &answered.outcome, answered.clock_note),
        Err(error) => call.refuse_as_board(op, &error, None),
    };
    (answer, facts)
}

fn run(operations: &dyn BoardOperations, value: Value) -> Value {
    answer(operations, value, ListSummaries::Omitted, OpNoting::Unnoted).0
}

fn create(id: &str) -> Value {
    json!({"op": "create", "task": id, "title": "A task", "status": "ready"})
}

fn code(answer: &Value) -> &Value {
    assert_eq!(answer["ok"], json!(false), "refused: {answer}");
    &answer["error"]["code"]
}

fn bootstrap(operations: &dyn BoardOperations) {
    let made = run(operations, json!({"op": "bootstrap"}));
    assert_eq!(made["ok"], json!(true), "{made}");
}

fn ids(listing: &Value) -> Vec<&str> {
    let tasks = listing["tasks"].as_array().expect("a listing's tasks");
    tasks
        .iter()
        .filter_map(|task| task["id"].as_str())
        .collect()
}

/// The board is created once: a bootstrap after it finds it, at the head
/// it was created at.
pub fn check_bootstrap(fixture: Fixture) {
    let operations = &*fixture.operations;
    let made = run(operations, json!({"op": "bootstrap"}));
    assert_eq!(
        (&made["ok"], &made["created"]),
        (&json!(true), &json!(true))
    );
    let found = run(operations, json!({"op": "bootstrap"}));
    assert_eq!(found["created"], json!(false));
    assert_eq!(found["head"], made["head"], "{made} {found}");
}

/// A task created is shown and listed as written.
pub fn check_create_show_and_list(fixture: Fixture) {
    let operations = &*fixture.operations;
    bootstrap(operations);
    let created = run(operations, create("a"));
    assert_eq!(created["task"]["id"], json!("a"), "{created}");
    assert_eq!(created.get("completion"), None, "{created}");
    let shown = run(operations, json!({"op": "show", "task": "a"}));
    assert_eq!(shown["task"], created["task"]);
    assert_eq!(ids(&run(operations, json!({"op": "list"}))), ["a"]);
}

/// A listing carries summaries only when they are asked for: one per task
/// it lists, or why none was read; asked for none, it carries none. Asked
/// of a request that lists nothing, summaries are refused as invalid.
pub fn check_summaries(fixture: Fixture) {
    let operations = &*fixture.operations;
    bootstrap(operations);
    run(operations, create("a"));
    let omitted = run(operations, json!({"op": "list"}));
    assert_eq!(omitted["tasks"][0].get("summary"), None, "{omitted}");
    let listing = json!({"op": "list"});
    let (wanted, _) = answer(
        operations,
        listing,
        ListSummaries::Wanted,
        OpNoting::Unnoted,
    );
    assert_eq!(
        ids(&wanted),
        ["a"],
        "a listing is never refused for its summaries"
    );
    assert!(wanted["tasks"][0]["summary"].is_object(), "{wanted}");
    assert_eq!(
        wanted.get("summary_skipped"),
        None,
        "a quiet board: {wanted}"
    );
    let show = json!({"op": "show", "task": "a"});
    let (refused, _) = answer(operations, show, ListSummaries::Wanted, OpNoting::Unnoted);
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert!(
        refused.to_string().contains(SUMMARIES_LISTINGS_ONLY),
        "{refused}"
    );
}

/// The board's refusal reaches the front end as the use cases refuse:
/// before the board exists, of a task it does not hold, and of a task
/// created twice.
pub fn check_refusals(fixture: Fixture) {
    let operations = &*fixture.operations;
    let show = json!({"op": "show", "task": "missing"});
    assert_eq!(code(&run(operations, show.clone())), "no_board");
    bootstrap(operations);
    assert_eq!(code(&run(operations, show)), "not_found");
    run(operations, create("a"));
    assert_eq!(code(&run(operations, create("a"))), "exists");
}

/// What a request's use cases saw comes back exactly when it is noted:
/// every noted request, answered or refused, gives it back.
pub fn check_noting(fixture: Fixture) {
    let operations = &*fixture.operations;
    let show = json!({"op": "show", "task": "a"});
    let listing = json!({"op": "list"});
    let bootstrap = json!({"op": "bootstrap"});
    let unnoted = answer(
        operations,
        bootstrap.clone(),
        ListSummaries::Omitted,
        OpNoting::Unnoted,
    );
    assert_eq!(unnoted.1, None);
    let noted = |value: &Value, summaries| {
        let (answer, facts) = answer(operations, value.clone(), summaries, OpNoting::Noted);
        assert!(facts.is_some(), "noted: {value}");
        (answer["ok"] == json!(true), facts)
    };
    // Refused before the board exists, then answered.
    assert!(!noted(&show, ListSummaries::Omitted).0);
    assert!(noted(&bootstrap, ListSummaries::Omitted).0);
    let (created, facts) = noted(&create("a"), ListSummaries::Omitted);
    assert!(created);
    assert_eq!(facts.and_then(|facts| facts.attempts), Some(1));
    assert!(!noted(&create("a"), ListSummaries::Omitted).0);
    assert!(noted(&show, ListSummaries::Omitted).0);
    assert!(noted(&listing, ListSummaries::Wanted).0);
    assert!(!noted(&show, ListSummaries::Wanted).0);
}

/// A request that reads the clock carries the note the fixture's clock
/// gives; one that does not carries none.
pub fn check_clock_notes(fixture: Fixture) {
    let operations = &*fixture.operations;
    for value in [json!({"op": "bootstrap"}), json!({"op": "list"})] {
        let answered = run(operations, value.clone());
        assert_eq!(answered.get("clock"), None, "{value}: {answered}");
    }
    let created = run(operations, create("a"));
    assert_eq!(
        created["clock"]["reason"].as_str(),
        fixture.clock_note.as_deref(),
        "a create reads the clock: {created}"
    );
}

/// A front end that launches nothing owes no ends as its session ends.
pub fn check_no_left_runs(fixture: Fixture) {
    assert_eq!(fixture.operations.left_runs(), Ok(vec![]));
}

fn vars(repo: &Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    vec![
        ("PATH".into(), std::env::var_os("PATH").expect("PATH")),
        ("HOME".into(), repo.as_os_str().to_owned()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        (
            "QUECTO_BOARD_COMMITTER".into(),
            "Ada <ada@example.com>".into(),
        ),
    ]
}

/// The operations composition builds for `quecto board` in `repo`, at
/// the board server `url` when one is given.
fn built(repo: &Path, known: KnownLocation) -> Box<dyn BoardOperations> {
    let environment = ProjectBoardEnvironment {
        repo: repo.to_path_buf(),
        remote: "origin".into(),
        access: Access::Write,
        vars: vars(repo),
        cancelled: None,
        launcher: None,
        swarm_session: None,
        known,
    };
    match build_project_board_handles(&environment) {
        Ok(handles) => handles.operations,
        Err(error) => panic!("the board is built: {error:?}"),
    }
}

/// The local adapter over a fresh clone of `remote`, writing as `Ada`, as
/// composition builds it for `quecto board` (no launcher). A local remote
/// gives no outside time, for the reason [`reference_source`] says.
fn local(remote: &Remote) -> Fixture {
    let repo = remote.clone_dir(0).to_path_buf();
    let url = sh(&repo, &["remote", "get-url", "origin"], b"");
    let ReferenceSource::Unavailable(why) = reference_source(&url) else {
        panic!("a local remote gives no time: {url}")
    };
    Fixture {
        operations: built(&repo, KnownLocation::default()),
        clock_note: Some(why),
        _server: None,
    }
}

/// A board server serving until dropped.
pub struct Server {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

/// The remote adapter composition picks for `--url`, against a board
/// server for a fresh clone of `remote`, its board in a fresh state
/// directory; the server's clock reads no outside time.
fn served(remote: &Remote) -> Fixture {
    let repo = remote.clone_dir(0).to_path_buf();
    let state = repo.join(".git").join("served");
    let environment = BoardServerEnvironment {
        repo: repo.clone(),
        remote: "origin".into(),
        vars: vars(&repo),
        config: state.join("config.json"),
        state: Some(state.join("state")),
        base_dir: state.clone(),
    };
    let handles = build_board_server(&environment).expect("the board server is built");
    let board = handles.serve.board().clone();
    let socket = state.join("s").join("board.sock");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = std::sync::mpsc::channel();
    let at = socket.clone();
    let thread = std::thread::spawn(move || {
        let stopping = move || async move {
            let _ = stopped.await;
        };
        let announce = |_: &Started| {
            let _ = ready.send(());
        };
        let began = std::time::Instant::now();
        serve(handles, &at, &[], began, stopping, announce).expect("served");
    });
    started.recv().expect("the server started");
    let known = KnownLocation {
        board_id: Some(board),
        remote_url: None,
        url: Some(format!("unix://{}", socket.display())),
    };
    Fixture {
        operations: built(&repo, known),
        clock_note: Some("this machine's clock reads no outside time".into()),
        _server: Some(Server {
            stop: Some(stop),
            thread: Some(thread),
        }),
    }
}

fn each(check: fn(Fixture)) {
    let remote = Remote::new();
    check(local(&remote));
    let remote = Remote::new();
    check(served(&remote));
}

#[test]
fn board_operations_bootstrap() {
    each(check_bootstrap);
}

#[test]
fn board_operations_create_show_and_list() {
    each(check_create_show_and_list);
}

#[test]
fn board_operations_summaries() {
    each(check_summaries);
}

#[test]
fn board_operations_refusals() {
    each(check_refusals);
}

#[test]
fn board_operations_noting() {
    each(check_noting);
}

#[test]
fn board_operations_clock_notes() {
    each(check_clock_notes);
}

#[test]
fn board_operations_no_left_runs() {
    each(check_no_left_runs);
}
