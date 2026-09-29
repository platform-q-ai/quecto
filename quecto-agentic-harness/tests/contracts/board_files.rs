//! `BoardFiles` on the SQLite adapter (#2272): a claim's file reservations
//! are deleted by task and claim together, never another claim's or
//! another task's, each bound as Python's `sqlite3` binds it.
use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardRepository;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

#[test]
fn claim_files_are_deleted_by_task_and_claim_only() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "INSERT INTO files VALUES('a',1,'w','claim','r1');
             INSERT INTO files VALUES('b',1,'w','older','r2');
             INSERT INTO files VALUES('c',2,'w','claim','r3');
             INSERT INTO files VALUES('d',1,'x','claim','r4');",
        )
        .unwrap();
    repository
        .atomic(false, &mut |transaction| {
            transaction.delete_claim_files(&json!("1"), &json!("claim"))
        })
        .unwrap();
    let left: Vec<String> = rusqlite::Connection::open(&database)
        .unwrap()
        .prepare("SELECT path FROM files ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, ["b", "c"], "every owner's rows under that claim go");
}
