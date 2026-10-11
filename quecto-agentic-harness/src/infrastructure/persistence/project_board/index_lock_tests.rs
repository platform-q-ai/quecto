//! The writer lock: bounded, never held across the network, and the build
//! files of live writers are never swept.
use super::*;

/// A history of known ancestry; `remote_head` records whether the index's
/// writer lock was free when it was asked.
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
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)
                .unwrap();
            if file.try_lock().is_err() {
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
