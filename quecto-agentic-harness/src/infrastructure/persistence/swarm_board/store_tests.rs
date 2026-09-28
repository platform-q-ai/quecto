use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{
    BoardStore, StoreRefusal, TransactionError, event, file_uri, opening_message, retry,
    sqlite_message,
};
use crate::infrastructure::persistence::swarm_board::py_json::{self, PyJson};
use crate::infrastructure::persistence::swarm_board::schema::SCHEMA;

const SCHEMA_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/schema_after_create.json"
));
const PYTHON_STORE: &str = include_str!("../../tools/swarm_helpers/swarm_store.py");

type MasterRow = (String, String, String, Option<String>);

fn json(text: &str) -> PyJson {
    py_json::decode(text).expect("test JSON is valid")
}

fn board(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("swarm.sqlite")
}

fn created(dir: &tempfile::TempDir) -> BoardStore {
    let store = BoardStore::new(board(dir));
    store
        .transaction(true, |_| Ok(()))
        .expect("the board is created");
    store
}

/// A plain second connection, as another process would hold one.
fn other(path: &Path) -> Connection {
    let connection = Connection::open(path).expect("a second connection opens");
    connection
        .busy_timeout(Duration::from_millis(500))
        .expect("busy timeout set");
    connection
}

fn master(path: &Path) -> Vec<MasterRow> {
    let connection = other(path);
    let mut statement = connection
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY name")
        .expect("sqlite_master is readable");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("sqlite_master rows")
        .collect::<Result<_, _>>()
        .expect("sqlite_master rows read")
}

fn fixture() -> Vec<MasterRow> {
    serde_json::from_str(SCHEMA_FIXTURE)
        .expect("the schema fixture is [type, name, tbl_name, sql] rows")
}

fn count(path: &Path, table: &str) -> i64 {
    other(path)
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("the table is countable")
}

#[test]
fn the_schema_is_the_python_text_verbatim() {
    let start = PYTHON_STORE
        .find("SCHEMA = '''")
        .expect("swarm_store.py defines SCHEMA")
        + "SCHEMA = '''".len();
    let length = PYTHON_STORE[start..]
        .find("'''")
        .expect("SCHEMA's literal ends");
    assert_eq!(SCHEMA, &PYTHON_STORE[start..start + length]);
}

#[test]
fn a_missing_store_is_refused_with_the_2145_message_before_opening() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = board(&dir);
    let mut ran = false;
    let refused = BoardStore::new(&path).transaction(false, |_| {
        ran = true;
        Ok(())
    });
    assert_eq!(
        refused,
        Err(StoreRefusal(format!(
            "coordination store missing at {}: it was deleted while the run was live, so this run's board is lost",
            path.display()
        )))
    );
    assert!(!ran, "the body never runs");
    assert!(
        !path.exists(),
        "the refusal comes before opening, which would create nothing anyway"
    );
}

#[test]
fn a_relative_missing_store_is_named_by_its_absolute_path() {
    let relative = Path::new("missing-board-2269-never-created").join("swarm.sqlite");
    let absolute = std::path::absolute(&relative).expect("the working directory exists");
    let refused = BoardStore::new(&relative).transaction(false, |_| Ok(()));
    assert_eq!(
        refused.map_err(|refusal| refusal.0),
        Err(format!(
            "coordination store missing at {}: it was deleted while the run was live, so this run's board is lost",
            absolute.display()
        ))
    );
}

#[test]
fn create_writes_the_python_schema_text_exactly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    assert_eq!(master(store.path()), fixture());
    store
        .transaction(true, |_| Ok(()))
        .expect("create is idempotent");
    assert_eq!(
        master(store.path()),
        fixture(),
        "a second create changes nothing"
    );
}

#[test]
fn every_transaction_adds_missing_added_columns_to_an_older_store() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = board(&dir);
    other(&path)
        .execute_batch(
            "CREATE TABLE run (id TEXT PRIMARY KEY, goal TEXT, constraints TEXT, criteria TEXT,
 coordinator TEXT, integrator TEXT, member_limit INTEGER, deadline REAL, status TEXT);
CREATE TABLE members (id TEXT PRIMARY KEY, reservation TEXT UNIQUE, status TEXT,
 pid INTEGER, started TEXT, socket TEXT);
CREATE TABLE messages (id INTEGER PRIMARY KEY, sender TEXT, recipient TEXT, body TEXT, status TEXT);",
        )
        .expect("an older store is written");
    BoardStore::new(&path)
        .transaction(false, |_| Ok(()))
        .expect("an older store opens");
    let upgraded: Vec<MasterRow> = fixture()
        .into_iter()
        .filter(|row| row.0 == "table" && ["run", "members", "messages"].contains(&row.1.as_str()))
        .collect();
    let tables: Vec<MasterRow> = master(&path)
        .into_iter()
        .filter(|row| row.0 == "table")
        .collect();
    assert_eq!(
        tables, upgraded,
        "every added column, in Python's order and DDL text"
    );

    // A store upgraded before #1961 lacks only `launcher`.
    other(&path)
        .execute_batch("ALTER TABLE members DROP COLUMN launcher")
        .expect("launcher is dropped");
    BoardStore::new(&path)
        .transaction(false, |_| Ok(()))
        .expect("the store opens");
    let tables: Vec<MasterRow> = master(&path)
        .into_iter()
        .filter(|row| row.0 == "table")
        .collect();
    let members = tables
        .iter()
        .find(|row| row.1 == "members")
        .expect("members exists");
    assert!(
        members
            .3
            .as_deref()
            .is_some_and(|sql| sql.ends_with(", launcher TEXT)")),
        "launcher is added again: {members:?}"
    );
}

#[test]
fn transactions_begin_immediate_and_hold_the_write_lock() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let (elapsed, error) = store
        .transaction(false, |_| {
            let started = Instant::now();
            let error = other(store.path())
                .execute_batch("BEGIN IMMEDIATE")
                .expect_err("the write lock is held");
            Ok((started.elapsed(), error.to_string()))
        })
        .expect("the transaction commits");
    assert_eq!(error, "database is locked");
    assert!(
        elapsed >= Duration::from_millis(450),
        "waited the busy timeout: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "gave up after the busy timeout: {elapsed:?}"
    );
}

#[test]
fn a_contended_store_error_names_the_sqlite_message() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let holder = other(store.path());
    holder
        .execute_batch("BEGIN IMMEDIATE")
        .expect("the other connection holds the lock");
    let started = Instant::now();
    let mut ran = false;
    let refused = store.transaction(false, |_| {
        ran = true;
        Ok(())
    });
    let elapsed = started.elapsed();
    assert_eq!(
        refused,
        Err(StoreRefusal(
            "coordination store unavailable or contended: database is locked".into()
        ))
    );
    assert!(!ran, "the body never runs without the lock");
    assert!(
        elapsed >= Duration::from_millis(450),
        "waited the busy timeout: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "gave up after the busy timeout: {elapsed:?}"
    );
    holder
        .execute_batch("ROLLBACK")
        .expect("the lock is released");
    store
        .transaction(false, |_| Ok(()))
        .expect("the store is free again");
}

#[test]
fn a_store_that_cannot_be_opened_names_the_sqlite_message() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("no-such-directory").join("swarm.sqlite");
    let refused = BoardStore::new(&path).transaction(true, |_| Ok(()));
    assert_eq!(
        refused,
        Err(StoreRefusal(
            "coordination store unavailable or contended: unable to open database file".into()
        ))
    );
}

#[test]
fn a_board_error_rolls_back_and_is_returned_unchanged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let refused: Result<(), _> = store.transaction(false, |tx| {
        event(tx, "worker", 1.5, "claimed", &json("{}"))?;
        Err(TransactionError::board("task 3 is already claimed"))
    });
    assert_eq!(
        refused,
        Err(StoreRefusal("task 3 is already claimed".into()))
    );
    assert_eq!(
        count(store.path(), "events"),
        0,
        "the event before the refusal is rolled back"
    );

    let failed: Result<(), _> = store.transaction(false, |tx| {
        event(tx, "worker", 1.5, "claimed", &json("{}"))?;
        tx.execute("INSERT INTO no_such_table VALUES (1)", [])?;
        Ok(())
    });
    assert_eq!(
        failed,
        Err(StoreRefusal(
            "coordination store unavailable or contended: no such table: no_such_table".into()
        ))
    );
    assert_eq!(
        count(store.path(), "events"),
        0,
        "a SQLite error rolls back too"
    );

    store
        .transaction(false, |tx| event(tx, "worker", 1.5, "claimed", &json("{}")))
        .expect("a successful body commits");
    assert_eq!(count(store.path(), "events"), 1);
}

#[test]
fn journal_mode_stays_delete_and_foreign_keys_are_on() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let (journal, foreign_keys, busy, autocommit) = store
        .transaction(false, |tx| {
            let journal: String = tx.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            let foreign_keys: i64 = tx.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
            let busy: i64 = tx.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
            Ok((journal, foreign_keys, busy, tx.is_autocommit()))
        })
        .expect("pragmas read");
    assert_eq!(
        (journal.as_str(), foreign_keys, busy, autocommit),
        ("delete", 1, 500, false)
    );
    let persisted: String = other(store.path())
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode reads");
    assert_eq!(
        persisted, "delete",
        "the file keeps the default rollback journal"
    );
    let wal = PathBuf::from(format!("{}-wal", store.path().display()));
    assert!(!wal.exists(), "no WAL file is ever written");
}

#[test]
fn a_path_with_a_space_and_a_percent_is_opened_as_itself() {
    assert_eq!(
        file_uri(Path::new("/a b/c%d?e#f")),
        "file:///a%20b/c%25d%3Fe%23f"
    );
    assert_eq!(
        file_uri(Path::new("/x/\u{fc}~_.-!$&'()*+,;=:@[]")),
        "file:///x/%C3%BC~_.-%21%24%26%27%28%29%2A%2B%2C%3B%3D%3A%40%5B%5D"
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let folder = dir.path().join("a b%20c");
    std::fs::create_dir(&folder).expect("folder is created");
    let path = folder.join("board 100%.sqlite");
    BoardStore::new(&path)
        .transaction(true, |_| Ok(()))
        .expect("the board is created");
    assert!(path.exists(), "the file has the literal name");
    assert_eq!(
        std::fs::read_dir(&folder).expect("folder lists").count(),
        1,
        "and no other"
    );
    BoardStore::new(&path)
        .transaction(false, |_| Ok(()))
        .expect("and reopens");
}

#[test]
fn retry_replays_the_stored_result_and_refuses_a_different_payload() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let payload = json("{\"title\":\"t\",\"acceptance\":[\"pass\"]}");
    let mut actions = 0;
    let first = store
        .transaction(false, |tx| {
            retry(tx, "worker", "r1", &payload, || {
                actions += 1;
                Ok(json("{\"id\":1,\"title\":\"t\"}"))
            })
        })
        .expect("a new request runs");
    assert_eq!(first, json("{\"id\":1,\"title\":\"t\"}"));
    let stored: (String, String, String, String) = other(store.path())
        .query_row("SELECT * FROM requests", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("the request is recorded");
    assert_eq!(
        stored,
        (
            "worker".into(),
            "r1".into(),
            "{\"acceptance\":[\"pass\"],\"title\":\"t\"}".into(),
            "{\"id\":1,\"title\":\"t\"}".into()
        )
    );

    let replayed = store
        .transaction(false, |tx| {
            retry(tx, "worker", "r1", &payload, || {
                actions += 1;
                Ok(json("null"))
            })
        })
        .expect("a retry replays");
    assert_eq!((replayed, actions), (first, 1), "the action ran once");

    let reused = store.transaction(false, |tx| {
        retry(tx, "worker", "r1", &json("{\"title\":\"u\"}"), || {
            Ok(json("null"))
        })
    });
    assert_eq!(
        reused,
        Err(StoreRefusal(
            "request id reused with different payload".into()
        ))
    );
    let other_actor = store
        .transaction(false, |tx| {
            retry(tx, "coordinator", "r1", &json("{}"), || Ok(json("2")))
        })
        .expect("request ids are per actor");
    assert_eq!(other_actor, json("2"));

    for bad in [
        "",
        " \t\n",
        "\u{1c}\u{1f}\u{3000}",
        &"x".repeat(129),
        &"\u{e9}".repeat(65),
    ] {
        let refused = store.transaction(false, |tx| {
            retry(tx, "worker", bad, &json("{}"), || Ok(json("null")))
        });
        assert_eq!(
            refused,
            Err(StoreRefusal(
                "request id must be nonempty and at most 128 bytes".into()
            )),
            "{bad:?}"
        );
    }
    store
        .transaction(false, |tx| {
            retry(tx, "worker", &"\u{e9}".repeat(64), &json("{}"), || {
                Ok(json("null"))
            })
        })
        .expect("128 bytes is allowed");
}

#[test]
fn retry_refuses_when_the_ledger_holds_10000_requests() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    store
        .transaction(false, |tx| {
            let mut insert =
                tx.prepare("INSERT INTO requests VALUES('filler', ?, '{}', 'null')")?;
            for index in 0..9_999 {
                insert.execute([index.to_string()])?;
            }
            Ok(())
        })
        .expect("9999 requests are recorded");
    store
        .transaction(false, |tx| {
            retry(tx, "worker", "last", &json("{}"), || Ok(json("1")))
        })
        .expect("the 10000th request fits");
    let mut ran = false;
    let full = store.transaction(false, |tx| {
        retry(tx, "worker", "one-more", &json("{}"), || {
            ran = true;
            Ok(json("1"))
        })
    });
    assert_eq!(
        full,
        Err(StoreRefusal(
            "coordination request ledger full (10000)".into()
        ))
    );
    assert!(!ran, "a full ledger refuses before the action");
    let replayed = store
        .transaction(false, |tx| {
            retry(tx, "worker", "last", &json("{}"), || Ok(json("2")))
        })
        .expect("a recorded request still replays");
    assert_eq!(replayed, json("1"));
}

#[test]
fn event_records_the_actor_the_clock_and_the_encoded_detail() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    store
        .transaction(false, |tx| {
            event(
                tx,
                "worker",
                1_700_000_000.25,
                "claimed",
                &json("{\"task\":3,\"b\":[1.0,\"\u{e9}\"]}"),
            )
        })
        .expect("the event is recorded");
    let row: (i64, String, f64, String, String, String) = other(store.path())
        .query_row(
            "SELECT id, actor, time, action, detail, typeof(time) FROM events",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("the event reads");
    assert_eq!(
        row,
        (
            1,
            "worker".into(),
            1_700_000_000.25,
            "claimed".into(),
            "{\"b\":[1.0,\"\\u00e9\"],\"task\":3}".into(),
            "real".into()
        )
    );
}

#[test]
fn sqlite_errors_read_as_python_str_of_the_sqlite3_error() {
    let failure = |code| rusqlite::ffi::Error::new(code);
    let busy = rusqlite::Error::SqliteFailure(failure(rusqlite::ffi::SQLITE_BUSY), None);
    assert_eq!(
        sqlite_message(&busy),
        "database is locked",
        "no recorded message: the code's text"
    );
    let recorded = rusqlite::Error::SqliteFailure(
        failure(rusqlite::ffi::SQLITE_ERROR),
        Some("no such table: tasks".into()),
    );
    assert_eq!(sqlite_message(&recorded), "no such table: tasks");

    let uri = "file:///board.sqlite?mode=rw";
    let cannot_open = rusqlite::Error::SqliteFailure(
        failure(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(format!("unable to open database file: {uri}")),
    );
    assert_eq!(
        opening_message(&cannot_open, uri),
        "unable to open database file"
    );
    let no_handle =
        rusqlite::Error::SqliteFailure(failure(rusqlite::ffi::SQLITE_NOMEM), Some(uri.into()));
    assert_eq!(opening_message(&no_handle, uri), "out of memory");
    let other = rusqlite::Error::SqliteFailure(
        failure(rusqlite::ffi::SQLITE_ERROR),
        Some("no such access mode: rx".into()),
    );
    assert_eq!(opening_message(&other, uri), "no such access mode: rx");
}

/// The body's SQL error as the transaction reports it.
fn failing(store: &BoardStore, sql: &str, parameters: &[i64]) -> Result<(), StoreRefusal> {
    store.transaction(false, |tx| {
        tx.execute(sql, rusqlite::params_from_iter(parameters))?;
        Ok(())
    })
}

#[test]
fn sql_input_and_binding_errors_read_as_python_str_of_the_sqlite3_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    // Each text is Python's str() of the sqlite3.Error the same call raises.
    let cases: [(&str, &[i64], &str); 6] = [
        ("SELEC 1", &[], "near \"SELEC\": syntax error"),
        ("SELECT 1,", &[], "incomplete input"),
        ("SELECT * FROM nope", &[], "no such table: nope"),
        (
            "SELECT ?",
            &[1, 2],
            "Incorrect number of bindings supplied. The current statement uses 1, and there are 2 supplied.",
        ),
        (
            "SELECT ?, ?",
            &[1],
            "Incorrect number of bindings supplied. The current statement uses 2, and there are 1 supplied.",
        ),
        (
            "SELECT 1; SELECT 2",
            &[],
            "You can only execute one statement at a time.",
        ),
    ];
    for (sql, parameters, message) in cases {
        assert_eq!(
            failing(&store, sql, parameters),
            Err(StoreRefusal(format!(
                "coordination store unavailable or contended: {message}"
            ))),
            "{sql}"
        );
    }
}

#[test]
fn a_path_is_absolutised_as_pathlib_absolute_does() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    // A trailing separator names the same file, as pathlib normalises it.
    let trailing = PathBuf::from(format!("{}/", store.path().display()));
    BoardStore::new(&trailing)
        .transaction(false, |tx| event(tx, "worker", 1.0, "seen", &json("{}")))
        .expect("the board opens through a trailing separator");
    assert_eq!(count(store.path(), "events"), 1, "the same file");
    let missing = dir.path().join("missing.sqlite");
    assert_eq!(
        BoardStore::new(format!("{}//", missing.display())).transaction(false, |_| Ok(())),
        Err(StoreRefusal(format!(
            "coordination store missing at {}: it was deleted while the run was live, so this run's board is lost",
            missing.display()
        )))
    );
    // An empty path is the working directory, which exists and is no database.
    assert_eq!(
        BoardStore::new("").transaction(false, |_| Ok(())),
        Err(StoreRefusal(
            "coordination store unavailable or contended: unable to open database file".into()
        ))
    );
}

#[test]
fn the_bundled_sqlite_binds_as_many_parameters_as_the_system_library() {
    // Debian's and Arch's libsqlite3, which Python's sqlite3 uses, are built
    // with SQLITE_MAX_VARIABLE_NUMBER=250000 (the bundled default is 32766).
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    let query = |count: usize| {
        let marks = vec!["?"; count].join(",");
        let ids: Vec<i64> = (0..count as i64).collect();
        store.transaction(false, |tx| {
            let matched: i64 = tx.query_row(
                &format!("SELECT count(*) FROM (SELECT 7 AS id) WHERE id IN ({marks})"),
                rusqlite::params_from_iter(ids.iter()),
                |row| row.get(0),
            )?;
            Ok(matched)
        })
    };
    assert_eq!(query(32_767), Ok(1));
    assert_eq!(query(250_000), Ok(1));
    assert_eq!(
        query(250_001),
        Err(StoreRefusal(
            "coordination store unavailable or contended: too many SQL variables".into()
        ))
    );
}

/// Records request `r` for `worker` with payload `{}` and the raw `result`.
fn stored_request(
    store: &BoardStore,
    payload: rusqlite::types::Value,
    result: rusqlite::types::Value,
) {
    store
        .transaction(false, |tx| {
            tx.execute(
                "INSERT INTO requests VALUES('worker', 'r', ?, ?)",
                rusqlite::params![payload, result],
            )?;
            Ok(())
        })
        .expect("the request is recorded");
}

fn replay(store: &BoardStore) -> Result<PyJson, StoreRefusal> {
    store.transaction(false, |tx| {
        retry(tx, "worker", "r", &json("{}"), || Ok(json("null")))
    })
}

#[test]
fn a_stored_result_is_read_as_json_loads_reads_the_column() {
    use rusqlite::types::Value;
    let payload = || Value::Text("{}".into());
    let cases: [(Value, Result<PyJson, StoreRefusal>); 6] = [
        (Value::Blob(b"{\"id\":1}".to_vec()), Ok(json("{\"id\":1}"))),
        (Value::Blob(b"\xef\xbb\xbf[2]".to_vec()), Ok(json("[2]"))),
        (
            Value::Integer(3),
            Err(StoreRefusal(
                "the JSON object must be str, bytes or bytearray, not int".into(),
            )),
        ),
        (
            Value::Real(3.5),
            Err(StoreRefusal(
                "the JSON object must be str, bytes or bytearray, not float".into(),
            )),
        ),
        (
            Value::Null,
            Err(StoreRefusal(
                "the JSON object must be str, bytes or bytearray, not NoneType".into(),
            )),
        ),
        (
            Value::Text("{\"id\":".into()),
            Err(StoreRefusal(
                "Expecting value: line 1 column 7 (char 6)".into(),
            )),
        ),
    ];
    for (result, expected) in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = created(&dir);
        stored_request(&store, payload(), result.clone());
        assert_eq!(replay(&store), expected, "{result:?}");
    }
}

#[test]
fn an_undecodable_stored_text_is_python_s_decode_error() {
    use rusqlite::types::Value;
    let undecodable = || Value::Blob(b"\xffA".to_vec());
    let as_text = "CAST(? AS TEXT)";
    for (column, payload, result) in [("payload", as_text, "?"), ("result", "?", as_text)] {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = created(&dir);
        let (payload_value, result_value) = match column {
            "payload" => (undecodable(), Value::Text("null".into())),
            _ => (Value::Text("{}".into()), undecodable()),
        };
        store
            .transaction(false, |tx| {
                tx.execute(
                    &format!("INSERT INTO requests VALUES('worker', 'r', {payload}, {result})"),
                    rusqlite::params![payload_value, result_value],
                )?;
                Ok(())
            })
            .expect("the request is recorded");
        assert_eq!(
            replay(&store),
            Err(StoreRefusal(format!(
                "coordination store unavailable or contended: Could not decode to UTF-8 column '{column}' with text '\u{fffd}A'"
            ))),
            "{column}"
        );
    }
}
