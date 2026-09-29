//! `BoardEvidence` on the SQLite adapter (#2273): what completion reads,
//! by Python's SQL (`swarm_repository.Transaction`), the task evidence a
//! revalidation writes with plain `json.dumps`, and the criterion evidence
//! an amendment deletes and `evidence` records.
use quecto::application::swarm::dto::{BoardLocation, EvidenceEntry, NewEvidence, PriorEvidence};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::BoardError;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn board(seed: &str) -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(seed)
        .unwrap();
    (dir, database, repository)
}

fn within(
    repository: &SqliteBoardRepository,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) {
    repository
        .atomic(false, &mut |transaction| work(transaction))
        .unwrap();
}

const RUN: &str = r#"INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status)
    VALUES('r','g','[]','[{"description":"d","id":"t","kind":"command"}]','parent','parent',3,10.0,'running');"#;

/// Completion reads the criteria loaded, every evidence row as stored,
/// every task by id with only its evidence loaded as `Transaction.task`
/// does it (the stored status, never a derived one), and whether any file
/// reservation is left.
#[test]
fn completion_state_reads_what_python_reads() {
    let seed = format!(
        "{RUN}
         INSERT INTO evidence VALUES('t','ci.log','R1','command','parent',1);
         INSERT INTO evidence VALUES('t','w.log','R0','command','worker',0);
         INSERT INTO tasks(id,title,acceptance,dependencies,status,evidence) VALUES(2,'b','[]','[1]','ready','[]');
         INSERT INTO tasks(id,title,acceptance,dependencies,status,evidence) VALUES(1,'a','[]','[]','completed','[{{\"artifact\": \"a\", \"revision\": \"R1\"}}]');"
    );
    let (_dir, database, repository) = board(&seed);
    within(&repository, |transaction| {
        let state = transaction.completion_state()?;
        assert_eq!(
            state.criteria,
            json!([{"description": "d", "id": "t", "kind": "command"}])
        );
        assert_eq!(
            state.evidence,
            [
                EvidenceEntry {
                    criterion: json!("t"),
                    artifact: json!("ci.log"),
                    revision: json!("R1"),
                    kind: json!("command"),
                    accepted: json!(1),
                },
                EvidenceEntry {
                    criterion: json!("t"),
                    artifact: json!("w.log"),
                    revision: json!("R0"),
                    kind: json!("command"),
                    accepted: json!(0),
                },
            ]
        );
        let tasks: Vec<_> = state
            .tasks
            .iter()
            .map(|task| {
                (
                    task.get("id").cloned(),
                    task.text("status"),
                    task.get("evidence").cloned(),
                )
            })
            .collect();
        assert_eq!(
            tasks,
            [
                (
                    Some(json!(1)),
                    Some("completed"),
                    Some(json!([{"artifact": "a", "revision": "R1"}]))
                ),
                (Some(json!(2)), Some("ready"), Some(json!([]))),
            ],
            "by id, the stored status"
        );
        assert!(!state.has_reservations);
        Ok(())
    });
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute("INSERT INTO files VALUES('p',1,'w','c','t')", [])
        .unwrap();
    within(&repository, |transaction| {
        assert!(transaction.completion_state()?.has_reservations);
        Ok(())
    });
}

/// A task's evidence is replaced with plain `json.dumps` text (insertion
/// order, `", "` and `": "`, `ensure_ascii`), the id bound as Python binds
/// it; every criterion evidence row is deleted at once.
#[test]
fn task_evidence_is_written_by_plain_dumps_and_evidence_deleted() {
    let seed = format!(
        "{RUN}
         INSERT INTO evidence VALUES('t','ci.log','R1','command','parent',1);
         INSERT INTO evidence VALUES('u','ci.log','R1','review','worker',0);
         INSERT INTO tasks(id,title,acceptance,dependencies,status,evidence) VALUES(1,'a','[]','[]','completed','[]');"
    );
    let (_dir, database, repository) = board(&seed);
    within(&repository, |transaction| {
        transaction.replace_task_evidence(
            &json!("1"),
            &json!([{"revision": "R2", "artifact": "é", "n": 1.0}]),
        )?;
        transaction.delete_all_evidence()
    });
    let connection = rusqlite::Connection::open(&database).unwrap();
    let evidence: String = connection
        .query_row("SELECT evidence FROM tasks WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        evidence,
        r#"[{"revision": "R2", "artifact": "\u00e9", "n": 1.0}]"#
    );
    let left: i64 = connection
        .query_row("SELECT count(*) FROM evidence", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

/// `evidence` reads the row an actor recorded for a criterion and replaces
/// it (one row per criterion and actor), `accepted` stored as 0 or 1.
#[test]
fn evidence_is_read_and_recorded_per_criterion_and_actor() {
    let (_dir, database, repository) = board(RUN);
    let record = |actor: &str, artifact: &str, accepted: bool| NewEvidence {
        criterion: json!("t"),
        artifact: artifact.into(),
        revision: "R1".into(),
        kind: "command".into(),
        actor: actor.into(),
        accepted,
    };
    within(&repository, |transaction| {
        assert_eq!(transaction.prior_evidence(&json!("t"), "parent")?, None);
        transaction.record_evidence(&record("parent", "a", false))?;
        transaction.record_evidence(&record("worker", "w", false))?;
        transaction.record_evidence(&record("parent", "b", true))?;
        assert_eq!(
            transaction.prior_evidence(&json!("t"), "parent")?,
            Some(PriorEvidence {
                artifact: json!("b"),
                revision: json!("R1"),
                kind: json!("command"),
                accepted: json!(1),
            })
        );
        assert_eq!(transaction.prior_evidence(&json!("u"), "parent")?, None);
        Ok(())
    });
    let rows: Vec<(String, String, i64)> = rusqlite::Connection::open(&database)
        .unwrap()
        .prepare("SELECT actor, artifact, accepted FROM evidence ORDER BY actor")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            ("parent".to_owned(), "b".to_owned(), 1),
            ("worker".to_owned(), "w".to_owned(), 0),
        ]
    );
}
