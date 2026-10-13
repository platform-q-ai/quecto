//! Contract for [`ProjectBoardStore`] over every kind of artifact (#2481,
//! M3): a write of any of the six kinds reads back, a write naming one key
//! twice is invalid, a set-aside key refuses only the writes that name it,
//! each key holds one artifact, a file reads back as the very bytes it was
//! written as, and a value the codec refuses is invalid, named by its field
//! and line. Both stores keep all six kinds (the git store since S10).
use super::project_board_git_store::Remote;
use super::project_board_store::{Clients, task};
use quecto::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardStoreError, BoardWrite, ProjectBoardStore,
    SetAsideClear,
};
use quecto::domain::project_board::entities::change::Change;
use quecto::domain::project_board::entities::discussion::Discussion;
use quecto::domain::project_board::entities::plan::Plan;
use quecto::domain::project_board::entities::review::Review;
use quecto::domain::project_board::entities::run::{Run, RunItem};
use quecto::domain::project_board::value_objects::code_ref::RepoName;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use quecto::infrastructure::persistence::project_board::artifacts::file_of;
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

/// A write naming one artifact twice is invalid, for every kind, as is a
/// write of nothing: refused before anything moves.
pub fn check_one_key_twice_is_invalid((alice, _bob): Clients) {
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
    for artifacts in tasks.into_iter().chain(others) {
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
        assert!(
            matches!(&refused, Ok(BoardWrite::Refused(why)) if why.starts_with("runs/a.1.md is set aside: ")),
            "{refused:?}"
        );
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

/// Every kind of `every_kind`, each edited but under the same key.
fn every_kind_edited() -> Vec<BoardArtifact> {
    let mut plan = plan("a", 1).fields().clone();
    plan.approach = "Build, then ship.".into();
    let mut run = run("a", 1).fields().clone();
    run.plan_version = 2;
    let mut change = change("a", 1).fields().clone();
    change.pr = Some(7);
    let mut review = review("a", 1).fields().clone();
    review.rounds[0].reviewer.name = "Bea".into();
    let mut discussion = discussion("store-locking").fields().clone();
    discussion.title = "How the store locks, again".into();
    vec![
        BoardArtifact::Task(task("a", "Two")),
        BoardArtifact::Plan(Plan::new(plan).unwrap()),
        BoardArtifact::Run(Run::new(run).unwrap()),
        BoardArtifact::Change(Change::new(change).unwrap()),
        BoardArtifact::Review(Review::new(review).unwrap()),
        BoardArtifact::Discussion(Discussion::new(discussion).unwrap()),
    ]
}

/// Each key holds one artifact: written again, every kind's artifact is
/// replaced whole, and a read lists it once, in its own list.
pub fn check_one_artifact_per_key((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let moved = written(alice.write(&head, &every_kind(), "board: add every kind"));
    let edited = every_kind_edited();
    assert_ne!(
        keyed(&edited),
        keyed(&every_kind()),
        "every artifact edited"
    );
    let again = written(bob.write(&moved, &edited, "board: edit every kind"));
    let board = alice.read().unwrap();
    assert_eq!(board.head, again);
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
        "one artifact per key"
    );
    assert_eq!(held(&*alice), keyed(&edited));
}

/// Run 1 of task `a`, folded back: ended, with its outcome, cost and a
/// task item done.
fn folded_run() -> Run {
    let mut fields = run("a", 1).fields().clone();
    fields.ended = Some(Timestamp::parse("2026-10-10T11:00:00Z").unwrap());
    fields.outcome = Some(RunOutcome::Succeeded);
    fields.cost = Some(1_840_000);
    fields.items = vec![RunItem {
        id: slug("build"),
        state: ItemState::Done,
        owner: Some("W1 builder".into()),
        evidence: vec!["26 tests pass, `a|b` included".into()],
    }];
    Run::new(fields).unwrap()
}

/// A plan and a run read back as the very files they were written as,
/// byte for byte.
pub fn check_plan_and_run_read_back_byte_exact((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Run(folded_run()),
    ];
    written(alice.write(&head, &artifacts, "board: plan and run a"));
    let board = bob.read().unwrap();
    let read = [
        BoardArtifact::Plan(board.plans[0].clone()),
        BoardArtifact::Run(board.runs[0].clone()),
    ];
    for (wrote, read) in artifacts.iter().zip(&read) {
        assert_eq!(file_of(read), file_of(wrote), "{:?}", wrote.key());
        assert_eq!(read, wrote);
    }
}

/// A value the codec refuses is an invalid write that names its key, its
/// field and its line; nothing of it lands.
pub fn check_a_value_the_codec_refuses_is_invalid((alice, _bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let mut fields = plan("a", 1).fields().clone();
    fields.approach = "abc\n".into();
    let refused = alice.write(
        &head,
        &[
            BoardArtifact::Task(task("a", "One")),
            BoardArtifact::Plan(Plan::new(fields).unwrap()),
        ],
        "board: plan a",
    );
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(why)) if why.starts_with("plan 1 of task a: Approach: line 2:")),
        "{refused:?}"
    );
    let board = alice.read().unwrap();
    assert_eq!((board.head, board.tasks.len()), (head, 0));
}

/// A change only in case (a repository's name, whose equality ignores
/// case) is still written: what decides is the file, never `==`.
pub fn check_a_change_only_in_case_is_written((alice, bob): Clients) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let lower = change("a", 1);
    let moved = written(alice.write(
        &head,
        &[BoardArtifact::Change(lower.clone())],
        "board: link a",
    ));
    let mut fields = lower.fields().clone();
    fields.head_repo = RepoName::parse("Platform-Q-AI/Quecto").unwrap();
    let upper = Change::new(fields).unwrap();
    assert_eq!(upper, lower, "the values are equal");
    written(bob.write(&moved, &[BoardArtifact::Change(upper)], "board: relink a"));
    let board = alice.read().unwrap();
    assert_eq!(
        board.changes[0].fields().head_repo.as_str(),
        "Platform-Q-AI/Quecto"
    );
}

/// A file set aside in a folder besides `tasks/` is cleared alone: the
/// clear removes it, frees its key, and keeps the folder's other files.
pub fn check_a_clear_in_a_folder_removes_its_file(
    (alice, bob): Clients,
    spoil: impl FnOnce(ArtifactKey),
) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let runs = [
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Run(run("a", 2)),
    ];
    written(alice.write(&head, &runs, "board: runs of a"));
    spoil(ArtifactKey::Run(slug("a"), 1));
    let board = alice.read().unwrap();
    let file = board.quarantined[0].clone();
    assert_eq!(file.path, "runs/a.1.md");
    let clear = SetAsideClear {
        path: file.path,
        object: file.object,
        repeated: BTreeMap::new(),
    };
    let cleared = written(alice.clear_set_aside(&board.head, &clear, "board: clear run 1"));
    let board = bob.read().unwrap();
    assert_eq!(board.head, cleared);
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
    assert_eq!(board.runs, [run("a", 2)]);
    written(bob.write(&cleared, &runs[..1], "board: run 1 of a again"));
    assert_eq!(held(&*alice), keyed(&runs));
}

/// A stray in a folder besides `tasks/` (`add_stray` puts
/// `plans/notes.txt` there) is dropped by the next write, whatever it
/// writes.
pub fn check_a_write_drops_a_stray_in_a_folder((alice, bob): Clients, add_stray: impl FnOnce()) {
    alice.bootstrap().unwrap();
    let head = alice.read().unwrap().head;
    let planned = [BoardArtifact::Plan(plan("a", 1))];
    written(alice.write(&head, &planned, "board: plan a"));
    add_stray();
    let board = bob.read().unwrap();
    let strays: Vec<(&str, bool)> = board
        .quarantined
        .iter()
        .map(|file| (file.path.as_str(), file.carried))
        .collect();
    assert_eq!(strays, [("plans/notes.txt", false)]);
    written(bob.write_tasks(&board.head, &[task("b", "Two")], "board: add b"));
    let board = alice.read().unwrap();
    assert!(board.quarantined.is_empty(), "{:?}", board.quarantined);
    assert_eq!(board.plans, [plan("a", 1)]);
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
    check_one_key_twice_is_invalid(memory().1);
}

#[test]
fn the_memory_store_refuses_only_the_write_of_a_set_aside_key() {
    let (store, clients) = memory();
    check_set_aside_key_refuses_only_its_write(clients, |key| {
        store.quarantine_key(key, "runs/a.1.md", "malformed")
    });
}

#[test]
fn the_memory_store_holds_one_artifact_per_key() {
    check_one_artifact_per_key(memory().1);
}

#[test]
fn the_memory_store_reads_a_plan_and_a_run_back_byte_exact() {
    check_plan_and_run_read_back_byte_exact(memory().1);
}

#[test]
fn the_memory_store_refuses_a_value_its_codec_refuses() {
    check_a_value_the_codec_refuses_is_invalid(memory().1);
}

#[test]
fn the_memory_store_writes_a_change_only_in_case() {
    check_a_change_only_in_case_is_written(memory().1);
}

#[test]
fn the_memory_store_clears_a_file_in_a_folder() {
    let (store, clients) = memory();
    check_a_clear_in_a_folder_removes_its_file(clients, |key| {
        store.quarantine_key(key, "runs/a.1.md", "malformed")
    });
}

#[test]
fn the_memory_store_drops_a_stray_in_a_folder() {
    let (store, clients) = memory();
    check_a_write_drops_a_stray_in_a_folder(clients, || {
        store.set_aside_stray("plans/notes.txt", "not a plan file")
    });
}

#[test]
fn the_git_store_writes_and_reads_every_kind() {
    check_write_and_read_every_kind(Remote::new().clients());
}

#[test]
fn the_git_store_refuses_one_key_twice() {
    check_one_key_twice_is_invalid(Remote::new().clients());
}

#[test]
fn the_git_store_refuses_only_the_write_of_a_set_aside_key() {
    let remote = Remote::new();
    check_set_aside_key_refuses_only_its_write(remote.clients(), |_| {
        remote.put("runs", "a.1.md", b"{")
    });
}

#[test]
fn the_git_store_holds_one_artifact_per_key() {
    check_one_artifact_per_key(Remote::new().clients());
}

#[test]
fn the_git_store_reads_a_plan_and_a_run_back_byte_exact() {
    check_plan_and_run_read_back_byte_exact(Remote::new().clients());
}

#[test]
fn the_git_store_refuses_a_value_its_codec_refuses() {
    check_a_value_the_codec_refuses_is_invalid(Remote::new().clients());
}

#[test]
fn the_git_store_writes_a_change_only_in_case() {
    check_a_change_only_in_case_is_written(Remote::new().clients());
}

#[test]
fn the_git_store_clears_a_file_in_a_folder() {
    let remote = Remote::new();
    check_a_clear_in_a_folder_removes_its_file(remote.clients(), |_| {
        remote.put("runs", "a.1.md", b"{")
    });
}

#[test]
fn the_git_store_drops_a_stray_in_a_folder() {
    let remote = Remote::new();
    check_a_write_drops_a_stray_in_a_folder(remote.clients(), || {
        remote.put("plans", "notes.txt", b"notes")
    });
}
