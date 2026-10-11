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
    assert_eq!(within(move || needs_rebuild(&reader.ready())), Some(true));
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

#[test]
fn a_link_at_a_side_file_is_never_followed_and_is_removed() {
    let (dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"keep me").unwrap();
    for suffix in SIDE_FILES {
        std::os::unix::fs::symlink(&victim, side(&index, suffix)).unwrap();
    }
    assert_eq!(ids(index.ready()), ["a"]);
    rebuild(&index, &moved("e2")).unwrap();
    for suffix in SIDE_FILES {
        assert!(
            std::fs::symlink_metadata(side(&index, suffix)).is_err(),
            "{suffix}"
        );
    }
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
}

#[test]
fn a_file_of_an_unsupported_schema_format_is_rebuilt() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let mut bytes = std::fs::read(index.path()).unwrap();
    // The schema format number, a big-endian u32 at offset 44: 1 to 4.
    bytes[44..48].copy_from_slice(&5u32.to_be_bytes());
    std::fs::write(index.path(), bytes).unwrap();
    assert!(needs_rebuild(&index.ready()));
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    assert_eq!(ids(index.ready()), ["a"]);
}

#[test]
fn a_directory_at_the_index_is_replaced() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    std::fs::remove_file(index.path()).unwrap();
    std::fs::create_dir(index.path()).unwrap();
    std::fs::write(index.path().join("inside"), b"x").unwrap();
    assert!(needs_rebuild(&index.ready()));
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    assert_eq!(ids(index.ready()), ["a"]);
    std::fs::remove_file(index.path()).unwrap();
    std::fs::create_dir(index.path()).unwrap();
    index.discard().unwrap();
    assert!(std::fs::symlink_metadata(index.path()).is_err());
}

#[test]
fn a_link_or_directory_at_the_lock_is_replaced_never_followed() {
    for planted in ["link", "directory"] {
        let (dir, index) = index();
        rebuild(&index, &board("a1")).unwrap();
        let lock = side(&index, ".lock");
        std::fs::remove_file(&lock).unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep me").unwrap();
        match planted {
            "link" => std::os::unix::fs::symlink(&victim, &lock).unwrap(),
            _ => std::fs::create_dir(&lock).unwrap(),
        }
        assert_eq!(
            rebuild(&index, &moved("e2")),
            Ok(Refreshed::Rebuilt {
                head: moved("e2").head.0
            }),
            "{planted}"
        );
        index.discard().unwrap();
        assert!(lock.metadata().unwrap().is_file(), "{planted}");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me", "{planted}");
    }
}

/// An index whose path holds `?`, `#` or `%` is opened as that path, never
/// as a query, a fragment or an escape.
#[test]
fn an_index_path_with_uri_characters_is_opened_as_itself() {
    let dir = tempfile::tempdir().unwrap();
    let index = BoardIndex::at(dir.path().join("a?b#c%41d").join("project-board.sqlite"));
    rebuild(&index, &board("a1")).unwrap();
    assert_eq!(index.head(), Ok(Some(board("a1").head.0)));
    assert_eq!(ids(index.ready()), ["a"]);
}

/// The index's directory is its owner's alone: one another user could
/// write to is made private again before anything is published there.
#[test]
fn a_shared_index_directory_is_made_private_again() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let parent = index.path().parent().unwrap();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    rebuild(&index, &moved("e2")).unwrap();
    let mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o022, 0, "{mode:o}");
}

/// Readers check the index's directory too: one this user owns but others
/// may write to is made private again, and a link in its place is refused.
#[test]
fn readers_check_the_index_directory() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let parent = index.path().parent().unwrap().to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(ids(index.ready()), ["a"]);
    let mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o022, 0, "{mode:o}");
    let moved_to = dir.path().join("elsewhere");
    std::fs::rename(&parent, &moved_to).unwrap();
    std::os::unix::fs::symlink(&moved_to, &parent).unwrap();
    let read = index.ready();
    assert!(matches!(&read, Err(IndexError::Io(_))), "{read:?}");
}
