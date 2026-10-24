//! Contract for [`BoardOperationsOver`] (epic #2705 B7), on the operations
//! a board server assembles for each request: they run on the store they
//! are handed, so every write of a request lands in that store alone, and
//! they launch nothing (a start crosses the wire with B8).
use std::sync::Arc;

use quecto::application::project_board::dto::{
    BoardOutcome, ListSummaries, NoPullRequestHost, OpNoting, ReadRequest,
};
use quecto::application::project_board::ports::{BoardOperationsOver, ProjectBoardStore};
use quecto::composition::bootstrap::board_server::ServedOperations;
use quecto::domain::project_board::entities::task_parts::EntryPolicy;
use quecto::domain::project_board::value_objects::telemetry::FrontEnd;
use quecto::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use quecto::infrastructure::time::board_clock::SystemBoardClock;
use quecto::interface::tools::project_board::request::parse;
use serde_json::json;

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
    let operations = served
        .over(Arc::new(here.client()))
        .expect("the operations are built");
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
