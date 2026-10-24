//! Contract for [`BoardOperationsOver`] (epic #2705 B7), on the operations
//! a board server assembles for each request: they run on the store they
//! are handed, so every write of a request lands in that store alone, and
//! they launch nothing (a start crosses the wire with B8).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use quecto::application::project_board::dto::{
    BoardOutcome, ListSummaries, NoPullRequestHost, OpNoting, ReadRequest,
};
use quecto::application::project_board::ports::{
    BoardOpStore, BoardOperationsOver, OpWrite, ProjectBoardStore, PullRequestState,
    PullRequestStates, PullRequestStatesError, WriteId,
};
use quecto::application::project_board::use_cases::ServeBoardOperation;
use quecto::composition::bootstrap::board_server::ServedOperations;
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::entities::task_parts::EntryPolicy;
use quecto::domain::project_board::value_objects::telemetry::Client;
use quecto::domain::project_board::value_objects::telemetry::FrontEnd;
use quecto::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use quecto::infrastructure::time::board_clock::SystemBoardClock;
use quecto::interface::tools::project_board::recording::BoardCall;
use quecto::interface::tools::project_board::request::parse;
use quecto::interface::tools::project_board::run_recorded_with;
use serde_json::{Value, json};

use super::project_board_git_store::Remote;

fn served(remote: &Remote) -> ServedOperations {
    let revisions = GitRemoteRevisions::new(remote.git(0, "Ada"), "origin").expect("revisions");
    ServedOperations {
        clock: Arc::new(SystemBoardClock::default()),
        prs: Err(NoPullRequestHost::NotAPullRequestHost),
        revisions: Arc::new(revisions),
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
    let over = served.prepare(None, &there.client());
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
#[derive(Default)]
struct SlowHost {
    slow: AtomicBool,
}

impl PullRequestStates for SlowHost {
    fn state(&self, _: u64) -> Result<PullRequestState, PullRequestStatesError> {
        if self.slow.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(3));
        }
        Ok(PullRequestState::Open)
    }
}

/// `value` served as the CLI sends it, a write as Ada with a fresh id.
fn serve(operations: &ServeBoardOperation, value: Value) -> Value {
    let text = value.to_string();
    let request = parse(&value, EntryPolicy::SectionedOrLegacy).expect("a request");
    let id = format!("{}", uuid::Uuid::new_v4());
    let write = OpWrite {
        write_id: WriteId::parse(&id).expect("a UUID"),
        request: text.clone(),
        committer: Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        },
        principal: "local".into(),
    };
    let write = request.writes().then_some(&write);
    let mut present = |board: &dyn quecto::application::project_board::ports::BoardOperations| {
        let mut call = BoardCall::start(None, FrontEnd::Cli, Client::Cli);
        Ok(run_recorded_with(board, &text, ListSummaries::Omitted, &mut call).to_string())
    };
    let served = operations.execute(Some(&request), write, &mut present);
    let answer = match served.expect("served") {
        quecto::application::project_board::ports::Transacted::Answered(answer) => answer,
        other => panic!("{other:?}"),
    };
    serde_json::from_str(&answer).expect("JSON")
}

/// B7 round 1: the pull requests a write may complete its task by are read
/// before the board's write lock: a slow host holds up neither an
/// unrelated create nor a listing.
#[test]
fn a_slow_host_holds_up_no_other_write_or_read() {
    let remote = Remote::new();
    let host = Arc::new(SlowHost::default());
    let dir = tempfile::tempdir().expect("a temp dir");
    let identity = Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    };
    let ops = SqliteBoardOps::open(&dir.path().join("board.sqlite3"), identity).expect("a board");
    let served = ServedOperations {
        prs: Ok(Arc::clone(&host) as Arc<dyn PullRequestStates>),
        ..served(&remote)
    };
    let operations = Arc::new(ServeBoardOperation::new(
        Arc::new(ops) as Arc<dyn BoardOpStore>,
        Arc::new(served),
    ));
    for value in [
        json!({"op": "bootstrap"}),
        json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
        json!({"op": "move", "task": "a", "to": "claimed"}),
        json!({"op": "link_pr", "task": "a", "pr": 7}),
    ] {
        let answer = serve(&operations, value.clone());
        assert_eq!(answer["ok"], json!(true), "{value}: {answer}");
    }
    host.slow.store(true, Ordering::SeqCst);
    let completing = {
        let operations = Arc::clone(&operations);
        std::thread::spawn(move || serve(&operations, json!({"op": "complete", "task": "a"})))
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
        (&created["ok"], &listed["ok"]),
        (&json!(true), &json!(true))
    );
    assert!(took < Duration::from_millis(1500), "held up {took:?}");
    let completed = completing.join().expect("the completion");
    assert_eq!(completed["op"], json!("complete"), "{completed}");
}
