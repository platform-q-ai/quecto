//! Unchanged reusable contracts over two independently opened SQLite clients.
//! B2 red only: the adapter is a typed-error skeleton, not an alternate store.
use super::project_board_kinds as kinds;
use super::project_board_store::{self as store, Clients};
use quecto::application::project_board::ports::{BoardStoreError, BoardWrite, ProjectBoardStore};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::path::Path;
use tempfile::TempDir;

struct Database {
    directory: TempDir,
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
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("board.sqlite")
    }

    fn client(&self) -> SqliteProjectBoardStore {
        open(&self.path(), "Fixture")
    }

    fn clients(&self) -> Clients {
        let path = self.path();
        (Box::new(open(&path, "Alice")), Box::new(open(&path, "Bob")))
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
