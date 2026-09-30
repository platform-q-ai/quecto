use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{
    BoardStore, CREATE_FLAGS, OPEN_FLAGS, StoreRefusal, TransactionError, Undecodable, absolutised,
    file_uri, opening_message, secure_delete_checked, sqlite_message, variable_limit_checked,
};
use crate::domain::swarm::{BoardError, RefusalKind};
use crate::infrastructure::persistence::swarm_board::binding::bound_statement;
use crate::infrastructure::persistence::swarm_board::ledger::event;
use crate::infrastructure::persistence::swarm_board::py_json::{self, PyJson};
use crate::infrastructure::persistence::swarm_board::schema::SCHEMA;

const SCHEMA_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/schema_after_create.json"
));
/// The SHA-256 and length of `swarm_store.py`'s `SCHEMA` literal, frozen
/// when #2283 deleted the Python board (until then this test compared the
/// two texts).
const PYTHON_SCHEMA_SHA256: &str =
    "d5a6fec2c48102683d3c3894772d8b5ffe19a5e93f0484d7a53d4bfbbd5f9df3";
const PYTHON_SCHEMA_LENGTH: usize = 1259;

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
    use sha2::{Digest, Sha256};
    assert_eq!(SCHEMA.len(), PYTHON_SCHEMA_LENGTH);
    let digest: String = Sha256::digest(SCHEMA.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(digest, PYTHON_SCHEMA_SHA256);
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
        Err(TransactionError::Board(BoardError::new(
            RefusalKind::WrongState,
            "task 3 is already claimed",
        )))
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

#[test]
fn only_an_undecodable_conversion_failure_reads_as_pythons_decode_error() {
    let undecodable = rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(Undecodable::new("title", b"a\xffb")),
    );
    assert_eq!(
        sqlite_message(&undecodable),
        "Could not decode to UTF-8 column 'title' with text 'a\u{FFFD}b'",
        "a non-UTF-8 TEXT value: Python's decode error alone"
    );
    let other_cause = rusqlite::Error::FromSqlConversionFailure(
        2,
        rusqlite::types::Type::Integer,
        Box::new(std::fmt::Error),
    );
    assert_eq!(
        sqlite_message(&other_cause),
        other_cause.to_string(),
        "any other conversion failure: rusqlite's own text"
    );
    assert_ne!(
        sqlite_message(&other_cause),
        std::fmt::Error.to_string(),
        "not the bare cause"
    );
}

/// The body's SQL error as the transaction reports it, for a statement
/// prepared and bound the way board SQL is.
fn failing(store: &BoardStore, sql: &str, parameters: &[i64]) -> Result<(), StoreRefusal> {
    let values: Vec<rusqlite::types::Value> = parameters
        .iter()
        .map(|&parameter| rusqlite::types::Value::Integer(parameter))
        .collect();
    store.transaction(false, |tx| {
        let mut statement = bound_statement(tx, sql, &values)?;
        statement.raw_query().next()?;
        Ok(())
    })
}

#[test]
fn sql_input_and_binding_errors_read_as_python_str_of_the_sqlite3_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = created(&dir);
    // Each text is Python's str() of the sqlite3.Error the same call raises.
    let cases: [(&str, &[i64], &str); 8] = [
        ("SELEC 1", &[], "near \"SELEC\": syntax error"),
        ("SELECT 1,", &[], "incomplete input"),
        ("SELECT * FROM nope", &[], "no such table: nope"),
        (
            "SELECT ?",
            &[1, 2],
            "Incorrect number of bindings supplied. The current statement uses 1, and there are 2 supplied.",
        ),
        // Python counts every surplus parameter; rusqlite's own check stops
        // at the first.
        (
            "SELECT ?",
            &[1, 2, 3],
            "Incorrect number of bindings supplied. The current statement uses 1, and there are 3 supplied.",
        ),
        (
            "SELECT 1",
            &[1, 2, 3],
            "Incorrect number of bindings supplied. The current statement uses 0, and there are 3 supplied.",
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
    let secure_delete = store
        .transaction(false, |tx| {
            Ok(tx.query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0))?)
        })
        .expect("secure_delete reads");
    assert_eq!(
        secure_delete, 1,
        "freed pages are zeroed, as the system library does"
    );
    assert_eq!(query(32_767), Ok(1));
    assert_eq!(query(250_000), Ok(1));
    assert_eq!(
        query(250_001),
        Err(StoreRefusal(
            "coordination store unavailable or contended: too many SQL variables".into()
        ))
    );
}

#[test]
fn a_relative_path_under_a_deleted_working_directory_is_refused_as_unavailable() {
    // Permitted divergence (L4): pathlib's absolute() raises FileNotFoundError
    // from os.getcwd(), which escapes Python's store as a non-SwarmError; the
    // Rust store returns it as the unavailable refusal. The working directory
    // is process-wide, so the test supplies getcwd's failure instead.
    let deleted = || Err(std::io::Error::from_raw_os_error(2));
    assert_eq!(
        absolutised(Path::new("swarm.sqlite"), deleted),
        Err(StoreRefusal(
            "coordination store unavailable or contended: No such file or directory (os error 2)"
                .into()
        ))
    );
    let unused = || -> std::io::Result<PathBuf> { panic!("an absolute path needs no cwd") };
    assert_eq!(
        absolutised(Path::new("/b/./swarm.sqlite"), unused),
        Ok(PathBuf::from("/b/swarm.sqlite"))
    );
    assert_eq!(
        absolutised(Path::new("b/../swarm.sqlite"), || Ok(PathBuf::from("/w"))),
        Ok(PathBuf::from("/w/b/../swarm.sqlite")),
        "`..` is kept, as pathlib keeps it"
    );
}

#[test]
fn a_leading_double_slash_is_one_root() {
    // Permitted divergence (N3): pathlib keeps POSIX's implementation-defined
    // leading `//`, so Python opens `file:////h/x`; Rust's path components
    // fold it to one root and open `file:///h/x`. Both name /h/x on Linux
    // and macOS.
    let path = absolutised(Path::new("//h/x"), || Ok(PathBuf::from("/w"))).expect("absolute");
    assert_eq!(path, PathBuf::from("/h/x"));
    assert_eq!(file_uri(&path), "file:///h/x");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_missing_path_is_named_with_replacement_characters() {
    // Permitted divergence (N1): Python formats the path with
    // surrogateescape ('\udcff'); a Rust message is UTF-8, so the byte
    // reads as U+FFFD.
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir
        .path()
        .join(std::ffi::OsStr::from_bytes(b"board\xff.sqlite"));
    assert_eq!(
        BoardStore::new(&missing).transaction(false, |_| Ok(())),
        Err(StoreRefusal(format!(
            "coordination store missing at {}/board\u{fffd}.sqlite: it was deleted while the run was live, so this run's board is lost",
            dir.path().display()
        )))
    );
}

#[test]
fn a_store_refuses_a_sqlite_built_without_the_system_variable_limit() {
    // LIBSQLITE3_FLAGS in .cargo/config.toml raises the bundled limit to the
    // system library's; a build that lost it refuses rather than diverges.
    assert_eq!(
        variable_limit_checked(32_766),
        Err(StoreRefusal(
            "coordination store refused: this build's SQLite binds at most 32766 SQL variables, not the system library's 250000 (build it with LIBSQLITE3_FLAGS from .cargo/config.toml)"
                .into()
        ))
    );
    assert_eq!(variable_limit_checked(250_000), Ok(()));
    assert_eq!(variable_limit_checked(500_000), Ok(()));
}

#[test]
fn a_store_refuses_a_sqlite_built_without_secure_delete() {
    // An exported LIBSQLITE3_FLAGS can keep the variable limit and drop
    // -DSQLITE_SECURE_DELETE; the board then refuses rather than leave
    // deleted content in freed pages.
    let refusal = |value: i64| {
        Err(StoreRefusal(format!(
            "coordination store refused: this build's SQLite has secure_delete={value}, not the system library's 1 (build it with LIBSQLITE3_FLAGS from .cargo/config.toml)"
        )))
    };
    assert_eq!(secure_delete_checked(0), refusal(0));
    assert_eq!(secure_delete_checked(2), refusal(2));
    assert_eq!(secure_delete_checked(1), Ok(()));
}

#[test]
fn a_board_opens_with_exactly_python_s_uri_read_write_flags() {
    // sqlite3.connect(uri, uri=True) with mode=rw[c]: SQLITE_OPEN_URI (0x40)
    // and SQLITE_OPEN_READWRITE (0x02), plus SQLITE_OPEN_CREATE (0x04) to
    // create; nothing else (no mutex or cache flag).
    assert_eq!(OPEN_FLAGS.bits(), 0x40 | 0x02);
    assert_eq!(CREATE_FLAGS.bits(), 0x40 | 0x02 | 0x04);
}

#[cfg(unix)]
#[test]
fn a_non_utf8_path_s_uri_percent_encodes_each_byte() {
    use std::os::unix::ffi::OsStrExt;
    // Python: Path(os.fsdecode(b'/b\xff\xfe/s.sqlite')).as_uri()
    assert_eq!(
        file_uri(Path::new(std::ffi::OsStr::from_bytes(
            b"/b\xff\xfe/s.sqlite"
        ))),
        "file:///b%FF%FE/s.sqlite"
    );
}
