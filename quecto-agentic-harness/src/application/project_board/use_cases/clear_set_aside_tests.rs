use super::ClearSetAside;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, board, fields, person,
};
use crate::domain::project_board::entities::task::Task;
use std::sync::Arc;

fn seeded() -> crate::infrastructure::test_support::memory_board_store::MemoryBoardStore {
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

#[test]
fn a_member_clears_a_file_set_aside_and_the_commit_names_them() {
    let store = seeded();
    let bob = store.client_as(person("Bob"));
    let clear = ClearSetAside::new(Arc::new(bob), SetClock::at("10:00:00")).unwrap();
    let head = clear.execute("tasks/b.json", None).unwrap();
    let board = store.read().unwrap();
    assert_eq!(
        (board.head.clone(), board.quarantined),
        (head.clone(), vec![])
    );
    assert_eq!(store.committed_by(&head), Some(person("Bob")));
}

#[test]
fn only_a_file_set_aside_is_cleared() {
    let store = seeded();
    let clear = ClearSetAside::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap();
    assert_eq!(
        clear.execute("tasks/a.json", None),
        Err(ProjectBoardError::NotSetAside("tasks/a.json".into()))
    );
    assert_eq!(store.writes(), 1, "nothing written");
}

#[test]
fn a_lost_race_is_read_again_and_a_refusal_reported() {
    let store = seeded();
    let other = store.client();
    store.before_next_write(move |_| {
        let head = other.read().unwrap().head;
        other
            .write(&head, &[Task::new(fields("c")).unwrap()], "other")
            .unwrap();
    });
    let clock = SetClock::at("10:00:00");
    let clear = ClearSetAside::new(Arc::new(store.client()), clock.clone()).unwrap();
    assert!(clear.execute("tasks/b.json", None).is_ok());
    assert_eq!(*clock.pauses.lock().unwrap(), [2], "one retry");
    let fresh = seeded();
    fresh.refuse_next_write("a hook said no");
    let clear = ClearSetAside::new(Arc::new(fresh.client()), SetClock::at("10:00:00")).unwrap();
    assert_eq!(
        clear.execute("tasks/b.json", None),
        Err(ProjectBoardError::Refused("a hook said no".into()))
    );
}
