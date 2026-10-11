//! Files planted beside the index, where SQLite looks for its own: a
//! write-ahead log, a shared-memory file, a rollback journal, or something
//! that is not a file at those names. Readers never apply, wait on or fail
//! for any of them, and every rebuild or discard removes them.
use super::*;

const SIDE_FILES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn side(index: &BoardIndex, suffix: &str) -> PathBuf {
    let mut path = index.path().as_os_str().to_owned();
    path.push(suffix);
    PathBuf::from(path)
}

/// A copy of the index changed by `sql` while the copy's `suffix` file
/// (`-wal` or `-journal`) holds the change, planted beside the index.
fn plant(index: &BoardIndex, journal_mode: &str, sql: &str, suffix: &str) {
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("copy.sqlite");
    std::fs::copy(index.path(), &copy).unwrap();
    let connection = rusqlite::Connection::open(&copy).unwrap();
    connection
        .pragma_update_and_check(None, "journal_mode", journal_mode, |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    connection
        .execute_batch("PRAGMA wal_autocheckpoint = 0; PRAGMA cache_size = 1; BEGIN;")
        .unwrap();
    connection.execute_batch(sql).unwrap();
    if journal_mode == "WAL" {
        connection.execute_batch("COMMIT;").unwrap();
    }
    let mut from = copy.as_os_str().to_owned();
    from.push(suffix);
    std::fs::copy(PathBuf::from(from), side(index, suffix)).unwrap();
    drop(connection);
}

/// `read` on its own thread: what it answered, or `None` if it was still
/// waiting after five seconds.
fn within<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(read());
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .ok()
}

fn fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid C string and a mode.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
}

#[test]
fn a_planted_write_ahead_log_is_never_applied() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let forged = "f".repeat(40);
    plant(
        &index,
        "WAL",
        &format!("UPDATE meta SET value = '{forged}' WHERE key = 'head';"),
        "-wal",
    );
    assert_eq!(index.head(), Ok(Some(board("a1").head.0)));
    assert_eq!(ids(index.ready()), ["a"]);
}

#[test]
fn a_planted_write_ahead_log_that_changes_the_schema_never_survives_a_rebuild() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    plant(
        &index,
        "WAL",
        "CREATE TABLE planted (x); DELETE FROM tasks;",
        "-wal",
    );
    assert_eq!(ids(index.ready()), ["a"]);
    rebuild(&index, &moved("e2")).unwrap();
    assert!(
        !side(&index, "-wal").exists(),
        "the planted log outlived a rebuild"
    );
    assert_eq!(ids(index.ready()), ["e"]);
}

#[test]
fn a_planted_rollback_journal_never_blocks_a_reader() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    plant(
        &index,
        "DELETE",
        "UPDATE meta SET value = 'x' WHERE key = 'head'; DELETE FROM tasks;",
        "-journal",
    );
    assert!(side(&index, "-journal").metadata().unwrap().len() > 0);
    assert_eq!(index.head(), Ok(Some(board("a1").head.0)));
    assert_eq!(ids(index.ready()), ["a"]);
}

#[test]
fn a_directory_at_the_journal_never_blocks_a_reader() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    std::fs::create_dir(side(&index, "-journal")).unwrap();
    assert_eq!(ids(index.ready()), ["a"]);
    rebuild(&index, &moved("e2")).unwrap();
    assert!(!side(&index, "-journal").exists());
}

#[test]
fn a_fifo_at_the_journal_never_hangs_a_reader() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    fifo(&side(&index, "-journal"));
    let reader = index.clone();
    assert_eq!(
        within(move || ids(reader.ready())),
        Some(vec!["a".to_string()])
    );
}

#[test]
fn a_fifo_at_the_index_never_hangs_a_reader() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    std::fs::remove_file(index.path()).unwrap();
    fifo(index.path());
    let reader = index.clone();
    assert_eq!(
        within(move || reader.ready()),
        Some(Err(IndexError::NeedsRebuild))
    );
}

#[test]
fn every_rebuild_and_discard_removes_the_side_files() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let plant_all = || {
        for suffix in SIDE_FILES {
            std::fs::write(side(&index, suffix), b"planted").unwrap();
        }
    };
    plant_all();
    rebuild(&index, &moved("e2")).unwrap();
    for suffix in SIDE_FILES {
        assert!(!side(&index, suffix).exists(), "{suffix} after a rebuild");
    }
    plant_all();
    index.discard().unwrap();
    for suffix in SIDE_FILES {
        assert!(!side(&index, suffix).exists(), "{suffix} after a discard");
    }
}
