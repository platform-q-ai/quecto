//! `BoardUsage` on the SQLite adapter (#2273): the usage report by
//! Python's SQL. The report creates `request_usage` and `usage_budget`
//! lazily, in Python's order and with its statements, and writes no
//! default budget row; the aggregates are keyed by their SQL aliases. The
//! budget and request-ledger writes (#2274) store Python's bytes.
use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardRepository;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

fn usage_board() -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    (dir, database, repository)
}

fn tables(database: &std::path::Path) -> Vec<(String, String)> {
    rusqlite::Connection::open(database)
        .unwrap()
        .prepare("SELECT name, sql FROM sqlite_master WHERE type='table' ORDER BY rowid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// On a fresh board the report creates the two tables, last and in
/// Python's order, inserts no budget row, and reads the default budget
/// and zero totals; a second report creates nothing more.
#[test]
fn the_report_on_a_fresh_board_creates_the_usage_tables_and_no_budget_row() {
    let (_dir, database, repository) = usage_board();
    let before = tables(&database);
    for _ in 0..2 {
        repository
            .atomic(false, &mut |transaction| {
                let report = transaction.usage_report()?;
                assert_eq!(
                    report.budget.to_string(),
                    r#"{"token_limit":null,"strict_unknown":false,"warned":false}"#
                );
                let totals: Vec<(&str, &Value)> = report
                    .totals
                    .columns
                    .iter()
                    .map(|(name, value)| (name.as_str(), value))
                    .collect();
                let zero = json!(0);
                assert_eq!(
                    totals,
                    [
                        ("requests", &zero),
                        ("observed_tokens", &zero),
                        ("unknown_usage_requests", &zero),
                        ("attempts", &zero),
                        ("reported_input_tokens", &zero),
                        ("reported_output_tokens", &zero),
                        ("reported_cache_read_tokens", &zero),
                        ("reported_cache_write_tokens", &zero),
                        ("cache_read_known_requests", &zero),
                        ("cache_write_known_requests", &zero),
                    ]
                );
                assert!(report.members.is_empty() && report.recent_requests.is_empty());
                Ok(())
            })
            .unwrap();
    }
    let after = tables(&database);
    assert_eq!(after[..before.len()], before[..], "the schema is kept");
    assert_eq!(
        after[before.len()..],
        [
            (
                "request_usage".to_owned(),
                "CREATE TABLE request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER)".to_owned()
            ),
            (
                "usage_budget".to_owned(),
                "CREATE TABLE usage_budget (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT)"
                    .to_owned()
            ),
        ]
    );
    let budget_rows: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM usage_budget", [], |row| row.get(0))
        .unwrap();
    assert_eq!(budget_rows, 0, "no default budget row is written");
}

/// Stored requests aggregate by the SQL: `count(cache_read_tokens)`
/// counts measured requests only, members group by actor in order, and
/// the ten latest observations come newest first; a stored budget is read
/// as its payload.
#[test]
fn stored_requests_aggregate_by_pythons_sql() {
    let (_dir, database, repository) = usage_board();
    repository
        .atomic(false, &mut |transaction| {
            transaction.usage_report().map(|_| ())
        })
        .unwrap();
    let mut inserts = String::from(
        "INSERT INTO usage_budget VALUES(1, '{\"token_limit\": 50, \"strict_unknown\": true, \"warned\": false}');",
    );
    for index in 0..12 {
        let actor = if index % 3 == 0 { "b" } else { "a" };
        let cache = if index == 0 {
            "NULL".to_owned()
        } else {
            index.to_string()
        };
        inserts.push_str(&format!(
            "INSERT INTO request_usage VALUES('r{index}','{actor}','{{\"request_id\":\"r{index}\"}}',{index},0,1,1,2,{cache},NULL);"
        ));
    }
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(&inserts)
        .unwrap();
    repository
        .atomic(false, &mut |transaction| {
            let report = transaction.usage_report()?;
            assert_eq!(
                report.budget,
                json!({"token_limit": 50, "strict_unknown": true, "warned": false})
            );
            let totals = &report.totals;
            assert_eq!(totals.get("requests"), Some(&json!(12)));
            assert_eq!(totals.get("observed_tokens"), Some(&json!(66)));
            assert_eq!(totals.get("cache_read_known_requests"), Some(&json!(11)));
            assert_eq!(totals.get("cache_write_known_requests"), Some(&json!(0)));
            let members: Vec<_> = report
                .members
                .iter()
                .map(|row| (row.columns[0].clone(), row.get("requests").cloned()))
                .collect();
            assert_eq!(
                members,
                [
                    (("member".to_owned(), json!("a")), Some(json!(8))),
                    (("member".to_owned(), json!("b")), Some(json!(4))),
                ]
            );
            let recent: Vec<_> = report
                .recent_requests
                .iter()
                .map(|recent| recent.observation["request_id"].clone())
                .collect();
            assert_eq!(recent.len(), 10);
            assert_eq!((&recent[0], &recent[9]), (&json!("r11"), &json!("r2")));
            Ok(())
        })
        .unwrap();
}

fn raw_rows(database: &std::path::Path, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| row.get::<_, rusqlite::types::Value>(index))
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The budget reads as the default without a row (and writes none), and
/// a configured budget is stored as plain `json.dumps` writes it: insertion
/// order, `", "` and `": "`, byte for byte; a second write replaces it.
#[test]
fn the_budget_payload_is_stored_as_json_dumps_writes_it() {
    let (_dir, database, repository) = usage_board();
    repository
        .atomic(false, &mut |transaction| {
            assert_eq!(
                transaction.usage_budget()?.to_string(),
                r#"{"token_limit":null,"strict_unknown":false,"warned":false}"#
            );
            Ok(())
        })
        .unwrap();
    assert!(raw_rows(&database, "SELECT * FROM usage_budget").is_empty());
    repository
        .atomic(false, &mut |transaction| {
            transaction.configure_usage_budget(
                &json!({"token_limit": 100, "strict_unknown": true, "warned": false}),
            )?;
            transaction.configure_usage_budget(
                &json!({"warned": true, "token_limit": 9_223_372_036_854_775_807_i64, "strict_unknown": false, "é": "é"}),
            )
        })
        .unwrap();
    assert_eq!(
        raw_rows(&database, "SELECT id, payload FROM usage_budget"),
        [vec![
            rusqlite::types::Value::Integer(1),
            rusqlite::types::Value::Text(
                r#"{"warned": true, "token_limit": 9223372036854775807, "strict_unknown": false, "\u00e9": "\u00e9"}"#
                    .to_owned()
            ),
        ]]
    );
    repository
        .atomic(false, &mut |transaction| {
            let budget = transaction.usage_budget()?;
            assert_eq!(
                serde_json::to_string(&budget).unwrap(),
                r#"{"warned":true,"token_limit":9223372036854775807,"strict_unknown":false,"é":"é"}"#,
                "read back in stored order"
            );
            Ok(())
        })
        .unwrap();
}

/// A request row is inserted in the table's column order (the record
/// stored with the board's `encode()`, the four reported counts NULL when
/// unreported), read back by its id as its actor and decoded payload, its
/// payload replaced in place, and counted.
#[test]
fn a_request_row_round_trips_its_payload_bytes() {
    use quecto::application::swarm::dto::{NewRequestUsage, StoredRequestUsage};
    let (_dir, database, repository) = usage_board();
    let record = json!({"request_id": "r1", "outcome": "succeeded", "model": "m\u{e9}",
        "duration_ms": 1.5, "runtime": {"process_instance_id": "p"}});
    repository
        .atomic(false, &mut |transaction| {
            assert_eq!(transaction.request_usage("r1")?, None);
            assert_eq!(transaction.request_usage_count()?, 0);
            transaction.insert_request_usage(&NewRequestUsage {
                request_id: "r1".to_owned(),
                actor: "worker".to_owned(),
                record: record.clone(),
                tokens: 80,
                unknown: 0,
                attempts: 2,
                input_tokens: Some(50),
                output_tokens: None,
                cache_read_tokens: Some(4_294_967_295),
                cache_write_tokens: None,
            })?;
            assert_eq!(
                transaction.request_usage("r1")?,
                Some(StoredRequestUsage {
                    actor: json!("worker"),
                    payload: record.clone(),
                })
            );
            assert_eq!(transaction.request_usage_count()?, 1);
            Ok(())
        })
        .unwrap();
    use rusqlite::types::Value as Sql;
    assert_eq!(
        raw_rows(&database, "SELECT * FROM request_usage"),
        [vec![
            Sql::Text("r1".to_owned()),
            Sql::Text("worker".to_owned()),
            Sql::Text(
                r#"{"duration_ms":1.5,"model":"m\u00e9","outcome":"succeeded","request_id":"r1","runtime":{"process_instance_id":"p"}}"#
                    .to_owned()
            ),
            Sql::Integer(80),
            Sql::Integer(0),
            Sql::Integer(2),
            Sql::Integer(50),
            Sql::Null,
            Sql::Integer(4_294_967_295),
            Sql::Null,
        ]]
    );
    repository
        .atomic(false, &mut |transaction| {
            transaction.update_request_usage("r1", &json!({"request_id": "r1", "b": 1, "a": 2}))
        })
        .unwrap();
    assert_eq!(
        raw_rows(&database, "SELECT payload, tokens FROM request_usage"),
        [vec![
            Sql::Text(r#"{"a":2,"b":1,"request_id":"r1"}"#.to_owned()),
            Sql::Integer(80),
        ]]
    );
}

/// The count the 10,000-row cap reads counts every row, including rows
/// only a file edited outside the board wrote; a row's actor is read as
/// stored (the column's TEXT affinity keeps the number 7 as `'7'`).
#[test]
fn the_ledger_count_reads_every_row() {
    let (_dir, database, repository) = usage_board();
    repository
        .atomic(false, &mut |transaction| {
            transaction.usage_report().map(|_| ())
        })
        .unwrap();
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    let filling = connection.transaction().unwrap();
    for index in 0..10_000 {
        filling
            .execute(
                "INSERT INTO request_usage VALUES(?,7,'{}',0,0,1,NULL,NULL,NULL,NULL)",
                [format!("fill-{index}")],
            )
            .unwrap();
    }
    filling.commit().unwrap();
    repository
        .atomic(false, &mut |transaction| {
            assert_eq!(transaction.request_usage_count()?, 10_000);
            let stored = transaction.request_usage("fill-9999")?.unwrap();
            assert_eq!((stored.actor, stored.payload), (json!("7"), json!({})));
            Ok(())
        })
        .unwrap();
}
