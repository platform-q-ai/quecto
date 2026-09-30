use rusqlite::Connection;

use super::{KeptSums, UsageSums, summed};
use crate::application::swarm::dto::NewRequestUsage;
use crate::domain::swarm::{BoardError, RefusalKind};
use crate::infrastructure::persistence::swarm_board::repository::atomic_on;
use crate::infrastructure::persistence::swarm_board::store::BoardStore;

const LEDGER: &str = "CREATE TABLE request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER)";

/// A board of run `run` whose ledger holds `rows` requests, the i-th
/// (from 1) of `i` tokens, the even ones unmeasured.
fn board(run: &str, rows: i64) -> (tempfile::TempDir, BoardStore) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = BoardStore::new(dir.path().join("swarm.sqlite"));
    store.transaction(true, |_| Ok(())).unwrap();
    let connection = Connection::open(store.path()).unwrap();
    connection.execute_batch(LEDGER).unwrap();
    connection
        .execute("INSERT INTO run(id) VALUES(?)", [run])
        .unwrap();
    for row in 1..=rows {
        insert(
            &connection,
            &format!("r{row}"),
            &row.to_string(),
            row % 2 == 0,
        );
    }
    (dir, store)
}

fn insert(connection: &Connection, request_id: &str, tokens: &str, unknown: bool) {
    connection
        .execute_batch(&format!(
            "INSERT INTO request_usage(request_id, actor, payload, tokens, unknown) VALUES('{request_id}', 'a', '{{}}', {tokens}, {})",
            i64::from(unknown)
        ))
        .unwrap();
}

fn sums_on(store: &BoardStore, known: Option<UsageSums>) -> Option<UsageSums> {
    let connection = Connection::open(store.path()).unwrap();
    summed(&connection, &KeptSums::default(), known).unwrap()
}

/// `count(*)`, `sum(tokens)` and `sum(unknown)` by the SQL.
fn by_sql(store: &BoardStore) -> (i64, i64, i64) {
    Connection::open(store.path())
        .unwrap()
        .query_row(
            "SELECT count(*), coalesce(sum(tokens),0), coalesce(sum(unknown),0) FROM request_usage",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn totals(sums: &UsageSums) -> (i64, i64, i64) {
    (sums.rows, sums.observed_tokens, sums.unknown_usage_requests)
}

/// Summed from the first row, the sums are the SQL's; an empty ledger's
/// are zero, with no last row.
#[test]
fn sums_from_the_first_row_are_the_sqls() {
    let (_empty_dir, empty) = board("run-1", 0);
    let none = sums_on(&empty, None).unwrap();
    assert_eq!((totals(&none), none.last), ((0, 0, 0), None));
    let (_dir, store) = board("run-1", 7);
    let sums = sums_on(&store, None).unwrap();
    assert_eq!(totals(&sums), by_sql(&store));
    assert_eq!(sums.last, Some((7, "r7".to_owned())));
}

/// Kept sums still this board's are extended by the later rows only:
/// what they hold is not summed again.
#[test]
fn kept_sums_are_extended_by_the_later_rows_only() {
    let (_dir, store) = board("run-1", 3);
    let kept = UsageSums {
        observed_tokens: 1_000,
        ..sums_on(&store, None).unwrap()
    };
    let connection = Connection::open(store.path()).unwrap();
    insert(&connection, "r4", "40", true);
    let sums = sums_on(&store, Some(kept)).unwrap();
    assert_eq!(totals(&sums), (4, 1_040, 2));
    assert_eq!(sums.last, Some((4, "r4".to_owned())));
}

/// Sums kept for another run, or whose last row no longer holds its
/// request id (the board recreated, or an edit), are summed afresh.
#[test]
fn sums_of_another_run_or_a_changed_last_row_are_summed_afresh() {
    let (_dir, store) = board("run-1", 3);
    let skewed = |run: &str, last: (i64, &str)| UsageSums {
        run_id: run.to_owned(),
        last: Some((last.0, last.1.to_owned())),
        rows: 50,
        observed_tokens: 5_000,
        unknown_usage_requests: 50,
    };
    for kept in [
        skewed("run-2", (3, "r3")),
        skewed("run-1", (3, "r9")),
        skewed("run-1", (9, "r9")),
    ] {
        let sums = sums_on(&store, Some(kept.clone())).unwrap();
        assert_eq!(totals(&sums), by_sql(&store), "{kept:?}");
    }
}

/// A count only an edit writes (a REAL, a text) or a request id that is
/// not text leaves the summing to the SQL, as does an overflow, which
/// SQLite's `sum` raises at the same row; so does a board without a run.
#[test]
fn values_only_an_edit_writes_leave_the_sums_to_the_sql() {
    for edit in [
        "INSERT INTO request_usage(request_id, tokens) VALUES('e', 1.5)",
        "INSERT INTO request_usage(request_id, tokens) VALUES('e', 'many')",
        "INSERT INTO request_usage(request_id, unknown) VALUES('e', 0.5)",
        "INSERT INTO request_usage(request_id, tokens) VALUES(x'37', 1)",
        "INSERT INTO request_usage(request_id, tokens) VALUES(CAST(x'ff' AS TEXT), 1)",
        "INSERT INTO request_usage(request_id, tokens) VALUES('e', 9223372036854775807)",
    ] {
        let (_dir, store) = board("run-1", 2);
        let kept = sums_on(&store, None);
        Connection::open(store.path())
            .unwrap()
            .execute_batch(edit)
            .unwrap();
        assert_eq!(sums_on(&store, kept.clone()), None, "{edit} after kept");
        assert_eq!(sums_on(&store, None), None, "{edit}");
    }
    let (_dir, store) = board("run-1", 2);
    Connection::open(store.path())
        .unwrap()
        .execute_batch("DELETE FROM run")
        .unwrap();
    assert_eq!(sums_on(&store, None), None, "no run");
}

/// Sums are kept only once the transaction that computed them commits:
/// a refused one keeps nothing, even after inserting a row; a committed
/// one keeps its own, including the row it inserted.
#[test]
fn sums_are_kept_only_after_their_transaction_commits() {
    let (_dir, store) = board("run-1", 2);
    let kept = KeptSums::default();
    let request = |request_id: &str| NewRequestUsage {
        request_id: request_id.to_owned(),
        actor: "a".to_owned(),
        payload: "{}".to_owned(),
        tokens: 10,
        unknown: 0,
        attempts: 1,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let refused = atomic_on(&store, &kept, None, false, &mut |transaction| {
        transaction.insert_request_usage(&request("gone"))?;
        assert_eq!(transaction.request_usage_count()?, 3);
        Err(BoardError::new(RefusalKind::Invalid, "refused"))
    });
    assert!(refused.is_err());
    assert_eq!(kept.kept(), None, "a refused transaction keeps nothing");
    atomic_on(&store, &kept, None, false, &mut |transaction| {
        assert_eq!(
            transaction.request_usage_count()?,
            2,
            "the refused row is gone"
        );
        transaction.insert_request_usage(&request("r3"))?;
        let standing = transaction.usage_standing()?;
        assert_eq!(standing.observed_tokens, serde_json::json!(13));
        Ok(())
    })
    .unwrap();
    let committed = kept.kept().expect("the committed sums are kept");
    assert_eq!(totals(&committed), (3, 13, 1));
    assert_eq!(totals(&committed), by_sql(&store));
}
