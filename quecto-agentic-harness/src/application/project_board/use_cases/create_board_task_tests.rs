use super::CreateBoardTask;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardStoreError, ProjectBoardStore};
use crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn create(store: &MemoryBoardStore, clock: Arc<SetClock>) -> CreateBoardTask {
    CreateBoardTask::new(Arc::new(store.client()), clock)
}

#[test]
fn a_task_is_created_and_stamped_now() {
    let store = board();
    let task = create(&store, SetClock::at("10:00:00"))
        .execute(fields("a"))
        .unwrap();
    assert_eq!(
        (&task.fields().created, &task.fields().updated),
        (&at("10:00:00"), &at("10:00:00"))
    );
    assert_eq!(store.read().unwrap().tasks, vec![task]);
}

#[test]
fn only_an_unclaimed_draft_or_ready_task_is_created() {
    let store = board();
    for status in [TaskStatus::Claimed, TaskStatus::Done, TaskStatus::Archived] {
        let mut task = fields("a");
        task.status = status;
        assert!(matches!(
            create(&store, SetClock::at("10:00:00")).execute(task),
            Err(ProjectBoardError::Rule(_))
        ));
    }
    let mut claimed = fields("a");
    claimed.claim = Some(Claim {
        holder: person("Ada"),
        since: at("10:00:00"),
        expires: at("11:00:00"),
    });
    assert!(matches!(
        create(&store, SetClock::at("10:00:00")).execute(claimed),
        Err(ProjectBoardError::Rule(_))
    ));
    assert_eq!(store.writes(), 0);
}

#[test]
fn an_id_already_on_the_board_is_refused() {
    let store = board();
    create(&store, SetClock::at("10:00:00"))
        .execute(fields("a"))
        .unwrap();
    let again = create(&store, SetClock::at("10:01:00")).execute(fields("a"));
    assert_eq!(again, Err(ProjectBoardError::Exists(fields("a").id)));
}

/// Another writer moves the board between the read and the write: the
/// create is built again against the fresh board and lands.
#[test]
fn a_lost_write_is_retried_against_the_fresh_board() {
    let store = board();
    store.before_next_write(|board| {
        let head = board.read().unwrap().head;
        let b = Task::new(TaskFields {
            created: at("09:00:00"),
            updated: at("09:00:00"),
            ..fields("b")
        })
        .unwrap();
        board.write(&head, &[b], "other").unwrap();
    });
    create(&store, SetClock::at("10:00:00"))
        .execute(fields("a"))
        .unwrap();
    let ids: Vec<_> = store
        .read()
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

/// The other writer created the same id: the rebuilt create is refused.
#[test]
fn a_retry_that_finds_the_id_taken_is_refused() {
    let store = board();
    store.before_next_write(|board| {
        let head = board.read().unwrap().head;
        let a = Task::new(TaskFields {
            created: at("09:00:00"),
            updated: at("09:00:00"),
            ..fields("a")
        })
        .unwrap();
        board.write(&head, &[a], "other").unwrap();
    });
    let refused = create(&store, SetClock::at("10:00:00")).execute(fields("a"));
    assert_eq!(refused, Err(ProjectBoardError::Exists(fields("a").id)));
}

#[test]
fn a_board_that_never_stops_moving_is_reported_contended() {
    let store = board();
    fn move_it(board: &MemoryBoardStore, left: usize) {
        let head = board.read().unwrap().head;
        let id = format!("other-{left}");
        let task = Task::new(TaskFields {
            created: at("09:00:00"),
            updated: at("09:00:00"),
            ..fields(&id)
        })
        .unwrap();
        board.write(&head, &[task], "other").unwrap();
        if left > 1 {
            board.before_next_write(move |board| move_it(board, left - 1));
        }
    }
    store.before_next_write(|board| move_it(board, MAX_WRITE_ATTEMPTS));
    let contended = create(&store, SetClock::at("10:00:00")).execute(fields("a"));
    assert_eq!(
        contended,
        Err(ProjectBoardError::Contended {
            attempts: MAX_WRITE_ATTEMPTS
        })
    );
}

#[test]
fn a_clock_too_far_ahead_of_its_reference_is_refused() {
    let store = board();
    let ahead = Arc::new(SetClock {
        now: std::sync::Mutex::new(at("10:01:01")),
        reference: Some(at("10:00:00")),
    });
    assert!(matches!(
        create(&store, ahead).execute(fields("a")),
        Err(ProjectBoardError::ClockAhead { .. })
    ));
    let close = Arc::new(SetClock {
        now: std::sync::Mutex::new(at("10:01:00")),
        reference: Some(at("10:00:00")),
    });
    assert!(
        create(&store, close).execute(fields("a")).is_ok(),
        "60 s ahead is allowed"
    );
}

#[test]
fn a_board_not_bootstrapped_is_reported() {
    let store = MemoryBoardStore::default();
    let refused = create(&store, SetClock::at("10:00:00")).execute(fields("a"));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Store(BoardStoreError::NoBoard))
    );
}
