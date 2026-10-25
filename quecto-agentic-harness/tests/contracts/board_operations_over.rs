//! Contract for [`BoardOperationsOver`] (epic #2705 B7), on the operations
//! a board server assembles for each request: they run on the store they
//! are handed, so every write of a request lands in that store alone, and
//! they launch nothing: a start is begun for its client to launch (B8).
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use quecto::application::project_board::dto::{
    BoardOutcome, ListSummaries, NoPullRequestHost, OpNoting, ReadRequest,
};
use quecto::application::project_board::ports::{
    BoardOpStore, BoardOperations, BoardOperationsOver, ClockSync, OpWrite, ProjectBoardStore,
    PullRequestState, PullRequestStates, PullRequestStatesError, ServeError, ServedRun, TimeSync,
    Transacted, WriteId,
};
use quecto::application::project_board::run_facts::RunFacts;
use quecto::application::project_board::start::StartUndo;
use quecto::application::project_board::use_cases::ServeBoardOperation;
use quecto::composition::bootstrap::board_server::ServedOperations;
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::entities::task_parts::EntryPolicy;
use quecto::domain::project_board::value_objects::telemetry::Client;
use quecto::domain::project_board::value_objects::telemetry::FrontEnd;
use quecto::domain::project_board::value_objects::vocabulary::TaskStatus;
use quecto::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use quecto::infrastructure::time::board_clock::SystemBoardClock;
use quecto::interface::tools::project_board::recording::BoardCall;
use quecto::interface::tools::project_board::request::parse;
use quecto::interface::tools::project_board::run_recorded_with;
use serde_json::{Value, json};

use super::project_board_git_store::Remote;

/// A host whose clock is synchronised (the board server's gate, #2705).
struct Synced;

impl TimeSync for Synced {
    fn sync(&self) -> ClockSync {
        ClockSync::Synced
    }
}

fn served(remote: &Remote) -> ServedOperations {
    let revisions = GitRemoteRevisions::new(remote.git(0, "Ada"), "origin").expect("revisions");
    ServedOperations {
        clock: Arc::new(SystemBoardClock::default()),
        sync: Arc::new(Synced),
        prs: Err(NoPullRequestHost::NotAPullRequestHost),
        revisions: Arc::new(revisions),
        read_budget: Duration::from_secs(30),
        revision_reads: Arc::default(),
    }
}

#[test]
fn the_operations_run_on_the_store_they_are_handed() {
    let remote = Remote::new();
    let served = served(&remote);
    let (here, there) = (MemoryBoardStore::default(), MemoryBoardStore::default());
    for store in [&here, &there] {
        store.bootstrap().expect("a board");
    }
    let over = served
        .prepare(None, &ServedRun::Plain, &there.client())
        .expect("prepared");
    let operations = over(Arc::new(here.client())).expect("the operations are built");
    let value = json!({"op": "create", "task": "a", "title": "A", "status": "ready"});
    let request = parse(&value, EntryPolicy::SectionedOrLegacy).expect("a request");
    let read = ReadRequest {
        request: &request,
        text: &value.to_string(),
        front_end: FrontEnd::Cli,
    };
    let executed = operations.execute(read, ListSummaries::Omitted, OpNoting::Noted);
    let outcome = executed.result.expect("created").outcome;
    assert!(
        matches!(outcome, BoardOutcome::Changed { .. }),
        "{outcome:?}"
    );
    assert_eq!(executed.facts.and_then(|facts| facts.attempts), Some(1));
    assert_eq!(here.client().read().expect("read").tasks.len(), 1);
    assert!(there.client().read().expect("read").tasks.is_empty());
    assert_eq!(operations.left_runs(), Ok(vec![]), "they launch nothing");
}

/// A host that answers every pull request open, taking 3 s while `slow`.
/// A host that answers every pull request open, after `delay_ms`,
/// counting its reads.
#[derive(Default)]
struct SlowHost {
    delay_ms: AtomicU64,
    calls: AtomicUsize,
}

impl PullRequestStates for SlowHost {
    fn state(&self, _: u64) -> Result<PullRequestState, PullRequestStatesError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(self.delay_ms.load(Ordering::SeqCst)));
        Ok(PullRequestState::Open)
    }
}

/// `value` served as the CLI sends it, a write as Ada with a fresh id.
fn serve(operations: &ServeBoardOperation, value: Value) -> Result<Value, ServeError> {
    let id = format!("{}", uuid::Uuid::new_v4());
    match served_as(operations, value, &id)? {
        Transacted::Answered(answer) => Ok(serde_json::from_str(&answer).expect("JSON")),
        other => panic!("{other:?}"),
    }
}

/// `value` served as the CLI sends it, a write as Ada with write id `id`.
fn served_as(
    operations: &ServeBoardOperation,
    value: Value,
    id: &str,
) -> Result<Transacted, ServeError> {
    let text = value.to_string();
    let request = parse(&value, EntryPolicy::SectionedOrLegacy).expect("a request");
    let write = OpWrite {
        write_id: WriteId::parse(id).expect("a UUID"),
        request: text.clone(),
        committer: Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        },
        principal: "local".into(),
    };
    let write = request.writes().then_some(&write);
    let mut present = |board: &dyn BoardOperations| {
        let mut call = BoardCall::start(None, FrontEnd::Cli, Client::Cli);
        Ok(run_recorded_with(board, &text, ListSummaries::Omitted, &mut call).to_string())
    };
    operations.execute(Some(&request), write, &mut present)
}

/// A board server's operations over a fresh SQLite board read with
/// `host` within `budget`, task `a` claimed and linking `prs` (read fast).
fn board(
    remote: &Remote,
    host: &Arc<SlowHost>,
    budget: Duration,
    prs: &[u64],
) -> (tempfile::TempDir, Arc<ServeBoardOperation>) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let identity = Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    };
    let ops = SqliteBoardOps::open(&dir.path().join("board.sqlite3"), identity).expect("a board");
    let served = ServedOperations {
        prs: Ok(Arc::clone(host) as Arc<dyn PullRequestStates>),
        read_budget: budget,
        ..served(remote)
    };
    let operations = Arc::new(ServeBoardOperation::new(
        Arc::new(ops) as Arc<dyn BoardOpStore>,
        Arc::new(served),
    ));
    let mut setup = vec![
        json!({"op": "bootstrap"}),
        json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
        json!({"op": "move", "task": "a", "to": "claimed"}),
    ];
    setup.extend(
        prs.iter()
            .map(|pr| json!({"op": "link_pr", "task": "a", "pr": pr})),
    );
    for value in setup {
        let answer = serve(&operations, value.clone()).expect("served");
        assert_eq!(answer["ok"], json!(true), "{value}: {answer}");
    }
    host.calls.store(0, Ordering::SeqCst);
    (dir, operations)
}

/// B7 round 1: the pull requests a write may complete its task by are read
/// before the board's write lock: a slow host holds up neither an
/// unrelated create nor a listing.
#[test]
fn a_slow_host_holds_up_no_other_write_or_read() {
    let (remote, host) = (Remote::new(), Arc::new(SlowHost::default()));
    let (_dir, operations) = board(&remote, &host, Duration::from_secs(30), &[7]);
    host.delay_ms.store(3000, Ordering::SeqCst);
    let completing = {
        let operations = Arc::clone(&operations);
        std::thread::spawn(move || {
            serve(
                &operations,
                json!({"op": "done_without_run", "task": "a", "reason": "r"}),
            )
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    let began = Instant::now();
    let created = serve(
        &operations,
        json!({"op": "create", "task": "b", "title": "B"}),
    );
    let listed = serve(&operations, json!({"op": "list"}));
    let took = began.elapsed();
    assert_eq!(
        (
            &created.expect("created")["ok"],
            &listed.expect("listed")["ok"]
        ),
        (&json!(true), &json!(true))
    );
    assert!(took < Duration::from_millis(1500), "held up {took:?}");
    let completed = completing.join().expect("the completion").expect("served");
    assert_eq!(completed["op"], json!("done_without_run"), "{completed}");
}

/// B7 round 2: a write is admitted before its host reads, so 16 slow
/// completions at once read the host once per write admitted, the rest
/// are busy at once with no read, and a listing is never held up; a task
/// not on the board reads no host at all.
#[test]
fn writes_are_admitted_before_their_host_reads() {
    let (remote, host) = (Remote::new(), Arc::new(SlowHost::default()));
    let (_dir, operations) = board(&remote, &host, Duration::from_secs(30), &[7]);
    host.delay_ms.store(2000, Ordering::SeqCst);
    let completing: Vec<_> = (0..16)
        .map(|_| {
            let operations = Arc::clone(&operations);
            std::thread::spawn(move || {
                let began = Instant::now();
                let served = serve(
                    &operations,
                    json!({"op": "done_without_run", "task": "a", "reason": "r"}),
                );
                (served, began.elapsed())
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    let began = Instant::now();
    assert!(serve(&operations, json!({"op": "list"})).is_ok());
    assert!(
        began.elapsed() < Duration::from_millis(1500),
        "{:?}",
        began.elapsed()
    );
    let mut admitted = 0;
    for thread in completing {
        match thread.join().expect("a completion") {
            (Ok(_), _) => admitted += 1,
            (Err(ServeError::Busy), took) => assert!(took < Duration::from_millis(500), "{took:?}"),
            (Err(error), _) => panic!("{error:?}"),
        }
    }
    assert!((1..=2).contains(&admitted), "{admitted}");
    assert_eq!(
        host.calls.load(Ordering::SeqCst),
        admitted,
        "one read per write admitted"
    );
    let absent = serve(
        &operations,
        json!({"op": "done_without_run", "task": "nosuch", "reason": "r"}),
    );
    assert_eq!(absent.expect("served")["ok"], json!(false));
    assert_eq!(
        host.calls.load(Ordering::SeqCst),
        admitted,
        "no read for a task not on the board"
    );
}

/// B7 round 2: the reads share one time budget, as locally: past it a
/// pull request left unread is unavailable, so the write is answered
/// `pull_requests_unavailable` in time, never failed while it lands.
#[test]
fn the_host_reads_share_one_budget() {
    let (remote, host) = (Remote::new(), Arc::new(SlowHost::default()));
    let (_dir, operations) = board(&remote, &host, Duration::from_millis(1000), &[1, 2, 3, 4]);
    host.delay_ms.store(400, Ordering::SeqCst);
    let began = Instant::now();
    let completed = serve(
        &operations,
        json!({"op": "done_without_run", "task": "a", "reason": "r"}),
    );
    let completed = completed.expect("answered, not failed");
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "{:?}",
        began.elapsed()
    );
    let code = &completed["error"]["code"];
    assert_eq!(code, &json!("pull_requests_unavailable"), "{completed}");
    assert_eq!(
        host.calls.load(Ordering::SeqCst),
        3,
        "reads stop at the budget"
    );
}

/// B7 round 3: a write already answered is replayed from its kept answer
/// before any host is read.
#[test]
fn a_replay_reads_no_host() {
    let (remote, host) = (Remote::new(), Arc::new(SlowHost::default()));
    let (_dir, operations) = board(&remote, &host, Duration::from_secs(30), &[7]);
    let id = "0b7a1c0d-e000-4000-8000-0000000000bb";
    let done = json!({"op": "done_without_run", "task": "a", "reason": "r"});
    let first = served_as(&operations, done.clone(), id).expect("served");
    assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    let Transacted::Answered(answer) = first else {
        panic!("{first:?}")
    };
    let again = served_as(&operations, done, id).expect("served");
    assert_eq!(again, Transacted::Replayed(answer));
    let other = json!({"op": "create", "task": "b", "title": "B"});
    assert_eq!(
        served_as(&operations, other, id),
        Err(ServeError::WriteIdReused)
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 1, "no read for a replay");
}

/// A host that answers every pull request merged.
struct Merged;

impl PullRequestStates for Merged {
    fn state(&self, _: u64) -> Result<PullRequestState, PullRequestStatesError> {
        Ok(PullRequestState::Merged)
    }
}

/// `value` served as the CLI sends it, a write as Ada with a fresh id,
/// with what its client sent for its run (B8).
fn serve_run(operations: &ServeBoardOperation, value: Value, sent: &ServedRun) -> Value {
    serve_run_as(operations, value, sent, "Ada")
}

/// [`serve_run`], written as `who`.
fn serve_run_as(
    operations: &ServeBoardOperation,
    value: Value,
    sent: &ServedRun,
    who: &str,
) -> Value {
    let text = value.to_string();
    let request = parse(&value, EntryPolicy::SectionedOrLegacy).expect("a request");
    let write = OpWrite {
        write_id: WriteId::parse(&uuid::Uuid::new_v4().to_string()).expect("a UUID"),
        request: format!("{text}\n{sent:?}"),
        committer: Identity {
            name: who.into(),
            email: format!("{}@example.com", who.to_lowercase()),
        },
        principal: "local".into(),
    };
    let mut present = |board: &dyn BoardOperations| {
        let mut call = BoardCall::start(None, FrontEnd::Cli, Client::Cli);
        Ok(run_recorded_with(board, &text, ListSummaries::Omitted, &mut call).to_string())
    };
    match operations.execute_run(Some(&request), Some(&write), sent, &mut present) {
        Ok(Transacted::Answered(answer)) => serde_json::from_str(&answer).expect("JSON"),
        other => panic!("{other:?}"),
    }
}

/// A board server's operations over a fresh SQLite board whose pull
/// requests `host` reads, task `a` ready with plan 1 accepted, and run 1
/// begun there: the server's half of a start (B8), which launches nothing
/// and answers what its client launches with.
fn begun(
    remote: &Remote,
    host: Arc<dyn PullRequestStates>,
) -> (tempfile::TempDir, ServeBoardOperation) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let identity = Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    };
    let ops = SqliteBoardOps::open(&dir.path().join("board.sqlite3"), identity).expect("a board");
    let served = ServedOperations {
        prs: Ok(host),
        ..served(remote)
    };
    let operations =
        ServeBoardOperation::new(Arc::new(ops) as Arc<dyn BoardOpStore>, Arc::new(served));
    let plain = ServedRun::Plain;
    for value in [
        json!({"op": "bootstrap"}),
        json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
        json!({"op": "propose_plan", "task": "a", "approach": "Build it.",
               "items": [{"id": "schema", "title": "Schema"}]}),
    ] {
        let answer = serve_run(&operations, value.clone(), &plain);
        assert_eq!(answer["ok"], json!(true), "{value}: {answer}");
    }
    let shown = serve(
        &operations,
        json!({"op": "show", "task": "a", "version": 1}),
    )
    .expect("shown");
    let accept = json!({"op": "accept_plan", "task": "a", "version": 1,
                        "expected_updated": shown["plan"]["updated"], "expected_accepted": null});
    assert_eq!(serve_run(&operations, accept, &plain)["ok"], json!(true));
    let started = serve_run(
        &operations,
        json!({"op": "start", "task": "a", "plan_version": 1}),
        &plain,
    );
    assert_eq!(started["ok"], json!(true), "{started}");
    let launch = &started["launch"];
    assert!(
        launch["brief"]
            .as_str()
            .is_some_and(|brief| brief.contains("schema")),
        "{started}"
    );
    assert_eq!(
        (&started["from"], &started.get("coordinator")),
        (&json!("ready"), &None)
    );
    (dir, operations)
}

fn facts(running: Option<&str>, ended: Value) -> ServedRun {
    let wire = json!({"liveness": {"seen": running}, "reports": {"ended": ended, "left": null}});
    ServedRun::Facts(serde_json::from_value::<RunFacts>(wire).expect("facts"))
}

/// Every plan item done, its evidence at `revision`.
fn done_at(revision: &str) -> Value {
    json!({"end": "succeeded", "observed_tokens": 10, "items": [{
        "title": "schema: Schema", "state": "completed", "owner": "w-1",
        "evidence": [{"artifact": "commit", "revision": revision}], "stood_before_death": null}]})
}

fn end(outcome: &str) -> Value {
    json!({"op": "end_run", "task": "a", "run": 1, "outcome": outcome})
}

/// B8: an end on the server is judged on its client's facts: refused
/// `run_live` while its coordinator ran there, `not_pushed` over evidence
/// the remote (read before the write) lacks.
#[test]
fn an_end_is_judged_on_its_clients_facts() {
    let remote = Remote::new();
    let (_dir, operations) = begun(&remote, Arc::new(Merged));
    let live = serve_run(
        &operations,
        end("failed"),
        &facts(Some("7f3a"), Value::Null),
    );
    assert_eq!(
        (&live["error"]["code"], &live["error"]["agent"]),
        (&json!("run_live"), &json!("7f3a")),
        "{live}"
    );
    let unpushed = done_at("def456def456def456def456def456def456def4");
    let refused = serve_run(&operations, end("succeeded"), &facts(None, unpushed));
    assert_eq!(
        (&refused["error"]["code"], &refused["error"]["rule"]["code"]),
        (&json!("rule"), &json!("not_pushed")),
        "{refused}"
    );
}

/// B8: a success recorded from its client's kept board moves the task to
/// review, and a merged pull request linked there completes it.
#[test]
fn a_merged_pull_request_completes_a_task_its_run_moved_to_review() {
    let remote = Remote::new();
    let (_dir, operations) = begun(&remote, Arc::new(Merged));
    let ended = serve_run(
        &operations,
        end("succeeded"),
        &facts(None, done_at("notes.md")),
    );
    assert_eq!(
        (&ended["ok"], &ended["status"]),
        (&json!(true), &json!("review")),
        "{ended}"
    );
    let linked = serve_run(
        &operations,
        json!({"op": "link_pr", "task": "a", "pr": 7}),
        &ServedRun::Plain,
    );
    assert_eq!(
        (
            &linked["completion"]["done"],
            &linked["completion"]["task"]["status"]
        ),
        (&json!(true), &json!("done")),
        "{linked}"
    );
}

/// B8: a start whose launch failed is undone on the server, folded back
/// as failed, its task released, answered as a local undo's refusal.
#[test]
fn a_begun_start_is_undone_on_the_server() {
    let remote = Remote::new();
    let (_dir, operations) = begun(&remote, Arc::new(Merged));
    let undo = ServedRun::Undo(StartUndo {
        task: quecto::domain::project_board::value_objects::slug::Slug::parse("a").expect("a slug"),
        plan_version: 1,
        run: 1,
        from: TaskStatus::Ready,
        reason: "no container runtime".into(),
    });
    let start = json!({"op": "start", "task": "a", "plan_version": 1});
    // Review round 1: only the holder undoes, and only from a status a
    // start takes a task from; refused, the run stays open.
    let not_hers = serve_run_as(&operations, start.clone(), &undo, "Bob");
    assert_eq!(
        (
            &not_hers["error"]["code"],
            &not_hers["error"]["rule"]["code"]
        ),
        (&json!("rule"), &json!("not_holder")),
        "{not_hers}"
    );
    let ServedRun::Undo(mut done) = undo.clone() else {
        unreachable!("an undo")
    };
    done.from = TaskStatus::Done;
    let from_done = serve_run(&operations, start.clone(), &ServedRun::Undo(done));
    assert_eq!(from_done["error"]["field"], json!("from"), "{from_done}");
    let shown = serve(&operations, json!({"op": "show", "task": "a"})).expect("shown");
    assert_eq!(
        shown["runs"][0]["state"],
        json!("running"),
        "nothing undone"
    );
    let undone = serve_run(&operations, start, &undo);
    let error = &undone["error"];
    assert_eq!(
        (
            &error["code"],
            &error["run_state"],
            &error["released"],
            &error["reason"]
        ),
        (
            &json!("launch_failed"),
            &json!("folded"),
            &json!(true),
            &json!("no container runtime")
        ),
        "{undone}"
    );
    let shown = serve(&operations, json!({"op": "show", "task": "a"})).expect("shown");
    assert_eq!(
        (&shown["task"]["status"], &shown["runs"][0]["state"]),
        (&json!("ready"), &json!("failed"))
    );
}
