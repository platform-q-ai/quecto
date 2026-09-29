//! `BoardUsage` on the SQLite adapter (#2273): the usage report by
//! Python's SQL. The report creates `request_usage` and `usage_budget`
//! lazily, in Python's order and with its statements, and writes no
//! default budget row; the aggregates are keyed by their SQL aliases.
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
