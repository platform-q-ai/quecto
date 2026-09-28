//! `BoardMembers` on the SQLite adapter (#2270): member rows in store
//! order, the admitted count, the not-dead count `create` uses and the
//! #1969 claim counts, each by Python's SQL.
use quecto::application::swarm::dto::{BoardLocation, MemberClaimCounts, MemberRow, NewMember};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, MemberState};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

fn within(
    repository: &SqliteBoardRepository,
    create: bool,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(create, &mut |transaction| work(transaction))
}

fn new_member(id: &str, status: MemberState) -> NewMember {
    NewMember {
        id: id.into(),
        reservation: format!("{id}-r"),
        status,
        pid: Some(7),
        started: Some("t".into()),
        socket: None,
        launcher: Some("parent".into()),
    }
}

#[test]
fn members_are_written_read_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    within(&repository, true, |transaction| {
        for (id, status) in [
            ("parent", MemberState::LIVE),
            ("worker", MemberState::RESERVED),
            ("gone", MemberState::DEAD),
        ] {
            transaction.insert_member(&new_member(id, status))?;
        }
        Ok(())
    })
    .unwrap();
    // A status the board never writes is read as it is.
    let raw = |sql: &str| {
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(sql, [])
            .unwrap();
    };
    raw("INSERT INTO members(id,status) VALUES('odd','paused')");
    within(&repository, false, |transaction| {
        let worker = transaction.member("worker")?.unwrap();
        assert_eq!(worker.status, MemberState::RESERVED);
        assert_eq!(worker.reservation.as_deref(), Some("worker-r"));
        assert!(transaction.member("stranger")?.is_none());
        let members = transaction.members()?;
        assert_eq!(
            members
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["parent", "worker", "gone", "odd"]
        );
        assert_eq!(
            members[0],
            MemberRow {
                id: "parent".into(),
                reservation: Some("parent-r".into()),
                status: "live".into(),
                pid: Some(7),
                started: Some("t".into()),
                socket: None,
                launcher: Some("parent".into()),
            }
        );
        Ok(())
    })
    .unwrap();
    // No status at all: counted by neither rule, and no row to list.
    raw("INSERT INTO members(id,status) VALUES('blank',NULL)");
    within(&repository, false, |transaction| {
        assert_eq!(transaction.usage()?, 2, "live and reserved");
        assert_eq!(
            transaction.not_dead()?,
            3,
            "status != 'dead': NULL is not counted"
        );
        assert_eq!(
            transaction.claim_counts(Some("parent"))?,
            MemberClaimCounts {
                members_without_claim: 1,
                members_dead: 1
            }
        );
        let refused = transaction.members().unwrap_err();
        assert!(
            refused
                .0
                .starts_with("coordination store unavailable or contended: "),
            "{refused}"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_second_row_for_one_member_is_a_store_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    let refused = within(&repository, true, |transaction| {
        transaction.insert_member(&new_member("parent", MemberState::LIVE))?;
        transaction.insert_member(&NewMember {
            reservation: "another".into(),
            ..new_member("parent", MemberState::LIVE)
        })
    })
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            "coordination store unavailable or contended: UNIQUE constraint failed: members.id"
        )
    );
}
