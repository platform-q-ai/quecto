//! `BoardRuns` on the SQLite adapter (#2270): the `run` row as Python's
//! board writes it — the contract's JSON through the board's `encode()`,
//! the deadline a REAL, the status text.
use quecto::application::swarm::dto::{BoardLocation, NewRun, RunContract};
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
        assert!(transaction.run_id()?.is_none());
        transaction.insert_run(&NewRun {
            id: "abc".into(),
            contract: contract("first", 0.0),
            coordinator: "parent".into(),
            integrator: "parent".into(),
            status: RunState::SETUP,
        })
    });
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status, RunState::SETUP);
        assert_eq!(run.coordinator, "parent");
        assert_eq!((run.deadline, run.member_limit), (0.0, 4));
        assert_eq!((run.outcome, run.outcome_reason), (None, None));
        assert_eq!(transaction.run_id()?.as_deref(), Some("abc"));
        transaction.update_run_contract(&contract("second", 99.5))
    });
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status, RunState::RUNNING, "the contract starts the run");
        assert_eq!(run.deadline, 99.5);
        assert_eq!(
            transaction.run_id()?.as_deref(),
            Some("abc"),
            "the id is kept"
        );
        transaction.propose_outcome("failed", "lost")
    });
    within(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status, RunState::PAUSED);
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
