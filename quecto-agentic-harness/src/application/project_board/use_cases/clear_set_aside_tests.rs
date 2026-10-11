use super::ClearSetAside;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardStoreError, BoardWrite, KeepCopy, ProjectBoardStore, SetAsideClear,
};
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, board, fields, person,
};
use crate::domain::project_board::entities::task::Task;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn seeded() -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let tasks = [
        Task::new(fields("a")).unwrap(),
        Task::new(fields("b")).unwrap(),
    ];
    store.write(&head, &tasks, "seed").unwrap();
    store.quarantine("b", "malformed");
    store
}

/// The object of the file set aside at `path`, as the listing shows it.
fn object_of(store: &MemoryBoardStore, path: &str) -> String {
    let board = store.read().unwrap();
    let file = board.quarantined.iter().find(|file| file.path == path);
    file.unwrap().object.clone()
}

#[test]
fn a_member_clears_a_file_set_aside_and_the_commit_names_them() {
    let store = seeded();
    let object = object_of(&store, "tasks/b.json");
    let bob = store.client_as(person("Bob"));
    let clear = ClearSetAside::new(Arc::new(bob), SetClock::at("10:00:00")).unwrap();
    let head = clear
        .execute("tasks/b.json", &object, &BTreeMap::new())
        .unwrap();
    let board = store.read().unwrap();
    assert_eq!(
        (board.head.clone(), board.quarantined),
        (head.clone(), vec![])
    );
    assert_eq!(store.committed_by(&head), Some(person("Bob")));
    assert_eq!(
        store.message_at(&head).as_deref(),
        Some("board: clear set-aside tasks/b.json")
    );
}

/// A clear's message names the file and, for each name listed more than
/// once, the copy kept or that none is; a message past one line's length
/// names only how many it clears.
#[test]
fn a_clear_message_records_the_copies_kept_and_dropped() {
    let repeated = BTreeMap::from([
        (
            "tasks/dup.json".to_string(),
            KeepCopy::Object {
                keep: "ab".repeat(20),
                seen: BTreeSet::from(["ab".repeat(20), "cd".repeat(20)]),
            },
        ),
        (
            "tasks/two.json".to_string(),
            KeepCopy::Drop(BTreeSet::from(["ef".repeat(20)])),
        ),
    ]);
    assert_eq!(
        super::message("tasks/x.json", &repeated),
        "board: clear set-aside tasks/x.json; tasks/dup.json keeps abababababab; tasks/two.json drops every copy"
    );
    let long = format!("tasks/{}.json", "x".repeat(190));
    assert_eq!(
        super::message(&long, &repeated),
        "board: clear set-aside 1 file and 2 names listed more than once"
    );
    assert_eq!(
        super::message("tasks/b.json", &BTreeMap::new()),
        "board: clear set-aside tasks/b.json"
    );
}

#[test]
fn only_a_file_set_aside_is_cleared() {
    let store = seeded();
    let clear = ClearSetAside::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap();
    let object = "a".repeat(40);
    assert_eq!(
        clear.execute("tasks/a.json", &object, &BTreeMap::new()),
        Err(ProjectBoardError::NotSetAside {
            path: "tasks/a.json".into(),
            object: object.clone()
        })
    );
    assert_eq!(store.writes(), 1, "nothing written");
    let shown = ProjectBoardError::NotSetAside {
        path: "tasks/a.json".into(),
        object,
    }
    .to_string();
    assert!(
        shown.contains("tasks/a.json") && shown.contains(&"a".repeat(40)),
        "{shown}"
    );
}

/// A clear names the file the caller saw: when the board sets another
/// file aside at that path while it retries, the clear stops there and
/// never clears the other in its place.
#[test]
fn a_file_set_aside_again_with_other_contents_is_never_cleared_in_its_place() {
    let store = seeded();
    let seen = object_of(&store, "tasks/b.json");
    let other = store.client();
    store.before_next_write(move |_| {
        other.delete_quarantined("b");
        other.quarantine("b", "a newer schema");
    });
    let clear = ClearSetAside::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap();
    assert_eq!(
        clear.execute("tasks/b.json", &seen, &BTreeMap::new()),
        Err(ProjectBoardError::NotSetAside {
            path: "tasks/b.json".into(),
            object: seen.clone()
        })
    );
    let board = store.read().unwrap();
    assert_eq!(board.quarantined.len(), 1);
    assert_ne!(board.quarantined[0].object, seen, "the other file stays");
}

/// What a clear names for names listed more than once is checked against
/// the listing before anything is built from it, the commit message
/// included: a path or object the listing does not show never reaches
/// the store.
#[test]
fn names_and_objects_the_listing_does_not_show_never_reach_the_store() {
    let store = seeded();
    let object = object_of(&store, "tasks/b.json");
    let clear = ClearSetAside::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap();
    for (path, keep) in [
        (
            "tasks/\u{1b}[2Jgone.json".to_string(),
            KeepCopy::Drop(BTreeSet::new()),
        ),
        (
            "tasks/b.json".to_string(),
            KeepCopy::Object {
                keep: "zz\u{1b}[2J".into(),
                seen: BTreeSet::from(["zz\u{1b}[2J".to_string()]),
            },
        ),
        (
            "tasks/b.json".to_string(),
            KeepCopy::Drop(BTreeSet::from([object.clone(), "c".repeat(40)])),
        ),
    ] {
        let outcome = clear.execute(
            "tasks/b.json",
            &object,
            &BTreeMap::from([(path.clone(), keep)]),
        );
        assert!(
            matches!(&outcome, Err(ProjectBoardError::NotSetAside { path: shown, .. }) if *shown == path),
            "{path:?}: {outcome:?}"
        );
    }
    assert_eq!(store.writes(), 1, "nothing written");
}

#[test]
fn a_lost_race_is_read_again_and_a_refusal_reported() {
    let store = seeded();
    let object = object_of(&store, "tasks/b.json");
    let other = store.client();
    store.before_next_write(move |_| {
        let head = other.read().unwrap().head;
        other
            .write(&head, &[Task::new(fields("c")).unwrap()], "other")
            .unwrap();
    });
    let clock = SetClock::at("10:00:00");
    let clear = ClearSetAside::new(Arc::new(store.client()), clock.clone()).unwrap();
    assert!(
        clear
            .execute("tasks/b.json", &object, &BTreeMap::new())
            .is_ok()
    );
    assert_eq!(*clock.pauses.lock().unwrap(), [2], "one retry");
    let fresh = seeded();
    let object = object_of(&fresh, "tasks/b.json");
    fresh.refuse_next_write("a hook said no");
    let clear = ClearSetAside::new(Arc::new(fresh.client()), SetClock::at("10:00:00")).unwrap();
    assert_eq!(
        clear.execute("tasks/b.json", &object, &BTreeMap::new()),
        Err(ProjectBoardError::Refused("a hook said no".into()))
    );
}

/// The memory store judges as the git store does: an object id is
/// lowercase hex of 40 or 64 characters, and a stale head is lost before
/// any refusal.
#[test]
fn the_memory_store_judges_as_the_git_store_does() {
    let store = seeded();
    let object = object_of(&store, "tasks/b.json");
    let head = store.read().unwrap().head;
    let clearing = |object: &str| SetAsideClear {
        path: "tasks/b.json".into(),
        object: object.into(),
        repeated: BTreeMap::new(),
    };
    for odd in [object.to_uppercase(), "a".repeat(41), "g".repeat(40)] {
        let invalid = store.clear_set_aside(&head, &clearing(&odd), "board: clear");
        assert!(
            matches!(invalid, Err(BoardStoreError::InvalidWrite(_))),
            "{odd}"
        );
    }
    let long = "b".repeat(64);
    assert!(matches!(
        store.clear_set_aside(&head, &clearing(&long), "board: clear"),
        Ok(BoardWrite::Refused(_))
    ));
    let stale = head;
    let fresh = store.read().unwrap().head;
    store
        .write(&fresh, &[Task::new(fields("c")).unwrap()], "other")
        .unwrap();
    store.refuse_next_write("a hook said no");
    assert_eq!(
        store.clear_set_aside(&stale, &clearing(&object), "board: clear"),
        Ok(BoardWrite::Lost)
    );
}
