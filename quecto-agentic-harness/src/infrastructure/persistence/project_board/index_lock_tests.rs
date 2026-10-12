//! The writer lock: bounded, never held across the network, and the build
//! files of live writers are never swept.
use super::*;
use crate::application::project_board::ports::ArtifactKey;

/// A history of known ancestry; `remote_head` records whether the index's
/// writer had taken its lock when it was asked: the lock file exists only
/// once a writer's `lock` makes it, so a test that removes it first sees it
/// again only if the remote is asked after the lock was taken.
///
/// Whether the lock is held is not asked of `flock` itself: a process
/// another test forks (to run git) holds a copy of every open file of this
/// process until it execs, the writer's lock file included, so a lock the
/// writer already let go can still look held for a moment.
struct Ancestry {
    remote: Option<String>,
    ancestors: Vec<(String, String)>,
    lock: Option<PathBuf>,
    asked_under_lock: std::sync::Mutex<bool>,
}

impl Ancestry {
    fn new(remote: Option<&str>, ancestors: &[(&str, &str)]) -> Self {
        Self {
            remote: remote.map(str::to_string),
            ancestors: ancestors
                .iter()
                .map(|(older, newer)| (older.to_string(), newer.to_string()))
                .collect(),
            lock: None,
            asked_under_lock: std::sync::Mutex::new(false),
        }
    }
}

impl BoardRevisions for Ancestry {
    fn remote_head(&self) -> Result<Option<String>, BoardStoreError> {
        if let Some(lock) = &self.lock {
            if std::fs::symlink_metadata(lock).is_ok() {
                *self.asked_under_lock.lock().unwrap() = true;
            }
        }
        Ok(self.remote.clone())
    }

    fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool, BoardStoreError> {
        Ok(older == newer || self.ancestors.iter().any(|(o, n)| o == older && n == newer))
    }
}

fn head(snapshot: &BoardSnapshot) -> String {
    snapshot.head.0.clone()
}

fn lock_path(index: &BoardIndex) -> PathBuf {
    let mut path = index.path().as_os_str().to_owned();
    path.push(".lock");
    PathBuf::from(path)
}

#[test]
fn the_remote_is_never_asked_while_the_writer_lock_is_held() {
    let (_dir, index) = index();
    let (older, newer) = (board("a1"), moved("e2"));
    rebuild(&index, &newer).unwrap();
    // Gone until a writer takes its lock again.
    std::fs::remove_file(lock_path(&index)).unwrap();
    let mut revisions = Ancestry::new(Some(&head(&newer)), &[(&head(&older), &head(&newer))]);
    revisions.lock = Some(lock_path(&index));
    assert_eq!(
        index.rebuild(&older, &revisions),
        Ok(Refreshed::Newer { head: head(&newer) })
    );
    assert!(!*revisions.asked_under_lock.lock().unwrap());
}

/// The stored head is kept over an older snapshot only while the remote
/// board still holds it; one a force push abandoned is replaced.
#[test]
fn a_stored_head_the_board_abandoned_is_replaced_by_an_older_snapshot() {
    let (_dir, index) = index();
    let (older, abandoned, elsewhere) = (board("a1"), moved("e2"), board("c3"));
    rebuild(&index, &abandoned).unwrap();
    let revisions = Ancestry::new(
        Some(&head(&elsewhere)),
        &[
            (&head(&older), &head(&abandoned)),
            (&head(&older), &head(&elsewhere)),
        ],
    );
    assert_eq!(
        index.rebuild(&older, &revisions),
        Ok(Refreshed::Rebuilt { head: head(&older) })
    );
    let unknown = Ancestry::new(None, &[(&head(&older), &head(&abandoned))]);
    rebuild(&index, &abandoned).unwrap();
    assert_eq!(
        index.rebuild(&older, &unknown),
        Ok(Refreshed::Rebuilt { head: head(&older) }),
        "no remote board: nothing keeps the stored head"
    );
}

#[test]
fn a_held_writer_lock_is_waited_on_for_a_bounded_time() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let dir = index.path().parent().unwrap().to_path_buf();
    let name = index
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let _held = index_file::lock(&dir, &name).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let other = index.clone();
    std::thread::spawn(move || {
        let _ = sender.send(other.discard());
    });
    let waited = receiver.recv_timeout(std::time::Duration::from_secs(10));
    assert!(
        matches!(&waited, Ok(Err(IndexError::Io(why))) if why.contains("lock")),
        "{waited:?}"
    );
    assert!(index.path().exists(), "the held index was kept");
}

#[test]
fn a_build_a_live_writer_holds_is_never_swept_however_old() {
    let (dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let parent = index.path().parent().unwrap().to_path_buf();
    let name = index
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let live = index_file::build(&parent, &name, |_| Ok(())).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let age = |path: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    age(live.path());
    let dead = parent.join(format!("{name}.1.2.3.build"));
    std::fs::write(&dead, b"left by a writer that died").unwrap();
    age(&dead);
    index_file::sweep(&parent, &name);
    assert!(live.path().exists(), "a live build was swept");
    assert!(!dead.exists(), "an abandoned build was kept");
    drop(dir);
}

#[test]
fn errors_while_building_are_never_a_reason_to_rebuild() {
    for code in [
        rusqlite::ffi::SQLITE_NOTADB,
        rusqlite::ffi::SQLITE_CORRUPT,
        rusqlite::ffi::SQLITE_TOOBIG,
    ] {
        let error = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        assert!(
            matches!(index_file::built(error), IndexError::Sqlite(_)),
            "{code}"
        );
    }
    let error = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        None,
    );
    assert!(matches!(index_file::built(error), IndexError::Io(_)));
}

/// `ready` re-checks each dependency by its decoded task, never by the
/// status column, which is only a prefilter.
#[test]
fn ready_checks_dependencies_by_their_records() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    tamper(&index, "UPDATE tasks SET status = 'done' WHERE id = 'a';");
    let ready = ids(index.ready());
    assert!(!ready.contains(&"b".to_string()), "{ready:?}");
}

/// Two quarantined files of one path (a name listed twice) are both kept,
/// in board order.
#[test]
fn a_path_set_aside_twice_is_listed_twice() {
    let (_dir, index) = index();
    let mut snapshot = board("a1");
    snapshot.quarantined = vec![
        QuarantinedFile {
            path: "tasks/x.json".into(),
            key: Some(ArtifactKey::Task(slug("x"))),
            reason: "named more than once in the tree".into(),
            object: "c".repeat(40),
            carried: false,
        },
        QuarantinedFile {
            path: "tasks/x.json".into(),
            key: Some(ArtifactKey::Task(slug("x"))),
            reason: "named more than once in the tree".into(),
            object: "d".repeat(40),
            carried: false,
        },
    ];
    rebuild(&index, &snapshot).unwrap();
    assert_eq!(index.ready().unwrap().quarantined, snapshot.quarantined);
}

/// A remote name of any length makes an index name well under the file
/// system's 255 bytes, with room for a build's suffix; two long names
/// that share a prefix stay apart.
#[test]
fn a_long_remote_name_makes_a_short_index_name() {
    let (one, two) = ("r".repeat(300) + "1", "r".repeat(300) + "2");
    let (one, two) = (file_name(&one), file_name(&two));
    assert!(one.len() <= 80, "{one}");
    assert_eq!(
        one,
        "rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr~e953c9d30c9aadf6.sqlite"
    );
    assert_ne!(one, two);
    assert_eq!(file_name("origin"), "origin.sqlite");
}

#[test]
fn index_errors_read_as_text() {
    assert_eq!(
        IndexError::Sqlite("boom".into()).to_string(),
        "the index's SQLite failed: boom"
    );
    assert_eq!(
        IndexError::NeedsRebuild("file is not a database".into()).to_string(),
        "the index needs a rebuild: file is not a database"
    );
}

/// Only the failures a rebuild mends ask for one, each with SQLite's own
/// cause: a file that is not this index, or a value of another type. Any
/// other failure is SQLite's.
#[test]
fn only_the_failures_a_rebuild_mends_ask_for_one() {
    use rusqlite::ffi;
    let failure = |code| rusqlite::Error::SqliteFailure(ffi::Error::new(code), None);
    for code in [
        ffi::SQLITE_NOTADB,
        ffi::SQLITE_CORRUPT,
        ffi::SQLITE_ERROR,
        ffi::SQLITE_CANTOPEN,
    ] {
        let error = IndexError::from(failure(code));
        assert!(
            matches!(&error, IndexError::NeedsRebuild(why) if !why.is_empty()),
            "{code}: {error:?}"
        );
    }
    let mistyped =
        rusqlite::Error::InvalidColumnType(0, "status".into(), rusqlite::types::Type::Integer);
    assert!(matches!(
        IndexError::from(mistyped),
        IndexError::NeedsRebuild(_)
    ));
    for code in [ffi::SQLITE_NOMEM, ffi::SQLITE_INTERRUPT, ffi::SQLITE_MISUSE] {
        assert!(
            matches!(IndexError::from(failure(code)), IndexError::Sqlite(_)),
            "{code}"
        );
    }
    assert!(matches!(
        IndexError::from(failure(ffi::SQLITE_TOOBIG)),
        IndexError::Corrupt(_)
    ));
}

/// A writer that finds no lock file it can use clears only what is not a
/// regular file there: a fresh lock file another writer just made stays.
#[test]
fn clearing_the_lock_place_never_removes_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("index.lock");
    std::fs::write(&lock, b"").unwrap();
    index_file::clear_non_regular(&lock).unwrap();
    assert!(lock.is_file(), "another writer's lock file was removed");
    std::fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), &lock).unwrap();
    index_file::clear_non_regular(&lock).unwrap();
    assert!(std::fs::symlink_metadata(&lock).is_err());
}

/// An object the index holds for a file set aside is an object id, or the
/// index is corrupt.
#[test]
fn a_set_aside_object_that_is_no_object_id_is_corrupt() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    tamper(&index, "UPDATE quarantined SET object = 'zz\u{1b}[2J';");
    let read = index.ready();
    assert!(matches!(read, Err(IndexError::Corrupt(_))), "{read:?}");
}
