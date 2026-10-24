use std::sync::Arc;

use serde_json::{Value, json};

use super::fixtures::{QueryListings, assembled};
use super::*;
use crate::application::project_board::operations::{BoardOutcome, NoPullRequestHost};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::entities::task_parts::EntryPolicy;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::request::parse;

/// The local port over `store`, bootstrapped.
fn local(store: &MemoryBoardStore) -> LocalBoardOperations {
    store.bootstrap().unwrap();
    let listings = Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client()))));
    assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        listings,
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap()
}

fn request(value: Value) -> BoardRequest {
    parse(&value, EntryPolicy::Sectioned).unwrap()
}

fn ready(id: &str) -> BoardRequest {
    request(json!({"op": "create", "task": id, "title": "A", "status": "ready"}))
}

/// A noted request gives back what its use case saw, an unnoted one
/// nothing; the answer is the use case's either way, with its clock note
/// (this machine's clock has no outside time).
#[test]
fn a_request_is_answered_as_the_use_case_answers_it_noted_as_asked() {
    let store = MemoryBoardStore::default();
    let operations = local(&store);
    let unnoted = operations.execute(&ready("a"), ListSummaries::Omitted, OpNoting::Unnoted);
    let Ok(answered) = unnoted.result else {
        panic!("created: {unnoted:?}")
    };
    assert!(matches!(answered.outcome, BoardOutcome::Changed { .. }));
    assert_eq!(
        answered.clock_note,
        operations.operations.clock_note(&ready("a"))
    );
    assert!(answered.clock_note.is_some(), "a create reads the clock");
    assert_eq!(unnoted.facts, None);
    let noted = operations.execute(&ready("b"), ListSummaries::Omitted, OpNoting::Noted);
    assert!(noted.result.is_ok(), "{noted:?}");
    let facts = noted.facts.expect("a noted request gives back its facts");
    assert_eq!(facts.attempts, Some(1));
}

/// A refusal is the use case's own, noted all the same; a listing reads
/// no clock, so it carries no note.
#[test]
fn a_refusal_and_a_listing_pass_through_unchanged() {
    let store = MemoryBoardStore::default();
    let operations = local(&store);
    let missing = request(json!({"op": "show", "task": "missing"}));
    let refused = operations.execute(&missing, ListSummaries::Omitted, OpNoting::Noted);
    let direct = operations.operations.execute(&missing);
    assert!(direct.is_err(), "{direct:?}");
    assert_eq!(refused.result.map(|answered| answered.outcome), direct);
    assert!(refused.facts.is_some(), "a refusal is noted too");
    let listing = request(json!({"op": "list"}));
    let listed = operations.execute(&listing, ListSummaries::Wanted, OpNoting::Unnoted);
    let Ok(answered) = listed.result else {
        panic!("listed: {listed:?}")
    };
    assert!(matches!(answered.outcome, BoardOutcome::Listing(_)));
    assert_eq!(answered.clock_note, None);
}

/// A front end that launches nothing owes no ends.
#[test]
fn no_launcher_leaves_no_runs() {
    let store = MemoryBoardStore::default();
    assert_eq!(local(&store).left_runs(), Ok(vec![]));
}
