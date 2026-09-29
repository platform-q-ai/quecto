//! `BoardTasks` on the SQLite adapter (#2272): task rows as `dict(row)`
//! with their JSON columns loaded, the dependency graph, the count, and
//! the task writes, each by Python's SQL; a task id bound as Python's
//! `sqlite3` binds it (epic P3).
use quecto::application::swarm::dto::{BoardLocation, NewTask, TaskUpdate};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, RefusalKind};
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

fn within(
    repository: &SqliteBoardRepository,
    create: bool,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(create, &mut |transaction| work(transaction))
}

fn new_task(title: &str, dependencies: Value) -> NewTask {
    NewTask {
        title: title.to_owned(),
        acceptance: json!(["é passes", "b"]),
        dependencies,
    }
}

fn raw(database: &std::path::Path, sql: &str) -> Vec<Vec<String>> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| {
                    row.get::<_, rusqlite::types::Value>(index)
                        .map(|value| format!("{value:?}"))
                })
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn tasks_are_inserted_read_and_counted() {
    let (_dir, database, repository) = board();
    within(&repository, true, |transaction| {
        assert_eq!(transaction.task_count()?, 0);
        assert_eq!(transaction.insert_task(&new_task("first", json!([])))?, 1);
        assert_eq!(transaction.insert_task(&new_task("second", json!([1])))?, 2);
        assert_eq!(transaction.task_count()?, 2);
        Ok(())
    })
    .unwrap();
    // The board's `encode()`: sorted, compact, ASCII-escaped.
    assert_eq!(
        raw(
            &database,
            "SELECT acceptance, dependencies, status, evidence FROM tasks WHERE id=2"
        ),
        [[
            r#"Text("[\"\\u00e9 passes\",\"b\"]")"#,
            r#"Text("[1]")"#,
            r#"Text("ready")"#,
            r#"Text("[]")"#,
        ]]
    );
    within(&repository, false, |transaction| {
        let row = transaction.task(&json!(2))?.expect("task 2");
        assert_eq!(
            row.into_value(),
            json!({"id": 2, "title": "second", "acceptance": ["é passes", "b"],
                   "dependencies": [1], "status": "ready", "owner": null, "token": null,
                   "evidence": [], "blocker": null})
        );
        let order: Vec<String> = transaction
            .task(&json!(1))?
            .unwrap()
            .columns
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            order,
            [
                "id",
                "title",
                "acceptance",
                "dependencies",
                "status",
                "owner",
                "token",
                "evidence",
                "blocker"
            ]
        );
        assert_eq!(
            transaction.all_task_dependencies()?,
            vec![(1, json!([])), (2, json!([1]))]
        );
        Ok(())
    })
    .unwrap();
}

/// A task id is bound as Python binds it: numeric text, `true` and an
/// integral float find the task through INTEGER affinity, NULL and other
/// text find nothing, and a list is refused as Python's `sqlite3` refuses
/// it.
#[test]
fn task_ids_bind_as_python_binds_them() {
    let (_dir, database, repository) = board();
    within(&repository, true, |transaction| {
        transaction.insert_task(&new_task("first", json!([])))?;
        Ok(())
    })
    .unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute("INSERT INTO tasks(id,status) VALUES(5, x'6869')", [])
        .unwrap();
    within(&repository, false, |transaction| {
        for found in [json!(1), json!("1"), json!(true), json!(1.0)] {
            assert!(transaction.task(&found)?.is_some(), "{found}");
            assert_eq!(
                transaction.task_status(&found)?.as_deref(),
                Some("ready"),
                "{found}"
            );
        }
        for missing in [
            json!(null),
            json!("one"),
            json!(1.5),
            json!(false),
            json!(9),
        ] {
            assert!(transaction.task(&missing)?.is_none(), "{missing}");
            assert_eq!(transaction.task_status(&missing)?, None, "{missing}");
        }
        // A status that is not text (here a BLOB) is none.
        assert_eq!(transaction.task_status(&json!(5))?, None);
        Ok(())
    })
    .unwrap();
    let refused = within(&repository, false, |transaction| {
        transaction.task(&json!([1])).map(|_| ())
    })
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::Invalid,
            "coordination store unavailable or contended: Error binding parameter 1: type 'list' is not supported"
        )
    );
}

/// The claim columns: claimed by an owner under a token, released back to
/// ready; dependencies replaced with the board's encoding.
#[test]
fn claims_and_dependencies_are_written_by_pythons_sql() {
    let (_dir, database, repository) = board();
    within(&repository, true, |transaction| {
        transaction.insert_task(&new_task("first", json!([])))?;
        transaction.insert_task(&new_task("second", json!([])))?;
        transaction.update_task_claim(&json!("1"), "worker", "tok")?;
        transaction.set_task_dependencies(&json!(2), &json!([1, 1]))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        raw(
            &database,
            "SELECT status, owner, token, dependencies FROM tasks ORDER BY id"
        ),
        [
            [
                r#"Text("claimed")"#,
                r#"Text("worker")"#,
                r#"Text("tok")"#,
                r#"Text("[]")"#
            ],
            [r#"Text("ready")"#, "Null", "Null", r#"Text("[1,1]")"#],
        ]
    );
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute("UPDATE tasks SET blocker='held' WHERE id=1", [])
        .unwrap();
    within(&repository, false, |transaction| {
        transaction.update_task_status(&json!(true), &TaskUpdate::Release)
    })
    .unwrap();
    assert_eq!(
        raw(
            &database,
            "SELECT status, owner, token, blocker FROM tasks WHERE id=1"
        ),
        [[r#"Text("ready")"#, "Null", "Null", "Null"]]
    );
}
