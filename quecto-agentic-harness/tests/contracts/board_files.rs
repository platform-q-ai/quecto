//! `BoardFiles` on the SQLite adapter (#2272, #2275): a claim's file
//! reservations are deleted by task and claim together, never another
//! claim's or another task's, each bound as Python's `sqlite3` binds it;
//! a reservation set is inserted, counted, paged and deleted by Python's
//! SQL.
use quecto::application::swarm::dto::{BoardLocation, FileRow, NewReservation};
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

fn seeded(seed: &str) -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
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

fn rows(database: &std::path::Path) -> Vec<(String, String, String, String, String)> {
    rusqlite::Connection::open(database)
        .unwrap()
        .prepare(
            "SELECT path, typeof(task)||':'||task, owner, claim, token FROM files ORDER BY path",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A reservation set is inserted one row per path, the task bound as
/// Python binds it (INTEGER affinity keeps `"1"` as 1); the count, the
/// reserved check, a task's count and a page read what Python reads.
#[test]
fn a_reservation_set_is_inserted_counted_and_paged() {
    let (_dir, database, repository) = seeded("INSERT INTO files VALUES('z',2,'x','c2','t2');");
    let mut seen = Vec::new();
    let mut page = Vec::new();
    repository
        .atomic(false, &mut |transaction| {
            transaction.insert_files(&NewReservation {
                task: json!("1"),
                owner: "w".to_owned(),
                claim: json!("c1"),
                token: "t1".to_owned(),
                paths: vec!["a".to_owned(), "b".to_owned()],
            })?;
            seen.push(transaction.file_count()?);
            seen.push(i64::from(transaction.file_reserved("a")?));
            seen.push(i64::from(transaction.file_reserved("c")?));
            seen.push(transaction.task_file_count(&json!(1))?);
            seen.push(transaction.task_file_count(&json!(true))?);
            page = transaction.file_page(1, 5)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, [3, 1, 0, 2, 2]);
    assert_eq!(
        page.into_iter()
            .map(FileRow::into_value)
            .collect::<Vec<_>>(),
        [
            json!({"path": "b", "task": 1, "owner": "w", "claim": "c1", "token": "t1"}),
            json!({"path": "z", "task": 2, "owner": "x", "claim": "c2", "token": "t2"}),
        ]
    );
    assert_eq!(rows(&database)[0].1, "integer:1");
}

/// One set goes by task, owner, claim and token together; a task's files
/// go whichever claim made them.
#[test]
fn a_set_or_a_tasks_files_are_deleted_and_nothing_else() {
    let (_dir, database, repository) = seeded(
        "INSERT INTO files VALUES('a',1,'w','c','t1');
         INSERT INTO files VALUES('b',1,'w','c','t2');
         INSERT INTO files VALUES('c',1,'x','c','t1');
         INSERT INTO files VALUES('d',1,'w','old','t1');
         INSERT INTO files VALUES('e',2,'w','c','t1');",
    );
    // How many files it released (#2394 round-1 review L1): `a` alone, and
    // none for the same set again.
    for released in [1, 0] {
        repository
            .atomic(false, &mut |transaction| {
                let deleted =
                    transaction.delete_reservation(&json!(1), "w", &json!("c"), &json!("t1"))?;
                assert_eq!(deleted, released);
                Ok(())
            })
            .unwrap();
    }
    let left: Vec<String> = rows(&database).into_iter().map(|row| row.0).collect();
    assert_eq!(left, ["b", "c", "d", "e"]);
    repository
        .atomic(false, &mut |transaction| {
            transaction.delete_task_files(&json!("1"))
        })
        .unwrap();
    let left: Vec<String> = rows(&database).into_iter().map(|row| row.0).collect();
    assert_eq!(left, ["e"]);
}

/// An offset SQLite cannot bind is refused as Python's `sqlite3` refuses
/// an integer beyond i64 (the `integer_beyond_i64_is_refused` divergence).
#[test]
fn an_offset_beyond_sqlite_integers_is_refused() {
    let (_dir, _database, repository) = seeded("");
    let refused = repository
        .atomic(false, &mut |transaction| {
            transaction.file_page(u64::MAX, 5).map(|_| ())
        })
        .unwrap_err();
    assert!(
        refused.message().contains("Error binding parameter 2"),
        "{refused:?}"
    );
}
