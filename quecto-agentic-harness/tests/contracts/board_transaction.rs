//! `BoardTransaction` on the SQLite adapter (#2270): every role port of one
//! `atomic` is the same transaction. A write through one role is visible to
//! the others' reads before the commit, and a rollback undoes the writes of
//! every role together.
use quecto::application::swarm::dto::{BoardLocation, NewMember, NewRun, RunContract};
use quecto::application::swarm::ports::{
    BoardEvents, BoardMembers, BoardRepository, BoardRuns, BoardTransaction,
};
use quecto::domain::swarm::{BoardError, MemberState, RefusalKind, RunState};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn write_through_every_role(transaction: &dyn BoardTransaction) -> Result<(), BoardError> {
    let runs: &dyn BoardRuns = transaction;
    let members: &dyn BoardMembers = transaction;
    let events: &dyn BoardEvents = transaction;
    runs.insert_run(&NewRun {
        id: "run".into(),
        contract: RunContract {
            goal: "g".into(),
            constraints: json!([]),
            criteria: json!([]),
            member_limit: 3,
            deadline: 50.0,
        },
        coordinator: "parent".into(),
        integrator: "parent".into(),
        status: RunState::RUNNING,
    })?;
    members.insert_member(&NewMember {
        id: "worker".into(),
        reservation: "w".into(),
        status: MemberState::RESERVED,
        pid: serde_json::Value::Null,
        started: serde_json::Value::Null,
        socket: serde_json::Value::Null,
        launcher: Some("parent".into()),
    })?;
    events.event("parent", 1.0, "paused", &json!({}))?;
    // Each write is read back through a role handle other than its own
    // writer's, on the one transaction.
    let run = transaction
        .run()?
        .expect("the run written through BoardRuns");
    assert_eq!(
        members
            .claim_counts(run.coordinator.as_deref())?
            .members_without_claim,
        1
    );
    assert_eq!(
        runs.run_status()?.and_then(|row| row.id).as_deref(),
        Some("run")
    );
    assert_eq!(transaction.control_generation()?, 1);
    assert_eq!(transaction.usage()?, 1);
    Ok(())
}

#[test]
fn every_role_shares_one_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();

    let refused = repository.atomic(false, &mut |transaction| {
        write_through_every_role(transaction)?;
        Err(BoardError::new(RefusalKind::WrongState, "rolled back"))
    });
    assert_eq!(
        refused.unwrap_err(),
        BoardError::new(RefusalKind::WrongState, "rolled back")
    );
    repository
        .atomic(false, &mut |transaction| {
            assert!(transaction.run()?.is_none());
            assert!(transaction.members()?.is_empty());
            assert_eq!(transaction.control_generation()?, 0);
            Ok(())
        })
        .unwrap();

    repository
        .atomic(false, &mut |transaction| {
            write_through_every_role(transaction)
        })
        .unwrap();
    repository
        .atomic(false, &mut |transaction| {
            assert_eq!(
                transaction.run_status()?.and_then(|row| row.id).as_deref(),
                Some("run")
            );
            assert_eq!(transaction.members()?.len(), 1);
            assert_eq!(transaction.control_generation()?, 1);
            Ok(())
        })
        .unwrap();
}

/// A board transaction takes the write lock at `BEGIN` (#2272 review L3):
/// `BEGIN IMMEDIATE`, not a deferred `BEGIN` that locks at its first
/// write. While a transaction that has written nothing is open, another
/// connection's `BEGIN IMMEDIATE` is refused as busy at once; after it
/// commits, the same `BEGIN IMMEDIATE` succeeds.
#[test]
fn a_board_transaction_holds_the_write_lock_from_its_begin() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    let other = rusqlite::Connection::open(&database).unwrap();
    other.busy_timeout(std::time::Duration::ZERO).unwrap();

    let mut refused = None;
    repository
        .atomic(false, &mut |_| {
            refused = Some(other.execute_batch("BEGIN IMMEDIATE"));
            Ok(())
        })
        .unwrap();
    let error = refused
        .expect("the body ran")
        .expect_err("the board transaction holds the write lock");
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy),
        "{error}"
    );
    other.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
}
