//! `BoardWakes` on the SQLite adapter (#2276): `wake_cursors` is created
//! only by a wake claim, inside its transaction (a refused claim leaves no
//! table); a claimed generation is not returned twice; a member's
//! notification events are its own after its cursor, which
//! `advance_notifications` moves to the board's generation; and the
//! notification state reads every task and the unread message ids.
use std::collections::BTreeSet;

use quecto::application::swarm::dto::{BoardLocation, NotificationCursor};
use quecto::application::swarm::ports::BoardRepository;
use quecto::domain::swarm::{BoardError, RefusalKind};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn board(dir: &tempfile::TempDir) -> (SqliteBoardRepository, std::path::PathBuf) {
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    (repository, database)
}

fn has_wake_cursors(database: &std::path::Path) -> bool {
    rusqlite::Connection::open(database)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='wake_cursors'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

/// Events 1 (`parent`), 2 (`worker`) and 3 (`parent`).
fn three_events(repository: &SqliteBoardRepository) {
    repository
        .atomic(false, &mut |transaction| {
            transaction.event("parent", 1.0, "task_created", &json!({"task": 1}))?;
            transaction.event(
                "worker",
                2.0,
                "message_accepted",
                &json!({"message": 1, "recipient": "parent", "revision": null}),
            )?;
            transaction.event("parent", 3.0, "amended", &json!({"reason": "r"}))
        })
        .unwrap();
}

#[test]
fn wake_cursors_is_created_only_by_a_wake_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, database) = board(&dir);
    three_events(&repository);
    repository
        .atomic(false, &mut |transaction| {
            transaction.notification_events("parent")?;
            transaction.advance_notifications("parent")?;
            transaction.notification_state()?;
            assert_eq!(transaction.event_generation()?, 3);
            Ok(())
        })
        .unwrap();
    assert!(
        !has_wake_cursors(&database),
        "no notification op creates it"
    );
    let refused = repository.atomic(false, &mut |transaction| {
        transaction.create_wake_cursors()?;
        Err(BoardError::new(
            RefusalKind::Invalid,
            "wake generation is ahead of the board",
        ))
    });
    assert!(refused.is_err());
    assert!(
        !has_wake_cursors(&database),
        "a refused claim rolls it back"
    );
    repository
        .atomic(false, &mut |transaction| {
            transaction.create_wake_cursors()?;
            transaction.create_wake_cursors()?;
            assert_eq!(transaction.wake_cursor("parent")?, None);
            Ok(())
        })
        .unwrap();
    assert!(has_wake_cursors(&database), "the first claim creates it");
    let sql: String = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='wake_cursors'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        sql,
        "CREATE TABLE wake_cursors (actor TEXT PRIMARY KEY, event INTEGER)"
    );
}

#[test]
fn a_claimed_generation_is_not_returned_twice() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, _) = board(&dir);
    three_events(&repository);
    let mut seen = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            transaction.create_wake_cursors()?;
            let claimed = transaction.wake_events(0, 2, "parent")?;
            seen.push(
                claimed
                    .iter()
                    .map(|event| event.action.clone())
                    .collect::<Vec<_>>(),
            );
            assert_eq!(claimed[0].actor.as_deref(), Some("worker"));
            assert_eq!(
                claimed[0].detail,
                json!({"message": 1, "recipient": "parent", "revision": null})
            );
            transaction.set_wake_cursor("parent", 2)?;
            assert_eq!(transaction.wake_cursor("parent")?, Some(2));
            let again = transaction.wake_events(2, 2, "parent")?;
            seen.push(again.iter().map(|event| event.action.clone()).collect());
            let worker = transaction.wake_events(0, 3, "worker")?;
            seen.push(worker.iter().map(|event| event.action.clone()).collect());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        seen,
        [
            vec!["message_accepted".to_owned()],
            Vec::new(),
            vec!["task_created".to_owned(), "amended".to_owned()],
        ]
    );
}

#[test]
fn notification_events_follow_their_own_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, database) = board(&dir);
    three_events(&repository);
    let mut seen = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            let events = transaction.notification_events("parent")?;
            assert!(
                events
                    .iter()
                    .all(|event| event.actor.as_deref() == Some("parent"))
            );
            seen.push(events.len());
            assert_eq!(
                transaction.advance_notifications("parent")?,
                NotificationCursor {
                    previous: None,
                    current: 3
                }
            );
            seen.push(transaction.notification_events("parent")?.len());
            assert_eq!(
                transaction.advance_notifications("parent")?,
                NotificationCursor {
                    previous: Some(3),
                    current: 3
                }
            );
            transaction.event("parent", 4.0, "amended", &json!({"reason": "again"}))?;
            seen.push(transaction.notification_events("parent")?.len());
            seen.push(transaction.notification_events("worker")?.len());
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, [2, 0, 1, 1]);
    let cursors: Vec<(String, String, i64)> = rusqlite::Connection::open(&database)
        .unwrap()
        .prepare("SELECT actor, typeof(event), event FROM notification_cursors")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(cursors, [("parent".to_owned(), "integer".to_owned(), 3)]);
}

#[test]
fn the_notification_state_reads_tasks_and_unread_messages() {
    let dir = tempfile::tempdir().unwrap();
    let (repository, database) = board(&dir);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "INSERT INTO tasks(title,acceptance,dependencies,status,owner,evidence) VALUES('a','[]','[]','claimed','worker','[]');
             INSERT INTO tasks(title,acceptance,dependencies,status,evidence) VALUES('b','[]','[1]','ready','[]');
             INSERT INTO messages(sender,recipient,body,status) VALUES('a','parent','x','consumed');
             INSERT INTO messages(sender,recipient,body,status) VALUES('a','parent','x','accepted');",
        )
        .unwrap();
    let mut seen = None;
    repository
        .atomic(false, &mut |transaction| {
            seen = Some(transaction.notification_state()?);
            Ok(())
        })
        .unwrap();
    let state = seen.unwrap();
    let tasks: Vec<(i64, String, Vec<i64>, Option<String>)> = state
        .tasks
        .iter()
        .map(|task| {
            (
                task.id,
                task.status.as_str().to_owned(),
                task.dependencies.clone(),
                task.owner.clone(),
            )
        })
        .collect();
    assert_eq!(
        tasks,
        [
            (1, "claimed".to_owned(), vec![], Some("worker".to_owned())),
            (2, "ready".to_owned(), vec![1], None),
        ]
    );
    assert_eq!(state.unread, BTreeSet::from([2]));
}
