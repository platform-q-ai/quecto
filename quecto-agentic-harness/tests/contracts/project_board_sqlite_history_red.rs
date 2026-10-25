//! B3 requirements reachable through the merged B2 store interface.
//!
//! These are deliberately partial: no history transfer, prepared import,
//! epoch-opening or export port exists on the pinned baseline. In particular,
//! an unknown `board-server import` action is not history acceptance evidence.
use super::project_board_store::task;
use quecto::application::project_board::ports::{
    BoardArtifact, BoardBootstrap, BoardHead, BoardStoreError, BoardWrite, ProjectBoardStore,
    SetAsideClear,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::{
    project_board::artifacts::file_of, sqlite::project_board::SqliteProjectBoardStore,
};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn identity() -> Identity {
    Identity {
        name: "History writer".into(),
        email: "history@example.com".into(),
    }
}

struct Database {
    _directory: TempDir,
    path: PathBuf,
    store: SqliteProjectBoardStore,
}

impl Database {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("board.sqlite");
        let store = SqliteProjectBoardStore::open(&path, identity()).unwrap();
        Self {
            _directory: directory,
            path,
            store,
        }
    }
}

fn written(result: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match result.expect("valid native write reaches persistence") {
        BoardWrite::Written(head) => head,
        other => panic!("native write was not published: {other:?}"),
    }
}

#[test]
fn bootstrap_head_is_the_first_epoch_revision_and_survives_reopen() {
    let database = Database::new();
    assert_eq!(database.store.read(), Err(BoardStoreError::NoBoard));
    let created = database.store.bootstrap().unwrap();
    let observed = database.store.read().unwrap().head;
    assert_eq!(created, BoardBootstrap::Created(observed.clone()));
    let reopened = SqliteProjectBoardStore::open(&database.path, identity()).unwrap();
    assert_eq!(reopened.read().unwrap().head, observed);
    assert_eq!(
        reopened.bootstrap(),
        Ok(BoardBootstrap::Existing(observed.clone()))
    );
    assert_eq!(
        observed,
        BoardHead("1:1".into()),
        "B3 native SQL heads are epoch:revision, not Git or framed object IDs"
    );
}

#[test]
fn unchanged_native_writes_are_separate_contiguous_revisions() {
    let database = Database::new();
    database.store.bootstrap().unwrap();
    let initial = database.store.read().unwrap().head;
    let value = task("a", "Unchanged bytes still advance history");
    let first = written(database.store.write_tasks(
        &initial,
        std::slice::from_ref(&value),
        "board: first",
    ));
    let second = written(database.store.write_tasks(
        &first,
        std::slice::from_ref(&value),
        "board: unchanged child",
    ));
    let reopened = SqliteProjectBoardStore::open(&database.path, identity()).unwrap();
    let snapshot = reopened.read().unwrap();
    assert_eq!(snapshot.tasks, vec![value]);
    assert_eq!(snapshot.head, second);
    assert_ne!(first, second, "an unchanged write remains a distinct child");
    assert_eq!(
        (first, second),
        (BoardHead("1:2".into()), BoardHead("1:3".into())),
        "successful native revisions are ordered and contiguous"
    );
}

#[test]
fn lost_refused_and_invalid_writes_do_not_consume_a_native_revision() {
    let database = Database::new();
    database.store.bootstrap().unwrap();
    let initial = database.store.read().unwrap().head;
    let value = task("a", "Retained after unsuccessful writes");
    let first = written(database.store.write_tasks(
        &initial,
        std::slice::from_ref(&value),
        "board: first",
    ));
    let before = database.store.fixture_fingerprint().unwrap();
    assert_eq!(
        database
            .store
            .write_tasks(&initial, std::slice::from_ref(&value), "board: stale"),
        Ok(BoardWrite::Lost)
    );
    assert!(matches!(
        database.store.write_tasks(&first, &[], "board: empty"),
        Err(BoardStoreError::InvalidWrite(_))
    ));
    assert!(matches!(
        database.store.write_tasks(
            &BoardHead("1:999".into()),
            std::slice::from_ref(&value),
            "board: future"
        ),
        Err(BoardStoreError::InvalidWrite(_))
    ));
    database.store.fixture_refuse_next_write().unwrap();
    assert!(matches!(
        database
            .store
            .write_tasks(&first, std::slice::from_ref(&value), "board: refused"),
        Ok(BoardWrite::Refused(_))
    ));
    assert_eq!(database.store.fixture_fingerprint().unwrap(), before);
    let next = written(database.store.write_tasks(&first, &[value], "board: next"));
    assert_eq!(
        next,
        BoardHead("1:3".into()),
        "only the two successful writes consume revisions after bootstrap"
    );
}

#[test]
fn clearing_a_set_aside_file_advances_the_same_native_sequence() {
    let database = Database::new();
    database.store.bootstrap().unwrap();
    database.store.fixture_put_raw("tasks/b.md", b"{").unwrap();
    let before = database.store.read().unwrap();
    assert_eq!(before.quarantined.len(), 1);
    let file = &before.quarantined[0];
    let clear = SetAsideClear {
        path: file.path.clone(),
        object: file.object.clone(),
        repeated: Default::default(),
    };
    let next = written(database.store.clear_set_aside(
        &before.head,
        &clear,
        "board: clear broken task",
    ));
    let reopened = SqliteProjectBoardStore::open(&database.path, identity()).unwrap();
    let snapshot = reopened.read().unwrap();
    assert!(snapshot.quarantined.is_empty());
    assert_eq!(snapshot.head, next);
    assert_eq!(
        next,
        BoardHead("1:3".into()),
        "clear and structural fixture revisions share the native lineage sequence"
    );
}

// Frozen merged B2 schema, not the schema generated by the implementation under
// test. A future schema3 implementation must refuse this old file, not migrate it.
const SCHEMA2: &str = "
CREATE TABLE objects (oid TEXT PRIMARY KEY NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('blob','tree')), bytes BLOB, tree_size INTEGER, CHECK((kind='blob' AND typeof(bytes)='blob' AND tree_size IS NULL) OR (kind='tree' AND bytes IS NULL AND typeof(tree_size)='integer' AND tree_size>=0)));
CREATE TABLE tree_entries (tree TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=0), mode TEXT NOT NULL, raw_name BLOB NOT NULL CHECK(typeof(raw_name)='blob'), oid TEXT NOT NULL, PRIMARY KEY(tree,ordinal));
CREATE TABLE revisions (head TEXT PRIMARY KEY NOT NULL, root TEXT NOT NULL, name TEXT NOT NULL, email TEXT NOT NULL);
CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), head TEXT NOT NULL);
PRAGMA user_version=2;
PRAGMA application_id=0x51424f32;
";

fn schema2_file(path: &Path) {
    let raw = Connection::open(path).unwrap();
    raw.execute_batch(SCHEMA2).unwrap();
    let bytes = b"old schema2 unreachable bytes\0\xff";
    raw.execute(
        "INSERT INTO objects VALUES (?1,'blob',?2,NULL)",
        params![
            SqliteProjectBoardStore::fixture_blob_id(bytes),
            bytes.as_slice()
        ],
    )
    .unwrap();
    let integrity: String = raw
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let stored: Vec<u8> = raw
        .query_row("SELECT bytes FROM objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, bytes);
    let version: i64 = raw
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let application: i64 = raw
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .unwrap();
    assert_eq!((version, application), (2, 0x51424f32));
    raw.close().unwrap();
}

#[test]
fn genuine_schema2_is_refused_before_data_or_journal_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.sqlite");
    schema2_file(&path);
    let before = std::fs::read(&path).unwrap();
    let result = SqliteProjectBoardStore::open(&path, identity());
    let refused = matches!(&result, Err(BoardStoreError::Clash(_)));
    drop(result);
    assert!(
        refused,
        "B3 must refuse the complete recognized old schema2, not open/migrate/reset it"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "old file is byte-exact on refusal"
    );
    let files: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        files,
        vec![std::ffi::OsString::from("old.sqlite")],
        "schema refusal must not create WAL/SHM/journal files"
    );
}

fn collision_write(kind: &str) {
    let database = Database::new();
    database.store.bootstrap().unwrap();
    let artifact = BoardArtifact::Task(task("a", "A full equality check is required"));
    let canonical = file_of(&artifact).unwrap();
    let oid = SqliteProjectBoardStore::fixture_blob_id(&canonical);
    let raw = Connection::open(&database.path).unwrap();
    match kind {
        "blob" => {
            // A kind-and-length-only guard is insufficient: the conflicting
            // object differs in one byte, at exactly the canonical length.
            let mut conflicting = canonical.clone();
            assert!(conflicting.len() > 1);
            let last = conflicting.len() - 1;
            conflicting[last] ^= 1;
            assert_eq!(conflicting.len(), canonical.len());
            assert_ne!(conflicting, canonical);
            raw.execute(
                "INSERT INTO objects VALUES (?1,'blob',?2,NULL)",
                params![oid, conflicting],
            )
            .unwrap()
        }
        "tree" => raw
            .execute("INSERT INTO objects VALUES (?1,'tree',NULL,0)", [&oid])
            .unwrap(),
        _ => unreachable!("fixture kind allowlist"),
    };
    raw.close().unwrap();
    // The conflict is unreachable: ordinary read and CAS admission are healthy.
    let snapshot = database.store.read().unwrap();
    assert!(snapshot.tasks.is_empty());
    assert!(snapshot.quarantined.is_empty());
    let before = database.store.fixture_fingerprint().unwrap();
    let result = database
        .store
        .write(&snapshot.head, &[artifact], "board: conflicting OID");
    assert!(
        matches!(
            &result,
            Err(BoardStoreError::Clash(_)
                | BoardStoreError::InvalidWrite(_)
                | BoardStoreError::Unavailable(_))
        ),
        "same OID with differing kind or full bytes must fail closed, not silently deduplicate: {result:?}"
    );
    assert_eq!(
        database.store.fixture_fingerprint().unwrap(),
        before,
        "collision refusal rolls back every row including unreachable objects"
    );
    assert_eq!(database.store.read().unwrap(), snapshot);
}

#[test]
fn same_oid_different_blob_bytes_refuses_without_publishing() {
    // Handcrafted conflicting storage row, not a generated SHA256 collision.
    collision_write("blob");
}

#[test]
fn same_oid_different_actual_kind_refuses_without_publishing() {
    collision_write("tree");
}

#[test]
fn same_oid_same_kind_and_full_bytes_is_a_valid_deduplication() {
    let database = Database::new();
    database.store.bootstrap().unwrap();
    let value = task("a", "An exact existing object is reusable");
    let artifact = BoardArtifact::Task(value.clone());
    let canonical = file_of(&artifact).unwrap();
    let oid = SqliteProjectBoardStore::fixture_blob_id(&canonical);
    let raw = Connection::open(&database.path).unwrap();
    raw.execute(
        "INSERT INTO objects VALUES (?1,'blob',?2,NULL)",
        params![oid, canonical],
    )
    .unwrap();
    raw.close().unwrap();
    let before = database.store.read().unwrap();
    let next = written(
        database
            .store
            .write(&before.head, &[artifact], "board: exact dedup"),
    );
    let snapshot = database.store.read().unwrap();
    assert_eq!(snapshot.tasks, vec![value]);
    assert!(snapshot.quarantined.is_empty());
    assert_eq!(snapshot.head, next);
    let raw = Connection::open(&database.path).unwrap();
    let stored: Vec<u8> = raw
        .query_row("SELECT bytes FROM objects WHERE oid=?1", [&oid], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        stored, canonical,
        "exact full canonical bytes survive the same write route"
    );
}
