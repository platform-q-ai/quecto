//! Contract for [`ProjectBoardStore`] over every kind of artifact (#2481,
//! M3): a write of any of the six kinds reads back, a write naming one key
//! twice is invalid, and a set-aside key refuses only the writes that name
//! it. The in-memory store keeps all six kinds; the git store keeps tasks
//! only until it learns the rest (S10), refusing the others as invalid.
use super::project_board_git_store::Remote;
use super::project_board_store::{Clients, task};
use quecto::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardStoreError, BoardWrite, ProjectBoardStore,
};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::infrastructure::test_support::board_artifacts::{
    change, discussion, plan, review, run,
};
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::collections::BTreeMap;

/// One artifact of each kind, all of task `a`.
fn every_kind() -> Vec<BoardArtifact> {
    vec![
        BoardArtifact::Task(task("a", "One")),
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Change(change("a", 1)),
        BoardArtifact::Review(review("a", 1)),
        BoardArtifact::Discussion(discussion("store-locking")),
    ]
}

/// Every artifact the board holds, by key: the store's order is not part
/// of the contract.
fn held(store: &dyn ProjectBoardStore) -> BTreeMap<ArtifactKey, BoardArtifact> {
    let board = store.read().unwrap();
    let all = board
        .tasks
        .into_iter()
        .map(BoardArtifact::Task)
        .chain(board.plans.into_iter().map(BoardArtifact::Plan))
        .chain(board.runs.into_iter().map(BoardArtifact::Run))
        .chain(board.changes.into_iter().map(BoardArtifact::Change))
        .chain(board.reviews.into_iter().map(BoardArtifact::Review))
        .chain(board.discussions.into_iter().map(BoardArtifact::Discussion));
    all.map(|artifact| (artifact.key(), artifact)).collect()
}

fn keyed(artifacts: &[BoardArtifact]) -> BTreeMap<ArtifactKey, BoardArtifact> {
    artifacts
        .iter()
        .map(|artifact| (artifact.key(), artifact.clone()))
        .collect()
}

fn written(write: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match write.unwrap() {
        BoardWrite::Written(head) => head,
        other => panic!("not written: {other:?}"),
    }
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// One write of every kind lands whole, and the other client reads each
/// back in its own list; a later write of one kind replaces only that
/// artifact.
pub fn check_write_and_read_every_kind((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let first = every_kind();
    let moved = written(alice.write(&head, &first, "board: add every kind"));
    let board = bob.read().unwrap();
    assert_eq!(board.head, moved);
    assert_eq!(
        [
            board.tasks.len(),
            board.plans.len(),
            board.runs.len(),
            board.changes.len(),
            board.reviews.len(),
            board.discussions.len(),
        ],
        [1; 6],
        "each kind in its own list"
    );
    assert_eq!(held(&*bob), keyed(&first));
    let second = BoardArtifact::Run(run("a", 2));
    written(bob.write(&moved, std::slice::from_ref(&second), "board: run a again"));
    let mut both = keyed(&first);
    both.insert(second.key(), second);
    assert_eq!(held(&*alice), both);
}

/// A write naming one artifact twice is invalid, as is a write of
/// nothing: refused before anything moves. A store that keeps `every_kind`
/// is held to it for each kind; one that keeps tasks only (the git store
/// until S10) refuses the other kinds for that alone, which would hide
/// whether it checks their keys, so it is held to tasks.
pub fn check_one_key_twice_is_invalid((alice, _bob): Clients, every_kind: bool) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let twice = |a: BoardArtifact, b: BoardArtifact| vec![a, b];
    let tasks = [
        vec![],
        twice(
            BoardArtifact::Task(task("a", "One")),
            BoardArtifact::Task(task("a", "Two")),
        ),
    ];
    let others = [
        twice(
            BoardArtifact::Plan(plan("a", 1)),
            BoardArtifact::Plan(plan("a", 1)),
        ),
        twice(
            BoardArtifact::Run(run("a", 1)),
            BoardArtifact::Run(run("a", 1)),
        ),
        twice(
            BoardArtifact::Change(change("a", 1)),
            BoardArtifact::Change(change("a", 1)),
        ),
        twice(
            BoardArtifact::Review(review("a", 1)),
            BoardArtifact::Review(review("a", 1)),
        ),
        twice(
            BoardArtifact::Discussion(discussion("d")),
            BoardArtifact::Discussion(discussion("d")),
        ),
    ];
    let held = match every_kind {
        true => others.len(),
        false => 0,
    };
    for artifacts in tasks.into_iter().chain(others.into_iter().take(held)) {
        let refused = alice.write(&head, &artifacts, "board: twice");
        assert!(
            matches!(refused, Err(BoardStoreError::InvalidWrite(_))),
            "{artifacts:?}: {refused:?}"
        );
    }
    assert_eq!(alice.read().unwrap().head, head);
}

/// While run 1 of task `a` is set aside (`spoil` sets it aside), a write
/// naming it is refused whole and moves nothing, whatever else it holds;
/// writes of other runs of the task, and of its other kinds, land.
pub fn check_set_aside_key_refuses_only_its_write(
    (alice, bob): Clients,
    spoil: impl FnOnce(ArtifactKey),
) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let start = [
        BoardArtifact::Task(task("a", "One")),
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Run(run("a", 2)),
    ];
    written(alice.write(&head, &start, "board: add a, runs 1 and 2"));
    spoil(ArtifactKey::Run(slug("a"), 1));
    let spoiled = bob.read().unwrap().head;
    for artifacts in [
        vec![BoardArtifact::Run(run("a", 1))],
        vec![
            BoardArtifact::Run(run("a", 3)),
            BoardArtifact::Run(run("a", 1)),
        ],
    ] {
        let refused = bob.write(&spoiled, &artifacts, "board: write run 1");
        assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
        let board = alice.read().unwrap();
        assert_eq!(board.head, spoiled);
        let runs: Vec<u32> = board.runs.iter().map(|run| run.fields().number).collect();
        assert_eq!(runs, [2], "no part of a refused write lands");
    }
    let next = written(bob.write(
        &spoiled,
        &[
            BoardArtifact::Run(run("a", 2)),
            BoardArtifact::Run(run("a", 3)),
            BoardArtifact::Plan(plan("a", 1)),
            BoardArtifact::Change(change("a", 1)),
        ],
        "board: write around run 1",
    ));
    let board = alice.read().unwrap();
    assert_eq!(board.head, next);
    let mut runs: Vec<u32> = board.runs.iter().map(|run| run.fields().number).collect();
    runs.sort_unstable();
    assert_eq!(runs, [2, 3]);
    assert!(
        board
            .quarantined
            .iter()
            .any(|file| file.key == Some(ArtifactKey::Run(slug("a"), 1))),
        "run 1 stays set aside"
    );
}

fn memory() -> (MemoryBoardStore, Clients) {
    let store = MemoryBoardStore::default();
    let clients: Clients = (Box::new(store.client()), Box::new(store.client()));
    (store, clients)
}

#[test]
fn the_memory_store_writes_and_reads_every_kind() {
    check_write_and_read_every_kind(memory().1);
}

#[test]
fn the_memory_store_refuses_one_key_twice() {
    check_one_key_twice_is_invalid(memory().1, true);
}

#[test]
fn the_memory_store_refuses_only_the_write_of_a_set_aside_key() {
    let (store, clients) = memory();
    check_set_aside_key_refuses_only_its_write(clients, |key| {
        store.quarantine_key(key, "runs/a.1.md", "malformed")
    });
}

#[test]
fn the_git_store_refuses_one_key_twice() {
    check_one_key_twice_is_invalid(Remote::new().clients(), false);
}

/// Until the git store keeps the other kinds (S10), a write holding any of
/// them is refused as invalid, whole: even its tasks are not written.
#[test]
fn the_git_store_refuses_kinds_it_does_not_store_yet() {
    let remote = Remote::new();
    let (alice, _bob) = remote.clients();
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    for artifact in every_kind().into_iter().skip(1) {
        let with_a_task = [BoardArtifact::Task(task("a", "One")), artifact.clone()];
        let refused = alice.write(&head, &with_a_task, "board: add a kind");
        assert!(
            matches!(refused, Err(BoardStoreError::InvalidWrite(_))),
            "{artifact:?}: {refused:?}"
        );
    }
    let board = alice.read().unwrap();
    assert_eq!(board.head, head);
    assert!(board.tasks.is_empty());
}
