//! `BoardRuns` on the SQLite adapter (#2270): the `run` row as Python's
//! board writes it — the contract's JSON through the board's `encode()`,
//! the deadline a REAL, the status text.
use quecto::application::swarm::dto::{
    BoardLocation, NewRun, RunContract, RunOwnerRow, RunStatusRow,
};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, RunState};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn board() -> (tempfile::TempDir, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
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
) {
    repository
        .atomic(create, &mut |transaction| work(transaction))
        .unwrap();
}

fn contract(goal: &str, deadline: f64) -> RunContract {
    RunContract {
        goal: goal.into(),
        constraints: json!(["é", "b"]),
        criteria: json!([{"kind": "review", "id": "c", "description": "d"}]),
        member_limit: 4,
        deadline,
    }
}

#[test]
fn a_run_is_inserted_read_updated_and_paused() {
    let (dir, repository) = board();
    within(&repository, true, |transaction| {
        assert!(transaction.run()?.is_none());
        assert!(!transaction.run_exists()?);
        assert!(transaction.run_owner()?.is_none());
        assert!(transaction.run_status()?.is_none());
        transaction.insert_run(&NewRun {
            id: "abc".into(),
            contract: contract("first", 0.0),
            coordinator: "parent".into(),
            integrator: "parent".into(),
            status: RunState::SETUP,
        })
    });
    within(&repository, false, |transaction| {
        assert!(transaction.run_exists()?);
        assert_eq!(
            transaction.run_owner()?,
            Some(RunOwnerRow {
                status: Some("setup".into()),
                coordinator: Some("parent".into()),
            })
        );
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status, Some(RunState::SETUP));
        assert_eq!(run.coordinator.as_deref(), Some("parent"));
        assert_eq!((run.deadline, run.member_limit), (0.0, 4));
        assert_eq!((run.outcome, run.outcome_reason), (None, None));
        assert_eq!(
            transaction.run_status()?,
            Some(RunStatusRow {
                id: Some("abc".into()),
                status: Some("setup".into()),
                deadline: json!(0.0),
                coordinator: Some("parent".into()),
                outcome: None,
            })
        );
        transaction.update_run_contract(&contract("second", 99.5))
    });
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(
            run.status,
            Some(RunState::RUNNING),
            "the contract starts the run"
        );
        assert_eq!(run.deadline, 99.5);
        assert_eq!(
            transaction.run_status()?.and_then(|row| row.id).as_deref(),
            Some("abc"),
            "the id is kept"
        );
        transaction.propose_outcome("failed", "lost")
    });
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status, Some(RunState::PAUSED));
        assert_eq!(run.outcome.as_deref(), Some("failed"));
        assert_eq!(run.outcome_reason.as_deref(), Some("lost"));
        Ok(())
    });
    let stored: (String, String, String, String, String) =
        rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
            .unwrap()
            .query_row(
                "SELECT goal, constraints, criteria, typeof(deadline), integrator FROM run",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
    assert_eq!(
        stored,
        (
            "second".to_owned(),
            r#"["\u00e9","b"]"#.to_owned(),
            r#"[{"description":"d","id":"c","kind":"review"}]"#.to_owned(),
            "real".to_owned(),
            "parent".to_owned(),
        )
    );
}

/// Rows edited outside the board (#2270 review L5, round-2 L2, round-4
/// L1): `run()` reads a NULL coordinator or status as `None`;
/// `run_status()` fetches only `_status`'s columns, `run_owner()` the whole
/// row as `create` does, reading only its status and coordinator, and
/// `run_exists()` none, each as stored, so a column one does not read
/// never refuses it unless its text is not UTF-8; a column `run()` needs
/// (for `_snapshot`) holding a type the board never writes is a store
/// refusal (the `outside_edited_columns` divergence).
#[test]
fn loosely_typed_run_columns_read_as_stored() {
    let (dir, repository) = board();
    within(&repository, true, |transaction| {
        transaction.insert_run(&NewRun {
            id: "abc".into(),
            contract: contract("first", 0.0),
            coordinator: "parent".into(),
            integrator: "parent".into(),
            status: RunState::SETUP,
        })
    });
    let edit = |sql: &str| {
        rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    };
    edit("UPDATE run SET coordinator=NULL, member_limit=4, status=NULL");
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.coordinator, None);
        assert_eq!(run.status, None, "a NULL status reads as Python's None");
        Ok(())
    });
    edit("UPDATE run SET member_limit='many', status=NULL, deadline='soon', outcome=4");
    within(&repository, false, |transaction| {
        assert_eq!(
            transaction.run_status()?,
            Some(RunStatusRow {
                id: Some("abc".into()),
                status: None,
                deadline: json!("soon"),
                coordinator: None,
                outcome: Some("4".into()),
            })
        );
        // `_bootstrap` and `create` read no column `run()` refuses.
        assert!(transaction.run_exists()?);
        assert_eq!(
            transaction.run_owner()?,
            Some(RunOwnerRow {
                status: None,
                coordinator: None,
            })
        );
        let refused = transaction.run().unwrap_err();
        assert!(
            refused
                .0
                .starts_with("coordination store unavailable or contended: "),
            "{refused}"
        );
        Ok(())
    });
    for (deadline, expected) in [("7", json!(7.0)), ("x'00'", json!(null))] {
        edit(&format!("UPDATE run SET deadline={deadline}"));
        let read = repository.atomic(false, &mut |transaction| {
            let row = transaction.run_status()?.unwrap();
            assert_eq!(row.deadline, expected, "{deadline}");
            Ok(())
        });
        // A BLOB is no JSON value: refused.
        assert_eq!(read.is_ok(), deadline == "7", "{deadline}: {read:?}");
    }
}

/// #2271: `join_process` reads the coordinator column alone (`SELECT
/// coordinator FROM run`): no run is `None`, a NULL or non-text one is
/// `Some(None)`, and a column it does not read never refuses it.
#[test]
fn the_coordinator_is_read_alone() {
    let (dir, repository) = board();
    within(&repository, true, |transaction| {
        assert_eq!(transaction.run_coordinator()?, None);
        transaction.insert_run(&NewRun {
            id: "abc".into(),
            contract: contract("first", 0.0),
            coordinator: "parent".into(),
            integrator: "parent".into(),
            status: RunState::SETUP,
        })?;
        assert_eq!(transaction.run_coordinator()?, Some(Some("parent".into())));
        Ok(())
    });
    let edit = |sql: &str| {
        rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    };
    edit("UPDATE run SET goal=CAST(x'ff' AS TEXT), member_limit='many'");
    within(&repository, false, |transaction| {
        assert_eq!(transaction.run_coordinator()?, Some(Some("parent".into())));
        Ok(())
    });
    for value in ["NULL", "x'706172656e74'"] {
        edit(&format!("UPDATE run SET coordinator={value}"));
        within(&repository, false, |transaction| {
            assert_eq!(transaction.run_coordinator()?, Some(None), "{value}");
            Ok(())
        });
    }
}
