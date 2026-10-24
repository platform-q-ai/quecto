//! Fresh, real-SQL regression red on the integrated, intentionally unfixed adapter.
//! Raw fixture construction is confined to the existing flat regular-file schema.
use super::project_board_store::task;
use quecto::application::project_board::ports::{
    BoardArtifact, BoardHead, BoardStoreError, ProjectBoardStore, SetAsideClear,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::{
    project_board::{artifacts::file_of, board_tree::MAX_TASK_FILE_BYTES},
    sqlite::project_board::SqliteProjectBoardStore,
};
use quecto::infrastructure::test_support::board_artifacts::run;
use rusqlite::{Connection, params, types::Value};
use sha2::{Digest, Sha256};

struct Fixture {
    _dir: tempfile::TempDir,
    raw: Connection,
    store: SqliteProjectBoardStore,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.sqlite");
        let store = SqliteProjectBoardStore::open(
            &path,
            Identity {
                name: "Regression".into(),
                email: "regression@example.com".into(),
            },
        )
        .unwrap();
        store.bootstrap().unwrap();
        Self {
            _dir: dir,
            raw: Connection::open(path).unwrap(),
            store,
        }
    }

    fn put_many(&mut self, files: impl IntoIterator<Item = (String, Vec<u8>)>) {
        let tx = self.raw.transaction().unwrap();
        for (path, bytes) in files {
            let object = format!("{:x}", Sha256::digest(&bytes));
            tx.execute(
                "INSERT INTO files VALUES (?1,?2,?3)",
                params![path, object, bytes],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    // Every physical user table, including rows unreachable from the current head.
    // No logical snapshot substitution: object IDs and BLOB values are included.
    fn fingerprint(&self) -> Vec<(String, Vec<Vec<Value>>)> {
        let names: Vec<String> = self
            .raw
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        names
            .into_iter()
            .map(|name| {
                let sql = format!(
                    "SELECT * FROM \"{}\" ORDER BY rowid",
                    name.replace('"', "\"\"")
                );
                let mut statement = self.raw.prepare(&sql).unwrap();
                let columns = statement.column_count();
                let rows = statement
                    .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
                    .unwrap()
                    .collect::<Result<Vec<Vec<Value>>, _>>()
                    .unwrap();
                (name, rows)
            })
            .collect()
    }
}

fn canonical(id: &str) -> (String, Vec<u8>) {
    (
        format!("tasks/{id}.md"),
        file_of(&BoardArtifact::Task(task(id, "Canonical"))).unwrap(),
    )
}

#[test]
fn mixed_canonical_and_blocked_legacy_hides_task_without_touching_raw_files() {
    let mut f = Fixture::new();
    f.put_many([canonical("a"), ("tasks/a.json".into(), b"{}".to_vec())]);
    let before = f.fingerprint();
    let board = f.store.read().unwrap();
    assert_eq!(
        f.fingerprint(),
        before,
        "reading preserves both original raw BLOBs and head"
    );
    assert!(
        board
            .quarantined
            .iter()
            .any(|q| q.path == "tasks/a.json" && q.carried),
        "legacy key remains visibly set aside"
    );
    assert!(
        board.tasks.is_empty(),
        "blocked legacy a.json must hide canonical a.md; got {:?}",
        board.tasks
    );
}

#[test]
fn admitted_payload_fetch_positive_control_is_connected_to_real_sql_blob() {
    let mut f = Fixture::new();
    let (path, bytes) = canonical("a");
    let object = format!("{:x}", Sha256::digest(&bytes));
    f.put_many([(path.clone(), bytes.clone())]);
    assert!(f.store.fixture_take_payload_fetches().is_empty());
    assert_eq!(f.store.read().unwrap().tasks.len(), 1);
    assert_eq!(
        f.store.fixture_take_payload_fetches(),
        [(path, object, bytes.len())]
    );
}

#[test]
fn oversized_canonical_blob_is_not_materialized() {
    let mut f = Fixture::new();
    f.put_many([("tasks/a.md".into(), vec![b'x'; MAX_TASK_FILE_BYTES + 1])]);
    let board = f.store.read().unwrap();
    assert!(board.tasks.is_empty());
    assert_eq!(board.quarantined.len(), 1);
    assert!(
        f.store.fixture_take_payload_fetches().is_empty(),
        "oversized canonical BLOB must be rejected by metadata before Vec<u8> fetch"
    );
}

#[test]
fn opaque_stray_blob_is_not_materialized() {
    let mut f = Fixture::new();
    f.put_many([(
        "tasks/notes.txt".into(),
        vec![b'x'; MAX_TASK_FILE_BYTES + 1],
    )]);
    assert!(f.store.read().unwrap().tasks.is_empty());
    assert!(
        f.store.fixture_take_payload_fetches().is_empty(),
        "stray payload stays opaque, independent of its byte size"
    );
}

#[test]
fn complete_folder_entry_admission_precedes_any_canonical_payload() {
    let mut f = Fixture::new();
    let mut files = vec![canonical("a")];
    files.extend((0..4097).map(|i| (format!("runs/stray-{i}.txt"), vec![b'x'])));
    f.put_many(files);
    assert!(matches!(f.store.read(), Err(BoardStoreError::TooLarge(_))));
    let fetched = f.store.fixture_take_payload_fetches();
    assert!(
        fetched.iter().all(|(path, _, _)| path != "tasks/a.md"),
        "folder entry admission must finish before canonical task fetch; canonical observed: {:?}",
        fetched
            .iter()
            .filter(|(path, _, _)| path == "tasks/a.md")
            .collect::<Vec<_>>()
    );
}

#[test]
fn complete_board_entry_admission_precedes_any_canonical_payload() {
    let mut f = Fixture::new();
    let mut files = vec![canonical("a")];
    files.extend((0..4095).map(|i| (format!("tasks/stray-{i}.txt"), vec![b'x'])));
    for folder in ["runs", "plans", "changes"] {
        files.extend((0..4096).map(|i| (format!("{folder}/stray-{i}.txt"), vec![b'x'])));
    }
    files.push(("discussions/notes.txt".into(), vec![b'x']));
    assert_eq!(
        files.len(),
        16385,
        "each folder admitted individually, board over by one"
    );
    f.put_many(files);
    assert!(matches!(f.store.read(), Err(BoardStoreError::TooLarge(_))));
    let fetched = f.store.fixture_take_payload_fetches();
    assert!(
        fetched.iter().all(|(path, _, _)| path != "tasks/a.md"),
        "aggregate metadata admission must precede canonical payload fetch"
    );
}

#[test]
fn admitted_4096_runs_then_distinct_4097th_is_invalid_write_with_exact_rollback() {
    let mut f = Fixture::new();
    f.put_many((0..4096).map(|i| {
        let id = format!("r{i}");
        (
            format!("runs/{id}.1.md"),
            file_of(&BoardArtifact::Run(run(&id, 1))).unwrap(),
        )
    }));
    let before = f
        .store
        .read()
        .expect("4096 real canonical run entries are readable");
    assert_eq!(before.runs.len(), 4096);
    let raw = f.fingerprint();
    let result = f.store.write(
        &before.head,
        &[BoardArtifact::Run(run("new", 1))],
        "board: one more run",
    );
    assert_eq!(
        f.fingerprint(),
        raw,
        "failed prospective write rolls back every physical table"
    );
    assert_eq!(
        f.store.read().unwrap(),
        before,
        "head and logical board unchanged"
    );
    assert!(
        matches!(result, Err(BoardStoreError::InvalidWrite(_))),
        "entry exhaustion is InvalidWrite, not byte WriteTooLarge: {result:?}"
    );
}

#[test]
fn prospective_run_bytes_remain_write_too_large_characterization() {
    let mut f = Fixture::new();
    // Malformed but size-admitted canonical files count toward run read bytes.
    // 32 * 512 KiB equals the 16 MiB run-folder cap; no giant allocation.
    f.put_many((0..32).map(|i| (format!("runs/r{i}.1.md"), vec![b'x'; 512 * 1024])));
    let before = f.store.read().expect("exact run byte boundary is admitted");
    assert_eq!(before.quarantined.len(), 32);
    let raw = f.fingerprint();
    let result = f.store.write(
        &before.head,
        &[BoardArtifact::Run(run("new", 1))],
        "board: byte overflow",
    );
    assert_eq!(f.fingerprint(), raw);
    assert_eq!(f.store.read().unwrap(), before);
    assert!(
        matches!(result, Err(BoardStoreError::WriteTooLarge(_))),
        "byte exhaustion stays distinct: {result:?}"
    );
}

fn check_invalid_head(head: BoardHead, clear: bool) {
    let mut f = Fixture::new();
    f.put_many([canonical("a"), ("tasks/b.md".into(), b"{".to_vec())]);
    let board = f.store.read().unwrap();
    let q = board
        .quarantined
        .iter()
        .find(|q| q.path == "tasks/b.md")
        .unwrap();
    let raw = f.fingerprint();
    let result = match clear {
        true => f.store.clear_set_aside(
            &head,
            &SetAsideClear {
                path: q.path.clone(),
                object: q.object.clone(),
                repeated: Default::default(),
            },
            "board: invalid head clear",
        ),
        false => f.store.write(
            &head,
            &[BoardArtifact::Task(task("c", "New"))],
            "board: invalid head write",
        ),
    };
    assert!(
        matches!(result, Err(BoardStoreError::InvalidWrite(_))),
        "healthy invalid-head attempt: {result:?}"
    );
    assert_eq!(
        f.fingerprint(),
        raw,
        "invalid head changes no table or BLOB"
    );
    assert_eq!(f.store.read().unwrap(), board);
}

#[test]
fn healthy_unknown_head_write_characterization() {
    check_invalid_head(BoardHead("0".repeat(64)), false);
}
#[test]
fn healthy_unknown_head_clear_characterization() {
    check_invalid_head(BoardHead("0".repeat(64)), true);
}
#[test]
fn healthy_malformed_head_write_characterization() {
    check_invalid_head(BoardHead("not-a-head".into()), false);
}
#[test]
fn healthy_malformed_head_clear_characterization() {
    check_invalid_head(BoardHead("not-a-head".into()), true);
}

// Canonical names, size-admitted but undecodable payloads, which still count
// toward the same cumulative byte bounds as valid canonical files.
fn padding(folder: &str, total: usize) -> Vec<(String, Vec<u8>)> {
    let mut remaining = total;
    let mut files = Vec::new();
    while remaining > 0 {
        let size = remaining.min(512 * 1024);
        let stem = format!("pad{}", files.len());
        let name = match folder {
            "tasks" => format!("{stem}.md"),
            _ => format!("{stem}.1.md"),
        };
        files.push((format!("{folder}/{name}"), vec![b'x'; size]));
        remaining -= size;
    }
    assert_eq!(
        files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>(),
        total
    );
    files
}

#[test]
fn complete_folder_byte_admission_precedes_any_canonical_payload() {
    let mut f = Fixture::new();
    let mut files = vec![canonical("a")];
    files.extend(padding("runs", 16 * 1024 * 1024 + 1));
    f.put_many(files);
    assert!(matches!(f.store.read(), Err(BoardStoreError::TooLarge(_))));
    let fetched = f.store.fixture_take_payload_fetches();
    assert!(
        fetched.iter().all(|(path, _, _)| path != "tasks/a.md"),
        "complete folder length admission must precede canonical task BLOB fetch"
    );
}

#[test]
fn complete_board_byte_admission_precedes_any_canonical_payload() {
    let mut f = Fixture::new();
    let witness = canonical("a");
    let task_padding = 16 * 1024 * 1024 - witness.1.len();
    let mut files = vec![witness];
    files.extend(padding("tasks", task_padding));
    files.extend(padding("plans", 24 * 1024 * 1024));
    files.extend(padding("runs", 16 * 1024 * 1024));
    files.extend(padding("reviews", 8 * 1024 * 1024 + 1));
    assert_eq!(
        files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>(),
        64 * 1024 * 1024 + 1,
        "each folder length admitted individually, aggregate over by one"
    );
    f.put_many(files);
    assert!(matches!(f.store.read(), Err(BoardStoreError::TooLarge(_))));
    let fetched = f.store.fixture_take_payload_fetches();
    assert!(
        fetched.iter().all(|(path, _, _)| path != "tasks/a.md"),
        "complete aggregate length admission must precede canonical task BLOB fetch"
    );
}

#[test]
fn healthy_unknown_40_head_write_characterization() {
    check_invalid_head(BoardHead("0".repeat(40)), false);
}
#[test]
fn healthy_unknown_40_head_clear_characterization() {
    check_invalid_head(BoardHead("0".repeat(40)), true);
}

fn check_unrelated_schema(version: u32) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unrelated.sqlite");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "PRAGMA journal_mode=DELETE;
        CREATE TABLE unrelated (id INTEGER PRIMARY KEY, bytes BLOB NOT NULL);
        INSERT INTO unrelated VALUES (1,X'0001FF80');",
    )
    .unwrap();
    raw.execute_batch(&format!("PRAGMA user_version={version}"))
        .unwrap();
    let snapshot = || {
        let version: i64 = raw
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let journal: String = raw
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let schema: Vec<(String, String, Option<String>)> = raw
            .prepare("SELECT name,type,sql FROM sqlite_schema ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let data: Vec<(i64, Vec<u8>)> = raw
            .prepare("SELECT * FROM unrelated ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (version, journal, schema, data)
    };
    let before = snapshot();
    assert_eq!(before.0, i64::from(version));
    assert_eq!(before.1, "delete");
    let result = SqliteProjectBoardStore::open(
        &path,
        Identity {
            name: "Regression".into(),
            email: "regression@example.com".into(),
        },
    );
    assert_eq!(
        snapshot(),
        before,
        "all schema, data, version and journal mode unchanged"
    );
    assert!(
        matches!(result, Err(BoardStoreError::Clash(_))),
        "unsupported unrelated schema must be explicitly rejected"
    );
}

#[test]
fn unrelated_unsupported_schema_is_rejected_before_any_logical_mutation_characterization() {
    check_unrelated_schema(71);
}

#[test]
fn unrelated_nonempty_version_zero_is_rejected_before_any_logical_mutation() {
    check_unrelated_schema(0);
}
