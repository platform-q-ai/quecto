//! Unchanged reusable contracts over two independently opened SQLite clients.
//! B2 red only: the adapter is a typed-error skeleton, not an alternate store.
use super::project_board_kinds as kinds;
use super::project_board_store::{self as store, Clients};
use quecto::application::project_board::ports::{BoardStoreError, BoardWrite, ProjectBoardStore};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::sqlite::project_board::{
    FixtureHooks, SqliteProjectBoardStore,
};
use std::path::Path;
use tempfile::TempDir;

pub(super) struct Database {
    directory: TempDir,
    hooks: std::sync::Arc<FixtureHooks>,
}

fn identity(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn open(path: &Path, name: &str) -> SqliteProjectBoardStore {
    SqliteProjectBoardStore::open(path, identity(name)).expect("open a real SQLite client")
}

impl Database {
    pub(super) fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            hooks: std::sync::Arc::new(FixtureHooks::default()),
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("board.sqlite")
    }

    pub(super) fn client(&self) -> SqliteProjectBoardStore {
        self.named("Fixture")
    }

    fn named(&self, name: &str) -> SqliteProjectBoardStore {
        SqliteProjectBoardStore::open_with_hooks(&self.path(), identity(name), self.hooks.clone())
            .expect("independent connection sharing fixture hooks")
    }

    fn clients(&self) -> Clients {
        (Box::new(self.named("Alice")), Box::new(self.named("Bob")))
    }

    fn put(&self, path: &str, bytes: &[u8]) {
        self.client()
            .fixture_put_raw(path, bytes)
            .expect("fixture writes raw durable bytes and advances head");
    }

    fn spoil(&self) {
        self.put("tasks/b.md", b"{");
    }

    fn refuse(&self) {
        self.client()
            .fixture_refuse_next_write()
            .expect("arm one transaction refusal");
    }
}

macro_rules! shared {
    ($name:ident, $check:path) => {
        #[test]
        fn $name() {
            let database = Database::new();
            $check(database.clients());
        }
    };
}

// All fifteen store checks; the shared assertion bodies remain untouched.
shared!(bootstrap, store::check_bootstrap);
shared!(write_and_read, store::check_write_and_read);
shared!(object_at, store::check_object_at);
shared!(lock_races, store::check_lock_races);
shared!(unchanged_write, store::check_unchanged_write);
shared!(compare_and_swap, store::check_compare_and_swap);
shared!(invalid_writes, store::check_invalid_writes);
shared!(write_before_bootstrap, store::check_write_before_bootstrap);
shared!(bootstrap_race, store::check_bootstrap_race);
shared!(writer, store::check_writer);

#[test]
fn refused() {
    let database = Database::new();
    store::check_refused(database.clients(), || database.refuse());
}

#[test]
fn quarantine() {
    let database = Database::new();
    store::check_quarantine(database.clients(), || database.spoil());
}

#[test]
fn clear_set_aside() {
    let database = Database::new();
    store::check_clear_set_aside(database.clients(), || database.spoil());
}

#[test]
fn write_after_lost_and_refused() {
    let database = Database::new();
    store::check_write_after_lost_and_refused(database.clients(), || database.refuse());
}

#[test]
fn committed_as_writer() {
    let database = Database::new();
    store::check_committed_as_writer(database.clients(), |head| {
        database
            .client()
            .fixture_committed_by(head)
            .expect("durable writer at head")
    });
}

#[test]
fn carried_set_aside() {
    let database = Database::new();
    store::check_carried_set_aside(
        database.clients(),
        || database.spoil(),
        || database.put("tasks/notes.txt", b"notes"),
        "tasks/notes.txt",
    );
}

// All nine every-kind checks, reusing canonical codec checks already in them.
shared!(every_kind, kinds::check_write_and_read_every_kind);
shared!(one_key_twice, kinds::check_one_key_twice_is_invalid);
shared!(one_artifact_per_key, kinds::check_one_artifact_per_key);
shared!(byte_exact, kinds::check_plan_and_run_read_back_byte_exact);
shared!(
    codec_refusal,
    kinds::check_a_value_the_codec_refuses_is_invalid
);
shared!(
    case_only_change,
    kinds::check_a_change_only_in_case_is_written
);

#[test]
fn set_aside_key() {
    let database = Database::new();
    kinds::check_set_aside_key_refuses_only_its_write(database.clients(), |_| {
        database.put("runs/a.1.md", b"{");
    });
}

#[test]
fn clear_in_folder() {
    let database = Database::new();
    kinds::check_a_clear_in_a_folder_removes_its_file(database.clients(), |_| {
        database.put("runs/a.1.md", b"{");
    });
}

#[test]
fn drop_stray_in_folder() {
    let database = Database::new();
    kinds::check_a_write_drops_a_stray_in_a_folder(database.clients(), || {
        database.put("plans/notes.txt", b"notes");
    });
}

#[test]
fn wal_and_full_on_both_connections() {
    let database = Database::new();
    let alice = open(&database.path(), "Alice");
    let bob = open(&database.path(), "Bob");
    for client in [alice, bob] {
        assert_eq!(
            client.connection_settings().unwrap(),
            ("wal".into(), 2),
            "WAL and synchronous=FULL on each adapter connection"
        );
    }
}

#[test]
fn mixed_codec_refusal_rolls_back_head_and_all_rows() {
    // The canonical check writes a valid task followed by an invalid plan,
    // asserts InvalidWrite with field/line, then verifies head and zero tasks.
    let database = Database::new();
    kinds::check_a_value_the_codec_refuses_is_invalid(database.clients());
}

#[test]
fn acknowledged_write_survives_owned_child_exit_without_destructors() {
    // Only this test's owned child receives this one path; no host service,
    // git environment changes, or existing/live board is involved.
    const CHILD_PATH: &str = "QUECTO_B2_RED_CHILD_DATABASE";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let client = open(Path::new(&path), "Child");
        client.bootstrap().expect("child bootstraps durable board");
        let head = client.read().unwrap().head;
        let result = client.write_tasks(
            &head,
            &[store::task("a", "Acknowledged")],
            "board: child writes",
        );
        assert!(
            matches!(result, Ok(BoardWrite::Written(_))),
            "acknowledged child write: {result:?}"
        );
        // Deliberately bypass Connection::drop and WAL checkpoint-on-close.
        std::process::exit(0);
    }
    let database = Database::new();
    // File-backed output cannot fill a pipe and block the owned child.
    let mut output = tempfile::tempfile().expect("owned child output");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "project_board_sqlite::acknowledged_write_survives_owned_child_exit_without_destructors", "--nocapture"])
        .env(CHILD_PATH, database.path())
        .stdout(output.try_clone().unwrap())
        .stderr(output.try_clone().unwrap())
        .spawn()
        .expect("start owned contract child");
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(10) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            outcome => {
                // Child::kill sends SIGKILL on Unix, only to our owned PID.
                // Always reap, including a race where the child just exited.
                let killed = child.kill();
                let reaped = child.wait();
                assert!(reaped.is_ok(), "reap owned child: {reaped:?}");
                panic!("owned child exceeded 10s or wait failed: {outcome:?}; kill: {killed:?}");
            }
        }
    };
    use std::io::{Read, Seek};
    output.rewind().unwrap();
    let mut diagnostic = String::new();
    output.read_to_string(&mut diagnostic).unwrap();
    assert!(status.success(), "owned child failed: {diagnostic}");
    let board = database.client().read().expect("reopen after child exit");
    assert_eq!(board.tasks, [store::task("a", "Acknowledged")]);
}

#[test]
fn clear_before_bootstrap_is_no_board() {
    let database = Database::new();
    let client = database.client();
    let clear = quecto::application::project_board::ports::SetAsideClear {
        path: "tasks/b.md".into(),
        object: "0".repeat(40),
        repeated: Default::default(),
    };
    assert_eq!(
        client.clear_set_aside(
            &quecto::application::project_board::ports::BoardHead("0".repeat(40)),
            &clear,
            "board: clear b"
        ),
        Err(BoardStoreError::NoBoard)
    );
}

#[test]
fn sqlite_failure_after_first_row_rolls_back_then_writer_recovers() {
    let database = Database::new();
    let alice = database.client();
    let bob = database.client();
    alice
        .bootstrap()
        .expect("bootstrap before transactional fault");
    let before = bob.read().unwrap();
    alice
        .fixture_fail_after_first_row()
        .expect("arm fault inside SQLite transaction");
    let tasks = [store::task("a", "One"), store::task("b", "Two")];
    assert!(
        matches!(
            alice.write_tasks(&before.head, &tasks, "board: two rows"),
            Err(BoardStoreError::Unavailable(_))
        ),
        "injected SQL failure is typed"
    );
    assert_eq!(
        bob.read().unwrap(),
        before,
        "head and every row roll back together"
    );
    let retry = alice.write_tasks(&before.head, &tasks, "board: retry two rows");
    assert!(
        matches!(retry, Ok(BoardWrite::Written(_))),
        "writer recovers after rollback: {retry:?}"
    );
    assert_eq!(bob.read().unwrap().tasks.len(), 2);
}

mod remediation {
    use super::*;
    use quecto::application::project_board::ports::{BoardArtifact, BoardHead, SetAsideClear};
    use quecto::infrastructure::persistence::sqlite::project_board::RawEntry;

    fn assert_after_first_insert_rollback_and_retry(
        database: &Database,
        mutate: impl Fn(&SqliteProjectBoardStore) -> Result<BoardWrite, BoardStoreError>,
    ) {
        let alice = database.named("Alice");
        let bob = database.named("Bob");
        let before = alice.read().unwrap();
        let raw_before = alice.fixture_fingerprint().unwrap();
        let raw = rusqlite::Connection::open(database.path()).unwrap();
        let count = || {
            raw.query_row("SELECT count(*) FROM revisions", [], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        let revisions = count();
        database.client().fixture_fail_after_first_row().unwrap();
        let failed = mutate(&alice);
        assert!(
            matches!(failed, Err(BoardStoreError::Unavailable(_))),
            "actual first inserted row must trigger SQL fault: {failed:?}"
        );
        assert_eq!(
            bob.read().unwrap(),
            before,
            "head and complete snapshot roll back"
        );
        assert_eq!(
            bob.fixture_fingerprint().unwrap(),
            raw_before,
            "every object/tree/revision/state row, including unreachable rows, rolls back"
        );
        assert_eq!(count(), revisions, "failed writer leaves no provenance");
        let retry = mutate(&bob);
        let Ok(BoardWrite::Written(head)) = retry else {
            panic!("separate client retries after one-shot fault: {retry:?}");
        };
        assert_eq!(bob.fixture_committed_by(&head).unwrap(), identity("Bob"));
        assert_eq!(
            count(),
            revisions + 1,
            "exactly the successful retry adds provenance"
        );
    }

    #[test]
    fn clear_after_first_insert_fault_rolls_back_then_another_writer_recovers() {
        let database = Database::new();
        database.client().bootstrap().unwrap();
        database.put("tasks/b.md", b"{");
        let board = database.client().read().unwrap();
        let file = &board.quarantined[0];
        let clearing = SetAsideClear {
            path: file.path.clone(),
            object: file.object.clone(),
            repeated: Default::default(),
        };
        assert_after_first_insert_rollback_and_retry(&database, |client| {
            client.clear_set_aside(&board.head, &clearing, "board: clear fault")
        });
        assert!(database.client().read().unwrap().quarantined.is_empty());
    }

    #[test]
    fn identical_write_after_first_insert_fault_rolls_back_then_another_writer_recovers() {
        let database = Database::new();
        let alice = database.named("Alice");
        alice.bootstrap().unwrap();
        let task = store::task("a", "Same bytes");
        let head = alice.read().unwrap().head;
        assert!(matches!(
            alice.write_tasks(&head, std::slice::from_ref(&task), "board: initial"),
            Ok(BoardWrite::Written(_))
        ));
        let before = alice.read().unwrap();
        assert_after_first_insert_rollback_and_retry(&database, |client| {
            client.write_tasks(
                &before.head,
                std::slice::from_ref(&task),
                "board: identical fault",
            )
        });
        assert_eq!(database.client().read().unwrap().tasks, [task]);
    }

    #[test]
    fn shared_hooks_reject_overwrite_survive_invalid_unknown_stale_and_are_one_shot() {
        let database = Database::new();
        let alice = database.client();
        let bob = database.client();
        alice.bootstrap().unwrap();
        let stale = alice.read().unwrap().head;
        database.put("tasks/b.md", b"{");
        let head = bob.read().unwrap().head;
        database.refuse(); // Short-lived arming client is deliberately dropped.
        assert!(matches!(
            bob.fixture_fail_after_first_row(),
            Err(BoardStoreError::Unavailable(_))
        ));
        let before = bob.fixture_fingerprint().unwrap();
        for expected in [
            BoardHead("a".repeat(40)),
            BoardHead("b".repeat(64)),
            BoardHead("invalid".into()),
        ] {
            assert!(matches!(
                alice.write_tasks(&expected, &[store::task("a", "A")], "board: invalid"),
                Err(BoardStoreError::InvalidWrite(_))
            ));
        }
        assert_eq!(
            alice.write_tasks(&stale, &[store::task("a", "A")], "board: stale"),
            Ok(BoardWrite::Lost)
        );
        assert!(matches!(
            alice.write(&head, &[], "board: empty"),
            Err(BoardStoreError::InvalidWrite(_))
        ));
        let clearing = SetAsideClear {
            path: "tasks/b.md".into(),
            object: "invalid".into(),
            repeated: Default::default(),
        };
        assert!(matches!(
            alice.clear_set_aside(&head, &clearing, "board: invalid clear"),
            Err(BoardStoreError::InvalidWrite(_))
        ));
        assert!(matches!(
            alice.write_tasks(&head, &[store::task("a", "A")], "bad\nmessage"),
            Err(BoardStoreError::InvalidWrite(_))
        ));
        assert_eq!(bob.fixture_fingerprint().unwrap(), before);
        assert!(matches!(
            bob.write_tasks(&head, &[store::task("a", "A")], "board: refused"),
            Ok(BoardWrite::Refused(_))
        ));
        assert_eq!(bob.fixture_fingerprint().unwrap(), before);
        assert!(matches!(
            alice.write_tasks(&head, &[store::task("a", "A")], "board: retry"),
            Ok(BoardWrite::Written(_))
        ));
    }

    #[test]
    fn failed_sql_has_no_orphan_object_tree_revision_or_provenance_residue() {
        let database = Database::new();
        let alice = database.client();
        alice.bootstrap().unwrap();
        let head = alice.read().unwrap().head;
        let before = alice.fixture_fingerprint().unwrap();
        database.client().fixture_fail_after_first_row().unwrap();
        assert!(matches!(
            alice.write_tasks(
                &head,
                &[store::task("a", "One"), store::task("b", "Two")],
                "board: fault"
            ),
            Err(BoardStoreError::Unavailable(_))
        ));
        assert_eq!(database.client().fixture_fingerprint().unwrap(), before);
        assert!(matches!(
            database
                .client()
                .write_tasks(&head, &[store::task("a", "One")], "board: retry"),
            Ok(BoardWrite::Written(_))
        ));
    }

    #[test]
    fn full_fingerprint_observes_unreachable_objects_entries_and_typed_nulls() {
        let database = Database::new();
        let client = database.client();
        client.bootstrap().unwrap();
        let raw = rusqlite::Connection::open(database.path()).unwrap();
        let before = client.fixture_fingerprint().unwrap();
        let blob = "c".repeat(64);
        let tree = "d".repeat(64);
        raw.execute("INSERT INTO objects VALUES (?1,'blob',X'',NULL)", [&blob])
            .unwrap();
        let empty = client.fixture_fingerprint().unwrap();
        assert_ne!(
            empty, before,
            "unreachable empty BLOB/cardinality is included"
        );
        raw.execute("UPDATE objects SET bytes=X'00' WHERE oid=?1", [&blob])
            .unwrap();
        assert_ne!(
            client.fixture_fingerprint().unwrap(),
            empty,
            "exact unreachable bytes are included"
        );
        raw.execute("INSERT INTO objects VALUES (?1,'tree',NULL,0)", [&tree])
            .unwrap();
        let no_rows = client.fixture_fingerprint().unwrap();
        raw.execute(
            "INSERT INTO tree_entries VALUES (?1,0,'120000',X'FF00',?2)",
            rusqlite::params![tree, blob],
        )
        .unwrap();
        assert_ne!(
            client.fixture_fingerprint().unwrap(),
            no_rows,
            "orphan ordinal rows/raw bytes are included"
        );
        raw.execute(
            "UPDATE objects SET kind='blob',bytes=X'',tree_size=NULL WHERE oid=?1",
            [&tree],
        )
        .unwrap();
        assert_ne!(
            client.fixture_fingerprint().unwrap(),
            no_rows,
            "NULL/type domains do not collapse to empty bytes"
        );
    }

    #[test]
    fn opaque_modes_raw_names_and_archive_are_carried_by_reference_without_blob_reads() {
        let database = Database::new();
        let client = database.client();
        client.bootstrap().unwrap();
        client
            .fixture_edit(|editor| {
                let (_, root) = editor.current()?;
                let mut roots = editor.entries(&root)?;
                let opaque = editor.blob(&vec![b'x'; 600 * 1024])?;
                let subtree = editor.tree(&[RawEntry {
                    mode: "100644".into(),
                    name: b"hidden".to_vec(),
                    oid: opaque.clone(),
                }])?;
                let tasks = editor.tree(&[
                    RawEntry {
                        mode: "100755".into(),
                        name: b"a.md".to_vec(),
                        oid: opaque.clone(),
                    },
                    RawEntry {
                        mode: "120000".into(),
                        name: b"a.json".to_vec(),
                        oid: opaque.clone(),
                    },
                    RawEntry {
                        mode: "040000".into(),
                        name: b"nested.md".to_vec(),
                        oid: subtree.clone(),
                    },
                    RawEntry {
                        mode: "100644".into(),
                        name: vec![0xff, b'/', 0],
                        oid: opaque,
                    },
                ])?;
                roots.retain(|row| row.name != b"tasks");
                roots.push(RawEntry {
                    mode: "040000".into(),
                    name: b"tasks".to_vec(),
                    oid: tasks,
                });
                roots.push(RawEntry {
                    mode: "040000".into(),
                    name: b"archive".to_vec(),
                    oid: subtree,
                });
                editor.tree(&roots)
            })
            .unwrap();
        let board = client.read().unwrap();
        assert!(board.tasks.is_empty());
        assert_eq!(board.quarantined.len(), 4);
        assert!(client.fixture_take_payload_fetches().is_empty());
        let before = client.fixture_fingerprint().unwrap();
        let archive = before
            .lines()
            .find(|row| row.ends_with("\tarchive"))
            .unwrap();
        assert!(matches!(
            client.write(
                &board.head,
                &[BoardArtifact::Task(store::task("new", "New"))],
                "board: retain opaque"
            ),
            Ok(BoardWrite::Written(_))
        ));
        assert!(client.fixture_take_payload_fetches().is_empty());
        assert!(
            client
                .fixture_fingerprint()
                .unwrap()
                .lines()
                .any(|row| row == archive)
        );
        let after = client.read().unwrap();
        assert_eq!(after.tasks, [store::task("new", "New")]);
        for path in ["tasks/a.md", "tasks/a.json", "tasks/nested.md"] {
            let old = board
                .quarantined
                .iter()
                .find(|row| row.path == path)
                .unwrap();
            assert!(
                after
                    .quarantined
                    .iter()
                    .any(|row| row.path == path && row.object == old.object && row.carried)
            );
        }
    }

    #[test]
    fn falsified_tree_size_cannot_admit_an_unbounded_listing_and_mismatch_is_unavailable() {
        let database = Database::new();
        let client = database.client();
        client.bootstrap().unwrap();
        database.put("tasks/a.md", b"{");
        let raw = rusqlite::Connection::open(database.path()).unwrap();
        let tree:String=raw.query_row("SELECT folder.oid FROM state JOIN revisions ON revisions.head=state.head JOIN tree_entries folder ON folder.tree=revisions.root WHERE folder.raw_name=CAST('tasks' AS BLOB)",[],|r| r.get(0)).unwrap();
        raw.execute("UPDATE objects SET tree_size=0 WHERE oid=?1", [&tree])
            .unwrap();
        assert!(
            matches!(client.read(), Err(BoardStoreError::Unavailable(_))),
            "stored size must equal structural rows"
        );
        raw.execute(
            "UPDATE tree_entries SET raw_name=?1 WHERE tree=?2",
            rusqlite::params![vec![b'x'; 3 * 1024 * 1024], tree],
        )
        .unwrap();
        assert!(
            matches!(client.read(), Err(BoardStoreError::Unavailable(_))),
            "independent length preflight rejects allocation before listing collection"
        );
    }

    #[test]
    fn unreleased_old_schema_is_refused_before_wal_or_schema_mutation() {
        let database = Database::new();
        let raw = rusqlite::Connection::open(database.path()).unwrap();
        raw.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA user_version=1; CREATE TABLE revisions(head TEXT PRIMARY KEY,name TEXT NOT NULL,email TEXT NOT NULL); CREATE TABLE state(id INTEGER PRIMARY KEY CHECK(id=1),head TEXT NOT NULL,refuse INTEGER NOT NULL DEFAULT 0,fault INTEGER NOT NULL DEFAULT 0); CREATE TABLE files(path TEXT PRIMARY KEY,object TEXT NOT NULL,bytes BLOB NOT NULL); INSERT INTO files VALUES ('tasks/a.md','original',X'00FF');").unwrap();
        let snapshot = || {
            let schema: Vec<(String, Option<String>)> = raw
                .prepare("SELECT name,sql FROM sqlite_schema ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let journal: String = raw
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            let version: i64 = raw
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            let bytes: Vec<u8> = raw
                .query_row("SELECT bytes FROM files", [], |r| r.get(0))
                .unwrap();
            (schema, journal, version, bytes)
        };
        let before = snapshot();
        assert!(matches!(
            SqliteProjectBoardStore::open(&database.path(), identity("Reader")),
            Err(BoardStoreError::Clash(_))
        ));
        assert_eq!(snapshot(), before);
    }
}

#[test]
fn round2_memory_filename_is_refused_with_unavailable() {
    assert!(
        matches!(
            SqliteProjectBoardStore::open(Path::new(":memory:"), identity("Transient")),
            Err(BoardStoreError::Unavailable(_))
        ),
        "an in-memory connection must not expose an acknowledged durable board store"
    );
}

#[test]
fn round2_empty_filename_is_refused_with_unavailable() {
    assert!(
        matches!(
            SqliteProjectBoardStore::open(Path::new(""), identity("Transient")),
            Err(BoardStoreError::Unavailable(_))
        ),
        "a temporary empty-filename connection must not expose a durable board store"
    );
}

#[test]
fn round2_file_backed_wal_full_write_survives_reopen() {
    let database = Database::new();
    let head = {
        let client = database.client();
        assert_eq!(client.connection_settings().unwrap(), ("wal".into(), 2));
        client.bootstrap().unwrap();
        let head = client.read().unwrap().head;
        match client.write_tasks(&head, &[store::task("a", "Durable")], "board: durable") {
            Ok(BoardWrite::Written(head)) => head,
            result => panic!("expected acknowledged write, got {result:?}"),
        }
    };
    let reopened = database.client();
    assert_eq!(reopened.connection_settings().unwrap(), ("wal".into(), 2));
    let board = reopened.read().unwrap();
    assert_eq!(board.head, head);
    assert_eq!(board.tasks, [store::task("a", "Durable")]);
}

#[test]
fn round2_nonutf8_same_object_alias_is_quarantined_not_fetched_independently() {
    use quecto::infrastructure::persistence::sqlite::project_board::RawEntry;
    let database = Database::new();
    let client = database.client();
    client.bootstrap().unwrap();
    let head = client.read().unwrap().head;
    client
        .write_tasks(&head, &[store::task("a", "A")], "board: canonical")
        .unwrap();
    client.fixture_take_payload_fetches();
    assert_eq!(client.read().unwrap().tasks, [store::task("a", "A")]);
    let connected = client.fixture_take_payload_fetches();
    assert_eq!(
        connected.len(),
        1,
        "positive control observes actual admitted object"
    );
    assert_eq!(connected[0].0, "tasks/a.md");
    assert!(connected[0].2 > 0);
    let oid = connected[0].1.clone();
    let raw_name = vec![0xff];
    client
        .fixture_edit(|editor| {
            let (_, root) = editor.current()?;
            let mut roots = editor.entries(&root)?;
            let tasks = roots.iter().find(|entry| entry.name == b"tasks").unwrap();
            let mut entries = editor.entries(&tasks.oid)?;
            assert_eq!(
                entries
                    .iter()
                    .find(|entry| entry.name == b"a.md")
                    .unwrap()
                    .oid,
                oid
            );
            entries.push(RawEntry {
                mode: "100644".into(),
                name: raw_name.clone(),
                oid: oid.clone(),
            });
            let tasks = editor.tree(&entries)?;
            roots.retain(|entry| entry.name != b"tasks");
            roots.push(RawEntry {
                mode: "040000".into(),
                name: b"tasks".to_vec(),
                oid: tasks,
            });
            editor.tree(&roots)
        })
        .unwrap();
    let before = client.fixture_fingerprint().unwrap();
    let board = client.read().expect("observational witness must not reject canonical task sharing its object with a raw nonUTF8 alias");
    assert_eq!(board.tasks, [store::task("a", "A")]);
    assert_eq!(board.quarantined.len(), 1);
    assert_eq!(
        (
            board.quarantined[0].carried,
            board.quarantined[0].key.clone()
        ),
        (false, None),
        "opaque alias is a stray, not an admitted task"
    );
    assert_eq!(board.quarantined[0].path, "tasks/�");
    assert_eq!(board.quarantined[0].object, oid);
    assert_eq!(
        client.fixture_fingerprint().unwrap(),
        before,
        "raw identity and every stored row remain untouched"
    );
    assert_eq!(
        client.fixture_take_payload_fetches(),
        connected,
        "the canonical object is fetched once; opaque alias is not an independently admitted payload"
    );
    // Inspect structural metadata only: the alias retains its exact bytes/OID.
    let raw = rusqlite::Connection::open(database.path()).unwrap();
    let count: i64 = raw.query_row("SELECT count(*) FROM state JOIN revisions ON revisions.head=state.head JOIN tree_entries root ON root.tree=revisions.root JOIN tree_entries file ON file.tree=root.oid WHERE root.raw_name=CAST('tasks' AS BLOB) AND file.raw_name=?1 AND file.oid=?2", rusqlite::params![raw_name, oid], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn round2_recognized_names_wrong_ddl_refused_before_any_mutation() {
    use rusqlite::{Connection, types::Value};
    fn rows(c: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut statement = c.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    // Typed SQLite values retain NULL, raw BLOBs, extra columns and unreachable
    // rows; unlike a head-only snapshot this checks every cell in every table.
    fn snapshot(c: &Connection) -> Vec<Vec<Vec<Value>>> {
        [
            "PRAGMA journal_mode",
            "PRAGMA application_id",
            "PRAGMA user_version",
            "SELECT * FROM sqlite_schema ORDER BY name",
            "SELECT * FROM objects ORDER BY rowid",
            "SELECT * FROM tree_entries ORDER BY rowid",
            "SELECT * FROM revisions ORDER BY rowid",
            "SELECT * FROM state ORDER BY rowid",
            "SELECT * FROM git_objects ORDER BY rowid",
            "SELECT * FROM history_meta ORDER BY rowid",
            "SELECT * FROM epochs ORDER BY rowid",
        ]
        .iter()
        .map(|sql| rows(c, sql))
        .collect()
    }
    let database = Database::new();
    {
        let client = database.client();
        client.bootstrap().unwrap();
        let head = client.read().unwrap().head;
        assert!(matches!(
            client.write_tasks(&head, &[store::task("a", "A")], "board: populate"),
            Ok(BoardWrite::Written(_))
        ));
    }
    let raw = Connection::open(database.path()).unwrap();
    raw.execute_batch("PRAGMA journal_mode=DELETE;
        INSERT INTO objects VALUES ('unreachable-blob','blob',X'00FF',NULL);
        INSERT INTO objects VALUES ('unreachable-tree','tree',NULL,19);
        INSERT INTO tree_entries VALUES ('unreachable-tree',0,'100644',X'FF00','unreachable-blob');
        INSERT INTO revisions VALUES ('unreachable-revision','unreachable-tree','Unreachable','unreachable@example.com',3,NULL,'unreachable fixture',0,NULL,NULL);").unwrap();
    let names = rows(
        &raw,
        "SELECT name,type,tbl_name FROM sqlite_schema ORDER BY name",
    );
    assert_eq!(
        names.len(),
        16,
        "nine schema-4 tables and seven indexes (B7 adds op_results and revision_principals)"
    );
    assert_eq!(
        rows(&raw, "PRAGMA application_id"),
        vec![vec![Value::Integer(0x51424f32)]]
    );
    assert_eq!(
        rows(&raw, "PRAGMA user_version"),
        vec![vec![Value::Integer(4)]]
    );
    assert_eq!(
        rows(&raw, "PRAGMA integrity_check"),
        vec![vec![Value::Text("ok".into())]]
    );
    let recognized = SqliteProjectBoardStore::open(&database.path(), identity("Recognized"))
        .expect("schema-4 fixture is recognized before the deliberate wrong DDL");
    drop(recognized);
    drop(raw);
    let raw = Connection::open(database.path()).unwrap();
    assert_eq!(
        rows(&raw, "PRAGMA journal_mode=DELETE"),
        vec![vec![Value::Text("delete".into())]]
    );
    // Only SQL text changes: all recognized names/indexes, count, ownership and
    // usable original columns survive. name == n remains true; SQL == s is false.
    raw.execute_batch("ALTER TABLE state ADD COLUMN unused BLOB; UPDATE state SET unused=X'FE00';")
        .unwrap();
    assert_eq!(
        rows(
            &raw,
            "SELECT name,type,tbl_name FROM sqlite_schema ORDER BY name"
        ),
        names
    );
    assert_eq!(
        rows(&raw, "SELECT length(head),typeof(unused) FROM state")[0][1],
        Value::Text("blob".into())
    );
    let before = snapshot(&raw);
    // Unsupported schemas already use Clash on dd01. This is a deliberately
    // passing strength regression, not a manufactured behavioral RED.
    assert!(matches!(
        SqliteProjectBoardStore::open(&database.path(), identity("Refused")),
        Err(BoardStoreError::Clash(_))
    ));
    assert_eq!(
        snapshot(&raw),
        before,
        "refusal precedes WAL/header/schema/all-table data mutation"
    );
    assert_eq!(
        rows(&raw, "PRAGMA journal_mode"),
        vec![vec![Value::Text("delete".into())]]
    );
}
