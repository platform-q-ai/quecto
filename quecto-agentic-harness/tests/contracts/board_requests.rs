//! `BoardRequests` on the SQLite adapter (#2272): `Store.retry`, the
//! request ledger. A new request runs its action once and stores the
//! result with the board's `encode()`; the same payload replays the stored
//! text as `json.loads` reads it (keys sorted); another payload under the
//! same id is refused; a full ledger takes no new request.
use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::BoardError;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

fn board() -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    (dir, database, repository)
}

fn retry(
    repository: &SqliteBoardRepository,
    actor: &str,
    request: &str,
    payload: &Value,
    result: Result<Value, BoardError>,
) -> (Result<Value, BoardError>, usize) {
    let mut runs = 0;
    let mut answer = None;
    let outcome = repository.atomic(true, &mut |transaction: &dyn BoardTransaction| {
        let result = result.clone();
        answer = Some(transaction.retry(actor, request, payload, &mut || {
            runs += 1;
            result.clone()
        })?);
        Ok(())
    });
    (outcome.map(|()| answer.expect("an answer")), runs)
}

#[test]
fn ledger_replays_and_refuses_conflicts() {
    let (_dir, database, repository) = board();
    let payload = json!(["task", "é", ["a"], [1]]);
    let result = json!({"z": 1, "a": [2.5, "é"]});
    let (first, runs) = retry(&repository, "worker", "r1", &payload, Ok(result.clone()));
    assert_eq!((first.unwrap(), runs), (result.clone(), 1));
    let stored: (String, String) = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT payload, result FROM requests", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(
        stored,
        (
            r#"["task","\u00e9",["a"],[1]]"#.to_owned(),
            r#"{"a":[2.5,"\u00e9"],"z":1}"#.to_owned()
        )
    );
    let (replayed, runs) = retry(&repository, "worker", "r1", &payload, Ok(json!("unused")));
    let replayed = replayed.unwrap();
    assert_eq!(runs, 0);
    assert_eq!(
        serde_json::to_string(&replayed).unwrap(),
        r#"{"a":[2.5,"é"],"z":1}"#
    );
    // The payload compares as encoded text: `1.0` is not `1`.
    let (conflict, runs) = retry(
        &repository,
        "worker",
        "r1",
        &json!(["task", "é", ["a"], [1.0]]),
        Ok(json!(null)),
    );
    assert_eq!(
        (conflict.unwrap_err(), runs),
        (
            BoardError::new("request id reused with different payload"),
            0
        )
    );
    // Request ids are per actor.
    let (other, runs) = retry(&repository, "parent", "r1", &json!([]), Ok(json!(3)));
    assert_eq!((other.unwrap(), runs), (json!(3), 1));
}

/// The action's refusal is the call's, and nothing is stored for it.
#[test]
fn a_refused_action_stores_nothing() {
    let (_dir, database, repository) = board();
    let refusal = BoardError::new("task board full (1000); settle existing work");
    let (answer, runs) = retry(
        &repository,
        "worker",
        "r1",
        &json!([]),
        Err(refusal.clone()),
    );
    assert_eq!((answer.unwrap_err(), runs), (refusal, 1));
    let held: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM requests", [], |row| row.get(0))
        .unwrap();
    assert_eq!(held, 0);
}

#[test]
fn a_full_ledger_takes_no_new_request() {
    let (_dir, database, repository) = board();
    retry(&repository, "worker", "seed", &json!([]), Ok(json!(1)))
        .0
        .unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < 9999) \
             INSERT INTO requests SELECT 'bulk', i, '[]', '1' FROM n",
        )
        .unwrap();
    let (full, runs) = retry(&repository, "worker", "new", &json!([]), Ok(json!(1)));
    assert_eq!(
        (full.unwrap_err(), runs),
        (
            BoardError::new("coordination request ledger full (10000)"),
            0
        )
    );
    let (replayed, runs) = retry(&repository, "worker", "seed", &json!([]), Ok(json!(2)));
    assert_eq!(
        (replayed.unwrap(), runs),
        (json!(1), 0),
        "replays still answer"
    );
}
