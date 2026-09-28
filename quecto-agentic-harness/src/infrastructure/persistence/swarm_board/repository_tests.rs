use serde_json::{Value, json};

use super::SqliteBoardRepository;
use crate::application::swarm::dto::{BoardLocation, MemberClaimCounts, NewMember};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction};
use crate::domain::swarm::{BoardError, MemberState};

fn repository() -> (tempfile::TempDir, SqliteBoardRepository) {
    let dir = tempfile::TempDir::new().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    (dir, repository)
}

fn within(
    repository: &SqliteBoardRepository,
    create: bool,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(create, &mut |transaction| work(transaction))
}

fn member(id: &str, status: MemberState) -> NewMember {
    NewMember {
        id: id.to_owned(),
        reservation: format!("{id}-r"),
        status,
        pid: Value::Null,
        started: Value::Null,
        socket: Value::Null,
        launcher: None,
    }
}

/// `member_claim_counts`: an admitted member holding an active task is
/// claimed; the coordinator is never counted; dead members are counted.
#[test]
fn claim_counts_follow_active_task_ownership() {
    let (dir, repository) = repository();
    within(&repository, true, |transaction| {
        for (id, status) in [
            ("parent", MemberState::LIVE),
            ("busy", MemberState::LIVE),
            ("idle", MemberState::RESERVED),
            ("done", MemberState::LIVE),
            ("gone", MemberState::DEAD),
        ] {
            transaction.insert_member(&member(id, status))?;
        }
        Ok(())
    })
    .unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO tasks(id,title,status,owner) VALUES(1,'a','claimed','busy');
             INSERT INTO tasks(id,title,status,owner) VALUES(2,'b','completed','done');",
        )
        .unwrap();
    drop(connection);
    within(&repository, false, |transaction| {
        assert_eq!(
            transaction.claim_counts(Some("parent"))?,
            MemberClaimCounts {
                members_without_claim: 2,
                members_dead: 1
            }
        );
        assert_eq!(
            transaction.claim_counts(None)?.members_without_claim,
            3,
            "with no coordinator every unclaimed admitted member counts"
        );
        assert_eq!(transaction.usage()?, 4);
        assert_eq!(transaction.not_dead()?, 4);
        Ok(())
    })
    .unwrap();
}

/// A row the board never writes (a NULL status) is refused as a store
/// failure, not mistaken for a status.
#[test]
fn a_run_row_of_the_wrong_shape_is_a_store_refusal() {
    let (dir, repository) = repository();
    within(&repository, true, |_| Ok(())).unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO run(id,coordinator,deadline) VALUES('r','p',1.0)",
            [],
        )
        .unwrap();
    drop(connection);
    let refused = within(&repository, false, |transaction| {
        transaction.run().map(|_| ())
    })
    .unwrap_err();
    assert!(
        refused
            .0
            .starts_with("coordination store unavailable or contended: "),
        "{refused}"
    );
}

#[test]
fn an_event_detail_is_stored_with_the_boards_encoding() {
    let (dir, repository) = repository();
    within(&repository, true, |transaction| {
        transaction.event("parent", 1.5, "created", &json!({"b": "é", "a": 1}))
    })
    .unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    let row: (String, f64, String, String) = connection
        .query_row("SELECT actor,time,action,detail FROM events", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap();
    assert_eq!(
        row,
        (
            "parent".to_owned(),
            1.5,
            "created".to_owned(),
            r#"{"a":1,"b":"\u00e9"}"#.to_owned()
        )
    );
}

#[test]
fn a_missing_board_is_refused_before_any_work() {
    let (_dir, repository) = repository();
    let refused = within(&repository, false, |_| panic!("no work on a missing board")).unwrap_err();
    assert!(
        refused.0.starts_with("coordination store missing at "),
        "{refused}"
    );
    assert_eq!(
        within(&repository, true, |transaction| transaction.run().map(
            |run| {
                assert!(run.is_none());
            }
        ))
        .map(|()| "created"),
        Ok("created")
    );
}
