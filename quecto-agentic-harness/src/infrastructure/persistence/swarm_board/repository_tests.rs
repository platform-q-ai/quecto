use serde_json::{Value, json};

use super::SqliteBoardRepository;
use crate::application::swarm::dto::{BoardLocation, MemberClaimCounts, NewMember, RunOwnerRow};
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

/// A run row the board never writes is refused as a store failure where
/// its type is not one the board reads (a NULL `member_limit`), not
/// mistaken for a number.
#[test]
fn a_run_row_of_the_wrong_shape_is_a_store_refusal() {
    let (dir, repository) = repository();
    within(&repository, true, |_| Ok(())).unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO run(id,coordinator,deadline,status) VALUES('r','p',1.0,'running')",
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
            .message()
            .starts_with("coordination store unavailable or contended: "),
        "{refused}"
    );
}

/// A NULL `run.status` reads as `None`, as Python reads it (#2270 review
/// L2), not as a store failure.
#[test]
fn a_null_run_status_reads_as_none() {
    let (dir, repository) = repository();
    within(&repository, true, |_| Ok(())).unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO run(id,coordinator,deadline,member_limit) VALUES('r','p',1.0,3)",
            [],
        )
        .unwrap();
    drop(connection);
    within(&repository, false, |transaction| {
        let run = transaction.run()?.expect("the run row");
        assert_eq!(run.status, None);
        assert_eq!(run.member_limit, 3);
        Ok(())
    })
    .unwrap();
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
        refused
            .message()
            .starts_with("coordination store missing at "),
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

/// A `run` table rebuilt without column types, holding a numeric status
/// and coordinator.
const UNTYPED_RUN: &str = "DROP TABLE run;
    CREATE TABLE run (id, goal, constraints, criteria, coordinator, integrator, member_limit, deadline, status);
    INSERT INTO run(id,status,coordinator) VALUES('r',1,2.5)";

/// `create` fetches the whole run row as Python does (#2270 round-4 review
/// L1): text that is not UTF-8 in any column is Python's `Could not decode`
/// refusal, its bytes one U+FFFD each, and a status or coordinator that is
/// not text (a BLOB, or a number in a table without TEXT affinity) reads
/// as `None`, never the setup placeholder.
#[test]
fn the_run_owner_row_is_fetched_as_python_fetches_it() {
    let (dir, repository) = repository();
    within(&repository, true, |_| Ok(())).unwrap();
    let edit = |sql: &str| {
        rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    };
    edit("INSERT INTO run(id,coordinator,deadline,status) VALUES('r','p',1.0,'setup')");
    for (sql, column, text) in [
        (
            "UPDATE run SET goal=CAST(x'61e282ff62' AS TEXT)",
            "goal",
            "a\u{fffd}\u{fffd}\u{fffd}b",
        ),
        (
            "UPDATE run SET goal='', status=CAST(x'ff' AS TEXT)",
            "status",
            "\u{fffd}",
        ),
        (
            "UPDATE run SET status='setup', coordinator=CAST(x'70ff00ff' AS TEXT)",
            "coordinator",
            "p\u{fffd}",
        ),
    ] {
        edit(sql);
        let refused = within(&repository, false, |transaction| {
            transaction.run_owner().map(|_| ())
        })
        .unwrap_err();
        assert_eq!(
            refused.message(),
            format!(
                "coordination store unavailable or contended: Could not decode to UTF-8 column '{column}' with text '{text}'"
            ),
            "{sql}"
        );
    }
    for (sql, coordinator) in [
        (
            "UPDATE run SET status=x'7365747570', coordinator='p'",
            Some("p"),
        ),
        ("UPDATE run SET status=NULL, coordinator=x'70'", None),
        // Without TEXT affinity a number stays a number.
        (UNTYPED_RUN, None),
    ] {
        edit(sql);
        within(&repository, false, |transaction| {
            assert_eq!(
                transaction.run_owner()?,
                Some(RunOwnerRow {
                    status: None,
                    coordinator: coordinator.map(str::to_owned),
                }),
                "{sql}"
            );
            Ok(())
        })
        .unwrap();
    }
}

/// `_bootstrap`'s member statement, as `swarm.py` held it (frozen here
/// when #2283 deleted the Python board): `pid`, `started` and `socket` are
/// its 3rd, 4th and 5th parameters, since `status` is the literal `'live'`.
const PYTHON_BOOTSTRAP_MEMBER_INSERT: &str =
    "INSERT INTO members(id,reservation,status,pid,started,socket) VALUES(?,?,'live',?,?,?)";

/// The 1-based parameter `column` binds in `statement`: the `?`s among the
/// values up to the column's own.
fn parameter_of(statement: &str, column: &str) -> usize {
    let (columns, values) = statement
        .split_once(") VALUES(")
        .expect("an INSERT ... VALUES statement");
    let columns: Vec<&str> = columns
        .split_once('(')
        .expect("a column list")
        .1
        .split(',')
        .collect();
    let values: Vec<&str> = values.trim_end_matches(')').split(',').collect();
    assert_eq!(columns.len(), values.len(), "{statement}");
    let index = columns
        .iter()
        .position(|name| *name == column)
        .unwrap_or_else(|| panic!("{column} in {statement}"));
    assert_eq!(values[index], "?", "{column} is bound");
    values[..=index]
        .iter()
        .filter(|value| **value == "?")
        .count()
}

/// A member value Python's `sqlite3` cannot bind is refused naming the
/// parameter Python names (#2270 review M1): its position in `_bootstrap`'s
/// statement, not in the Rust statement, which also binds the status and
/// the launcher.
#[test]
fn an_unbindable_member_value_names_pythons_parameter() {
    for column in ["pid", "started", "socket"] {
        let position = parameter_of(PYTHON_BOOTSTRAP_MEMBER_INSERT, column);
        let mut row = member("parent", MemberState::LIVE);
        let unbindable = json!([1]);
        match column {
            "pid" => row.pid = unbindable,
            "started" => row.started = unbindable,
            "socket" => row.socket = unbindable,
            _ => unreachable!("{column}"),
        }
        let (_dir, repository) = repository();
        let refused = within(&repository, true, |transaction| {
            transaction.insert_member(&row)
        })
        .unwrap_err();
        assert_eq!(
            refused.message(),
            format!(
                "coordination store unavailable or contended: \
                 Error binding parameter {position}: type 'list' is not supported"
            ),
            "{column}"
        );
    }
    assert_eq!(
        [
            super::PYTHON_PID_PARAMETER,
            super::PYTHON_STARTED_PARAMETER,
            super::PYTHON_SOCKET_PARAMETER
        ],
        ["pid", "started", "socket"]
            .map(|column| parameter_of(PYTHON_BOOTSTRAP_MEMBER_INSERT, column))
    );
}

/// A request's refusal keeps its kind through the ledger (#2303 reconcile
/// with #2272): the action's own refusal (a full task board) is returned
/// with the kind it was raised under, and the ledger's own refusals carry
/// theirs, never a catch-all.
#[test]
fn a_retried_request_keeps_its_refusal_kind() {
    use crate::domain::swarm::RefusalKind;

    let (_dir, repository) = repository();
    within(&repository, true, |_| Ok(())).unwrap();
    let full = within(&repository, false, |transaction| {
        transaction
            .retry("worker", "r1", &json!({"title": "t"}), &mut || {
                Err(BoardError::new(
                    RefusalKind::CapacityFull,
                    "task board full (1000); settle existing work",
                ))
            })
            .map(drop)
    })
    .unwrap_err();
    assert_eq!(
        (full.kind(), full.message()),
        (
            RefusalKind::CapacityFull,
            "task board full (1000); settle existing work"
        )
    );
    within(&repository, false, |transaction| {
        transaction
            .retry("worker", "r2", &json!({"title": "t"}), &mut || Ok(json!(1)))
            .map(drop)
    })
    .unwrap();
    let reused = within(&repository, false, |transaction| {
        transaction
            .retry("worker", "r2", &json!({"title": "u"}), &mut || Ok(json!(2)))
            .map(drop)
    })
    .unwrap_err();
    assert_eq!(
        (reused.kind(), reused.message()),
        (
            RefusalKind::RequestIdReused,
            "request id reused with different payload"
        )
    );
    let blank = within(&repository, false, |transaction| {
        transaction
            .retry("worker", " ", &json!({}), &mut || Ok(json!(3)))
            .map(drop)
    })
    .unwrap_err();
    assert_eq!(blank.kind(), RefusalKind::Invalid, "{blank}");
}
