//! `BoardRepository` on the SQLite adapter (#2270): one transaction per
//! `atomic`, committed when the work succeeds and rolled back, with every
//! role's writes, when it fails; the work's refusal comes back unchanged.
use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::BoardError;
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

fn atomic(
    repository: &dyn BoardRepository,
    create: bool,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(create, &mut |transaction| work(transaction))
}

#[test]
fn an_error_inside_the_work_leaves_no_rows_and_returns_the_refusal() {
    let (_dir, repository) = board();
    atomic(&repository, true, |_| Ok(())).unwrap();
    let refused = atomic(&repository, false, |transaction| {
        transaction.insert_member(&quecto::application::swarm::dto::NewMember {
            id: "parent".into(),
            reservation: "r".into(),
            status: quecto::domain::swarm::MemberState::LIVE,
            pid: serde_json::Value::Null,
            started: serde_json::Value::Null,
            socket: serde_json::Value::Null,
            launcher: None,
        })?;
        transaction.event(
            "parent",
            1.0,
            "container_setup",
            &json!({"member": "parent"}),
        )?;
        Err(BoardError::new("refused by the work"))
    })
    .unwrap_err();
    assert_eq!(refused, BoardError::new("refused by the work"));
    atomic(&repository, false, |transaction| {
        assert!(transaction.members()?.is_empty());
        assert_eq!(transaction.control_generation()?, 0);
        Ok(())
    })
    .unwrap();
}

/// The operation gate's guarantee rests on this: a committed transaction
/// (the expiry pause) survives a following one that is refused.
#[test]
fn the_expiry_commit_survives_a_rejected_mutation() {
    let (dir, repository) = board();
    atomic(&repository, true, |transaction| {
        transaction.insert_run(&quecto::application::swarm::dto::NewRun {
            id: "run".into(),
            contract: quecto::application::swarm::dto::RunContract {
                goal: "g".into(),
                constraints: json!([]),
                criteria: json!([]),
                member_limit: 2,
                deadline: 10.0,
            },
            coordinator: "parent".into(),
            integrator: "parent".into(),
            status: quecto::domain::swarm::RunState::RUNNING,
        })
    })
    .unwrap();
    atomic(&repository, false, |transaction| {
        transaction.propose_outcome("budget-exhausted", "deadline")?;
        transaction.event(
            "parent",
            10.0,
            "stop",
            &json!({"status": "budget-exhausted"}),
        )?;
        transaction.event("parent", 10.0, "paused", &json!({"started": 10.0}))
    })
    .unwrap();
    let refused = atomic(&repository, false, |transaction| {
        transaction.event("parent", 11.0, "claimed", &json!({"task": 1}))?;
        Err(BoardError::new(
            "run is paused (budget-exhausted: deadline); no new work permitted",
        ))
    });
    assert!(refused.is_err());
    atomic(&repository, false, |transaction| {
        let run = transaction.run()?.unwrap();
        assert_eq!(run.status.as_str(), "paused");
        assert_eq!(run.outcome.as_deref(), Some("budget-exhausted"));
        assert_eq!(transaction.control_generation()?, 2);
        Ok(())
    })
    .unwrap();
    let actions: Vec<String> = rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .prepare("SELECT action FROM events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(actions, ["stop", "paused"]);
}

#[test]
fn a_missing_board_is_refused_unless_created() {
    let (dir, repository) = board();
    let refused = atomic(&repository, false, |_| Ok(())).unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(format!(
            "coordination store missing at {}: it was deleted while the run was live, so this run's board is lost",
            dir.path().join("swarm.sqlite").display()
        ))
    );
    assert!(
        !dir.path().join("swarm.sqlite").exists(),
        "mode=rw never creates"
    );
    atomic(&repository, true, |_| Ok(())).unwrap();
    assert!(dir.path().join("swarm.sqlite").exists());
}
