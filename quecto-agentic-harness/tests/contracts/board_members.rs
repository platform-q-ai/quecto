//! `BoardMembers` on the SQLite adapter (#2270): member rows in store
//! order, the admitted count, the not-dead count `create` uses and the
//! #1969 claim counts, each by Python's SQL; loosely typed values bound as
//! Python's `sqlite3` binds them and read back as stored (epic P3).
use quecto::application::swarm::dto::{BoardLocation, MemberClaimCounts, MemberRow, NewMember};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, MemberState};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

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
        pid: json!(7),
        started: json!("t"),
        socket: Value::Null,
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
        assert_eq!(worker.status, Some(MemberState::RESERVED));
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
                status: Some("live".into()),
                pid: json!(7),
                started: Some("t".into()),
                socket: None,
                launcher: Some("parent".into()),
            }
        );
        Ok(())
    })
    .unwrap();
    // No status at all: counted by neither rule, and listed as `None`.
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
        let blank = transaction.members()?.pop().unwrap();
        assert_eq!((blank.id.as_str(), blank.status), ("blank", None));
        assert_eq!(transaction.member("blank")?.unwrap().status, None);
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

/// P3: each JSON value binds as Python's `sqlite3` binds it and the column
/// affinity decides what is kept; a pid reads back in whatever storage
/// class it holds. A value Python cannot bind is a store refusal naming the
/// parameter.
#[test]
fn loose_values_bind_as_python_binds_them_and_read_as_stored() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    let cases = [
        (
            json!(true),
            json!(5),
            json!(null),
            json!(1),
            Some("5"),
            None,
        ),
        (
            json!("12"),
            json!(1.5),
            json!(false),
            json!(12),
            Some("1.5"),
            Some("0"),
        ),
        (
            json!(3.5),
            json!(""),
            json!("s"),
            json!(3.5),
            Some(""),
            Some("s"),
        ),
        (
            json!("abc"),
            json!(null),
            json!(-2),
            json!("abc"),
            None,
            Some("-2"),
        ),
        (json!(3.0), json!(null), json!(null), json!(3), None, None),
    ];
    for (index, (pid, started, socket, read_pid, read_started, read_socket)) in
        cases.into_iter().enumerate()
    {
        let id = format!("m{index}");
        within(&repository, true, |transaction| {
            transaction.insert_member(&NewMember {
                pid: pid.clone(),
                started: started.clone(),
                socket: socket.clone(),
                launcher: None,
                ..new_member(&id, MemberState::LIVE)
            })
        })
        .unwrap();
        within(&repository, false, |transaction| {
            let row = transaction
                .members()?
                .into_iter()
                .find(|row| row.id == id)
                .unwrap();
            assert_eq!(row.pid, read_pid, "{pid}");
            assert_eq!(row.started.as_deref(), read_started, "{started}");
            assert_eq!(row.socket.as_deref(), read_socket, "{socket}");
            Ok(())
        })
        .unwrap();
    }
    // Refusals name the pid's position in Python's `_bootstrap` statement
    // (`VALUES(?,?,'live',?,?,?)`: parameter 3), as Python's do.
    for (pid, message) in [
        (
            json!([1]),
            "Error binding parameter 3: type 'list' is not supported",
        ),
        (
            json!({"a": 1}),
            "Error binding parameter 3: type 'dict' is not supported",
        ),
        (
            json!(u64::MAX),
            "Error binding parameter 3: Python int too large to convert to SQLite INTEGER",
        ),
    ] {
        let refused = within(&repository, false, |transaction| {
            transaction.insert_member(&NewMember {
                pid: pid.clone(),
                ..new_member("refused", MemberState::LIVE)
            })
        })
        .unwrap_err();
        assert_eq!(
            refused,
            BoardError::new(format!(
                "coordination store unavailable or contended: {message}"
            )),
            "{pid}"
        );
    }
}
